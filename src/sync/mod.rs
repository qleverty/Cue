pub mod identity;
pub mod tombstones;
pub mod oplog;
pub mod bootstrap;
pub mod cursors;
pub mod apply;

// Stage 3+: pub mod peers; pub mod engine; pub mod server;
// Stage 4+: pub mod discovery;

use std::collections::HashSet;

pub struct SyncHandle {
    pub identity:   identity::DeviceIdentity,
    pub tombstones: tombstones::Tombstones,
    pub oplog:      oplog::OpLog,
    pub cursors:    cursors::Cursors,
    pub seen_ops:   HashSet<String>,
}

impl SyncHandle {
    pub fn init(projects: &mut Vec<crate::project::LoadedProject>) -> Self {
        let dir        = crate::app_dir();
        let identity   = identity::DeviceIdentity::load_or_create(&dir);
        let tombstones = tombstones::Tombstones::load(&dir);
        let cursors    = cursors::Cursors::load(&dir);
        let mut oplog  = oplog::OpLog::open(&dir);

        // Seed seen_ops so incoming peer ops can be dedup'd immediately
        let seen_ops: HashSet<String> = oplog.all_ops()
            .into_iter().map(|op| op.op_id).collect();

        bootstrap::run_if_needed(projects, &identity, &mut oplog);
        Self { identity, tombstones, oplog, cursors, seen_ops }
    }

    /// Write op to log + mark as seen. Call before mutating state.
    pub fn record_op(&mut self, kind: oplog::OpKind) -> std::io::Result<oplog::Op> {
        let ts = crate::project::current_time();
        let op = self.oplog.append(kind, &self.identity.device_id, ts)?;
        self.seen_ops.insert(op.op_id.clone());
        Ok(op)
    }
}