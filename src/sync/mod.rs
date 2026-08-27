pub mod identity;
pub mod tombstones;
pub mod oplog;
pub mod bootstrap;
pub mod cursors;
pub mod apply;
pub mod peers;
pub mod server;
pub mod engine;

pub mod discovery;

use std::collections::{HashMap, HashSet};
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock, mpsc};

pub use server::{PairingRequest, SharedState};

// ── background oplog loading ──────────────────────────────────────────────────

/// A locally-generated op that couldn't be written to `ops.ndjson` yet because
/// the oplog was still being loaded in the background. Missing `seq` — that's
/// assigned in file order once the real `OpLog` is ready (see `ensure_oplog_ready`).
pub struct PendingOp {
    pub op_id:     String,
    pub device_id: String,
    pub ts:        u64,
    pub kind:      oplog::OpKind,
}

/// Startup state of the local oplog. On a device with an existing (non-empty)
/// `ops.ndjson`, reading + parsing it is moved off the main thread into the
/// `engine` thread so the window can show up instantly. Anything the UI wants
/// to record while that read is still in flight lands in `Loading`'s queue
/// instead of blocking; `ensure_oplog_ready` drains it once the real `OpLog`
/// is available.
pub enum OplogState {
    Loading(Vec<PendingOp>),
    Ready {
        oplog: oplog::OpLog,
        /// op_ids found in the pre-existing file, handed to the main thread
        /// once so it can merge them into `SyncHandle::seen_ops` (dedup for
        /// incoming remote ops applied while the load was still in flight).
        initial_ids: HashSet<String>,
    },
}

/// If `state` is still `Loading`, synchronously reads `ops.ndjson` from `dir`,
/// flushes any queued ops (in the order they were recorded) and transitions
/// to `Ready`. No-op if another caller already did this — the whole thing
/// runs under `state`'s lock, so at most one thread ever does the actual read.
///
/// Called from two places: once by the `engine` thread right after startup
/// (the normal path), and once from the main thread on window-close, in case
/// the user closes the app before the engine thread got to it — without this,
/// anything still sitting in the `Loading` queue at that point would be lost.
pub fn ensure_oplog_ready(dir: &Path, state: &Mutex<OplogState>) {
    let mut guard = state.lock().unwrap();
    let OplogState::Loading(_) = &*guard else { return };
    let OplogState::Loading(queue) = std::mem::replace(&mut *guard, OplogState::Loading(Vec::new()))
        else { unreachable!() };

    crate::clog!("[sync] ensure_oplog_ready — loading ops.ndjson, {} queued op(s)", queue.len());
    let (mut oplog, existing) = oplog::OpLog::open_with_ops(dir);
    let initial_ids: HashSet<String> = existing.into_iter().map(|op| op.op_id).collect();

    for pending in queue {
        let _ = oplog.append_op(pending.op_id, pending.kind, &pending.device_id, pending.ts);
    }
    crate::clog!("[sync] ensure_oplog_ready done, next_seq={}", oplog.next_seq);

    *guard = OplogState::Ready { oplog, initial_ids };
}

// ── Sync status types ─────────────────────────────────────────────────────────

/// Тип устройства — сейчас используется только в протоколе (discovery/hello/
/// pairing), в UI пока нигде не отображается (задел под будущую иконку в
/// списке устройств). `Unknown` — и дефолт для уже сохранённых на диске
/// записей без этого поля, и fallback при разборе незнакомого будущего
/// значения (см. `#[serde(other)]`).
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Desktop,
    Phone,
    #[serde(other)]
    Unknown,
}

impl Default for DeviceType {
    fn default() -> Self { DeviceType::Unknown }
}

#[derive(Clone, Default)]
pub struct PeerStatus {
    pub online:  bool,
    pub error:   bool,
    /// true when the peer returned 403 — they revoked our token.
    pub revoked: bool,
}

#[derive(Default)]
pub struct SyncStatus {
    /// device_id → current connectivity status.
    pub peer_statuses: HashMap<String, PeerStatus>,
}

