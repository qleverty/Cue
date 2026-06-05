use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone)]
pub struct DeviceIdentity {
    pub device_id:   String,
    pub device_name: String,
}

impl DeviceIdentity {
    pub fn load_or_create(dir: &Path) -> Self {
        let path = dir.join("identity.json");
        if let Ok(s) = std::fs::read_to_string(&path) {
            if let Ok(id) = serde_json::from_str::<Self>(&s) { return id; }
        }
        let id = Self {
            device_id:   crate::project::gen_id(),
            device_name: hostname(),
        };
        if let Ok(j) = serde_json::to_string(&id) { let _ = std::fs::write(&path, j); }
        id
    }

    pub fn save(&self, dir: &Path) {
        if let Ok(j) = serde_json::to_string(self) {
            let _ = std::fs::write(dir.join("identity.json"), j);
        }
    }
}

fn hostname() -> String {
    // Try platform-specific sources in order
    #[cfg(target_os = "windows")]
    if let Ok(v) = std::env::var("COMPUTERNAME") { return v; }
    if let Ok(s) = std::fs::read_to_string("/etc/hostname") {
        let s = s.trim().to_owned();
        if !s.is_empty() { return s; }
    }
    std::env::var("HOSTNAME").unwrap_or_else(|_| "Cue Device".to_owned())
}