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

fn gen_random_string(len: usize) -> String {
    static CTR: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut n = t
        ^ ((std::process::id() as u64) << 32)
        ^ CTR.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9e3779b97f4a7c15);
    const CH: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut id = String::with_capacity(len);
    for _ in 0..len {
        id.push(CH[(n as usize) % 62] as char);
        n = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    }
    id
}

pub const COMPACT_STEP: f64 = 1000.0;
pub const COMPACT_GAP: f64 = 1e-6;

pub fn gen_id() -> String {
    gen_random_string(12)
}

pub fn gen_task_id(project_id: &str) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{project_id}-{}-{ts}", gen_random_string(4))
}

pub fn gen_token() -> String {
    gen_random_string(24)
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

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Cycle {
    #[serde(default)]
    pub every: u64,
    #[serde(default)]
    pub from:  u64,
}

fn is_zero_u64(v: &u64) -> bool { *v == 0 }

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Routine {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub week:   Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub month:  Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle:  Option<Cycle>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub last_triggered_at: u64,
}

impl Routine {
    pub fn is_empty(&self) -> bool {
        self.week.as_ref().map_or(true, |v| v.is_empty())
            && self.month.as_ref().map_or(true, |v| v.is_empty())
            && self.direct.as_ref().map_or(true, |v| v.is_empty())
            && self.cycle.as_ref().map_or(true, |c| c.every == 0)
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TaskData {
    pub text:       String,
    #[serde(default)]
    pub routine:    Option<Routine>,
    pub created_at: u64,
    #[serde(default)]
    pub order_key:  f64,
    #[serde(default)]
    pub text_edited_at:    u64,
    #[serde(default)]
    pub routine_edited_at: u64,
    #[serde(default)]
    pub pos_edited_at:     u64,
    #[serde(default)]
    pub transferred_at:    u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub completed_at:      u64,
}

pub fn is_active_task(t: &TaskData) -> bool {
    t.routine.as_ref().map_or(true, |r| r.active)
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
    #[serde(default)]
    main_edited_at: u64,
    #[serde(default)]
    name_edited_at:  u64,
    #[serde(default)]
    color_edited_at: u64,
    #[serde(default)]
    order_key:       f64,
    #[serde(default)]
    order_key_edited_at: u64,
}

pub struct LoadedProject {
    pub id:         String,
    pub name:       String,
    pub color:      Color32,
    pub main:       IndexMap<String, TaskData>,
    pub subs:       IndexMap<String, TaskData>,
    pub color_hex:  String,
    pub created_at: u64,
    pub last_edited: u64,
    pub loaded:     bool,
    pub main_edited_at: u64,
    pub name_edited_at:  u64,
    pub color_edited_at: u64,
    pub order_key:           f64,
    pub order_key_edited_at: u64,
    pub task_count: usize,
}

impl LoadedProject {
    pub fn apply_edit_task(&mut self, task_id: &str, text: &str, ts: u64) -> bool {
        let Some(t) = self.main.get_mut(task_id).or_else(|| self.subs.get_mut(task_id)) else {
            return false;
        };
        if ts <= t.text_edited_at { return false; }
        t.text = text.to_owned();
        t.text_edited_at = ts;
        true
    }

    pub fn apply_move_task(&mut self, task_id: &str, order_key: f64, ts: u64) -> bool {
        if let Some(t) = self.main.get_mut(task_id) {
            if ts <= t.pos_edited_at { return false; }
            t.order_key     = order_key;
            t.pos_edited_at = ts;
            return true;
        }

        let Some(idx) = self.subs.get_index_of(task_id) else { return false; };
        let Some((_, t)) = self.subs.get_index(idx) else { return false; };
        if ts <= t.pos_edited_at { return false; }

        let Some((id, mut task)) = self.subs.shift_remove_index(idx) else { return false; };
        task.order_key     = order_key;
        task.pos_edited_at = ts;

        let target = self.subs.iter()
            .position(|(_, t)| t.order_key > order_key)
            .unwrap_or(self.subs.len());
        self.subs.shift_insert(target, id, task);
        true
    }

    pub fn apply_compact_order(&mut self, order: &[String], ts: u64) -> bool {
        let mut changed = false;
        for (rank, id) in order.iter().enumerate() {
            let Some(task) = self.subs.get_mut(id.as_str()) else { continue; };
            if ts < task.pos_edited_at { continue; }
            task.order_key     = rank as f64 * COMPACT_STEP;
            task.pos_edited_at = task.pos_edited_at.max(ts);
            changed = true;
        }
        if changed {
            self.subs.sort_by(|_, a, _, b| a.order_key.total_cmp(&b.order_key));
        }
        changed
    }

    pub fn compact_order(&mut self, ts: u64) -> Vec<String> {
        let order: Vec<String> = self.subs.keys().cloned().collect();
        self.apply_compact_order(&order, ts);
        order
    }

    pub fn needs_compaction(&self) -> bool {
        self.subs.values().zip(self.subs.values().skip(1))
            .any(|(a, b)| b.order_key - a.order_key < COMPACT_GAP)
    }

    pub fn apply_promote_task(&mut self, task_id: &str, ts: u64) -> bool {
        if ts <= self.main_edited_at { return false; }
        let Some(i) = self.subs.get_index_of(task_id) else { return false; };
        self.promote_sub(i);
        self.main_edited_at = ts;
        true
    }

    pub fn apply_add_to_main(&mut self, task_id: String, mut task: TaskData, ts: u64) {
        if ts > self.main_edited_at {
            if !self.main.is_empty() {
                let (old_id, mut old) = self.main.shift_remove_index(0).unwrap();
                old.order_key = self.next_end_key();
                self.subs.insert(old_id, old);
            }
            self.main.insert(task_id, task);
            self.main_edited_at = ts;
        } else {
            task.order_key = self.next_end_key();
            self.subs.insert(task_id, task);
        }
    }

    pub fn apply_set_routine(&mut self, task_id: &str, mut routine: Option<Routine>, ts: u64) -> bool {
        let Some(t) = self.main.get_mut(task_id).or_else(|| self.subs.get_mut(task_id)) else {
            return false;
        };
        if ts <= t.routine_edited_at { return false; }
        t.routine_edited_at = ts;

        if let Some(r) = routine.as_mut() {
            r.last_triggered_at = t.routine.as_ref().map(|old| old.last_triggered_at)
                .unwrap_or(r.last_triggered_at);
            let now = crate::routine_scheduler::local_now();
            let utc = current_time();
            if let Some(ago) = crate::routine_scheduler::due_secs_ago(r, t.completed_at, now, utc) {
                r.active = true;
                r.last_triggered_at = now;
                crate::routine_scheduler::prune_expired_direct(r, now);
                if ago <= crate::routine_scheduler::NOTIFY_WINDOW_SECS {
                    crate::notify::send(&t.text, &self.name, &self.color_hex);
                }
            } else {
                r.active = false;
            }
        }
        t.routine = routine;
        true
    }

    pub fn has_active_routine(&self) -> bool {
        self.main.values().chain(self.subs.values())
            .any(|t| t.routine.as_ref().is_some_and(|r| r.active))
    }
    pub fn new(id: String, name: String, color: Color32, created_at: u64) -> Self {
        Self {
            color_hex: color32_to_hex(color),
            id, name, color,
            main:       IndexMap::new(),
            subs:       IndexMap::new(),
            created_at,
            last_edited: created_at,
            loaded:     true,
            main_edited_at: 0, name_edited_at: 0, color_edited_at: 0,
            order_key: 0.0, order_key_edited_at: 0,
            task_count: 0,
        }
    }

    pub fn touch(&mut self, ts: u64) {
        self.last_edited = self.last_edited.max(ts);
    }

    pub fn main_text(&self) -> Option<&str> {
        self.main.values().next().map(|t| t.text.as_str())
    }

    pub fn save(&mut self) {
        self.task_count = self.main.len() + self.subs.len();
        let file = ProjectFile {
            ver:   1,
            name:  self.name.clone(),
            color: self.color_hex.clone(),
            tasks: TasksFile {
                main: self.main.clone(),
                subs: self.subs.clone(),
            },
            last_edited: self.last_edited,
            created_at:  self.created_at,
            main_edited_at: self.main_edited_at, name_edited_at: self.name_edited_at, color_edited_at: self.color_edited_at,
            order_key: self.order_key, order_key_edited_at: self.order_key_edited_at,
        };
        let path = projects_dir().join(format!("{}.json", self.id));
        let tmp  = projects_dir().join(format!("{}.json.tmp", self.id));
        if let Ok(j) = serde_json::to_string(&file) {
            if std::fs::write(&tmp, &j).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        crate::manifest::upsert_entry(&self.id, crate::manifest::ManifestEntry {
            name:               self.name.clone(),
            color_hex:          self.color_hex.clone(),
            task_count:         self.task_count,
            has_active_routine: self.has_active_routine(),
            order_key:          self.order_key,
            order_key_edited_at: self.order_key_edited_at,
            created_at:         self.created_at,
            last_edited:        self.last_edited,
        });
    }



    pub(crate) fn next_end_key(&self) -> f64 {
        self.subs.values().map(|t| t.order_key)
            .reduce(f64::max).map_or(0.0, |m| m + 1000.0)
    }
    pub(crate) fn next_beg_key(&self) -> f64 {
        self.subs.values().map(|t| t.order_key)
            .reduce(f64::min).map_or(0.0, |m| m - 1000.0)
    }

    pub fn delete_file(&self) {
        let path = projects_dir().join(format!("{}.json", self.id));
        let _    = std::fs::remove_file(path);
        crate::manifest::remove_entry(&self.id);
    }

    pub fn add_task(&mut self, id: String, text: String, s: &Settings) {
        let mut task = TaskData {
            text, routine: None, created_at: current_time(), order_key: 0.0,
            text_edited_at: current_time(), routine_edited_at: 0, pos_edited_at: 0,
            transferred_at: 0, completed_at: 0,
        };

        if self.main.is_empty() {
            self.main.insert(id, task);
            self.main_edited_at = current_time();
            return;
        }
        if s.replace_main {
            let (old_id, mut old_task) = self.main.shift_remove_index(0).unwrap();
            self.main.insert(id, task);
            self.main_edited_at = current_time();
            match s.new_task_pos {
                NewTaskPos::End => {
                    old_task.order_key = self.next_end_key();
                    self.subs.insert(old_id, old_task);
                }
                NewTaskPos::Beginning => {
                    old_task.order_key = self.next_beg_key();
                    self.subs.shift_insert(0, old_id, old_task);
                }
            }
        } else {
            match s.new_task_pos {
                NewTaskPos::End => {
                    task.order_key = self.next_end_key();
                    self.subs.insert(id, task);
                }
                NewTaskPos::Beginning => {
                    task.order_key = self.next_beg_key();
                    self.subs.shift_insert(0, id, task);
                }
            }
        }
    }

    pub fn complete_task(&mut self, task_id: &str, now: u64, ts: u64, sender_routine_ts: u64) -> Option<bool> {
        if self.main.contains_key(task_id) {
            let (id, mut task) = self.main.shift_remove_entry(task_id).unwrap();
            let had_routine = Self::counts_as_routine(&task, sender_routine_ts);
            let spent = Self::settle_routine(&mut task, now, ts);
            if had_routine { task.completed_at = task.completed_at.max(ts); }

            if let Some(pos) = self.subs.iter().position(|(_, t)| is_active_task(t)) {
                let (next_id, next_task) = self.subs.shift_remove_index(pos).unwrap();
                self.main.insert(next_id, next_task);
                self.main_edited_at = ts;
            }

            if had_routine {
                task.order_key = self.next_end_key();
                self.subs.insert(id, task);
            }
            return Some(spent);
        }

        let task = self.subs.get_mut(task_id)?;
        if !Self::counts_as_routine(task, sender_routine_ts) {
            self.subs.shift_remove(task_id);
            return Some(false);
        }
        let spent = Self::settle_routine(task, now, ts);
        task.completed_at = task.completed_at.max(ts);
        Some(spent)
    }

    fn counts_as_routine(task: &TaskData, sender_routine_ts: u64) -> bool {
        task.routine.is_some()
            || (sender_routine_ts > 0 && sender_routine_ts > task.routine_edited_at)
    }

    fn settle_routine(task: &mut TaskData, now: u64, ts: u64) -> bool {
        let Some(routine) = task.routine.as_mut() else { return false; };
        crate::routine_scheduler::prune_expired_direct(routine, now);
        routine.active = false;
        if routine.is_empty() && ts >= task.routine_edited_at {
            task.routine = None;
            task.routine_edited_at = ts;
            return true;
        }
        false
    }

    pub fn promote_sub(&mut self, i: usize) {
        let (sub_id, sub_task) = self.subs.shift_remove_index(i).unwrap();
        if !self.main.is_empty() {
            let (old_id, mut old_task) = self.main.shift_remove_index(0).unwrap();
            old_task.order_key = self.next_beg_key();
            self.subs.shift_insert(0, old_id, old_task);
        }
        self.main.insert(sub_id, sub_task);
    }

    pub fn delete_sub(&mut self, i: usize) {
        self.subs.shift_remove_index(i);
    }

    pub fn edit_sub(&mut self, i: usize, text: String) {
        if let Some((_, task)) = self.subs.get_index_mut(i) {
            task.text = text;
        }
    }

    pub fn reorder_sub(&mut self, from: usize, to_before: Option<usize>, ts: u64) -> Option<(String, f64)> {
        if Some(from) == to_before { return None; }
        let (id, mut task) = self.subs.shift_remove_index(from)?;

        let target = match to_before {
            None => self.subs.len(), 
            Some(before) if before > from => before - 1,
            Some(before) => before,
        };

        let prev_key = target.checked_sub(1)
            .and_then(|i| self.subs.get_index(i))
            .map(|(_, t)| t.order_key);
        let next_key = self.subs.get_index(target).map(|(_, t)| t.order_key);
        task.order_key = match (prev_key, next_key) {
            (Some(p), Some(n)) => (p + n) / 2.0,
            (Some(p), None)    => p + 1000.0,
            (None, Some(n))    => n - 1000.0,
            (None, None)       => 0.0,
        };
        task.pos_edited_at = ts;

        let order_key = task.order_key;
        self.subs.shift_insert(target, id.clone(), task);
        Some((id, order_key))
    }

}

pub fn load_one(id: &str) -> Option<LoadedProject> {
    let path = projects_dir().join(format!("{id}.json"));
    let text = std::fs::read_to_string(&path).ok()?;
    let file: ProjectFile = serde_json::from_str(&text).ok()?;
    let color = hex_to_color32(&file.color)?;
    let task_count = file.tasks.main.len() + file.tasks.subs.len();
    Some(LoadedProject {
        id: id.to_owned(),
        name:       file.name,
        color,
        color_hex:  file.color,
        main:       file.tasks.main,
        subs:       file.tasks.subs,
        created_at: file.created_at,
        last_edited: file.last_edited.max(file.created_at),
        loaded:     true,
        main_edited_at: file.main_edited_at, name_edited_at: file.name_edited_at, color_edited_at: file.color_edited_at,
        order_key: file.order_key, order_key_edited_at: file.order_key_edited_at,
        task_count,
    })
}

const STARTUP_LOAD_ATTEMPTS: usize = 3;

pub fn load_active_with_fallback(
    manifest:     &crate::manifest::Manifest,
    preferred_id: Option<&str>,
) -> LoadedProject {
    let mut rest: Vec<&str> = manifest.keys()
        .map(String::as_str)
        .filter(|id| Some(*id) != preferred_id)
        .collect();
    rest.sort_by(|a, b| manifest[*a].order_key.partial_cmp(&manifest[*b].order_key)
        .unwrap_or(std::cmp::Ordering::Equal));

    preferred_id.into_iter().chain(rest)
        .take(STARTUP_LOAD_ATTEMPTS)
        .find_map(load_one)
        .unwrap_or_else(create_default_project)
}

pub fn resolve_active_project(projects: &[LoadedProject], settings: &crate::settings::Settings) -> Option<usize> {
    if projects.is_empty() { return None; }
    Some(settings.preferred_project_id()
        .and_then(|id| projects.iter().position(|p| p.id == id))
        .unwrap_or(0))
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
            let task_count = file.tasks.main.len() + file.tasks.subs.len();
            let proj = LoadedProject {
                id,
                name:       file.name,
                color,
                color_hex:  file.color,
                main:       file.tasks.main,
                subs:       file.tasks.subs,
                created_at: file.created_at,
                last_edited: file.last_edited.max(file.created_at),
                loaded:     true,
                main_edited_at: file.main_edited_at, name_edited_at: file.name_edited_at, color_edited_at: file.color_edited_at,
                order_key: file.order_key, order_key_edited_at: file.order_key_edited_at,
                task_count,
            };
            Some(proj)
        })
        .collect()
}

pub fn create_default_project() -> LoadedProject {
    let _ = std::fs::create_dir_all(projects_dir());
    let mut proj = LoadedProject::new(
        gen_id(),
        "Cue".to_owned(),
        Color32::from_rgb(74, 144, 217),
        current_time(),
    );
    proj.save();
    proj
}

pub fn apply_compact_projects_order(
    projects: &mut [LoadedProject],
    order:    &[String],
    ts:       u64,
) -> Vec<String> {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut changed = Vec::new();
    for (rank, id) in order.iter().enumerate() {
        if !seen.insert(id.as_str()) { continue; }
        let Some(p) = projects.iter_mut().find(|p| &p.id == id) else { continue; };
        if ts < p.order_key_edited_at { continue; }
        let new_key   = rank as f64 * COMPACT_STEP;
        let new_stamp = p.order_key_edited_at.max(ts);
        if p.order_key != new_key || p.order_key_edited_at != new_stamp {
            changed.push(id.clone());
        }
        p.order_key           = new_key;
        p.order_key_edited_at = new_stamp;
    }
    changed
}

#[cfg(test)]
mod compact_tests {
    use super::*;

    fn task(key: f64, stamp: u64) -> TaskData {
        TaskData {
            text: String::new(), routine: None, created_at: 0, order_key: key,
            text_edited_at: 0, routine_edited_at: 0, pos_edited_at: stamp,
            transferred_at: 0, completed_at: 0,
        }
    }

    fn proj(subs: &[(&str, f64, u64)]) -> LoadedProject {
        let mut p = LoadedProject::new("p".into(), "p".into(), Color32::WHITE, 0);
        for (id, key, stamp) in subs {
            p.subs.insert((*id).to_string(), task(*key, *stamp));
        }
        p
    }

    fn ids(p: &LoadedProject) -> Vec<&str> { p.subs.keys().map(String::as_str).collect() }
    fn keys(p: &LoadedProject) -> Vec<f64> { p.subs.values().map(|t| t.order_key).collect() }
    fn order(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn needs_compaction_detects_narrow_equal_and_unsorted() {
        assert!(!proj(&[]).needs_compaction());
        assert!(!proj(&[("a", 5.0, 0)]).needs_compaction());
        assert!(!proj(&[("a", 0.0, 0), ("b", 1000.0, 0), ("c", 2000.0, 0)]).needs_compaction());
        assert!(proj(&[("a", 0.0, 0), ("b", 1e-7, 0)]).needs_compaction());
        assert!(proj(&[("a", 7.0, 0), ("b", 7.0, 0)]).needs_compaction());
        assert!(proj(&[("a", 9.0, 0), ("b", 1.0, 0)]).needs_compaction());
        assert!(!proj(&[("a", -2000.0, 0), ("b", -1000.0, 0), ("c", 0.0, 0)]).needs_compaction());
    }

    #[test]
    fn compact_assigns_ranks_stamps_and_sorts() {
        let mut p = proj(&[("a", 10.5, 1), ("b", 10.6, 1), ("c", 99.0, 1)]);
        assert!(p.apply_compact_order(&order(&["c", "a", "b"]), 50));
        assert_eq!(ids(&p), ["c", "a", "b"]);
        assert_eq!(keys(&p), [0.0, 1000.0, 2000.0]);
        assert!(p.subs.values().all(|t| t.pos_edited_at == 50));
    }

    #[test]
    fn compact_respects_lww_strictly_newer_skipped_equal_applied() {
        let mut p = proj(&[("a", 1.0, 10), ("b", 2.0, 60), ("c", 3.0, 50)]);
        p.apply_compact_order(&order(&["a", "b", "c"]), 50);
        let b = &p.subs["b"];
        assert_eq!((b.order_key, b.pos_edited_at), (2.0, 60));
        let c = &p.subs["c"];
        assert_eq!((c.order_key, c.pos_edited_at), (2000.0, 50));
        assert_eq!(p.subs["a"].order_key, 0.0);
    }

    #[test]
    fn compact_unknown_ids_keep_rank_and_unlisted_untouched() {
        let mut p = proj(&[("a", 0.3, 0), ("b", 0.4, 0), ("d", 0.5, 0), ("extra", 7777.0, 3)]);
        p.apply_compact_order(&order(&["a", "b", "ghost", "d"]), 9);
        assert_eq!(p.subs["a"].order_key, 0.0);
        assert_eq!(p.subs["b"].order_key, 1000.0);
        assert_eq!(p.subs["d"].order_key, 3000.0);
        assert_eq!((p.subs["extra"].order_key, p.subs["extra"].pos_edited_at), (7777.0, 3));
        assert_eq!(ids(&p), ["a", "b", "d", "extra"]);
    }

    #[test]
    fn compact_is_idempotent() {
        let mut p = proj(&[("a", 0.1, 0), ("b", 0.2, 0), ("c", 0.3, 0)]);
        p.apply_compact_order(&order(&["a", "b", "c"]), 5);
        let (k1, i1) = (keys(&p), ids(&p).iter().map(|s| s.to_string()).collect::<Vec<_>>());
        p.apply_compact_order(&order(&["a", "b", "c"]), 5);
        assert_eq!(keys(&p), k1);
        assert_eq!(ids(&p), i1.iter().map(String::as_str).collect::<Vec<_>>());
    }

    #[test]
    fn compact_ignores_main_and_empty_list() {
        let mut p = proj(&[("a", 0.1, 0)]);
        p.main.insert("m".into(), task(0.5, 0));
        assert!(!p.apply_compact_order(&order(&["m", "zzz"]), 5));
        assert_eq!(p.main["m"].order_key, 0.5);
        assert!(!p.apply_compact_order(&[], 5));
    }

    #[test]
    fn repeated_drag_into_same_gap_triggers_compaction_before_ties() {
        let mut p = proj(&[("a", 0.0, 0), ("b", 1000.0, 0)]);
        for i in 0..60 { p.subs.insert(format!("x{i}"), task(2000.0 + i as f64 * 1000.0, 0)); }

        let mut compacted_at = None;
        for i in 0..60 {
            let from = p.subs.get_index_of(format!("x{i}").as_str()).unwrap();
            p.reorder_sub(from, Some(1), 100 + i as u64).unwrap();
            let ks = keys(&p);
            assert!(ks.windows(2).all(|w| w[0] < w[1]), "ничья/инверсия на шаге {i}: {ks:?}");
            if p.needs_compaction() {
                let before: Vec<String> = p.subs.keys().cloned().collect();
                let sent = p.compact_order(100 + i as u64);
                assert_eq!(sent, before, "компакция не должна менять физический порядок");
                assert_eq!(ids(&p), before.iter().map(String::as_str).collect::<Vec<_>>());
                assert!(!p.needs_compaction());
                assert_eq!(keys(&p)[1], 1000.0);
                compacted_at = Some(i);
                break;
            }
        }
        let at = compacted_at.expect("компакция так и не сработала");
        assert!((20..=40).contains(&at), "сработала на шаге {at}, ожидалось ~30");
    }

    #[test]
    fn sender_and_receiver_converge_in_any_arrival_order() {
        let base = [("a", 0.0, 1), ("b", 1e-9, 1), ("c", 2e-9, 1), ("d", 3e-9, 1)];
        let mut sender = proj(&base);
        let mut recv_ab = proj(&base);
        let mut recv_ba = proj(&base);

        let ts = 500;
        let (moved, key) = sender.reorder_sub(3, Some(1), ts).unwrap();
        assert!(sender.needs_compaction());
        let list = sender.compact_order(ts);

        recv_ab.apply_move_task(&moved, key, ts);
        recv_ab.apply_compact_order(&list, ts);
        recv_ba.apply_compact_order(&list, ts);
        recv_ba.apply_move_task(&moved, key, ts);

        for r in [&recv_ab, &recv_ba] {
            assert_eq!(ids(r), ids(&sender));
            assert_eq!(keys(r), keys(&sender));
        }
        assert_eq!(ids(&sender), ["a", "d", "b", "c"]);
        assert_eq!(keys(&sender), [0.0, 1000.0, 2000.0, 3000.0]);

        assert!(!sender.apply_move_task("c", 1e-9, ts - 10));
        assert_eq!(keys(&sender), [0.0, 1000.0, 2000.0, 3000.0]);
    }

    #[test]
    fn compact_order_op_roundtrips_through_json() {
        use crate::sync::oplog::OpKind;
        let op = OpKind::CompactOrder { project_id: "p".into(), order: order(&["a", "b"]) };
        let line = serde_json::to_string(&op).unwrap();
        assert!(line.contains("COMPACT_ORDER"), "{line}");
        match serde_json::from_str::<OpKind>(&line).unwrap() {
            OpKind::CompactOrder { project_id, order: o } => {
                assert_eq!(project_id, "p");
                assert_eq!(o, order(&["a", "b"]));
            }
            _ => panic!("не тот вариант"),
        }
        assert_eq!(op.project_id(), Some("p"));
        assert_eq!(op.task_id(), None);
    }

    #[test]
    fn move_task_already_applied_then_compaction_on_stale_receiver() {
        let mut sender = proj(&[("a", 0.0, 1), ("b", 1.0, 1), ("c", 2.0, 1), ("d", 3.0, 1)]);
        let mut recv   = proj(&[("a", 0.0, 1), ("b", 1.0, 1), ("d", 3.0, 1)]);
        let list = sender.compact_order(10);
        recv.apply_compact_order(&list, 10);
        assert_eq!(recv.subs["d"].order_key, sender.subs["d"].order_key);
        assert_eq!(recv.subs["b"].order_key, sender.subs["b"].order_key);
    }
}

#[cfg(test)]
mod compact_projects_tests {
    use super::*;
    fn pr(id: &str, key: f64, stamp: u64) -> LoadedProject {
        let mut p = LoadedProject::new(id.into(), id.into(), Color32::WHITE, 0);
        p.order_key = key;
        p.order_key_edited_at = stamp;
        p
    }
    fn ord(ids: &[&str]) -> Vec<String> { ids.iter().map(|s| s.to_string()).collect() }
    fn key_of(ps: &[LoadedProject], id: &str) -> f64 { ps.iter().find(|p| p.id == id).unwrap().order_key }
    fn stamp_of(ps: &[LoadedProject], id: &str) -> u64 { ps.iter().find(|p| p.id == id).unwrap().order_key_edited_at }
    fn ids(ps: &[LoadedProject]) -> Vec<&str> { ps.iter().map(|p| p.id.as_str()).collect() }

    #[test]
    fn ranks_follow_list_order() {
        let mut ps = vec![pr("a", 0.0, 0), pr("b", 0.0, 0), pr("c", 0.0, 0)];
        let changed = apply_compact_projects_order(&mut ps, &ord(&["c", "a", "b"]), 10);
        assert_eq!((key_of(&ps, "c"), key_of(&ps, "a"), key_of(&ps, "b")), (0.0, 1000.0, 2000.0));
        assert_eq!(stamp_of(&ps, "a"), 10);
        assert_eq!(changed.len(), 3);
    }

    #[test]
    fn unknown_ids_do_not_shift_ranks() {
        let mut ps = vec![pr("a", 5.0, 0), pr("c", 5.0, 0)];
        apply_compact_projects_order(&mut ps, &ord(&["a", "ghost", "c"]), 10);
        assert_eq!(key_of(&ps, "a"), 0.0);
        assert_eq!(key_of(&ps, "c"), 2000.0);
    }

    #[test]
    fn projects_not_in_list_are_untouched() {
        let mut ps = vec![pr("a", 7.0, 3), pr("extra", 7777.0, 3)];
        let changed = apply_compact_projects_order(&mut ps, &ord(&["a"]), 10);
        assert_eq!((key_of(&ps, "extra"), stamp_of(&ps, "extra")), (7777.0, 3));
        assert_eq!(changed, ord(&["a"]));
    }

    #[test]
    fn stale_stamp_skipped_equal_passes() {
        let mut ps = vec![pr("new", 42.0, 50), pr("eq", 42.0, 10), pr("old", 42.0, 5)];
        apply_compact_projects_order(&mut ps, &ord(&["new", "eq", "old"]), 10);
        assert_eq!((key_of(&ps, "new"), stamp_of(&ps, "new")), (42.0, 50));
        assert_eq!((key_of(&ps, "eq"), stamp_of(&ps, "eq")), (1000.0, 10));
        assert_eq!((key_of(&ps, "old"), stamp_of(&ps, "old")), (2000.0, 10));
    }

    #[test]
    fn stamp_is_never_lowered() {
        let mut ps = vec![pr("a", 1.0, 100)];
        apply_compact_projects_order(&mut ps, &ord(&["a"]), 100);
        assert_eq!(stamp_of(&ps, "a"), 100);
        apply_compact_projects_order(&mut ps, &ord(&["x", "a"]), 50);
        assert_eq!((key_of(&ps, "a"), stamp_of(&ps, "a")), (0.0, 100));
    }

    #[test]
    fn returns_only_changed_ids() {
        let mut ps = vec![pr("a", 0.0, 10), pr("b", 1000.0, 5), pr("c", 9.0, 10)];
        let changed = apply_compact_projects_order(&mut ps, &ord(&["a", "b", "c"]), 10);
        assert_eq!(changed, ord(&["b", "c"]));
    }

    #[test]
    fn physical_order_is_not_changed() {
        let mut ps = vec![pr("a", 0.0, 0), pr("b", 0.0, 0), pr("c", 0.0, 0)];
        apply_compact_projects_order(&mut ps, &ord(&["c", "b", "a"]), 10);
        assert_eq!(ids(&ps), ["a", "b", "c"]);
    }

    #[test]
    fn duplicate_id_first_occurrence_wins() {
        let mut ps = vec![pr("a", 0.0, 0), pr("b", 0.0, 0)];
        let changed = apply_compact_projects_order(&mut ps, &ord(&["a", "b", "a"]), 10);
        assert_eq!((key_of(&ps, "a"), key_of(&ps, "b")), (0.0, 1000.0));
        assert_eq!(changed.iter().filter(|i| *i == "a").count(), 1);
    }

    #[test]
    fn empty_inputs_do_not_panic() {
        let mut none: Vec<LoadedProject> = vec![];
        assert!(apply_compact_projects_order(&mut none, &ord(&["a"]), 1).is_empty());
        let mut ps = vec![pr("a", 1.0, 0)];
        assert!(apply_compact_projects_order(&mut ps, &[], 1).is_empty());
        assert_eq!(key_of(&ps, "a"), 1.0);
    }
}
