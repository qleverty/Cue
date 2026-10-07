use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub fn projects_dir() -> std::path::PathBuf {
    crate::app_dir().join("projects")
}

pub fn current_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn gen_random_string(len: usize) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
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

pub fn gen_token() -> String {
    gen_random_string(24)
}

pub fn is_valid_hex_color(hex: &str) -> bool {
    let h = hex.trim_start_matches('#');
    h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())
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
    pub color_hex:  String,
    pub main:       IndexMap<String, TaskData>,
    pub subs:       IndexMap<String, TaskData>,
    pub created_at: u64,
    pub last_edited: u64,
    pub main_edited_at:      u64,
    pub name_edited_at:      u64,
    pub color_edited_at:     u64,
    pub order_key:           f64,
    pub order_key_edited_at: u64,
}

impl LoadedProject {
    pub fn new(id: String, name: String, color_hex: String, created_at: u64) -> Self {
        Self {
            id, name, color_hex, created_at,
            main: IndexMap::new(), subs: IndexMap::new(),
            last_edited: created_at,
            main_edited_at: 0, name_edited_at: 0, color_edited_at: 0,
            order_key: 0.0, order_key_edited_at: 0,
        }
    }

    pub fn has_active_routine(&self) -> bool {
        self.main.values().chain(self.subs.values())
            .any(|t| t.routine.as_ref().is_some_and(|r| r.active))
    }

    pub fn touch(&mut self, ts: u64) {
        self.last_edited = self.last_edited.max(ts);
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

    pub fn promote_sub(&mut self, i: usize) {
        let (sub_id, sub_task) = self.subs.shift_remove_index(i).unwrap();
        if !self.main.is_empty() {
            let (old_id, mut old_task) = self.main.shift_remove_index(0).unwrap();
            old_task.order_key = self.next_beg_key();
            self.subs.shift_insert(0, old_id, old_task);
        }
        self.main.insert(sub_id, sub_task);
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

    pub fn save(&self) {
        let file = ProjectFile {
            ver: 1,
            name: self.name.clone(),
            color: self.color_hex.clone(),
            tasks: TasksFile { main: self.main.clone(), subs: self.subs.clone() },
            last_edited: self.last_edited,
            created_at: self.created_at,
            main_edited_at:  self.main_edited_at,
            name_edited_at:  self.name_edited_at,
            color_edited_at: self.color_edited_at,
            order_key:           self.order_key,
            order_key_edited_at: self.order_key_edited_at,
        };
        let Ok(json) = serde_json::to_string_pretty(&file) else { return };
        let path = projects_dir().join(format!("{}.json", self.id));
        let tmp  = projects_dir().join(format!("{}.json.tmp", self.id));
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }

        crate::manifest::upsert_entry(&self.id, crate::manifest::ManifestEntry {
            name:               self.name.clone(),
            color_hex:          self.color_hex.clone(),
            task_count:         self.main.len() + self.subs.len(),
            has_active_routine: self.has_active_routine(),
            order_key:          self.order_key,
            order_key_edited_at: self.order_key_edited_at,
            created_at:         self.created_at,
            last_edited:        self.last_edited,
        });
    }
}

pub const COMPACT_STEP: f64 = 1000.0;

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
                last_edited: file.last_edited.max(file.created_at),
                main_edited_at:      file.main_edited_at,
                name_edited_at:      file.name_edited_at,
                color_edited_at:     file.color_edited_at,
                order_key:           file.order_key,
                order_key_edited_at: file.order_key_edited_at,
            })
        })
        .collect()
}