/// Returns the local LAN (RFC1918 private) IP by probing common gateway
/// addresses. Falls back to the default route if no private IP is found.
pub fn get_lan_ip() -> Option<String> {
    // Try common LAN gateway addresses first — routing to these will
    // always pick the LAN interface, not a VPN.
    let candidates = ["192.168.1.1:80", "192.168.0.1:80", "10.0.0.1:80", "172.16.0.1:80", "8.8.8.8:80"];
    for target in candidates {
        let socket = match UdpSocket::bind("0.0.0.0:0") {
            Ok(s) => s,
            Err(_) => continue,
        };
        if socket.connect(target).is_err() { continue; }
        if let Ok(addr) = socket.local_addr() {
            let ip = addr.ip().to_string();
            if is_private_ip(&ip) {
                return Some(ip);
            }
        }
    }
    None
}

fn is_private_ip(ip: &str) -> bool {
    let parts: Vec<u8> = ip.split('.')
        .filter_map(|p| p.parse().ok())
        .collect();
    if parts.len() != 4 { return false; }
    match (parts[0], parts[1]) {
        (10, _)                    => true,  // 10.0.0.0/8
        (172, 16..=31)             => true,  // 172.16.0.0/12
        (192, 168)                 => true,  // 192.168.0.0/16
        _                          => false,
    }
}

pub struct SyncHandle {
    pub identity:   identity::DeviceIdentity,
    pub tombstones: tombstones::Tombstones,
    /// Shared with the engine thread — engine reads to know `since=N`.
    pub cursors:    Arc<Mutex<cursors::Cursors>>,
    /// Dedup set: op_ids we have already seen / written locally.
    pub seen_ops:   HashSet<String>,
    /// Set once `initial_ids` from a background oplog load have been merged
    /// into `seen_ops`, so `poll_oplog_ready` stops locking every frame.
    seen_merged:    bool,
    /// Incoming ops from the engine, drained each UI frame.
    pub ops_rx:     mpsc::Receiver<Vec<oplog::Op>>,
    /// Shared between server + engine threads (peers, ping signal, pairings,
    /// and — see `OplogState` — the oplog itself while it loads in the background).
    pub shared:     Arc<SharedState>,
    /// Signal notifier thread: "I wrote a local op, tell peers to pull from me."
    notify_tx:      mpsc::SyncSender<()>,
    /// Best-effort LAN IP of this device, detected at startup.
    pub local_ip:   Option<String>,
    /// app_dir, kept around only for `ensure_oplog_ready` on window-close.
    dir:            PathBuf,
}

impl SyncHandle {
    pub fn init(
        projects: &mut Vec<crate::project::LoadedProject>,
        egui_ctx: eframe::egui::Context,
        http_port: u16,
    ) -> Self {
        let dir        = crate::app_dir();
        crate::clog!("[sync] init — app_dir={:?}", dir);
        let identity   = identity::DeviceIdentity::load_or_create(&dir);
        crate::clog!("[sync] identity device_id={}", identity.device_id);
        let tombstones = tombstones::Tombstones::load(&dir);
        let cursors    = Arc::new(Mutex::new(cursors::Cursors::load(&dir)));

        // Cheap check, no parsing: an empty/missing file means there's
        // nothing to read (and bootstrap, below, is guaranteed to run) — so
        // there's no point deferring anything, just do it synchronously like
        // before. A non-empty file is the 99% case and the only one where
        // deferring the actual read is worth it: reading + parsing the whole
        // ops.ndjson happens in the `engine` thread instead, and bootstrap is
        // skipped outright — it would no-op anyway once next_seq > 1, which
        // holding a non-empty file already implies.
        let ops_path      = dir.join("ops.ndjson");
        let file_is_empty = std::fs::metadata(&ops_path).map(|m| m.len() == 0).unwrap_or(true);

        let (seen_ops, oplog_state) = if file_is_empty {
            let (mut oplog, _existing_ops) = oplog::OpLog::open_with_ops(&dir);
            crate::clog!("[sync] oplog empty, opened synchronously");
            bootstrap::run_if_needed(projects, &identity, &mut oplog);
            crate::clog!("[sync] bootstrap done, next_seq={}", oplog.next_seq);
            (HashSet::new(), OplogState::Ready { oplog, initial_ids: HashSet::new() })
        } else {
            crate::clog!("[sync] oplog non-empty, deferring load to engine thread");
            (HashSet::new(), OplogState::Loading(Vec::new()))
        };

        let (ping_tx,   ping_rx)   = mpsc::sync_channel::<()>(1);
        let (notify_tx, notify_rx) = mpsc::sync_channel::<()>(1);
        let (ops_tx,    ops_rx)    = mpsc::channel::<Vec<oplog::Op>>();

        let peers_loaded = peers::Peers::load(&dir);
        crate::clog!("[sync] peers loaded count={}", peers_loaded.all().len());

        let local_ip   = get_lan_ip();
        let discovered = discovery::start(
            identity.device_id.clone(),
            Arc::new(RwLock::new(identity.device_name.clone())),
            local_ip.clone(),
            DeviceType::Desktop,
        );
        crate::clog!("[sync] discovery started");

        let shared = Arc::new(SharedState {
            device_id:        identity.device_id.clone(),
            device_name:      RwLock::new(identity.device_name.clone()),
            peers:            RwLock::new(peers_loaded),
            oplog_path:       ops_path,
            oplog_state:      Mutex::new(oplog_state),
            pending_pairings: Mutex::new(server::load_pending_pairings(&dir)),
            sync_status:      Mutex::new(SyncStatus::default()),
            discovered,
            ping_tx,
            http_port,
            egui_ctx,
        });

        server::start(Arc::clone(&shared), http_port);
        engine::start(Arc::clone(&shared), Arc::clone(&cursors), ops_tx, ping_rx, dir.clone());
        engine::start_notifier(Arc::clone(&shared), notify_rx);

        crate::clog!("[sync] local_ip={:?}", local_ip);

        // If we opened synchronously above (empty-file path), there's no
        // background load to merge later — mark it done up front so
        // `poll_oplog_ready` never bothers locking.
        let seen_merged = file_is_empty;

        Self { identity, tombstones, cursors, seen_ops, seen_merged, ops_rx, shared, notify_tx, local_ip, dir }
    }

