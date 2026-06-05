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
    #[serde(rename = "PROMOTE_TASK")]
    PromoteTask    { project_id: String, task_id: String },
    #[serde(rename = "EDIT_TASK")]
    EditTask       { project_id: String, task_id: String, text: String },
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
    /// Open (or create) the log. Scans existing entries to find next_seq.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("ops.ndjson");
        let next_seq = std::fs::read_to_string(&path).ok()
            .map(|s| {
                s.lines()
                    .filter_map(|l| serde_json::from_str::<Op>(l).ok())
                    .map(|op| op.seq)
                    .max()
                    .map_or(1, |m| m + 1)
            })
            .unwrap_or(1);
        Self { path, next_seq }
    }

    /// Append one op, auto-assigning seq. Returns the written Op.
    pub fn append(&mut self, kind: OpKind, device_id: &str, ts: u64) -> std::io::Result<Op> {
        let op = Op {
            op_id:     crate::project::gen_id(),
            device_id: device_id.to_owned(),
            seq:       self.next_seq,
            ts,
            kind,
        };
        let mut line = serde_json::to_string(&op)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        std::fs::OpenOptions::new()
            .create(true).append(true).open(&self.path)?
            .write_all(line.as_bytes())?;
        self.next_seq += 1;
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