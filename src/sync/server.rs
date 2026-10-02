use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};

use eframe::egui;
use serde::{Deserialize, Serialize};
use tiny_http::{Method, Request, Response, Server, StatusCode};

use super::{oplog::Op, peers::Peers, DeviceType, OplogState};

pub const DEFAULT_PORT: u16 = 24684;
pub const PROTO_VER: u32 = 1;

/// Для `#[serde(default = "...")]` полей "порт HTTP-сервера пира": у записей и
/// сообщений от клиентов, где поля ещё не было, подставляем дефолтный порт.
pub fn default_port() -> u16 { DEFAULT_PORT }

// ── shared state (server + engine both hold an Arc<SharedState>) ─────────────

pub struct SharedState {
    pub device_id:        String,
    /// Общий Arc с discovery: переименование в настройках сразу видно в
    /// PING/PONG, а не только после перезапуска.
    pub device_name:      Arc<RwLock<String>>,
    pub peers:            RwLock<Peers>,
    pub oplog_path:       PathBuf,
    /// Loading/Ready state of the local oplog — see `OplogState` in `sync/mod.rs`.
    /// `serve_ops` below deliberately does NOT go through this: it reads
    /// `oplog_path` straight off disk, so answering peer pull requests never
    /// has to wait on this regardless of Loading/Ready.
    pub oplog_state:      Mutex<OplogState>,
    pub pending_pairings: Mutex<Vec<PairingRequest>>,
    /// Кому МЫ сами нажали "Подключить" и ещё не получили ответа
    /// (device_id → когда). Только в памяти, на диск не пишется. Нужно, чтобы
    /// распознать взаимный запрос (оба нажали друг на друга) — см. request_sync.
    pub pending_outgoing: Mutex<HashMap<String, Instant>>,
    pub sync_status:      Mutex<super::SyncStatus>,
    /// UDP discovery handle — exposes discovered list and ping trigger.
    pub discovered:       super::discovery::Discovery,
    /// Bounded-1 channel: server taps engine on POST /ping_sync.
    pub ping_tx:          mpsc::SyncSender<()>,
    /// НАШ порт HTTP-сервера. Сообщаем его пирам (тела request_sync/
    /// accept_sync, параметр `port=` наших запросов, discovery). Порт КАЖДОГО
    /// пира хранится отдельно — в PeerEntry.port, а не здесь.
    pub http_port:        Arc<std::sync::atomic::AtomicU16>,
    /// Работающий сейчас HTTP-сервер — нужен, чтобы при смене порта на ходу
    /// вызвать `unblock()` и заставить серверный поток перебиндиться.
    pub server_handle:    Mutex<Option<Arc<Server>>>,
    /// true, пока не удаётся занять текущий порт (занят другим приложением).
    /// Ставится вместе с системным уведомлением, читается UI.
    pub server_bind_failed: std::sync::atomic::AtomicBool,
    /// Used by engine to wake egui immediately after delivering ops.
    pub egui_ctx:         egui::Context,
    /// Пишется UI каждый кадр: сейчас открыта именно вкладка "Синхронизация"
    /// настроек? Если нет на момент входящего /1/request_sync — шлём
    /// системное уведомление (см. request_sync ниже), чтобы юзер не
    /// пропустил запрос, зайдя случайно в другую вкладку/экран.
    pub viewing_sync_panel: std::sync::atomic::AtomicBool,
}

