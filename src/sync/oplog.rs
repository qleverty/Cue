use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

// ── Op types ─────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AddTarget { Main, End, Beginning }

/// All mutation types. Tag+content serialises as:
/// `{ "op": "ADD_TASK", "payload": { ... } }`
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "op", content = "payload")]
pub enum OpKind {
    #[serde(rename = "CREATE_PROJECT")]
    CreateProject  { project_id: String, name: String, color: String, created_at: u64 },
    #[serde(rename = "DELETE_PROJECT")]
    DeleteProject  { project_id: String },
    #[serde(rename = "RENAME_PROJECT")]
    RenameProject  { project_id: String, name: String },
    #[serde(rename = "RECOLOR_PROJECT")]
    RecolorProject { project_id: String, color: String },
    #[serde(rename = "ADD_TASK")]
    AddTask        { project_id: String, task_id: String, text: String, target: AddTarget },
    #[serde(rename = "DELETE_TASK")]
    DeleteTask     { project_id: String, task_id: String },
    #[serde(rename = "COMPLETE_MAIN")]
    CompleteMain   { project_id: String, task_id: String },
    /// Досрочное завершение рутины прямо в subs (крестик по активной рутине
    /// = "сделал без main"), без промоушена следующей задачи и без
    /// перемещения по списку.
    #[serde(rename = "COMPLETE_SUB")]
    CompleteSub    { project_id: String, task_id: String },
    #[serde(rename = "PROMOTE_TASK")]
    PromoteTask    { project_id: String, task_id: String },
    #[serde(rename = "EDIT_TASK")]
    EditTask       { project_id: String, task_id: String, text: String },
    /// Полная перезапись расписания задачи. `routine: None` — убрать
    /// рутину целиком (задача остаётся обычной). Без диффов.
    #[serde(rename = "SET_ROUTINE")]
    SetRoutine     { project_id: String, task_id: String, routine: Option<crate::project::Routine> },
    #[serde(rename = "SET_SHARED_SETTING")]
    SetSharedSetting { key: String, value: serde_json::Value },
}

/// One record in ops.ndjson.
/// `flatten` on `kind` merges "op"/"payload" into the top-level object.
#[derive(Serialize, Deserialize, Clone)]
pub struct Op {
    pub op_id:     String,
    pub device_id: String,
    pub seq:       u64,
    pub ts:        u64,
    #[serde(flatten)]
    pub kind:      OpKind,
}

// ── OpLog ─────────────────────────────────────────────────────────────────────

pub struct OpLog {
    path:         PathBuf,
    pub next_seq: u64,
}

impl OpLog {
    /// Открыть (или создать) лог одним проходом по файлу: заодно возвращает
    /// уже распарсенные операции — раньше `open()` и `all_ops()` читали и
    /// парсили один и тот же файл дважды подряд при каждом старте.
    pub fn open_with_ops(dir: &Path) -> (Self, Vec<Op>) {
        let path = dir.join("ops.ndjson");
        let ops: Vec<Op> = std::fs::read_to_string(&path).ok()
            .map(|s| s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
            .unwrap_or_default();
        let next_seq = ops.iter().map(|op| op.seq).max().map_or(1, |m| m + 1);
        (Self { path, next_seq }, ops)
    }

    /// Append one op, auto-assigning seq and op_id. Returns the written Op.
    pub fn append(&mut self, kind: OpKind, device_id: &str, ts: u64) -> std::io::Result<Op> {
        self.append_op(crate::project::gen_id(), kind, device_id, ts)
    }

    /// Same as `append`, but takes a pre-generated `op_id`. Used to flush ops
    /// that were queued (with their op_id already handed out for dedup)
    /// while the oplog itself was still loading in the background — see
    /// `OplogState` in `sync/mod.rs`.
    pub fn append_op(&mut self, op_id: String, kind: OpKind, device_id: &str, ts: u64) -> std::io::Result<Op> {
        let op = Op {
            op_id,
            device_id: device_id.to_owned(),
            seq:       self.next_seq,
            ts,
            kind,
        };
        let op_name = match &op.kind {
            OpKind::CreateProject  { .. } => "CREATE_PROJECT",
            OpKind::DeleteProject  { .. } => "DELETE_PROJECT",
            OpKind::RenameProject  { .. } => "RENAME_PROJECT",
            OpKind::RecolorProject { .. } => "RECOLOR_PROJECT",
            OpKind::AddTask        { .. } => "ADD_TASK",
            OpKind::DeleteTask     { .. } => "DELETE_TASK",
            OpKind::CompleteMain   { .. } => "COMPLETE_MAIN",
            OpKind::CompleteSub    { .. } => "COMPLETE_SUB",
            OpKind::PromoteTask    { .. } => "PROMOTE_TASK",
            OpKind::EditTask       { .. } => "EDIT_TASK",
            OpKind::SetRoutine     { .. } => "SET_ROUTINE",
            OpKind::SetSharedSetting{..}  => "SET_SHARED_SETTING",
        };
        crate::clog!("[oplog] append seq={} op={op_name} path={:?}", self.next_seq, self.path);
        let mut line = serde_json::to_string(&op)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        match std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(line.as_bytes()) {
                    crate::clog!("[oplog] write_all FAILED: {e}");
                    return Err(e);
                }
            }
            Err(e) => {
                crate::clog!("[oplog] open FAILED: {e} path={:?}", self.path);
                return Err(e);
            }
        }
        self.next_seq += 1;
        crate::clog!("[oplog] append OK, next_seq={}", self.next_seq);
        Ok(op)
    }

    /// Return all ops with seq >= since. Malformed lines are silently skipped
    /// (forward compatibility: unknown op types fail to deserialise → skip).
    pub fn ops_since(&self, since: u64) -> Vec<Op> {
        let Ok(content) = std::fs::read_to_string(&self.path) else { return vec![]; };
        content.lines()
            .filter_map(|l| serde_json::from_str::<Op>(l).ok())
            .filter(|op| op.seq >= since)
            .collect()
    }

    /// All ops, for replay / compaction.
    pub fn all_ops(&self) -> Vec<Op> { self.ops_since(1) }
}