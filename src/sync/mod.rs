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
use std::sync::{Arc, Mutex, RwLock, mpsc};

pub use server::{PairingRequest, SharedState};

// ── Sync status types ─────────────────────────────────────────────────────────

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

/// Classic UDP trick: bind a socket, "connect" to an external address
/// (no data is sent), then read back the local address the OS chose.
pub fn get_lan_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

pub struct SyncHandle {
    pub identity:   identity::DeviceIdentity,
    pub tombstones: tombstones::Tombstones,
    pub oplog:      oplog::OpLog,
    /// Shared with the engine thread — engine reads to know `since=N`.
    pub cursors:    Arc<Mutex<cursors::Cursors>>,
    /// Dedup set: op_ids we have already seen / written locally.
    pub seen_ops:   HashSet<String>,
    /// Incoming ops from the engine, drained each UI frame.
    pub ops_rx:     mpsc::Receiver<Vec<oplog::Op>>,
    /// Shared between server + engine threads (peers, ping signal, pairings).
    pub shared:     Arc<SharedState>,
    /// Signal notifier thread: "I wrote a local op, tell peers to pull from me."
    notify_tx:      mpsc::SyncSender<()>,
    /// Best-effort LAN IP of this device, detected at startup.
    pub local_ip:   Option<String>,
}

impl SyncHandle {
    pub fn init(projects: &mut Vec<crate::project::LoadedProject>, egui_ctx: eframe::egui::Context) -> Self {
        let dir        = crate::app_dir();
        crate::clog!("[sync] init — app_dir={:?}", dir);
        let identity   = identity::DeviceIdentity::load_or_create(&dir);
        crate::clog!("[sync] identity device_id={}", identity.device_id);
        let tombstones = tombstones::Tombstones::load(&dir);
        let cursors    = Arc::new(Mutex::new(cursors::Cursors::load(&dir)));
        let mut oplog  = oplog::OpLog::open(&dir);
        crate::clog!("[sync] oplog opened, next_seq={}", oplog.next_seq);

        let seen_ops: HashSet<String> = oplog.all_ops()
            .into_iter().map(|op| op.op_id).collect();
        crate::clog!("[sync] seen_ops count={}", seen_ops.len());

        bootstrap::run_if_needed(projects, &identity, &mut oplog);
        crate::clog!("[sync] bootstrap done, next_seq={}", oplog.next_seq);

        let (ping_tx,   ping_rx)   = mpsc::sync_channel::<()>(1);
        let (notify_tx, notify_rx) = mpsc::sync_channel::<()>(1);
        let (ops_tx,    ops_rx)    = mpsc::channel::<Vec<oplog::Op>>();

        let peers_loaded = peers::Peers::load(&dir);
        crate::clog!("[sync] peers loaded count={}", peers_loaded.all().len());

        let discovered = discovery::start(
            identity.device_id.clone(),
            Arc::new(RwLock::new(identity.device_name.clone())),
        );
        crate::clog!("[sync] discovery started");

        let shared = Arc::new(SharedState {
            device_id:        identity.device_id.clone(),
            device_name:      RwLock::new(identity.device_name.clone()),
            peers:            RwLock::new(peers_loaded),
            oplog_path:       dir.join("ops.ndjson"),
            pending_pairings: Mutex::new(Vec::new()),
            sync_status:      Mutex::new(SyncStatus::default()),
            discovered,
            ping_tx,
            egui_ctx,
        });

        server::start(Arc::clone(&shared));
        engine::start(Arc::clone(&shared), Arc::clone(&cursors), ops_tx, ping_rx);
        engine::start_notifier(Arc::clone(&shared), notify_rx);

        let local_ip = get_lan_ip();
        crate::clog!("[sync] local_ip={:?}", local_ip);

        Self { identity, tombstones, oplog, cursors, seen_ops, ops_rx, shared, notify_tx, local_ip }
    }

    pub fn record_op(&mut self, kind: oplog::OpKind) -> std::io::Result<oplog::Op> {
        crate::clog!("[sync] record_op called");
        let ts = crate::project::current_time();
        let op = self.oplog.append(kind, &self.identity.device_id, ts)?;
        self.seen_ops.insert(op.op_id.clone());
        crate::clog!("[sync] record_op done op_id={}", op.op_id);
        // Signal notifier: it will POST /ping_sync to peers so they pull from us,
        // then wake our own engine to pull from them in parallel.
        self.notify_tx.try_send(()).ok();
        Ok(op)
    }
}