use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{cursors::Cursors, oplog::Op, server::{SharedState, PROTO_VER}};

const SYNC_INTERVAL:   Duration = Duration::from_secs(30);
const HTTP_TIMEOUT:    Duration = Duration::from_secs(10);
const WRITE_TIMEOUT:   Duration = Duration::from_secs(5);
const NOTIFY_TIMEOUT:  Duration = Duration::from_secs(3);

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

    loop {
        match ping_rx.recv_timeout(SYNC_INTERVAL) {
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {
                if pull_all(&state, &cursors, &ops_tx).is_err() { break; }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

// ── pull ──────────────────────────────────────────────────────────────────────

/// Returns Err if the ops channel is closed (app shutting down).
fn pull_all(
    state:   &SharedState,
    cursors: &Mutex<Cursors>,
    ops_tx:  &mpsc::Sender<Vec<Op>>,
) -> Result<(), ()> {
    let peers = state.peers.read().unwrap().all().to_vec();
    crate::clog!("[engine] pull_all — {} peer(s)", peers.len());

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
        let hello_url = format!("http://{ip}:{}/hello", state.http_port);

        #[derive(serde::Deserialize)]
        struct Hello {
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
                Ok(h)  => h,
                Err(e) => {
                    // 200, но тело не в нашем формате — не "обрыв связи", а
                    // именно незнакомый/несовместимый пир. Не трогаем /ops.
                    crate::clog!("[engine] /hello UNPARSEABLE from {} ({}): {e}", peer.device_id, ip);
                    state.sync_status.lock().unwrap().peer_statuses.insert(
                        peer.device_id.clone(),
                        super::PeerStatus { online: false, error: false, revoked: false, incompatible: true },
                    );
                    continue;
                }
            },
            Err(PullError::Revoked) => {
                crate::clog!("[engine] REVOKED by {} ({})", peer.device_id, ip);
                state.sync_status.lock().unwrap().peer_statuses.insert(
                    peer.device_id.clone(),
                    super::PeerStatus { online: false, error: false, revoked: true, incompatible: false },
                );
                continue;
            }
            Err(PullError::Unavailable) => {
                crate::clog!("[engine] /hello FAILED from {} ({})", peer.device_id, ip);
                state.sync_status.lock().unwrap().peer_statuses.insert(
                    peer.device_id.clone(),
                    super::PeerStatus { online: false, error: true, revoked: false, incompatible: false },
                );
                continue;
            }
        };

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
            state.sync_status.lock().unwrap().peer_statuses.insert(
                peer.device_id.clone(),
                super::PeerStatus { online: false, error: false, revoked: false, incompatible: true },
            );
            continue;
        }

        // Совместимы — обновляем имя/тип пира сразу по факту успешного
        // /hello, независимо от того, что вернёт /ops дальше (пустой ответ,
        // 0 новых опов и т.п. больше не мешают освежить это).
        {
            let mut peers = state.peers.write().unwrap();
            let changed = peers.list_mut()
                .find(|p| p.device_id == peer.device_id)
                .map(|p| {
                    let mut changed = false;
                    if p.device_name != hello.device_name {
                        p.device_name = hello.device_name.clone();
                        changed = true;
                    }
                    if p.device_type != hello.device_type {
                        p.device_type = hello.device_type;
                        changed = true;
                    }
                    changed
                })
                .unwrap_or(false);
            if changed { peers.save(); }
        }

        // ── шаг 3: /ops — только теперь, только если /hello подтвердил, что
        // мы понимаем формат этого пира.
        let since = cursors.lock().unwrap().get(&peer.device_id);
        let url   = format!("http://{ip}:{}/1/ops?since={since}&token={}", state.http_port, peer.token);
        crate::clog!("[engine] pulling from {} url={}", peer.device_id, url);

        match http_get(&url, HTTP_TIMEOUT) {
            Ok(body) => {
                state.peers.write().unwrap().update_last_synced(&peer.device_id, now_ts);
                state.sync_status.lock().unwrap().peer_statuses.insert(
                    peer.device_id.clone(),
                    super::PeerStatus { online: true, error: false, revoked: false, incompatible: false },
                );

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
                state.sync_status.lock().unwrap().peer_statuses.insert(
                    peer.device_id.clone(),
                    super::PeerStatus { online: false, error: false, revoked: true, incompatible: false },
                );
            }
            Err(PullError::Unavailable) => {
                crate::clog!("[engine] pull FAILED from {} ({})", peer.device_id, ip);
                state.sync_status.lock().unwrap().peer_statuses.insert(
                    peer.device_id.clone(),
                    super::PeerStatus { online: false, error: true, revoked: false, incompatible: false },
                );
            }
        }
    }
    Ok(())
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
        let url = format!("http://{ip}:{}/1/ping_sync?token={}", state.http_port, peer.token);
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