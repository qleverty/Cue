use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tiny_http::{Method, Request, Response, Server, StatusCode};

use crate::peers::{DeviceType, Peers};

pub const DEFAULT_PORT: u16 = 24684;
pub const PROTO_VER: u32 = 1;

pub fn default_port() -> u16 { DEFAULT_PORT }

// ── shared state ─────────────────────────────────────────────────────────────


pub struct SharedState {
    pub device_id:        String,
    pub device_name:      String,
    pub peers:            RwLock<Peers>,
    pub oplog_path:       PathBuf,
    pub pending_pairings: Mutex<Vec<PairingRequest>>,
    pub pending_outgoing: Mutex<HashMap<String, Instant>>,
    pub http_port:        std::sync::atomic::AtomicU16,
    pub ping_tx:          std::sync::mpsc::SyncSender<()>,
}

impl SharedState {
    pub fn port(&self) -> u16 {
        self.http_port.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    pub device_id:   String,
    pub device_name: String,
    pub from_ip:     String,
    #[serde(default = "default_port")]
    pub port:        u16,
    #[serde(default)]
    pub device_type: DeviceType,
    #[serde(skip)]
    pub mutual_wait_until: Option<Instant>,
}

impl PairingRequest {
    #[allow(dead_code)]
    pub fn waiting(&self) -> bool {
        self.mutual_wait_until.is_some_and(|t| Instant::now() < t)
    }
}

const OUTGOING_TTL: Duration = Duration::from_secs(600);
const MUTUAL_WAIT:  Duration = Duration::from_secs(15);

// ── start / bind-retry ──────────────────────────────────────────────────────

const BIND_RETRY_MS:       u64 = 300;
const RELEASE_COOLDOWN_MS: u64 = 5000;
const PORT_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

#[derive(serde::Deserialize)]
struct PartialSettings {
    #[serde(default = "default_port")]
    http_port: u16,
}

fn settings_path() -> std::path::PathBuf {
    crate::app_dir().join("settings.json")
}

fn read_configured_port() -> u16 {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str::<PartialSettings>(&s).ok())
        .map(|s| s.http_port)
        .unwrap_or(DEFAULT_PORT)
}

pub fn start(state: Arc<SharedState>) {
    std::thread::Builder::new()
        .name("cue-daemon-server".into())
        .spawn(move || run(state))
        .expect("spawn daemon server thread");
}

fn run(state: Arc<SharedState>) {
    loop {
        let (server, port) = bind_with_retry();
        state.http_port.store(port, std::sync::atomic::Ordering::SeqCst);
        println!("[cue_daemon] sync-сервер слушает порт {port}");
        serve(&server, &state);
        drop(server);
        println!("[cue_daemon] sync-сервер отпустил порт {port}, жду {RELEASE_COOLDOWN_MS}мс перед повтором");
        std::thread::sleep(Duration::from_millis(RELEASE_COOLDOWN_MS));
    }
}

fn bind_with_retry() -> (Server, u16) {
    let mut port = read_configured_port();
    let mut last_refresh = Instant::now();
    let mut logged_wait = false;

    loop {
        let attempt = crate::exclusive_bind::bind_exclusive(port).ok()
            .and_then(|l| Server::from_listener(l, None).ok());

        match attempt {
            Some(server) => {
                if crate::cue_liveness::cue_is_running() {
                    println!("[cue_daemon] занял порт {port}, но Cue жива — она просто сменила порт, отпускаю и повторяю");
                    drop(server);
                    port = read_configured_port();
                    last_refresh = Instant::now();
                    logged_wait = false;
                    std::thread::sleep(Duration::from_millis(BIND_RETRY_MS));
                    continue;
                }
                return (server, port);
            }
            None => {
                if !logged_wait {
                    println!("[cue_daemon] порт {port} занят, жду освобождения");
                    logged_wait = true;
                }
                if last_refresh.elapsed() >= PORT_REFRESH_INTERVAL {
                    let new_port = read_configured_port();
                    if new_port != port {
                        println!("[cue_daemon] настроенный порт сменился: {port} → {new_port}");
                        port = new_port;
                        logged_wait = false;
                    }
                    last_refresh = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(BIND_RETRY_MS));
            }
        }
    }
}

// ── request dispatch ─────────────────────────────────────────────────────────

fn serve(server: &Server, state: &SharedState) {
    for req in server.incoming_requests() {
        handle(req, server, state);
    }
}

fn handle(req: Request, server: &Server, state: &SharedState) {
    let raw = req.url().to_owned();
    let (path, query) = raw.split_once('?').unwrap_or((&raw, ""));
    let params = parse_query(query);

    match (req.method(), path) {
        (Method::Get,  "/hello")          => hello(req, state),
        (Method::Get,  "/1/ops")          => serve_ops(req, state, &params),
        (Method::Post, "/1/request_sync") => request_sync(req, state),
        (Method::Post, "/1/accept_sync")  => accept_sync(req, state),
        (Method::Post, "/1/ping_sync")    => ping_sync(req, state, &params),
        (Method::Get,  "/1/control")      => control(req, server, &params),
        _                                 => respond(req, 404, ""),
    }
}

