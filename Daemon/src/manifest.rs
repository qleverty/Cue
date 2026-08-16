use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct ManifestEntry {
    pub name:       String,
    pub color_hex:  String,
    pub task_count: usize,
}

pub type Manifest = HashMap<String, ManifestEntry>;

fn manifest_path() -> std::path::PathBuf {
    crate::app_dir().join("projects_index.json")
}

pub fn load() -> Manifest {
    std::fs::read_to_string(manifest_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_whole(m: &Manifest) {
    let dir = crate::app_dir();
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
            name:       p.name.clone(),
            color_hex:  p.color_hex.clone(),
            task_count,
        })
    }).collect();
    write_whole(&m);
}