impl SharedState {
    /// Текущий порт нашего HTTP-сервера.
    pub fn port(&self) -> u16 {
        self.http_port.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Сменить порт на ходу: новое значение подхватят discovery и engine, а
    /// серверный поток перебиндится (если сейчас как раз ждёт освобождения
    /// порта — увидит смену на ближайшей итерации сам).
    pub fn set_port(&self, port: u16) {
        self.http_port.store(port, std::sync::atomic::Ordering::SeqCst);
        // Красная пометка относилась к СТАРОМУ порту — сбрасываем сразу, а не
        // ждём, пока серверный поток начнёт новую попытку bind.
        self.server_bind_failed.store(false, std::sync::atomic::Ordering::SeqCst);
        if let Some(server) = self.server_handle.lock().unwrap().clone() {
            server.unblock();
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    pub device_id:   String,
    pub device_name: String,
    pub from_ip:     String,
    /// Порт HTTP-сервера отправителя заявки (IP берём с соединения, порт так
    /// узнать нельзя — он приходит в теле request_sync).
    #[serde(default = "default_port")]
    pub port:        u16,
    #[serde(default)]
    pub device_type: DeviceType,
    /// Только в памяти. Заявка взаимная и мы в ней "проигравшая" сторона
    /// (см. request_sync): до этого момента ждём accept_sync от второй
    /// стороны и не показываем баннер. Если не дождались — баннер появится
    /// как обычная заявка (запасной путь, ручное принятие).
    #[serde(skip)]
    pub mutual_wait_until: Option<Instant>,
}

impl PairingRequest {
    /// true, пока действует окно ожидания взаимной заявки — заявку нельзя
    /// ни показывать баннером, ни принимать вручную.
    pub fn waiting(&self) -> bool {
        self.mutual_wait_until.is_some_and(|t| Instant::now() < t)
    }
}

/// Сколько помним, что мы сами отправили заявку (для распознавания взаимности).
const OUTGOING_TTL: Duration = Duration::from_secs(600);
/// Сколько "проигравшая" сторона взаимной заявки ждёт accept_sync, прежде чем
/// разрешить ручное принятие.
const MUTUAL_WAIT: Duration = Duration::from_secs(15);

// ── start ─────────────────────────────────────────────────────────────────────

const BIND_RETRY_MS:    u64 = 300;
const YIELD_RESEND_MS:  u64 = 3000;
const YIELD_TIMEOUT_MS: u64 = 250;

static PORT_FAIL_COUNT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
const PORT_FAIL_NOTIFY_THRESHOLD: u8 = 5;

/// Серверный поток. Живёт в цикле "занять порт → обслуживать → (порт сменили)
/// → занять новый". Желаемый порт читает из `state` каждый раз заново, так что
/// смена порта на ходу — это просто `set_port()` + `unblock()`.
pub fn start(state: Arc<SharedState>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("cue-sync-server".into())
        .spawn(move || loop {
            let port = state.port();
            // None — пока ждали освобождения порта, юзер выбрал другой.
            let Some(server) = bind_with_retry(port, &state) else { continue; };
            let server = Arc::new(server);
            *state.server_handle.lock().unwrap() = Some(Arc::clone(&server));
            // Порт могли сменить между bind и записью handle — тогда unblock()
            // из set_port() не нашёл сервера и промахнулся. Досылаем сами
            // (сообщение unblock ждёт в очереди, цикл ниже сразу завершится).
            if state.port() != port { server.unblock(); }
            // Сразу будим engine: его запросы несут наш актуальный port=, так
            // пиры узнают новый адрес за один проход, не дожидаясь 30с цикла.
            state.ping_tx.try_send(()).ok();

            for req in server.incoming_requests() {
                handle(req, &state);
            }

            // Сюда попадаем после unblock() (смена порта) либо если сервер
            // сам отвалился — в обоих случаях просто занимаем порт заново.
            *state.server_handle.lock().unwrap() = None;
            drop(server);   // отпускаем порт ДО следующего bind
            crate::clog!("[sync/server] stopped listening on port {port}");
            std::thread::sleep(Duration::from_millis(BIND_RETRY_MS));
        })
        .expect("spawn sync server thread")
}

/// Занимает порт, ретраится пока не получится. Возвращает None, если за время
/// ожидания желаемый порт сменился — вызывающий начнёт заново уже с новым.
fn bind_with_retry(port: u16, state: &SharedState) -> Option<Server> {
    // Счётчики неудач — на КАЖДЫЙ порт заново: иначе после смены порта
    // "занят"-уведомление не сработало бы повторно (счётчик уже за порогом),
    // а "порт освобождён" могло бы прийти по итогам старого порта.
    PORT_FAIL_COUNT.store(0, std::sync::atomic::Ordering::SeqCst);
    state.server_bind_failed.store(false, std::sync::atomic::Ordering::SeqCst);

    let mut last_yield_sent = Instant::now() - Duration::from_millis(YIELD_RESEND_MS);
    loop {
        if state.port() != port { return None; }
        match crate::exclusive_bind::bind_exclusive(port) {
            Ok(listener) => match Server::from_listener(listener, None) {
                Ok(server) => {
                    crate::clog!("[sync/server] bind SUCCEEDED on port {port}");
                    let prev_count = PORT_FAIL_COUNT.swap(0, std::sync::atomic::Ordering::SeqCst);
                    if state.server_bind_failed.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        state.egui_ctx.request_repaint();
                    }
                    if prev_count >= PORT_FAIL_NOTIFY_THRESHOLD {
                        crate::notify::send_no_icon(
                            &format!("Порт {port} освобождён"),
                            "Синхронизация восстановлена",
                        );
                    }
                    return Some(server);
                }
                Err(e) => crate::clog!("[sync/server] from_listener failed: {e:?}"),
            },
            Err(e) => crate::clog!("[sync/server] bind_exclusive failed: {e:?}"),
        }
        let fails = PORT_FAIL_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        if fails == PORT_FAIL_NOTIFY_THRESHOLD {
            state.server_bind_failed.store(true, std::sync::atomic::Ordering::SeqCst);
            state.egui_ctx.request_repaint();
            crate::notify::send_no_icon(
                &format!("Порт {port} занят другим приложением"),
                "Синхронизация недоступна",
            );
        }
        if last_yield_sent.elapsed() >= Duration::from_millis(YIELD_RESEND_MS) {
            send_yield(port);
            last_yield_sent = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(BIND_RETRY_MS));
    }
}

fn send_yield(port: u16) {
    let addr = match format!("127.0.0.1:{port}").parse() {
        Ok(a)  => a,
        Err(_) => return,
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(YIELD_TIMEOUT_MS)) else {
        return;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(YIELD_TIMEOUT_MS)));
    let _ = stream.write_all(
        b"GET /1/control?cmd=yield HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    );
}

// ── request dispatch ─────────────────────────────────────────────────────────

fn handle(req: Request, state: &SharedState) {
    let raw = req.url().to_owned();
    crate::clog!("[server] {} {}", req.method(), raw);
    let (path, query) = raw.split_once('?').unwrap_or((&raw, ""));
    let params = parse_query(query);

    match (req.method(), path) {
        // /hello — единственный эндпоинт БЕЗ версионного префикса. Это корень
        // доверия: чтобы вообще узнать версию протокола пира, нельзя заранее
        // предполагать её в пути запроса. Форма ответа этой функции обязана
        // оставаться стабильной для любой прошлой и будущей версии протокола —
        // можно только добавлять опциональные поля, никогда не менять смысл
        // существующих и не требовать новых обязательных.
        (Method::Get,  "/hello")         => hello(req, state),
        (Method::Get,  "/1/ops")         => serve_ops(req, state, &params),
        (Method::Post, "/1/request_sync") => request_sync(req, state),
        (Method::Post, "/1/accept_sync")   => accept_sync(req, state),
        (Method::Post, "/1/ping_sync")   => ping_sync(req, state, &params),
        _                               => respond(req, 404, ""),
    }
}

// ── handlers ─────────────────────────────────────────────────────────────────

fn hello(req: Request, state: &SharedState) {
    #[derive(Serialize)]
    struct Hello<'a> {
        proto_ver:   u32,
        // Версии протокола, которые мы МОЖЕМ отдать по запросу, помимо
        // основной `proto_ver` — задел на будущее (см. обсуждение v3
        // с обратной совместимостью). Сегодня у нас только одна версия
        // протокола вообще, так что список всегда пуст — но поле уже
        // должно существовать в формате, потому что добавить его позже
        // так, чтобы уже выпущенные v2-клиенты научились его учитывать,
        // будет нельзя (см. раздел 1а общего обзора проекта).
        proto_vers:  &'a [u32],
        device_id:   &'a str,
        device_name: String,
        device_type: DeviceType,
    }
    let body = serde_json::to_string(&Hello {
        proto_ver:   PROTO_VER,
        proto_vers:  &[],
        device_id:   &state.device_id,
        device_name: state.device_name.read().unwrap().clone(),
        device_type: DeviceType::Desktop,
    })
    .unwrap_or_default();
    respond_json(req, 200, &body);
}

fn serve_ops(req: Request, state: &SharedState, params: &HashMap<&str, &str>) {
    if !authed(&req, params, state) { respond(req, 403, ""); return; }

    let since: u64 = params.get("since")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    crate::clog!("[server] serve_ops since={since}");

    let body = std::fs::read_to_string(&state.oplog_path).unwrap_or_default();
    let out: String = body.lines()
        .filter(|l| serde_json::from_str::<Op>(l).map_or(false, |op| op.seq >= since))
        .collect::<Vec<_>>()
        .join("\n");

    crate::clog!("[server] serve_ops sending {} lines", out.lines().count());
    respond(req, 200, &out);
}

fn request_sync(mut req: Request, state: &SharedState) {
    #[derive(Deserialize)]
    struct Body {
        device_id: String,
        device_name: String,
        #[serde(default)] device_type: DeviceType,
        #[serde(default = "default_port")] port: u16,
    }

    let mut buf = String::new();
    if req.as_reader().read_to_string(&mut buf).is_err() { respond(req, 400, ""); return; }
    let Ok(b) = serde_json::from_str::<Body>(&buf) else { respond(req, 400, ""); return; };

    // Защита от пейринга с самим собой — в норме такого не должно случаться
    // (discovery уже фильтрует свой же device_id из списка найденных), но
    // это дёшево проверить отдельно, а не полагаться только на UI.
    if b.device_id == state.device_id { respond(req, 400, ""); return; }

    // Already trusted → 200 (idempotent).
    if state.peers.read().unwrap().find_by_id(&b.device_id).is_some() {
        respond(req, 200, "{}"); return;
    }

    let from_ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();

    // ── взаимная заявка ──────────────────────────────────────────────────
    // Если МЫ сами недавно нажали "Подключить" на это же устройство — значит
    // оба хотят друг друга, и если бы обе стороны теперь независимо нажали
    // "Принять", у каждой родился бы СВОЙ токен, а Peers::add — перезапись
    // "кто последний" (итог на двух сторонах мог разойтись). Поэтому токен
    // рождается строго на одной стороне — на той, у кого device_id меньше.
    // Согласие второй стороны уже есть: она сама нажала "Подключить".
    let mutual = {
        let mut out = state.pending_outgoing.lock().unwrap();
        out.retain(|_, t| t.elapsed() < OUTGOING_TTL);
        out.contains_key(&b.device_id)
    };
    if mutual && state.device_id < b.device_id {
        crate::clog!("[server] mutual request with {} — we win tie-break, auto-accept", b.device_id);
        let pr = PairingRequest {
            device_id:   b.device_id,
            device_name: b.device_name,
            from_ip,
            port:        b.port,
            device_type: b.device_type,
            mutual_wait_until: None,
        };
        accept_pairing(state, &pr);
        respond(req, 202, "{}");
        return;
    }

    let device_name_for_notify = b.device_name.clone();
    let mutual_wait_until = if mutual {
        crate::clog!("[server] mutual request with {} — we lose tie-break, waiting for accept_sync", b.device_id);
        Some(Instant::now() + MUTUAL_WAIT)
    } else { None };
    let is_new;
    {
        let mut pending = state.pending_pairings.lock().unwrap();
        // Не плодим дубликаты, если это устройство уже прислало запрос
        // и мы его ещё не приняли/отклонили (например, юзер несколько раз
        // подряд нажал "Подключить") — просто обновляем данные на месте.
        match pending.iter_mut().find(|p| p.device_id == b.device_id) {
            Some(existing) => {
                is_new = false;
                existing.device_name = b.device_name;
                existing.from_ip     = from_ip;
                existing.port        = b.port;
                existing.device_type = b.device_type;
                if mutual_wait_until.is_some() { existing.mutual_wait_until = mutual_wait_until; }
            }
            None => {
                is_new = true;
                pending.push(PairingRequest {
                    device_id:   b.device_id,
                    device_name: b.device_name,
                    from_ip,
                    port:        b.port,
                    device_type: b.device_type,
                    mutual_wait_until,
                });
            }
        }
        save_pending_pairings(state.oplog_path.parent().unwrap_or(std::path::Path::new(".")), &pending);
    }
    // Уведомление — только на действительно новый запрос (не на повторные
    // от того же устройства), не во время ожидания взаимной заявки и только
    // если юзер прямо сейчас не смотрит на вкладку "Синхронизация".
    if is_new && !mutual && !state.viewing_sync_panel.load(std::sync::atomic::Ordering::Relaxed) {
        crate::notify::send_no_icon(
            "Запрос на подключение",
            &format!("{device_name_for_notify} хочет синхронизироваться с этим устройством"),
        );
    }
    // Wake egui so the pairing banner appears immediately.
    state.egui_ctx.request_repaint();
    respond(req, 202, "{}");
}

/// Принять заявку: секрет генерируется ЗДЕСЬ, на принимающей стороне, а не
/// вычисляется из device_id (те рассылаются открытым текстом в discovery-
/// broadcast, детерминированный токен был бы тривиально вычислим любым
/// слушающим). Секрет никогда не попадает в broadcast — только адресно, в
/// /1/accept_sync. Зовётся из UI (клик "Принять") и из request_sync
/// (автопринятие при взаимной заявке).
pub fn accept_pairing(state: &SharedState, req: &PairingRequest) {
    use std::io::Read;

    let real_token = crate::project::gen_token();

    state.peers.write().unwrap().add(super::peers::PeerEntry {
        device_id:      req.device_id.clone(),
        device_name:    req.device_name.clone(),
        token:          real_token.clone(),
        ip_hint:        Some(req.from_ip.clone()),
        port:           req.port,
        last_synced_at: None,
        device_type:    req.device_type,
    });
    clear_pairing_state(state, &req.device_id);
    state.ping_tx.try_send(()).ok();
    state.egui_ctx.request_repaint();

    // Сообщаем инициатору токен — он добавит нас к себе в trusted_peers.
    let ip       = req.from_ip.clone();
    let our_id   = state.device_id.clone();
    let our_name = state.device_name.read().unwrap().clone();
    let port     = req.port;          // порт СЕРВЕРА инициатора
    let our_port = state.port();      // наш — чтобы он записал нас верно
    std::thread::spawn(move || {
        let addr = format!("{ip}:{port}");
        let body = serde_json::json!({
            "device_id":   our_id,
            "device_name": our_name,
            "token":       real_token,
            "device_type": "desktop",
            "port":        our_port,
        }).to_string();
        let http = format!(
            "POST /1/accept_sync HTTP/1.0\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        );
        match TcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap()),
            Duration::from_secs(5),
        ) {
            Ok(mut s) => {
                let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
                let _ = s.write_all(http.as_bytes());
                let mut buf = [0u8; 64];
                let _ = s.read(&mut buf);
                crate::clog!("[server] accept_sync sent to {ip} ok");
            }
            Err(e) => crate::clog!("[server] accept_sync to {ip} FAILED: {e}"),
        }
    });
}