fn control(req: Request, server: &Server, params: &HashMap<&str, &str>) {
    let is_loopback = req.remote_addr()
        .map(|a| a.ip().is_loopback())
        .unwrap_or(false);
    if !is_loopback {
        println!("[cue_daemon] отклонена /1/control команда не с loopback: {:?}", req.remote_addr());
        respond(req, 403, "");
        return;
    }

    match params.get("cmd").copied() {
        Some("yield") => {
            println!("[cue_daemon] получен yield, отпускаю порт");
            respond(req, 200, "");
            server.unblock();
        }
        Some("shutdown") => {
            println!("[cue_daemon] получен shutdown, завершаюсь");
            respond(req, 200, "");
            std::process::exit(0);
        }
        _ => respond(req, 404, ""),
    }
}

// ── handlers ─────────────────────────────────────────────────────────────────

fn hello(req: Request, state: &SharedState) {
    #[derive(Serialize)]
    struct Hello<'a> {
        proto_ver:   u32,
        proto_vers:  &'a [u32],
        device_id:   &'a str,
        device_name: String,
        device_type: DeviceType,
    }
    let body = serde_json::to_string(&Hello {
        proto_ver:   PROTO_VER,
        proto_vers:  &[],
        device_id:   &state.device_id,
        device_name: state.device_name.clone(),
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

    let body = std::fs::read_to_string(&state.oplog_path).unwrap_or_default();
    let out: String = body.lines()
        .filter(|l| serde_json::from_str::<crate::oplog::Op>(l).map_or(false, |op| op.seq >= since))
        .collect::<Vec<_>>()
        .join("\n");

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

    if b.device_id == state.device_id { respond(req, 400, ""); return; }

    if state.peers.read().unwrap().find_by_id(&b.device_id).is_some() {
        respond(req, 200, "{}"); return;
    }

    let from_ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();

    let mutual = {
        let mut out = state.pending_outgoing.lock().unwrap();
        out.retain(|_, t| t.elapsed() < OUTGOING_TTL);
        out.contains_key(&b.device_id)
    };
    if mutual && state.device_id < b.device_id {
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
        Some(Instant::now() + MUTUAL_WAIT)
    } else { None };
    let is_new;
    {
        let mut pending = state.pending_pairings.lock().unwrap();
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
    if is_new && !mutual {
        crate::notify::send_no_icon(
            "Запрос на подключение",
            &format!("{device_name_for_notify} хочет синхронизироваться с этим устройством"),
        );
    }
    respond(req, 202, "{}");
}

pub fn accept_pairing(state: &SharedState, req: &PairingRequest) {
    use std::io::Read;

    let real_token = crate::project::gen_token();

    state.peers.write().unwrap().add(crate::peers::PeerEntry {
        device_id:      req.device_id.clone(),
        device_name:    req.device_name.clone(),
        token:          real_token.clone(),
        ip_hint:        Some(req.from_ip.clone()),
        port:           req.port,
        last_synced_at: None,
        revoked:        false,
        incompatible:   false,
        device_type:    req.device_type,
    });
    clear_pairing_state(state, &req.device_id);
    state.ping_tx.try_send(()).ok();

    let ip       = req.from_ip.clone();
    let our_id   = state.device_id.clone();
    let our_name = state.device_name.clone();
    let port     = req.port;
    let our_port = state.port();
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
                println!("[cue_daemon] accept_sync отправлен {ip} ok");
            }
            Err(e) => println!("[cue_daemon] accept_sync {ip} ОШИБКА: {e}"),
        }
    });
}

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

    let from_ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();

    let peer_id = b.device_id.clone();
    let entry = crate::peers::PeerEntry {
        device_id:      b.device_id,
        device_name:    b.device_name,
        token:          b.token,
        ip_hint:        Some(from_ip),
        port:           b.port,
        last_synced_at: None,
        revoked:        false,
        incompatible:   false,
        device_type:    b.device_type,
    };
    state.peers.write().unwrap().add(entry);
    clear_pairing_state(state, &peer_id);
    state.ping_tx.try_send(()).ok();
    respond(req, 200, "{}");
}

fn ping_sync(req: Request, state: &SharedState, params: &HashMap<&str, &str>) {
    if !authed(&req, params, state) { respond(req, 403, ""); return; }
    state.ping_tx.try_send(()).ok();
    respond(req, 200, "{}");
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn authed(req: &Request, params: &HashMap<&str, &str>, state: &SharedState) -> bool {
    let token = params.get("token").copied().unwrap_or("");
    let ok = !token.is_empty() && state.peers.read().unwrap().find_by_token(token).is_some();
    if ok { learn_peer_addr(req, params, state, token); }
    ok
}

fn learn_peer_addr(req: &Request, params: &HashMap<&str, &str>, state: &SharedState, token: &str) {
    let Some(remote) = req.remote_addr() else { return; };
    if remote.ip().is_loopback() { return; }
    let ip = remote.ip().to_string();

    let (device_id, cur_port) = {
        let peers = state.peers.read().unwrap();
        let Some(p) = peers.find_by_token(token) else { return; };
        if p.ip_hint.as_deref() == Some(ip.as_str())
            && params.get("port").and_then(|v| v.parse::<u16>().ok()).map_or(true, |v| v == p.port)
        {
            return;
        }
        (p.device_id.clone(), p.port)
    };
    let port = params.get("port")
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|&v| v != 0)
        .unwrap_or(cur_port);

    if state.peers.write().unwrap().update_addr(&device_id, &ip, port) {
        println!("[cue_daemon] адрес пира {device_id} обновлён → {ip}:{port}");
        state.ping_tx.try_send(()).ok();
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
