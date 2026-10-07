use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct ManifestEntry {
    pub name:               String,
    pub color_hex:          String,
    pub task_count:         usize,
    pub has_active_routine: bool,
    #[serde(default)]
    pub order_key:          f64,
    #[serde(default)]
    pub order_key_edited_at: u64,
    #[serde(default)]
    pub created_at:         u64,
    #[serde(default)]
    pub last_edited:        u64,
}

pub type Manifest = HashMap<String, ManifestEntry>;

fn manifest_path() -> std::path::PathBuf {
    super::app_dir().join("projects_index.json")
}

pub fn load() -> Manifest {
    std::fs::read_to_string(manifest_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_whole(m: &Manifest) {
    let dir = super::app_dir();
    let path = manifest_path();
    let tmp  = dir.join("projects_index.json.tmp");
    if let Ok(j) = serde_json::to_string(m) {
        if std::fs::write(&tmp, &j).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

pub fn upsert_entry(id: &str, entry: ManifestEntry) {
    let mut m = load();
    m.insert(id.to_owned(), entry);
    write_whole(&m);
}

pub fn remove_entry(id: &str) {
    let mut m = load();
    if m.remove(id).is_some() {
        write_whole(&m);
    }
}

pub fn rebuild_from(projects: &[crate::project::LoadedProject]) {
    let m: Manifest = projects.iter().map(|p| {
        let task_count = p.main.len() + p.subs.len();
        (p.id.clone(), ManifestEntry {
            name:               p.name.clone(),
            color_hex:          p.color_hex.clone(),
            task_count,
            has_active_routine: p.has_active_routine(),
            order_key:          p.order_key,
            order_key_edited_at: p.order_key_edited_at,
            created_at:         p.created_at,
            last_edited:        p.last_edited,
        })
    }).collect();
    write_whole(&m);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_manifest_entry_without_dates_deserializes_with_zeros() {
        let json = r##"{"p1":{"name":"A","color_hex":"#ffffff","task_count":3,"has_active_routine":false,"order_key":1000.0}}"##;
        let m: Manifest = serde_json::from_str(json).unwrap();
        let e = &m["p1"];
        assert_eq!((e.created_at, e.last_edited), (0, 0));
        assert_eq!(e.order_key_edited_at, 0);
        assert_eq!(e.task_count, 3);
    }

    #[test]
    fn new_fields_roundtrip() {
        let e = ManifestEntry {
            name: "A".into(), color_hex: "#ffffff".into(), task_count: 1,
            has_active_routine: false, order_key: 0.0,
            order_key_edited_at: 30, created_at: 10, last_edited: 20,
        };
        let back: ManifestEntry = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert!(back == e);
        assert_eq!(back.order_key_edited_at, 30);
    }
}
