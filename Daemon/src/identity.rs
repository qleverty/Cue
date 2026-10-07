use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone)]
pub struct DeviceIdentity {
    pub device_id:   String,
    pub device_name: String,
}

impl DeviceIdentity {
    pub fn load(dir: &Path) -> Option<Self> {
        let s = std::fs::read_to_string(dir.join("identity.json")).ok()?;
        serde_json::from_str(&s).ok()
    }
}
