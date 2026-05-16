use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use eframe::egui::Color32;
use std::sync::atomic::{AtomicU64, Ordering};
use crate::settings::{NewTaskPos, Settings};

pub fn current_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn gen_id() -> String {
    static CTR: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut n = t
        ^ ((std::process::id() as u64) << 32)
        ^ CTR.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9e3779b97f4a7c15);
    const CH: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut id = String::with_capacity(12);
    for _ in 0..12 {
        id.push(CH[(n as usize) % 62] as char);
        n = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    }
    id
}

pub fn hex_to_color32(hex: &str) -> Option<Color32> {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 { return None; }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

pub fn color32_to_hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

pub fn projects_dir() -> std::path::PathBuf {
    super::app_dir().join("projects")
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TaskData {
    pub text:       String,
    pub active:     bool,
    pub schedule:   Option<serde_json::Value>,
    pub created_at: u64,
}

#[derive(Serialize, Deserialize)]
struct TasksFile {
    main: IndexMap<String, TaskData>,
    subs: IndexMap<String, TaskData>,
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    ver:        u32,
    name:       String,
    color:      String,
    tasks:      TasksFile,
    tombstones: serde_json::Value,
    #[serde(default)]
    last_edited: u64,
    #[serde(default)]
    created_at:  u64,
}

pub struct LoadedProject {
    pub id:    String,
    pub name:  String,
    pub color: Color32,
    pub main:  IndexMap<String, TaskData>,
    pub subs:  IndexMap<String, TaskData>,
    color_hex:  String,
    tombstones: serde_json::Value,
    created_at: u64,
}

impl LoadedProject {
    pub fn new(id: String, name: String, color: Color32, created_at: u64) -> Self {
        Self {
            color_hex: color32_to_hex(color),
            id, name, color,
            main:       IndexMap::new(),
            subs:       IndexMap::new(),
            tombstones: serde_json::json!({}),
            created_at,
        }
    }

    pub fn main_text(&self) -> Option<&str> {
        self.main.values().next().map(|t| t.text.as_str())
    }

    pub fn save(&self) {
        let file = ProjectFile {
            ver:   1,
            name:  self.name.clone(),
            color: self.color_hex.clone(),
            tasks: TasksFile {
                main: self.main.clone(),
                subs: self.subs.clone(),
            },
            tombstones:  self.tombstones.clone(),
            last_edited: current_time(),
            created_at:  self.created_at,
        };
        let path = projects_dir().join(format!("{}.json", self.id));
        let tmp  = projects_dir().join(format!("{}.json.tmp", self.id));
        if let Ok(j) = serde_json::to_string(&file) {
            if std::fs::write(&tmp, &j).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }



    pub fn delete_file(&self) {
        let path = projects_dir().join(format!("{}.json", self.id));
        let _    = std::fs::remove_file(path);
    }

    pub fn add_task(&mut self, text: String, s: &Settings) {
        let task = TaskData { text, active: true, schedule: None, created_at: current_time() };
        let id   = gen_id();

        if self.main.is_empty() {
            self.main.insert(id, task);
            return;
        }
        if s.replace_main {
            let (old_id, old_task) = self.main.shift_remove_index(0).unwrap();
            self.main.insert(id, task);
            match s.new_task_pos {
                NewTaskPos::End       => { self.subs.insert(old_id, old_task); }
                NewTaskPos::Beginning => { self.subs.shift_insert(0, old_id, old_task); }
            }
        } else {
            match s.new_task_pos {
                NewTaskPos::End       => { self.subs.insert(id, task); }
                NewTaskPos::Beginning => { self.subs.shift_insert(0, id, task); }
            }
        }
    }

    pub fn complete_main(&mut self) {
        self.main.clear();
        if !self.subs.is_empty() {
            let (id, task) = self.subs.shift_remove_index(0).unwrap();
            self.main.insert(id, task);
        }
    }

    pub fn promote_sub(&mut self, i: usize) {
        let (sub_id, sub_task) = self.subs.shift_remove_index(i).unwrap();
        if !self.main.is_empty() {
            let (old_id, old_task) = self.main.shift_remove_index(0).unwrap();
            self.subs.shift_insert(i, old_id, old_task);
        }
        self.main.insert(sub_id, sub_task);
    }

    pub fn delete_sub(&mut self, i: usize) {
        self.subs.shift_remove_index(i);
    }
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
            let color = hex_to_color32(&file.color)?;
            Some(LoadedProject {
                id,
                name:       file.name,
                color,
                color_hex:  file.color,
                main:       file.tasks.main,
                subs:       file.tasks.subs,
                tombstones: file.tombstones,
                created_at: file.created_at,
            })
        })
        .collect()
}

pub fn create_default_project() -> LoadedProject {
    let _ = std::fs::create_dir_all(projects_dir());
    let proj = LoadedProject::new(
        gen_id(),
        "Cue".to_owned(),
        Color32::from_rgb(74, 144, 217),
        current_time(),
    );
    proj.save();
    proj
}