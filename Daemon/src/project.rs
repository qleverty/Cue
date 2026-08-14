use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub fn projects_dir() -> std::path::PathBuf {
    crate::app_dir().join("projects")
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Routine {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week:   Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub month:  Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct: Option<Vec<String>>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub last_triggered_at: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TaskData {
    pub text:       String,
    #[serde(default)]
    pub routine:    Option<Routine>,
    pub created_at: u64,
    #[serde(default)]
    pub order_key:  f64,
}

#[derive(Serialize, Deserialize)]
struct TasksFile {
    main: IndexMap<String, TaskData>,
    subs: IndexMap<String, TaskData>,
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    ver:   u32,
    name:  String,
    color: String,
    tasks: TasksFile,
    #[serde(default)]
    last_edited: u64,
    #[serde(default)]
    created_at:  u64,
}

pub struct LoadedProject {
    pub id:         String,
    pub name:       String,
    pub color_hex:  String,
    pub main:       IndexMap<String, TaskData>,
    pub subs:       IndexMap<String, TaskData>,
    pub created_at: u64,
}

pub fn load_all_projects() -> Vec<LoadedProject> {
    let Ok(entries) = std::fs::read_dir(projects_dir()) else { return vec![]; };

    entries.flatten()
        .filter_map(|e| {
            let path = e.path();
            let fname = path.file_name()?.to_str()?;
            if !fname.ends_with(".json") || fname.ends_with(".json.tmp") { return None; }
            let id   = path.file_stem()?.to_str()?.to_owned();
            let text = std::fs::read_to_string(&path).ok()?;
            let file: ProjectFile = serde_json::from_str(&text).ok()?;
            Some(LoadedProject {
                id,
                name:       file.name,
                color_hex:  file.color,
                main:       file.tasks.main,
                subs:       file.tasks.subs,
                created_at: file.created_at,
            })
        })
        .collect()
}