    pub fn record_op(&mut self, kind: oplog::OpKind) -> std::io::Result<oplog::Op> {
        crate::clog!("[sync] record_op called");
        let ts        = crate::project::current_time();
        let op_id     = crate::project::gen_id();
        let device_id = self.identity.device_id.clone();

        let op = {
            let mut state = self.shared.oplog_state.lock().unwrap();
            match &mut *state {
                OplogState::Ready { oplog, .. } => oplog.append_op(op_id, kind, &device_id, ts)?,
                OplogState::Loading(queue) => {
                    // Oplog still loading in the background — queue it (fully
                    // formed except `seq`, assigned in order once the real
                    // OpLog is ready) and hand back a placeholder. Every
                    // call site in main.rs discards the return value anyway.
                    queue.push(PendingOp { op_id: op_id.clone(), device_id: device_id.clone(), ts, kind: kind.clone() });
                    oplog::Op { op_id, device_id, seq: 0, ts, kind }
                }
            }
        };

        self.seen_ops.insert(op.op_id.clone());
        crate::clog!("[sync] record_op done op_id={}", op.op_id);
        // Signal notifier: it will POST /ping_sync to peers so they pull from us,
        // then wake our own engine to pull from them in parallel. If the op is
        // still only queued (not on disk yet), this ping is briefly premature —
        // harmless, the peer just finds nothing new and we retry on the next tick.
        self.notify_tx.try_send(()).ok();
        Ok(op)
    }

    /// Called once per UI frame. While the background oplog load is still in
    /// flight this is a single uncontended `try_lock`; once it merges the
    /// pre-existing op_ids into `seen_ops` it flips `seen_merged` and never
    /// locks again.
    pub fn poll_oplog_ready(&mut self) {
        if self.seen_merged { return; }
        let Ok(state) = self.shared.oplog_state.try_lock() else { return };
        if let OplogState::Ready { initial_ids, .. } = &*state {
            self.seen_ops.extend(initial_ids.iter().cloned());
            crate::clog!("[sync] poll_oplog_ready — merged {} historical id(s)", initial_ids.len());
            self.seen_merged = true;
        }
    }

    /// Called on window-close. If the engine thread hasn't finished loading
    /// the oplog yet, does so right here, synchronously, so nothing queued
    /// via `record_op` in the meantime is lost when the process exits.
    /// No-op (near-instant) once the engine thread already did this, which
    /// covers the overwhelming majority of closes.
    pub fn flush_oplog_before_exit(&self) {
        ensure_oplog_ready(&self.dir, &self.shared.oplog_state);
    }
}