/// Забыть всё "незавершённое" про это устройство: входящую заявку и нашу
/// собственную исходящую. Зовётся, когда пейринг завершён.
fn clear_pairing_state(state: &SharedState, device_id: &str) {
    {
        let mut pending = state.pending_pairings.lock().unwrap();
        pending.retain(|r| r.device_id != device_id);
        save_pending_pairings(state.oplog_path.parent().unwrap_or(std::path::Path::new(".")), &pending);
    }
    state.pending_outgoing.lock().unwrap().remove(device_id);
}

fn accept_sync(mut req: Request, state: &SharedState) {
    #[derive(Deserialize)]
    struct Body {
        device_id: String,
        device_name: String,
        token: String,
        #[serde(default)] device_type: DeviceType,
        #[serde(default = "default_port")] port: u16,
    }

    let mut buf = String::new();
    if req.as_reader().read_to_string(&mut buf).is_err() { respond(req, 400, ""); return; }
    let Ok(b) = serde_json::from_str::<Body>(&buf) else { respond(req, 400, ""); return; };

    if b.device_id == state.device_id { respond(req, 400, ""); return; }

    // ВАЖНО: IP берём с самого TCP-соединения (req.remote_addr()), а НЕ из
    // тела запроса. Раньше отправитель (клиент) сам присылал "from_ip" —
    // и на его стороне эта переменная случайно оказывалась IP-адресом
    // ПОЛУЧАТЕЛЯ (нас самих), а не своим собственным — из-за чего мы
    // сохраняли пира с ip_hint, указывающим на самих себя, после чего
    // движок синхронизации периодически опрашивал сам себя и затирал
    // сохранённое имя пира собственным именем. Самостоятельно наблюдаемый
    // адрес соединения тому же самому классу ошибок в принципе не
    // подвержен — он физически не может быть перепутан с чужим.
    let from_ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();

    crate::clog!("[server] accept_sync from {} ip={}", b.device_id, from_ip);

    let peer_id = b.device_id.clone();
    let entry = super::peers::PeerEntry {
        device_id:      b.device_id,
        device_name:    b.device_name,
        token:          b.token,
        ip_hint:        Some(from_ip),
        port:           b.port,
        last_synced_at: None,
        device_type:    b.device_type,
    };
    state.peers.write().unwrap().add(entry);
    // Пейринг с этим устройством завершён — убираем висящую заявку от него
    // (если была) и нашу пометку об отправленной.
    clear_pairing_state(state, &peer_id);
    state.ping_tx.try_send(()).ok();
    state.egui_ctx.request_repaint();
    respond(req, 200, "{}");
}

