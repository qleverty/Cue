use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, mpsc};

use eframe::egui;
use serde::{Deserialize, Serialize};
use tiny_http::{Method, Request, Response, Server, StatusCode};

use super::{oplog::Op, peers::Peers};

pub const PORT:      u16 = 52684;
pub const PROTO_VER: u32 = 1;

// ── shared state (server + engine both hold an Arc<SharedState>) ─────────────

pub struct SharedState {
    pub device_id:        String,
    pub device_name:      RwLock<String>,
    pub peers:            RwLock<Peers>,
    pub oplog_path:       PathBuf,
    pub pending_pairings: Mutex<Vec<PairingRequest>>,
    pub sync_status:      Mutex<super::SyncStatus>,
    /// UDP discovery handle — exposes discovered list and ping trigger.
    pub discovered:       super::discovery::Discovery,
    /// Bounded-1 channel: server taps engine on POST /ping_sync.
    pub ping_tx:          mpsc::SyncSender<()>,
    /// Used by engine to wake egui immediately after delivering ops.
    pub egui_ctx:         egui::Context,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    pub device_id:   String,
    pub device_name: String,
    pub token:       String,
    pub from_ip:     String,
}

// ── start ─────────────────────────────────────────────────────────────────────

pub fn start(state: Arc<SharedState>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("cue-sync-server".into())
        .spawn(move || {
            match Server::http(format!("0.0.0.0:{PORT}")) {
                Ok(server) => {
                    for req in server.incoming_requests() {
                        handle(req, &state);
                    }
                }
                Err(e) => crate::clog!("[sync/server] bind failed on port {PORT}: {e}"),
            }
        })
        .expect("spawn sync server thread")
}

// ── request dispatch ─────────────────────────────────────────────────────────

fn handle(req: Request, state: &SharedState) {
    let raw = req.url().to_owned();
    crate::clog!("[server] {} {}", req.method(), raw);
    let (path, query) = raw.split_once('?').unwrap_or((&raw, ""));
    let params = parse_query(query);

    match (req.method(), path) {
        (Method::Get,  "/1/hello")       => hello(req, state),
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
    struct Hello<'a> { proto_ver: u32, device_id: &'a str, device_name: String }
    let body = serde_json::to_string(&Hello {
        proto_ver:   PROTO_VER,
        device_id:   &state.device_id,
        device_name: state.device_name.read().unwrap().clone(),
    })
    .unwrap_or_default();
    respond_json(req, 200, &body);
}

fn serve_ops(req: Request, state: &SharedState, params: &HashMap<&str, &str>) {
    if !authed(params, &state.peers) { respond(req, 403, ""); return; }

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
    struct Body { device_id: String, device_name: String, token: String }

    let mut buf = String::new();
    if req.as_reader().read_to_string(&mut buf).is_err() { respond(req, 400, ""); return; }
    let Ok(b) = serde_json::from_str::<Body>(&buf) else { respond(req, 400, ""); return; };

    // Already trusted → 200 (idempotent).
    if state.peers.read().unwrap().find_by_id(&b.device_id).is_some() {
        respond(req, 200, "{}"); return;
    }

    let from_ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    {
        let mut pending = state.pending_pairings.lock().unwrap();
        pending.push(PairingRequest {
            device_id:   b.device_id,
            device_name: b.device_name,
            token:       b.token,
            from_ip,
        });
        save_pending_pairings(state.oplog_path.parent().unwrap_or(std::path::Path::new(".")), &pending);
    }
    // Wake egui so the pairing banner appears immediately.
    state.egui_ctx.request_repaint();
    respond(req, 202, "{}");
}

fn accept_sync(mut req: Request, state: &SharedState) {
    #[derive(Deserialize)]
    struct Body { device_id: String, device_name: String, token: String, from_ip: String }

    let mut buf = String::new();
    if req.as_reader().read_to_string(&mut buf).is_err() { respond(req, 400, ""); return; }
    let Ok(b) = serde_json::from_str::<Body>(&buf) else { respond(req, 400, ""); return; };

    crate::clog!("[server] accept_sync from {} ip={}", b.device_id, b.from_ip);

    let entry = super::peers::PeerEntry {
        device_id:      b.device_id,
        device_name:    b.device_name,
        token:          b.token,
        ip_hint:        Some(b.from_ip),
        last_synced_at: None,
    };
    state.peers.write().unwrap().add(entry);
    state.ping_tx.try_send(()).ok();
    state.egui_ctx.request_repaint();
    respond(req, 200, "{}");
}

fn ping_sync(req: Request, state: &SharedState, params: &HashMap<&str, &str>) {
    if !authed(params, &state.peers) { respond(req, 403, ""); return; }
    // Non-blocking: if the engine is already awake the send simply fails.
    state.ping_tx.try_send(()).ok();
    respond(req, 200, "{}");
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn authed(params: &HashMap<&str, &str>, peers: &RwLock<Peers>) -> bool {
    let token = params.get("token").copied().unwrap_or("");
    let ok = !token.is_empty() && peers.read().unwrap().find_by_token(token).is_some();
    crate::clog!("[server] auth token={token:?} ok={ok}");
    ok
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
