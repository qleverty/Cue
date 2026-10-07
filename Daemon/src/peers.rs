use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

pub fn default_port() -> u16 { crate::server::DEFAULT_PORT }

#[derive(Serialize, Deserialize, Clone)]
pub struct PeerEntry {
    pub device_id:   String,
    pub device_name: String,
    pub token:       String,
    pub ip_hint:     Option<String>,
    #[serde(default = "default_port")]
    pub port:        u16,
    #[serde(default)]
    pub last_synced_at: Option<u64>,
    #[serde(default)]
    pub revoked: bool,
    #[serde(default)]
    pub incompatible: bool,
    #[serde(default)]
    pub device_type: DeviceType,
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

    pub fn add(&mut self, entry: PeerEntry) {
        match self.list.iter_mut().find(|p| p.device_id == entry.device_id) {
            Some(existing) => *existing = entry,
            None           => self.list.push(entry),
        }
        self.save();
    }

    #[allow(dead_code)]
    pub fn remove(&mut self, device_id: &str) {
        self.list.retain(|p| p.device_id != device_id);
        self.save();
    }

    pub fn update_addr(&mut self, device_id: &str, ip: &str, port: u16) -> bool {
        let Some(p) = self.list.iter_mut().find(|p| p.device_id == device_id) else { return false; };
        if p.ip_hint.as_deref() == Some(ip) && p.port == port { return false; }
        p.ip_hint = Some(ip.to_owned());
        p.port    = port;
        self.save();
        true
    }

    pub fn apply_poll(&mut self, device_id: &str, u: PollUpdate) -> bool {
        let Some(p) = self.list.iter_mut().find(|p| p.device_id == device_id) else { return false; };
        let mut changed = false;
        if let Some(ts) = u.contact_ts {
            if p.last_synced_at != Some(ts) { p.last_synced_at = Some(ts); changed = true; }
        }
        if let Some(r) = u.revoked {
            if p.revoked != r { p.revoked = r; changed = true; }
        }
        if let Some(i) = u.incompatible {
            if p.incompatible != i { p.incompatible = i; changed = true; }
        }
        changed
    }
}

#[derive(Clone, Copy, Default)]
pub struct PollUpdate {
    pub contact_ts:   Option<u64>,
    pub revoked:      Option<bool>,
    pub incompatible: Option<bool>,
}
