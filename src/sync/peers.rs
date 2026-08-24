use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct PeerEntry {
    pub device_id:   String,
    pub device_name: String,
    pub token:       String,
    /// Last known LAN IP — populated by discovery (Stage 4) or set manually.
    pub ip_hint:     Option<String>,
    /// Unix timestamp (seconds) of the last successful op-pull from this peer.
    #[serde(default)]
    pub last_synced_at: Option<u64>,
    /// Обновляется на каждом успешном пуле через /1/hello (см. engine.rs) —
    /// пир может сменить платформу/переустановиться, так что не считаем
    /// зафиксированным раз и навсегда, как token.
    #[serde(default)]
    pub device_type: super::DeviceType,
}

pub struct Peers {
    list: Vec<PeerEntry>,
    dir:  PathBuf,
}

impl Peers {
    pub fn load(dir: &Path) -> Self {
        let list = std::fs::read_to_string(dir.join("trusted_peers.json")).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { list, dir: dir.to_owned() }
    }

    pub fn save(&self) {
        if let Ok(j) = serde_json::to_string_pretty(&self.list) {
            let _ = std::fs::write(self.dir.join("trusted_peers.json"), j);
        }
    }

    pub fn all(&self) -> &[PeerEntry] { &self.list }

    pub fn list_mut(&mut self) -> impl Iterator<Item = &mut PeerEntry> {
        self.list.iter_mut()
    }

    pub fn find_by_token(&self, token: &str) -> Option<&PeerEntry> {
        self.list.iter().find(|p| p.token == token)
    }

    pub fn find_by_id(&self, device_id: &str) -> Option<&PeerEntry> {
        self.list.iter().find(|p| p.device_id == device_id)
    }

    /// Upsert: replaces existing entry for same device_id, else appends.
    pub fn add(&mut self, entry: PeerEntry) {
        match self.list.iter_mut().find(|p| p.device_id == entry.device_id) {
            Some(existing) => *existing = entry,
            None           => self.list.push(entry),
        }
        self.save();
    }

    pub fn remove(&mut self, device_id: &str) {
        self.list.retain(|p| p.device_id != device_id);
        self.save();
    }

    /// Update stored ip_hint after successful contact.
    pub fn update_ip(&mut self, device_id: &str, ip: String) {
        if let Some(p) = self.list.iter_mut().find(|p| p.device_id == device_id) {
            if p.ip_hint.as_deref() != Some(&ip) {
                p.ip_hint = Some(ip);
                self.save();
            }
        }
    }

    /// Record the unix timestamp of the last successful pull from this peer.
    pub fn update_last_synced(&mut self, device_id: &str, ts: u64) {
        if let Some(p) = self.list.iter_mut().find(|p| p.device_id == device_id) {
            p.last_synced_at = Some(ts);
            self.save();
        }
    }
}