fn ping_sync(req: Request, state: &SharedState, params: &HashMap<&str, &str>) {
    if !authed(&req, params, state) { respond(req, 403, ""); return; }
    // Non-blocking: if the engine is already awake the send simply fails.
    state.ping_tx.try_send(()).ok();
    respond(req, 200, "{}");
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Проверяет токен. Заодно — раз запрос аутентифицирован — запоминает, где
/// сейчас живёт этот пир (см. learn_peer_addr).
fn authed(req: &Request, params: &HashMap<&str, &str>, state: &SharedState) -> bool {
    let token = params.get("token").copied().unwrap_or("");
    let ok = !token.is_empty() && state.peers.read().unwrap().find_by_token(token).is_some();
    crate::clog!("[server] auth token={token:?} ok={ok}");
    if ok { learn_peer_addr(req, params, state, token); }
    ok
}

/// Пассивное обновление адреса пира из его аутентифицированного запроса.
/// IP берём с самого соединения (`remote_addr()`), порт — из параметра `port=`
/// (порт СЕРВЕРА пира: с соединения его не узнать, там виден лишь случайный
/// исходящий порт ОС). Так пир, сменивший IP или порт, сам сообщает нам новый
/// адрес при первом же обращении — а он обращается каждый цикл. Токен уже
/// проверен, так что подделать это может только тот, кто знает токен.
fn learn_peer_addr(req: &Request, params: &HashMap<&str, &str>, state: &SharedState, token: &str) {
    let Some(remote) = req.remote_addr() else { return; };
    if remote.ip().is_loopback() { return; }   // curl/отладка с этой же машины
    let ip = remote.ip().to_string();

    let (device_id, cur_port) = {
        let peers = state.peers.read().unwrap();
        let Some(p) = peers.find_by_token(token) else { return; };
        if p.ip_hint.as_deref() == Some(ip.as_str())
            && params.get("port").and_then(|v| v.parse::<u16>().ok()).map_or(true, |v| v == p.port)
        {
            return;   // ничего не изменилось — не берём write-lock
        }
        (p.device_id.clone(), p.port)
    };
    // Нет валидного port= (старый клиент) — оставляем прежний порт.
    let port = params.get("port")
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|&v| v != 0)
        .unwrap_or(cur_port);

    if state.peers.write().unwrap().update_addr(&device_id, &ip, port) {
        crate::clog!("[server] peer {device_id} address updated → {ip}:{port}");
        state.ping_tx.try_send(()).ok();   // пусть engine сразу проверит новый адрес
        state.egui_ctx.request_repaint();
    }
}

fn parse_query<'q>(query: &'q str) -> HashMap<&'q str, &'q str> {
    query.split('&')
        .filter_map(|kv| kv.split_once('='))
        .collect()
}

fn respond(req: Request, code: u16, body: &str) {
    let _ = req.respond(Response::from_string(body).with_status_code(StatusCode(code)));
}

fn respond_json(req: Request, code: u16, body: &str) {
    let header = tiny_http::Header::from_bytes(b"Content-Type", b"application/json").unwrap();
    let _ = req.respond(
        Response::from_string(body)
            .with_status_code(StatusCode(code))
            .with_header(header),
    );
}
// ── pending pairings persistence ─────────────────────────────────────────────

pub fn load_pending_pairings(dir: &std::path::Path) -> Vec<PairingRequest> {
    std::fs::read_to_string(dir.join("pending_pairings.json")).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_pending_pairings(dir: &std::path::Path, list: &[PairingRequest]) {
    if let Ok(j) = serde_json::to_string_pretty(list) {
        let _ = std::fs::write(dir.join("pending_pairings.json"), j);
    }
}
