pub mod identity;
pub mod tombstones;
pub mod oplog;
pub mod bootstrap;

// Stage 3+: pub mod cursors; pub mod peers; pub mod engine; pub mod server;
// Stage 4+: pub mod discovery;

pub struct SyncHandle {
    pub identity:   identity::DeviceIdentity,
    pub tombstones: tombstones::Tombstones,
    pub oplog:      oplog::OpLog,
}

impl SyncHandle {
    pub fn init(projects: &mut Vec<crate::project::LoadedProject>) -> Self {
        let dir        = crate::app_dir();
        let identity   = identity::DeviceIdentity::load_or_create(&dir);
        let tombstones = tombstones::Tombstones::load(&dir);
        let mut oplog  = oplog::OpLog::open(&dir);
        bootstrap::run_if_needed(projects, &identity, &mut oplog);
        Self { identity, tombstones, oplog }
    }
}