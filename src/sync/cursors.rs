use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Default)]
struct Inner {
    cursors:          HashMap<String, u64>, // device_id → last seen seq
    last_compact_seq: u64,
}

pub struct Cursors {
    inner: Inner,
    dir:   PathBuf,
}

impl Cursors {
    pub fn load(dir: &Path) -> Self {
        let inner = std::fs::read_to_string(dir.join("sync_cursors.json")).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { inner, dir: dir.to_owned() }
    }

    fn save(&self) {
        if let Ok(j) = serde_json::to_string(&self.inner) {
            let _ = std::fs::write(self.dir.join("sync_cursors.json"), j);
        }
    }

    pub fn get(&self, device_id: &str) -> u64 {
        self.inner.cursors.get(device_id).copied().unwrap_or(0)
    }

    pub fn set(&mut self, device_id: &str, seq: u64) {
        self.inner.cursors.insert(device_id.to_owned(), seq);
        self.save();
    }

    pub fn min_cursor(&self) -> u64 {
        self.inner.cursors.values().copied().min().unwrap_or(0)
    }

    pub fn last_compact_seq(&self) -> u64 { self.inner.last_compact_seq }
    pub fn set_last_compact_seq(&mut self, seq: u64) {
        self.inner.last_compact_seq = seq;
        self.save();
    }
}