use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{cursors::Cursors, oplog::Op, server::{SharedState, PROTO_VER}};

const SYNC_INTERVAL:   Duration = Duration::from_secs(30);
const HTTP_TIMEOUT:    Duration = Duration::from_secs(10);
const WRITE_TIMEOUT:   Duration = Duration::from_secs(5);
const NOTIFY_TIMEOUT:  Duration = Duration::from_secs(3);

/// Сколько пир должен быть непрерывно недоступен, чтобы мы начали его искать
/// через discovery (короткие обрывы/сон не должны сразу слать broadcast).
const RECOVERY_AFTER:    Duration = Duration::from_secs(60);
/// Не чаще одного поиска за это время (общий, на всех потерянных пиров сразу).
const RECOVERY_COOLDOWN: Duration = Duration::from_secs(120);
/// Сколько ждём PONG-ов после broadcast-PING.
const RECOVERY_WAIT:     Duration = Duration::from_millis(2000);

// ── pull error ────────────────────────────────────────────────────────────────

enum PullError {
    /// Peer returned HTTP 403 — they revoked our token.
    Revoked,
    /// Any other connectivity or protocol failure.
    Unavailable,
}

// ── engine thread ─────────────────────────────────────────────────────────────

pub fn start(
    state:   Arc<SharedState>,
    cursors: Arc<Mutex<Cursors>>,
    ops_tx:  mpsc::Sender<Vec<Op>>,
    ping_rx: mpsc::Receiver<()>,
    dir:     PathBuf,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("cue-sync-engine".into())
        .spawn(move || run(state, cursors, ops_tx, ping_rx, dir))
        .expect("spawn sync engine thread")
}

/// Notifier thread: on each signal, POSTs /ping_sync to every known peer
/// so they pull from us immediately, then wakes our own engine to pull from them.
pub fn start_notifier(
    state:     Arc<SharedState>,
    notify_rx: mpsc::Receiver<()>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("cue-sync-notifier".into())
        .spawn(move || {
            while notify_rx.recv().is_ok() {
                notify_peers(&state);
                // Also wake our own engine so we pull any concurrent ops from peers.
                state.ping_tx.try_send(()).ok();
            }
        })
        .expect("spawn sync notifier thread")
}

// ── recovery (активный поиск потерявшихся пиров) ──────────────────────────────

/// Состояние поиска пиров, потерявшихся по адресу. Живёт в engine-потоке.
#[derive(Default)]
struct Recovery {
    /// device_id → с какого момента пир НЕПРЕРЫВНО недоступен по адресу.
    failing:   HashMap<String, Instant>,
    last_scan: Option<Instant>,
}

impl Recovery {
    fn note_failure(&mut self, id: &str) {
        self.failing.entry(id.to_owned()).or_insert_with(Instant::now);
    }
    fn note_reachable(&mut self, id: &str) {
        self.failing.remove(id);
    }
}

// ── main loop ─────────────────────────────────────────────────────────────────

