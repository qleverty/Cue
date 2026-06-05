use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TombKind { Project, Task }

#[derive(Serialize, Deserialize, Clone)]
pub struct TombEntry {
    #[serde(rename = "type")]
    pub kind:       TombKind,
    pub deleted_at: u64,
    pub deleted_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

pub struct Tombstones {
    pub map: HashMap<String, TombEntry>,
    dir:     PathBuf,
}

impl Tombstones {
    pub fn load(dir: &Path) -> Self {
        let map = std::fs::read_to_string(dir.join("tombstones.json")).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { map, dir: dir.to_owned() }
    }

    pub fn save(&self) {
        if let Ok(j) = serde_json::to_string(&self.map) {
            let _ = std::fs::write(self.dir.join("tombstones.json"), j);
        }
    }

    pub fn add_project(&mut self, id: &str, deleted_at: u64, deleted_by: &str) {
        self.map.insert(id.to_owned(), TombEntry {
            kind: TombKind::Project, deleted_at,
            deleted_by: deleted_by.to_owned(), project_id: None,
        });
        self.save();
    }

    pub fn add_task(&mut self, id: &str, project_id: &str, deleted_at: u64, deleted_by: &str) {
        self.map.insert(id.to_owned(), TombEntry {
            kind: TombKind::Task, deleted_at,
            deleted_by: deleted_by.to_owned(),
            project_id: Some(project_id.to_owned()),
        });
        self.save();
    }

    /// Returns deletion timestamp if the id is tombstoned.
    pub fn deleted_at(&self, id: &str) -> Option<u64> {
        self.map.get(id).map(|e| e.deleted_at)
    }

    /// Remove entries older than `cutoff_ts` only if all peers have
    /// confirmed (cursor >= seq of corresponding DELETE op).
    /// Called from compaction in Stage 6.
    pub fn prune(&mut self, cutoff_ts: u64) {
        self.map.retain(|_, e| e.deleted_at >= cutoff_ts);
        self.save();
    }
}