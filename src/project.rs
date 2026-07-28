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

/// Расписание рутины. week/month/direct — независимые опциональные списки,
/// могут присутствовать одновременно (см. Cue_Routines_Implementation_Plan.txt,
/// раздел 1 — это отличается от исходного design-дока, где был единственный
/// type). Пустой список никогда не хранится как `[]` — либо None, либо
/// непустой Vec.
#[derive(Serialize, Deserialize, Clone, Default)]
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

impl Routine {
    /// true, если во всех трёх списках пусто (значит рутину пора убрать
    /// целиком — см. раздел 6.2 плана: сохранение с пустым редактором = None).
    pub fn is_empty(&self) -> bool {
        self.week.as_ref().map_or(true, |v| v.is_empty())
            && self.month.as_ref().map_or(true, |v| v.is_empty())
            && self.direct.as_ref().map_or(true, |v| v.is_empty())
    }

    /// true, если у рутины есть ТОЛЬКО direct-записи (нет week/month).
    /// Используется, чтобы понять, может ли "исчерпание" direct-дат
    /// привести к удалению задачи целиком (раздел 2.4 плана).
    pub fn is_direct_only(&self) -> bool {
        self.week.is_none() && self.month.is_none() && self.direct.is_some()
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
}

/// Эффективно активна ли задача (для сортировки/выбора следующей main).
/// Обычная задача (без рутины) всегда считается активной.
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
}

pub struct LoadedProject {
    pub id:         String,
    pub name:       String,
    pub color:      Color32,
    pub main:       IndexMap<String, TaskData>,
    pub subs:       IndexMap<String, TaskData>,
    pub color_hex:  String,
    pub created_at: u64,
}

impl LoadedProject {
    pub fn new(id: String, name: String, color: Color32, created_at: u64) -> Self {
        Self {
            color_hex: color32_to_hex(color),
            id, name, color,
            main:       IndexMap::new(),
            subs:       IndexMap::new(),
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



    /// Индекс, на котором заканчивается группа "эффективно активных" задач
    /// в subs (т.е. индекс первой неактивной рутины, либо subs.len(), если
    /// неактивных нет). Новые АКТИВНЫЕ задачи нужно вставлять СЮДА (через
    /// shift_insert), а не в физический конец IndexMap через insert() —
    /// иначе они окажутся после неактивных рутин и сломают инвариант
    /// "активные всегда перед неактивными" (см. раздел 2.5 плана).
    pub(crate) fn active_group_end(&self) -> usize {
        self.subs.iter().position(|(_, t)| !is_active_task(t))
            .unwrap_or(self.subs.len())
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
    }

    pub fn add_task(&mut self, id: String, text: String, s: &Settings) {
        let mut task = TaskData { text, routine: None, created_at: current_time(), order_key: 0.0 };

        if self.main.is_empty() {
            self.main.insert(id, task);
            return;
        }
        if s.replace_main {
            let (old_id, mut old_task) = self.main.shift_remove_index(0).unwrap();
            self.main.insert(id, task);
            match s.new_task_pos {
                NewTaskPos::End => {
                    old_task.order_key = self.next_end_key();
                    let pos = self.active_group_end(); // старая main всегда активна
                    self.subs.shift_insert(pos, old_id, old_task);
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
                    let pos = self.active_group_end(); // новая задача всегда активна
                    self.subs.shift_insert(pos, id, task);
                }
                NewTaskPos::Beginning => {
                    task.order_key = self.next_beg_key();
                    self.subs.shift_insert(0, id, task);
                }
            }
        }
    }

    /// `now` — момент выполнения (см. раздел 2.4/5.2 плана: для локального
    /// клика передаётся routine_scheduler::local_now(), для входящего по
    /// сети CompleteMain-опа — op.ts самого опа).
    pub fn complete_main(&mut self, now: u64) {
        let Some((id, mut task)) = self.main.shift_remove_index(0) else { return; };

        if let Some(routine) = task.routine.as_mut() {
            // Чистка исчерпанных direct-записей — именно при ВЫПОЛНЕНИИ,
            // не при активации (см. решение обсуждения).
            crate::routine_scheduler::prune_expired_direct(routine, now);

            // Исчерпана целиком, только если это был "чистый" Direct
            // (нет week/month вообще) и все даты уже прошли/стёрты.
            let exhausted = routine.week.is_none()
                && routine.month.is_none()
                && routine.direct.as_ref().map_or(true, |d| d.is_empty());

            if !exhausted {
                routine.active = false;
                task.order_key = self.next_end_key();
                self.subs.insert(id, task);
            }
            // если exhausted — задача просто никуда не возвращается (удалена
            // фактом невставки обратно, как обычная выполненная задача)
        }
        // если task.routine было None — обычная задача, старое поведение:
        // просто пропадает, ничего не вставляем обратно.

        // Продвигаем в main первую ЭФФЕКТИВНО АКТИВНУЮ sub-задачу (группа
        // active всегда идёт перед not-active благодаря компаратору сортировки
        // в load_all_projects/insert — см. is_active_task). position() —
        // защитная подстраховка на случай рассинхрона сортировки.
        if let Some(pos) = self.subs.iter().position(|(_, t)| is_active_task(t)) {
            let (next_id, next_task) = self.subs.shift_remove_index(pos).unwrap();
            self.main.insert(next_id, next_task);
        }
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
            let mut proj = LoadedProject {
                id,
                name:       file.name,
                color,
                color_hex:  file.color,
                main:       file.tasks.main,
                subs:       file.tasks.subs,
                created_at: file.created_at,
            };
            // Две группы: сперва все эффективно активные (обычные задачи +
            // рутины с active=true), затем неактивные рутины — внутри каждой
            // группы порядок как раньше (order_key, затем created_at).
            proj.subs.sort_by(|_, a, _, b| {
                let ga = !is_active_task(a);
                let gb = !is_active_task(b);
                ga.cmp(&gb)
                    .then_with(|| a.order_key.partial_cmp(&b.order_key)
                        .unwrap_or(std::cmp::Ordering::Equal))
                    .then_with(|| a.created_at.cmp(&b.created_at))
            });
            Some(proj)
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