fn run(
    state:   Arc<SharedState>,
    cursors: Arc<Mutex<Cursors>>,
    ops_tx:  mpsc::Sender<Vec<Op>>,
    ping_rx: mpsc::Receiver<()>,
    dir:     PathBuf,
) {
    // First thing, before the normal pull loop: if the main thread deferred
    // loading ops.ndjson (non-empty file on startup — see SyncHandle::init),
    // do that read here instead of blocking the UI thread with it. No-op if
    // the file was already loaded synchronously (empty-file path).
    super::ensure_oplog_ready(&dir, &state.oplog_state);

    let mut recovery = Recovery::default();
    // Первый опрос — сразу, а не через SYNC_INTERVAL: статусы пиров из файла
    // (см. SyncHandle::init) обновятся за секунды, и оп-ы подтянутся быстрее.
    if pull_all(&state, &cursors, &ops_tx, &mut recovery).is_err() { return; }
    loop {
        match ping_rx.recv_timeout(SYNC_INTERVAL) {
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {
                if pull_all(&state, &cursors, &ops_tx, &mut recovery).is_err() { break; }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

// ── pull ──────────────────────────────────────────────────────────────────────

/// Меняет статус пира в памяти. `None` у revoked/incompatible = оставить
/// прежнее значение (нет ответа — ничего не узнали). Статусы при старте
/// заполняются из trusted_peers.json (см. SyncHandle::init).
fn set_status(
    state: &SharedState, id: &str, online: bool, error: bool,
    revoked: Option<bool>, incompatible: Option<bool>,
) {
    let mut g = state.sync_status.lock().unwrap();
    let s = g.peer_statuses.entry(id.to_owned()).or_default();
    s.online = online;
    s.error  = error;
    if let Some(r) = revoked      { s.revoked = r; }
    if let Some(i) = incompatible { s.incompatible = i; }
}

/// Пир ответил, но нашей версии протокола у него нет (или ответ не разобрался):
/// incompatible = true, revoked проверить не смогли — сбрасываем старый флаг,
/// и контакт засчитываем.
fn mark_incompatible(state: &SharedState, id: &str, now_ts: u64) {
    state.peers.write().unwrap().apply_poll(id, super::peers::PollUpdate {
        contact_ts: Some(now_ts), revoked: Some(false), incompatible: Some(true),
    });
    set_status(state, id, false, false, Some(false), Some(true));
}

/// 403: пир нас отвязал. Контакт засчитываем (пир ответил), incompatible = false
/// (/hello перед этим прошёл).
fn mark_revoked(state: &SharedState, id: &str, now_ts: u64) {
    state.peers.write().unwrap().apply_poll(id, super::peers::PollUpdate {
        contact_ts: Some(now_ts), revoked: Some(true), incompatible: Some(false),
    });
    set_status(state, id, false, false, Some(true), Some(false));
}

/// Returns Err if the ops channel is closed (app shutting down).
fn pull_all(
    state:   &SharedState,
    cursors: &Mutex<Cursors>,
    ops_tx:  &mpsc::Sender<Vec<Op>>,
    rec:     &mut Recovery,
) -> Result<(), ()> {
    let peers = state.peers.read().unwrap().all().to_vec();
    crate::clog!("[engine] pull_all — {} peer(s)", peers.len());
    // Забываем пиров, которых уже отвязали.
    rec.failing.retain(|id, _| peers.iter().any(|p| &p.device_id == id));

    // Хоть один осмысленный ответ за цикл (нужное устройство ответило на
    // /hello) — значит, в конце пишем trusted_peers.json ОДИН раз. Если не
    // ответил никто — диск не трогаем (иначе была бы запись каждые 30 с).
    let mut answered = false;

    for peer in peers {
        let Some(ip) = peer.ip_hint else {
            crate::clog!("[engine] skipping {} — no ip_hint", peer.device_id);
            continue;
        };
        let now_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // ── шаг 1: /hello — ВСЕГДА первым, безусловно. Это корень доверия:
        // до него мы не знаем, можно ли вообще понимать ответ этого пира на
        // /1/ops, так что сам /hello не версионирован в пути (см. server.rs).
        // Токен сюда НЕ передаётся и 403 тут не бывает: токен проверяет
        // только /1/ops (см. server::authed).
        let hello_url = format!("http://{ip}:{}/hello", peer.port);

        #[derive(serde::Deserialize)]
        struct Hello {
            #[serde(default)]
            device_id:   String,
            #[serde(default)]
            proto_ver:   u32,
            #[serde(default)]
            proto_vers:  Vec<u32>,
            device_name: String,
            #[serde(default)]
            device_type: super::DeviceType,
        }

        let hello = match http_get(&hello_url, HTTP_TIMEOUT) {
            Ok(body) => match serde_json::from_str::<Hello>(&body) {
                // По этому адресу отвечает ДРУГОЕ устройство (например, DHCP
                // отдал старый IP пира кому-то ещё, или запись указывает на нас
                // самих). Не наш пир: имя не трогаем, токен не шлём, /ops не
                // зовём. Считаем "недоступен" — это не отзыв доверия, флаги
                // revoked/incompatible и время контакта остаются прежними.
                Ok(h) if h.device_id != peer.device_id => {
                    crate::clog!(
                        "[engine] {ip}:{} answers as {:?}, expected {} — not our peer",
                        peer.port, h.device_id, peer.device_id
                    );
                    rec.note_failure(&peer.device_id);
                    set_status(state, &peer.device_id, false, true, None, None);
                    continue;
                }
                Ok(h)  => h,
                Err(e) => {
                    // 200, но тело не в нашем формате — не "обрыв связи", а
                    // именно незнакомый/несовместимый пир. Не трогаем /ops.
                    crate::clog!("[engine] /hello UNPARSEABLE from {} ({}): {e}", peer.device_id, ip);
                    rec.note_reachable(&peer.device_id);
                    answered = true;
                    mark_incompatible(state, &peer.device_id, now_ts);
                    continue;
                }
            },
            Err(PullError::Revoked) => {
                // Сервер на /hello 403 не отдаёт (токена там нет), ветка
                // оставлена на случай изменения сервера: ведём себя как при
                // 403 на /ops.
                crate::clog!("[engine] REVOKED by {} ({})", peer.device_id, ip);
                rec.note_reachable(&peer.device_id);
                answered = true;
                mark_revoked(state, &peer.device_id, now_ts);
                continue;
            }
            Err(PullError::Unavailable) => {
                crate::clog!("[engine] /hello FAILED from {} ({})", peer.device_id, ip);
                rec.note_failure(&peer.device_id);
                set_status(state, &peer.device_id, false, true, None, None);
                continue;
            }
        };

        // Нужное устройство по этому адресу ответило (даже если окажется
        // несовместимым) — искать его через discovery не нужно.
        rec.note_reachable(&peer.device_id);
        answered = true;

        // ── шаг 2: проверка совместимости. Направление проверки важно — не
        // "пересечение списков", а "я нахожу СЕБЯ у пира": единственное, что
        // имеет значение — пойму ли я формат, который пир мне отдаст на
        // /1/ops. Своего списка версий не нужно, достаточно одной константы.
        let peer_understands = hello.proto_ver == PROTO_VER
            || hello.proto_vers.contains(&PROTO_VER);

        if !peer_understands {
            crate::clog!(
                "[engine] INCOMPATIBLE {} ({}) — proto_ver={} proto_vers={:?}, we need {}",
                peer.device_id, ip, hello.proto_ver, hello.proto_vers, PROTO_VER
            );
            mark_incompatible(state, &peer.device_id, now_ts);
            continue;
        }

        // Совместимы — обновляем имя/тип пира сразу по факту успешного
        // /hello, независимо от того, что вернёт /ops дальше (пустой ответ,
        // 0 новых опов и т.п. больше не мешают освежить это). И снимаем
        // флаг incompatible. На диск ничего не пишем — одна запись в конце
        // цикла (answered уже true).
        {
            let mut peers = state.peers.write().unwrap();
            if let Some(p) = peers.list_mut().find(|p| p.device_id == peer.device_id) {
                if p.device_name != hello.device_name { p.device_name = hello.device_name.clone(); }
                if p.device_type != hello.device_type { p.device_type = hello.device_type; }
            }
            peers.apply_poll(&peer.device_id, super::peers::PollUpdate {
                incompatible: Some(false), ..Default::default()
            });
        }

        // ── шаг 3: /ops — только теперь, только если /hello подтвердил, что
        // мы понимаем формат этого пира.
        let since = cursors.lock().unwrap().get(&peer.device_id);
        // port= — НАШ порт: пир по нему узнаёт, где нас искать (см. server::learn_peer_addr).
        let url   = format!("http://{ip}:{}/1/ops?since={since}&token={}&port={}", peer.port, peer.token, state.port());
        crate::clog!("[engine] pulling from {} url={}", peer.device_id, url);

        match http_get(&url, HTTP_TIMEOUT) {
            Ok(body) => {
                state.peers.write().unwrap().apply_poll(&peer.device_id, super::peers::PollUpdate {
                    contact_ts: Some(now_ts), revoked: Some(false), incompatible: Some(false),
                });
                set_status(state, &peer.device_id, true, false, Some(false), Some(false));

                if body.trim().is_empty() {
                    crate::clog!("[engine] empty response from {}", peer.device_id);
                    continue;
                }

                let ops: Vec<Op> = body.lines()
                    .filter_map(|l| match serde_json::from_str::<Op>(l) {
                        Ok(op) => Some(op),
                        Err(e) => {
                            // ВРЕМЕННЫЙ дебаг — .ok() выше молча ел ошибку,
                            // из-за чего "got 0 ops" ничего не объясняло.
                            crate::clog!("[engine] PARSE FAIL: {e} | line={l}");
                            None
                        }
                    })
                    .collect();
                crate::clog!("[engine] got {} ops from {}", ops.len(), peer.device_id);
                if ops.is_empty() { continue; }

                let max_seq = ops.iter().map(|op| op.seq).max().unwrap_or(0);
                cursors.lock().unwrap().set(&peer.device_id, max_seq);
                crate::clog!("[engine] cursor updated to {max_seq}, sending to main thread");

                if ops_tx.send(ops).is_err() {
                    crate::clog!("[engine] ops channel closed, shutting down");
                    return Err(());
                }
                // Wake egui immediately so ops are applied without user prodding.
                state.egui_ctx.request_repaint();
            }
            Err(PullError::Revoked) => {
                crate::clog!("[engine] REVOKED by {} ({})", peer.device_id, ip);
                mark_revoked(state, &peer.device_id, now_ts);
            }
            Err(PullError::Unavailable) => {
                // /hello отвечал, а /ops оборвался: ничего нового не узнали —
                // revoked и время контакта не трогаем.
                crate::clog!("[engine] pull FAILED from {} ({})", peer.device_id, ip);
                set_status(state, &peer.device_id, false, true, None, Some(false));
            }
        }
    }

    // Один сохранённый файл за цикл — и только если кто-то ответил.
    if answered {
        state.peers.read().unwrap().save();
    }
    // Состояния (онлайн/оффлайн/отвязано/…) могли поменяться — перерисовать.
    state.egui_ctx.request_repaint();

    try_recover(state, rec);
    Ok(())
}

/// Активный поиск пиров, недоступных по сохранённому адресу дольше
/// RECOVERY_AFTER: broadcast-PING, затем сверка найденных по device_id с
/// потерянными. Новый адрес принимаем только после /hello на кандидате, где
/// device_id обязан совпасть. Нужен, когда адрес сменили ОБЕ стороны (или пир
/// долго был выключен) — если менялась одна, адрес и так обновится пассивно,
/// см. server::learn_peer_addr.
fn try_recover(state: &SharedState, rec: &mut Recovery) {
    let now = Instant::now();
    let lost: Vec<String> = rec.failing.iter()
        .filter(|(_, since)| now.duration_since(**since) >= RECOVERY_AFTER)
        .map(|(id, _)| id.clone())
        .collect();
    if lost.is_empty() { return; }
    if rec.last_scan.is_some_and(|t| now.duration_since(t) < RECOVERY_COOLDOWN) { return; }
    rec.last_scan = Some(now);

    crate::clog!("[engine] {} peer(s) unreachable for {:?}+ — searching via discovery", lost.len(), RECOVERY_AFTER);
    state.discovered.send_ping();
    std::thread::sleep(RECOVERY_WAIT);
    let found = super::discovery::current(&state.discovered.discovered);

    for id in lost {
        let Some(d) = found.iter().find(|d| d.device_id == id) else { continue; };
        let same_addr = state.peers.read().unwrap().find_by_id(&id)
            .is_some_and(|p| p.ip_hint.as_deref() == Some(d.ip.as_str()) && p.port == d.port);
        if same_addr { continue; }   // адрес тот же — значит дело не в адресе

        if !verify_peer_at(&d.ip, d.port, &id) {
            crate::clog!("[engine] discovery candidate {}:{} for {id} failed /hello verification", d.ip, d.port);
            continue;
        }
        if state.peers.write().unwrap().update_addr(&id, &d.ip, d.port) {
            crate::clog!("[engine] peer {id} found via discovery → {}:{}", d.ip, d.port);
            rec.note_reachable(&id);
            state.ping_tx.try_send(()).ok();   // сразу пробуем по новому адресу
        }
    }
}

/// Отвечает ли по этому адресу именно устройство `expected_id`.
fn verify_peer_at(ip: &str, port: u16, expected_id: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Id { #[serde(default)] device_id: String }
    http_get(&format!("http://{ip}:{port}/hello"), NOTIFY_TIMEOUT).ok()
        .and_then(|body| serde_json::from_str::<Id>(&body).ok())
        .is_some_and(|h| h.device_id == expected_id)
}

// ── notify ────────────────────────────────────────────────────────────────────

/// Send POST /ping_sync to every peer so their engine wakes and pulls from us.
fn notify_peers(state: &SharedState) {
    let peers = state.peers.read().unwrap().all().to_vec();
    crate::clog!("[notifier] notify_peers — {} peer(s)", peers.len());
    for peer in peers {
        let Some(ip) = peer.ip_hint else {
            crate::clog!("[notifier] skipping {} — no ip_hint", peer.device_id);
            continue;
        };
        let url = format!("http://{ip}:{}/1/ping_sync?token={}&port={}", peer.port, peer.token, state.port());
        match http_post(&url, NOTIFY_TIMEOUT) {
            Ok(_)  => crate::clog!("[notifier] ping_sync → {} ok", peer.device_id),
            Err(e) => crate::clog!("[notifier] ping_sync → {} FAILED: {e}", peer.device_id),
        }
    }
}

// ── minimal HTTP/1.0 ──────────────────────────────────────────────────────────

fn http_get(url: &str, timeout: Duration) -> Result<String, PullError> {
    let (host_port, path_query) = parse_url(url).map_err(|_| PullError::Unavailable)?;
    let mut stream = connect(host_port, timeout).map_err(|_| PullError::Unavailable)?;
    write!(stream, "GET {path_query} HTTP/1.0\r\nHost: {host_port}\r\nConnection: close\r\n\r\n")
        .map_err(|_| PullError::Unavailable)?;

    let mut raw = String::new();
    stream.read_to_string(&mut raw).map_err(|_| PullError::Unavailable)?;

    let status_line = raw.lines().next().unwrap_or("");
    crate::clog!("[engine/http] GET {path_query} → {status_line}");

    if status_line.contains(" 403 ") || status_line.ends_with(" 403") {
        return Err(PullError::Revoked);
    }
    if !status_line.contains(" 200 ") && !status_line.ends_with(" 200") {
        return Err(PullError::Unavailable);
    }
    Ok(raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("").to_owned())
}

fn http_post(url: &str, timeout: Duration) -> std::io::Result<()> {
    let (host_port, path_query) = parse_url(url)?;
    let mut stream = connect(host_port, timeout)?;
    write!(stream,
        "POST {path_query} HTTP/1.0\r\nHost: {host_port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )?;
    // Read just enough to confirm delivery; ignore errors on read.
    let mut buf = [0u8; 64];
    let _ = stream.read(&mut buf);
    Ok(())
}

fn parse_url(url: &str) -> std::io::Result<(&str, String)> {
    let rest = url.strip_prefix("http://")
        .ok_or_else(|| io_err("url must start with http://"))?;
    let (host_port, tail) = rest.split_once('/').unwrap_or((rest, ""));
    Ok((host_port, format!("/{tail}")))
}

fn connect(host_port: &str, timeout: Duration) -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect(host_port)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    Ok(stream)
}

fn io_err(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, msg.to_owned())
}

#[cfg(test)]
mod tests {
    //! Поведение опроса пира против настоящего TCP-сервера (локального):
    //! что меняется в статусе и в trusted_peers.json при каждом исходе.
    use super::*;
    use super::super::peers::{PeerEntry, Peers};
    use super::super::{OplogState, SyncStatus};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::RwLock;

    #[derive(Clone, Copy, PartialEq)]
    enum Mode { Ok, OpsForbidden, Incompatible, OtherDevice, OpsDead, Down }
    // Down: сервер не участвует — движку подставляется порт, где никто не слушает.

    fn start_fake(peer_id: &'static str) -> (u16, Arc<Mutex<Mode>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let mode = Arc::new(Mutex::new(Mode::Ok));
        let m = Arc::clone(&mode);
        std::thread::spawn(move || {
            for stream in l.incoming() {
                let Ok(mut s) = stream else { continue };
                let mode = *m.lock().unwrap();
                // Клиент пишет запрос несколькими write!-кусками — читаем до конца
                // заголовка, а не один раз (иначе иногда видим только "GET ").
                let mut req = String::new();
                let mut buf = [0u8; 512];
                while !req.contains("\r\n\r\n") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.push_str(&String::from_utf8_lossy(&buf[..n])),
                    }
                }
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                let resp: String = if path.starts_with("/hello") {
                    let (id, ver) = match mode {
                        Mode::OtherDevice  => ("someone-else", 1),
                        Mode::Incompatible => (peer_id, 999),
                        _                  => (peer_id, 1),
                    };
                    let body = format!(
                        r#"{{"proto_ver":{ver},"device_id":"{id}","device_name":"Peer","device_type":"desktop"}}"#);
                    format!("HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}", body.len(), body)
                } else if path.starts_with("/1/ops") {
                    match mode {
                        Mode::OpsForbidden => "HTTP/1.0 403 Forbidden\r\nContent-Length: 0\r\n\r\n".into(),
                        Mode::OpsDead      => continue, // закрыть без ответа
                        _                  => "HTTP/1.0 200 OK\r\nContent-Length: 0\r\n\r\n".into(),
                    }
                } else {
                    "HTTP/1.0 404 Not Found\r\n\r\n".into()
                };
                let _ = s.write_all(resp.as_bytes());
            }
        });
        (port, mode)
    }

    fn make_state(dir: &Path, peer: PeerEntry) -> SharedState {
        let (ping_tx, _rx) = mpsc::sync_channel(1);
        SharedState {
            device_id:          "me".into(),
            device_name:        Arc::new(RwLock::new("me".into())),
            peers:              RwLock::new(Peers::for_test(vec![peer], dir)),
            oplog_path:         dir.join("ops.ndjson"),
            oplog_state:        Mutex::new(OplogState::Loading(Vec::new())),
            pending_pairings:   Mutex::new(Vec::new()),
            pending_outgoing:   Mutex::new(HashMap::new()),
            sync_status:        Mutex::new(SyncStatus::default()),
            discovered:         super::super::discovery::Discovery::for_test(),
            ping_tx,
            http_port:          Arc::new(std::sync::atomic::AtomicU16::new(1)),
            server_handle:      Mutex::new(None),
            server_bind_failed: std::sync::atomic::AtomicBool::new(false),
            egui_ctx:           eframe::egui::Context::default(),
            viewing_sync_panel: std::sync::atomic::AtomicBool::new(false),
        }
    }

    struct Rig { state: SharedState, dir: PathBuf, mode: Arc<Mutex<Mode>>,
                 port: u16, dead_port: u16,
                 cursors: Mutex<Cursors>, rec: Recovery, tx: mpsc::Sender<Vec<Op>> }

    impl Rig {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("cue_engine_test_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let (port, mode) = start_fake("peer1");
            let peer = PeerEntry {
                device_id: "peer1".into(), device_name: "Peer".into(), token: "tok".into(),
                ip_hint: Some("127.0.0.1".into()), port, last_synced_at: None,
                revoked: false, incompatible: false, device_type: Default::default(),
            };
            let (tx, _rx) = mpsc::channel();
            // порт, на котором точно никто не слушает (занять и сразу отпустить)
            let dead_port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            Rig { state: make_state(&dir, peer), cursors: Mutex::new(Cursors::load(&dir)),
                  dir, mode, port, dead_port, rec: Recovery::default(), tx }
        }
        /// Один цикл опроса в заданном режиме пира. Перед ним стираем файл и
        /// ставим метку времени = 1, чтобы увидеть, трогал ли цикл файл/время.
        fn cycle(&mut self, mode: Mode) {
            *self.mode.lock().unwrap() = mode;
            let port = if mode == Mode::Down { self.dead_port } else { self.port };
            self.state.peers.write().unwrap().list_mut().for_each(|p| p.port = port);
            let _ = std::fs::remove_file(self.dir.join("trusted_peers.json"));
            self.state.peers.write().unwrap().list_mut().for_each(|p| {
                if p.last_synced_at.is_some() { p.last_synced_at = Some(1); }
            });
            pull_all(&self.state, &self.cursors, &self.tx, &mut self.rec).unwrap();
        }
        fn entry(&self) -> PeerEntry { self.state.peers.read().unwrap().find_by_id("peer1").unwrap().clone() }
        fn status(&self) -> super::super::PeerStatus {
            self.state.sync_status.lock().unwrap().peer_statuses.get("peer1").cloned().unwrap_or_default()
        }
        fn file_written(&self) -> bool { self.dir.join("trusted_peers.json").exists() }
        fn file_entry(&self) -> PeerEntry {
            let s = std::fs::read_to_string(self.dir.join("trusted_peers.json")).unwrap();
            serde_json::from_str::<Vec<PeerEntry>>(&s).unwrap().remove(0)
        }
    }

    #[test]
    fn poll_outcomes_update_flags_time_and_file_as_agreed() {
        let mut r = Rig::new("outcomes");

        // 1. пир недоступен: ничего не узнали — флаги/время/файл не трогаем
        r.cycle(Mode::Down);
        assert!(!r.file_written(), "никто не ответил — файл писать нельзя");
        assert_eq!(r.entry().last_synced_at, None);
        let s = r.status();
        assert!(!s.online && s.error && !s.revoked && !s.incompatible);

        // 2. всё хорошо: онлайн, время обновлено, ОДНА запись файла
        r.cycle(Mode::Ok);
        assert!(r.file_written());
        assert!(r.entry().last_synced_at.unwrap() > 1);
        assert!(r.status().online);
        assert!(!r.file_entry().revoked && !r.file_entry().incompatible);

        // 3. 403 на /ops: отвязан, контакт засчитан (время обновилось), в файле
        r.cycle(Mode::OpsForbidden);
        assert!(r.entry().revoked && !r.entry().incompatible);
        assert!(r.entry().last_synced_at.unwrap() > 1, "403 — это тоже ответ пира");
        assert!(r.file_written() && r.file_entry().revoked);
        let s = r.status();
        assert!(s.revoked && !s.online);

        // 4. пир пропал: revoked НЕ сбрасывается, время не меняется, файл не пишется
        r.cycle(Mode::Down);
        assert!(r.entry().revoked, "нет ответа — последнее известное состояние остаётся");
        assert_eq!(r.entry().last_synced_at, Some(1));
        assert!(!r.file_written());
        assert!(r.status().revoked && r.status().error);

        // 5. несовместимая версия: incompatible=true, revoked сброшен, время обновлено
        r.cycle(Mode::Incompatible);
        assert!(r.entry().incompatible && !r.entry().revoked);
        assert!(r.entry().last_synced_at.unwrap() > 1);
        assert!(r.file_written() && r.file_entry().incompatible);
        assert!(r.status().incompatible && !r.status().revoked);

        // 6. по адресу отвечает ДРУГОЕ устройство: всё остаётся, файл не пишется
        r.cycle(Mode::OtherDevice);
        assert!(r.entry().incompatible, "чужой ответ ничего не говорит про нашего пира");
        assert_eq!(r.entry().last_synced_at, Some(1));
        assert!(!r.file_written());

        // 7. /hello ок, а /ops оборвался: incompatible снят (hello совместим),
        //    время и revoked не тронуты; файл пишется (hello ответил)
        r.cycle(Mode::OpsDead);
        assert!(!r.entry().incompatible);
        assert_eq!(r.entry().last_synced_at, Some(1), "осмысленного ответа на /ops не было");
        assert!(r.file_written() && !r.file_entry().incompatible);
        assert!(r.status().error && !r.status().online);

        // 8. снова всё хорошо — всё чисто
        r.cycle(Mode::Ok);
        let e = r.entry();
        assert!(!e.revoked && !e.incompatible && e.last_synced_at.unwrap() > 1);
        assert!(r.status().online);

        let _ = std::fs::remove_dir_all(&r.dir);
    }
}
