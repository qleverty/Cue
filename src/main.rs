#![windows_subsystem = "windows"]

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod settings;
pub mod project;
pub mod project_sort;
pub mod manifest;
pub mod routine_scheduler;
pub mod notify;
pub mod icon_cache;
pub mod exclusive_bind;
pub mod sync;
pub mod ui;
pub mod updater;
pub mod daemon_liveness;
pub mod daemon_updater;
pub mod autostart;

// ── File logger (GUI app has no console on Windows) ──────────────────────────

static LOG_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

pub fn write_log(msg: &str) {
    let Some(path) = LOG_PATH.get() else { return };
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default().as_secs();
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

#[macro_export]
macro_rules! clog {
    ($($arg:tt)*) => { crate::write_log(&format!($($arg)*)) };
}

struct FileLogger;
impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            write_log(&format!("[{}] {} — {}", record.level(), record.target(), record.args()));
        }
    }
    fn flush(&self) {}
}
static FILE_LOGGER: FileLogger = FileLogger;

use eframe::egui::{
    self, Align, Color32, ImageSource, Layout, RichText,
    Sense, Stroke, Ui, ViewportCommand, vec2,
};
use serde::{Deserialize, Serialize};
use std::mem;

pub const W:     f32   = 254.0;
pub const MIN_W: f32   = 180.0;
pub const ROW:   f32   = 28.0;
pub const BG:  Color32 = Color32::from_rgba_premultiplied(9, 9, 9, 222);
pub const SEP: Color32 = Color32::from_rgba_premultiplied(255, 255, 255, 10);

static TICK_PNG:  &[u8] = include_bytes!("../pics/tick.png");
pub static CROSS_PNG: &[u8] = include_bytes!("../pics/cross.png");
static TICK_SMALL_PNG: &[u8] = include_bytes!("../pics/tick_small.png");
static PENCIL_PNG: &[u8] = include_bytes!("../pics/pencil.png");
static CLOCK_PNG: &[u8] = include_bytes!("../pics/clock.png");
static CROSS_LIGHT_PNG:  &[u8] = include_bytes!("../pics/cross_light.png");
static CLOCK_LIGHT_PNG:  &[u8] = include_bytes!("../pics/clock_light.png");
static PENCIL_LIGHT_PNG: &[u8] = include_bytes!("../pics/pencil_light.png");
pub(crate) static ICON_PNG:  &[u8] = include_bytes!("../icon.png");

// ── paths ─────────────────────────────────────────────────────────────────────

pub fn app_dir() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("Cue")
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| std::path::PathBuf::from(h).join(".config"))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
            })
            .join("cue")
    }
}

fn lock_path() -> std::path::PathBuf { app_dir().join("cue.lock") }

// ── lock ─────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct LockData { pid: u32, time: u64 }

fn read_lock() -> Option<LockData> {
    serde_json::from_str(&std::fs::read_to_string(lock_path()).ok()?).ok()
}
fn write_lock() {
    let d = LockData {
        pid:  std::process::id(),
        time: std::time::SystemTime::now()
                  .duration_since(std::time::UNIX_EPOCH)
                  .unwrap_or_default().as_secs(),
    };
    if let Ok(j) = serde_json::to_string(&d) { let _ = std::fs::write(lock_path(), j); }
}
fn delete_lock() { let _ = std::fs::remove_file(lock_path()); }
fn lock_is_fresh(l: &LockData) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    now.saturating_sub(l.time) < 3600
}

#[cfg(target_os = "windows")]
fn is_alive(pid: u32) -> bool {
    type HANDLE = *mut u8;
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> HANDLE;
        fn CloseHandle(h: HANDLE) -> i32;
    }
    unsafe {
        let h = OpenProcess(0x00100000, 0, pid);
        if h.is_null() { return false; }
        CloseHandle(h);
        true
    }
}
#[cfg(target_os = "linux")]
fn is_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn is_alive(_: u32) -> bool { false }

// ── focus ─────────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod focus {
    use std::sync::atomic::{AtomicU32, Ordering};
    static TARGET: AtomicU32 = AtomicU32::new(0);

    type HWND   = *mut u8;
    type LPARAM = isize;
    type BOOL   = i32;

    unsafe extern "system" {
        fn EnumWindows(cb: unsafe extern "system" fn(HWND, LPARAM) -> BOOL, l: LPARAM) -> BOOL;
        fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
        fn SetForegroundWindow(hwnd: HWND) -> BOOL;
        fn IsWindowVisible(hwnd: HWND) -> BOOL;
    }

    unsafe extern "system" fn cb(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == TARGET.load(Ordering::Relaxed) && IsWindowVisible(hwnd) != 0 {
                SetForegroundWindow(hwnd);
                return 0;
            }
        }
        1
    }

    pub fn focus_pid(pid: u32) {
        TARGET.store(pid, Ordering::Relaxed);
        unsafe { EnumWindows(cb, 0); }
    }
}

#[cfg(not(target_os = "windows"))]
mod focus {
    pub fn focus_pid(_: u32) {}
}

// ── delete confirmation dialog ────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn confirm_delete(name: &str) -> bool {
    type HWND = *mut u8;
    unsafe extern "system" {
        fn MessageBoxW(hwnd: HWND, text: *const u16, caption: *const u16, utype: u32) -> i32;
    }
    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0u16)).collect()
    }
    let text    = to_wide(&format!("Удалить проект «{}»?", name));
    let caption = to_wide("Удаление проекта");
    // MB_OKCANCEL | MB_ICONQUESTION = 0x00000021
    let result = unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x00000021)
    };
    result == 1
}

#[cfg(not(target_os = "windows"))]
fn confirm_delete(_name: &str) -> bool { true }

// ── screen & palette ──────────────────────────────────────────────────────────

enum Screen { Main, Settings, Routine }

fn add_target_for(s: &settings::Settings, main_empty: bool) -> sync::oplog::AddTarget {
    use settings::NewTaskPos;
    use sync::oplog::AddTarget;
    if main_empty || s.replace_main { return AddTarget::Main; }
    match s.new_task_pos {
        NewTaskPos::End       => AddTarget::End,
        NewTaskPos::Beginning => AddTarget::Beginning,
    }
}

const ROUTINE_UNDERLINE: Color32 = Color32::from_rgb(220, 180, 40);

const PROJECT_PALETTE: &[Color32] = &[
    Color32::from_rgb(220,  50,  50), // red
    Color32::from_rgb(249, 115,  22), // orange
    Color32::from_rgb(234, 179,   8), // yellow
    Color32::from_rgb( 34, 197,  94), // green
    Color32::from_rgb( 20, 184, 166), // cyan
    Color32::from_rgb( 59, 130, 246), // blue
    Color32::from_rgb(168,  85, 247), // violet
    Color32::from_rgb(236,  72, 153), // pink
    Color32::from_rgb(139,  90,  43), // brown
    Color32::from_rgb( 40,  40,  40), // black
    Color32::from_rgb(210, 210, 210), // white
];

// ── app ──────────────────────────────────────────────────────────────────────

struct App {
    settings:              settings::Settings,
    settings_ui:           settings::SettingsUiState,
    routine_ui:            ui::routine::RoutineUiState,
    sync:                  sync::SyncHandle,
    screen:                Screen,
    adding:                bool,
    need_focus:            bool,
    buf:                   String,
    last_h:                f32,
    w:                     f32,
    projects:              Vec<project::LoadedProject>,
    active_project_idx:    usize,
    project_open:          bool,
    project_keyboard_focus:    Option<usize>,
    project_display_order:     Vec<String>,
    project_focus_is_keyboard: bool,
    project_focus_bias:        f32,
    project_focus_bias_target: f32,
    project_focus_streak:      u32,
    project_focus_last_dir:    Option<bool>,
    project_scroll_offset:     f32,
    project_dropdown_h:    f32,
    project_adding:        bool,
    project_buf:           String,
    project_need_focus:    bool,
    project_new_color_idx: usize,
    last_routine_tick:     u64,
    last_lock_refresh:     u64,
    project_loader_rx:     Option<std::sync::mpsc::Receiver<Vec<project::LoadedProject>>>,
    deleted_during_load:    std::collections::HashSet<String>,
    pending_ops:            Vec<sync::oplog::Op>,
    updater_spawned:        bool,
    editing_task:            Option<usize>,
    edit_buf:                String,
    edit_need_focus:         bool,
    dragging_task:           Option<usize>,
}

impl App {
    fn new(cc: &eframe::CreationContext, mut settings: settings::Settings) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut vis = egui::Visuals::dark();
        vis.panel_fill    = Color32::TRANSPARENT;
        vis.window_fill   = Color32::TRANSPARENT;
        vis.window_stroke = Stroke::NONE;
        cc.egui_ctx.set_visuals(vis);

        let _ = std::fs::create_dir_all(project::projects_dir());

        let manifest = manifest::load();

        let ops_path       = app_dir().join("ops.ndjson");
        let ops_ndjson_empty = std::fs::metadata(&ops_path).map(|m| m.len() == 0).unwrap_or(true);

        let manifest_missing_or_empty = manifest.is_empty();

        let full_sync_load = ops_ndjson_empty || manifest_missing_or_empty;

        let (project_batch_tx, project_batch_rx) =
            std::sync::mpsc::channel::<Vec<project::LoadedProject>>();

        let mut projects: Vec<project::LoadedProject>;
        let active_idx: usize;
        let project_loader_rx: Option<std::sync::mpsc::Receiver<Vec<project::LoadedProject>>>;

        if full_sync_load {
            clog!(
                "[start] Ветка А (full sync load) — ops_ndjson_empty={} manifest_missing_or_empty={}",
                ops_ndjson_empty, manifest_missing_or_empty
            );
            let mut loaded = project::load_all_projects();
            if loaded.is_empty() {
                loaded.push(project::create_default_project());
            }
            clog!("[start] rebuilding manifest from {} loaded projects", loaded.len());
            manifest::rebuild_from(&loaded);

            active_idx = project::resolve_active_project(&loaded, &settings).unwrap_or(0);
            projects         = loaded;
            project_loader_rx = None;
        } else {
            clog!("[start] Ветка Б (partial load) — манифест содержит {} проектов", manifest.len());
            let active  = project::load_active_with_fallback(&manifest, settings.preferred_project_id());
            let active_id = active.id.clone();

            let mut built: Vec<project::LoadedProject> = manifest.iter()
                .filter(|(id, _)| id.as_str() != active_id.as_str())
                .map(|(id, entry)| {
                    let color = project::hex_to_color32(&entry.color_hex)
                        .unwrap_or(Color32::from_rgb(74, 144, 217));
                    project::LoadedProject {
                        id:         id.clone(),
                        name:       entry.name.clone(),
                        color,
                        color_hex:  entry.color_hex.clone(),
                        main:       indexmap::IndexMap::new(),
                        subs:       indexmap::IndexMap::new(),
                        created_at:  entry.created_at,
                        last_edited: entry.last_edited,
                        loaded:     false,
                        main_edited_at: 0, name_edited_at: 0, color_edited_at: 0,
                        order_key: entry.order_key, order_key_edited_at: entry.order_key_edited_at,
                        task_count: entry.task_count,
                    }
                })
                .collect();
            built.push(active);
            active_idx = built.len() - 1;

            projects          = built;
            project_loader_rx = Some(project_batch_rx);
        }

        let need_load_projects = !full_sync_load;
        std::thread::Builder::new()
            .name("cue-routine".into())
            .spawn(move || {
                icon_cache::load();

                let updater_path = app_dir().join("cue-updater.exe");
                if !updater_path.exists() {
                    let _ = std::fs::write(&updater_path, include_bytes!("../cue-updater.exe"));
                }

                if need_load_projects {
                    let all = project::load_all_projects();
                    let _   = project_batch_tx.send(all);
                }
                updater::check_and_stage_update();
                daemon_updater::check_and_update_daemon();
                autostart::reconcile(&settings::Settings::load());
            })
            .expect("spawn routine thread");

        let sync = sync::SyncHandle::init(&mut projects, cc.egui_ctx.clone(), settings.http_port);

        let actual_id = projects[active_idx].id.clone();
        if settings.last_project_id.as_deref() != Some(&actual_id) {
            settings.last_project_id = Some(actual_id);
            settings.save();
        }

        let initial_w = settings.last_width.unwrap_or(W);

        let app = Self {
            settings,
            settings_ui:           settings::SettingsUiState::default(),
            routine_ui:            ui::routine::RoutineUiState::default(),
            sync,
            screen:                Screen::Main,
            adding:                false,
            need_focus:            false,
            buf:                   String::new(),
            last_h:                0.0,
            w:                     initial_w,
            projects,
            active_project_idx:    active_idx,
            project_open:          false,
            project_keyboard_focus:    None,
            project_display_order:     Vec::new(),
            project_focus_is_keyboard: false,
            project_focus_bias:        0.5,
            project_focus_bias_target: 0.5,
            project_focus_streak:      0,
            project_focus_last_dir:    None,
            project_scroll_offset:     0.0,
            project_dropdown_h:    0.0,
            project_adding:        false,
            project_buf:           String::new(),
            project_need_focus:    false,
            project_new_color_idx: 0,
            last_routine_tick:     0,
            last_lock_refresh:     0,
            project_loader_rx,
            deleted_during_load:   std::collections::HashSet::new(),
            pending_ops:           Vec::new(),
            updater_spawned:       false,
            editing_task:          None,
            edit_buf:              String::new(),
            edit_need_focus:       false,
            dragging_task:         None,
        };

        app
    }

    fn switch_to_project(&mut self, idx: usize) -> bool {
        if idx == self.active_project_idx { return true; }

        if !self.projects[idx].loaded {
            match project::load_one(&self.projects[idx].id) {
                Some(real) => {
                    self.projects[idx] = real;
                }
                None => {
                    clog!("[switch] project {} unreadable — removing phantom placeholder", self.projects[idx].id);
                    self.projects.remove(idx);
                    if idx < self.active_project_idx {
                        self.active_project_idx -= 1;
                    }
                    if self.projects.is_empty() {
                        self.projects.push(project::create_default_project());
                        self.active_project_idx = 0;
                        self.settings.last_project_id =
                            Some(self.projects[self.active_project_idx].id.clone());
                        self.settings.save();
                    }
                    return false;
                }
            }
        }

        self.active_project_idx = idx;
        self.settings.last_project_id = Some(self.projects[idx].id.clone());
        self.settings.save();
        true
    }

    fn compact_if_needed(&mut self, idx: usize) {
        if !self.projects[idx].needs_compaction() { return; }
        let order = self.projects[idx].compact_order(project::current_time());
        let _ = self.sync.record_op(sync::oplog::OpKind::CompactOrder {
            project_id: self.projects[idx].id.clone(),
            order,
        });
        self.projects[idx].save();
    }

    fn complete_task_local(&mut self, idx: usize, task_id: String) {
        let now        = routine_scheduler::local_now();
        let project_id = self.projects[idx].id.clone();
        let routine_ts = {
            let p = &self.projects[idx];
            p.main.get(task_id.as_str()).or_else(|| p.subs.get(task_id.as_str()))
                .filter(|t| t.routine.is_some())
                .map_or(0, |t| t.routine_edited_at)
        };
        let done_ts = match self.sync.record_op(sync::oplog::OpKind::CompleteTask {
            project_id: project_id.clone(),
            task_id:    task_id.clone(),
            routine_edited_at: routine_ts,
        }) {
            Ok(op) => op.ts,
            Err(_) => project::current_time(),
        };
        let spent = self.projects[idx]
            .complete_task(&task_id, now, done_ts, 0)
            .unwrap_or(false);

        if spent && self.settings.delete_spent_routines {
            let ts = project::current_time();
            let _  = self.sync.record_op(sync::oplog::OpKind::DeleteTask {
                project_id: project_id.clone(), task_id: task_id.clone(),
            });
            self.sync.tombstones.add_task(&task_id, &project_id, ts, &self.sync.identity.device_id);
            self.projects[idx].subs.shift_remove(task_id.as_str());
            self.projects[idx].main.shift_remove(task_id.as_str());
        }
        self.projects[idx].touch(done_ts);
        self.projects[idx].save();
    }

    fn commit_routine_editor(&mut self) {
        let routine = self.routine_ui.build_routine();
        if routine == self.routine_ui.original { return; }

        let idx     = self.active_project_idx;
        let task_id = self.routine_ui.task_id.clone();
        let ts      = project::current_time();

        let _ = self.sync.record_op(sync::oplog::OpKind::SetRoutine {
            project_id: self.projects[idx].id.clone(),
            task_id:    task_id.clone(),
            routine:    routine.clone(),
        });
        self.projects[idx].apply_set_routine(&task_id, routine, ts);
        self.projects[idx].touch(ts);
        self.projects[idx].save();
    }

    fn routine_tick(&mut self) {
        const TICK_INTERVAL_SECS: u64 = 5;
        const LOCK_REFRESH_INTERVAL_SECS: u64 = 15 * 60;

        let now = routine_scheduler::local_now();
        let utc = project::current_time();

        if now >= self.last_lock_refresh + LOCK_REFRESH_INTERVAL_SECS {
            self.last_lock_refresh = now;
            write_lock();
        }

        if now < self.last_routine_tick + TICK_INTERVAL_SECS { return; }
        self.last_routine_tick = now;

        for proj in &mut self.projects {
            let mut changed = false;

            for task in proj.main.values_mut() {
                if let Some(routine) = task.routine.as_mut() {
                    if !routine.active {
                        if let Some(ago) = routine_scheduler::due_secs_ago(routine, task.completed_at, now, utc) {
                            routine.active = true;
                            routine.last_triggered_at = now;
                            routine_scheduler::prune_expired_direct(routine, now);
                            routine_scheduler::on_activated(&proj.name, &task.text);
                            if ago <= routine_scheduler::NOTIFY_WINDOW_SECS {
                                notify::send(&task.text, &proj.name, &proj.color_hex);
                            }
                            changed = true;
                        }
                    }
                }
            }

            for task in proj.subs.values_mut() {
                let Some(routine) = task.routine.as_mut() else { continue };
                if !routine.active {
                    if let Some(ago) = routine_scheduler::due_secs_ago(routine, task.completed_at, now, utc) {
                        routine.active = true;
                        routine.last_triggered_at = now;
                        routine_scheduler::prune_expired_direct(routine, now);
                        routine_scheduler::on_activated(&proj.name, &task.text);
                        if ago <= routine_scheduler::NOTIFY_WINDOW_SECS {
                            notify::send(&task.text, &proj.name, &proj.color_hex);
                        }
                        changed = true;
                    }
                }
            }

            if changed {
                proj.save();
            }
        }
    }
}

// ── render ───────────────────────────────────────────────────────────────────

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] { [0.0; 4] }


    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.sync.shared.viewing_sync_panel.store(
            matches!(self.screen, Screen::Settings)
                && matches!(self.settings_ui.tab, settings::SettingsTab::Sync),
            std::sync::atomic::Ordering::Relaxed,
        );

        if ctx.input(|i| i.viewport().close_requested()) {
            if matches!(self.screen, Screen::Routine) {
                self.commit_routine_editor();
            }
            self.sync.flush_oplog_before_exit();

            if !self.updater_spawned && !cfg!(debug_assertions) {
                self.updater_spawned = true;
                if let Ok(exe) = std::env::current_exe() {
                    let mut upd = exe.as_os_str().to_owned();
                    upd.push(".cueextraupd");
                    let upd_path = std::path::PathBuf::from(upd);
                    if upd_path.exists() {
                        #[cfg(windows)]
                        {
                            use std::os::windows::process::CommandExt;
                            const CREATE_NO_WINDOW: u32 = 0x08000000;
                            let _ = std::process::Command::new(app_dir().join("cue-updater.exe"))
                                .arg(&upd_path)
                                .creation_flags(CREATE_NO_WINDOW)
                                .spawn();
                        }
                    }
                }
            }
        }

        self.sync.poll_oplog_ready();

        {
            let mut dirty: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            while let Ok(ops) = self.sync.ops_rx.try_recv() {
                for op in ops {
                    if self.project_loader_rx.is_some() {
                        if let sync::oplog::OpKind::DeleteProject { project_id } = &op.kind {
                            self.deleted_during_load.insert(project_id.clone());
                        }
                    }

                    if let Some(pid) = op.kind.project_id() {
                        if let Some(idx) = self.projects.iter().position(|p| p.id == pid) {
                            if !self.projects[idx].loaded {
                                match project::load_one(pid) {
                                    Some(real) => {
                                        self.projects[idx] = real;
                                    }
                                    None => {
                                        self.projects.remove(idx);
                                        if idx < self.active_project_idx {
                                            self.active_project_idx -= 1;
                                        }
                                        if self.projects.is_empty() {
                                            self.projects.push(project::create_default_project());
                                            self.active_project_idx = 0;
                                        }
                                        continue;
                                    }
                                }
                            }
                        }
                    }

                    if matches!(op.kind, sync::oplog::OpKind::CompactProjectsOrder { .. })
                        && self.projects.iter().any(|p| !p.loaded)
                    {
                        self.pending_ops.push(op.clone());
                        continue;
                    }

                    if let Some(tid) = op.kind.task_id() {
                        let hint  = op.kind.project_id().unwrap_or("");
                        let found = sync::apply::find_task_project(&self.projects, hint, tid).is_some();
                        let any_stub_remains = self.projects.iter().any(|p| !p.loaded);
                        if !found && any_stub_remains {
                            self.pending_ops.push(op.clone());
                            continue;
                        }
                    }

                    dirty.extend(sync::apply::apply_op(
                        &op, &mut self.projects,
                        &mut self.sync.tombstones,
                        &mut self.settings,
                        &mut self.sync.seen_ops,
                    ));
                }
            }
            for pid in &dirty {
                if let Some(p) = self.projects.iter_mut().find(|p| p.id == *pid) {
                    p.save();
                }
            }
            let active_id = self.settings.last_project_id.clone();
            let idx_ok = self.projects.get(self.active_project_idx)
                .map(|p| Some(p.id.as_str()) == active_id.as_deref())
                .unwrap_or(false);
            if !idx_ok {
                match active_id.as_deref().and_then(|id| self.projects.iter().position(|p| p.id == id)) {
                    Some(idx) => { self.active_project_idx = idx; }
                    None => match project::resolve_active_project(&self.projects, &self.settings) {
                        Some(idx) => {
                            self.active_project_idx = idx;
                            self.settings.last_project_id = Some(self.projects[idx].id.clone());
                            self.settings.save();
                        }
                        None => {
                            self.projects.push(project::create_default_project());
                            self.active_project_idx = 0;
                            self.settings.last_project_id = Some(self.projects[0].id.clone());
                            self.settings.save();
                        }
                    }
                }
            }
        }
        if let Some(rx) = self.project_loader_rx.take() {
            match rx.try_recv() {
                Ok(batch) => {
                    for incoming in batch {
                        if self.deleted_during_load.contains(&incoming.id) {
                            continue;
                        }
                        match self.projects.iter().position(|p| p.id == incoming.id) {
                            Some(idx) if self.projects[idx].loaded => {
                            }
                            Some(idx) => {
                                self.projects[idx] = incoming;
                            }
                            None => {
                                self.projects.push(incoming);
                            }
                        }
                    }
                    self.deleted_during_load.clear();
                    if !self.pending_ops.is_empty() {
                        let mut dirty: std::collections::HashSet<String> =
                            std::collections::HashSet::new();
                        for op in mem::take(&mut self.pending_ops) {
                            dirty.extend(sync::apply::apply_op(
                                &op, &mut self.projects,
                                &mut self.sync.tombstones,
                                &mut self.settings,
                                &mut self.sync.seen_ops,
                            ));
                        }
                        for pid in &dirty {
                            if let Some(p) = self.projects.iter_mut().find(|p| p.id == *pid) {
                                p.save();
                            }
                        }
                    }
                    manifest::rebuild_from(&self.projects);
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.project_loader_rx = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                }
            }
        }

        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        self.routine_tick();

        let window_focused = ctx.input(|i| i.focused);
        if window_focused && !self.adding {
            let do_copy = ctx.input(|i| {
                i.events.iter().any(|e| matches!(e, egui::Event::Copy))
            });
            if do_copy {
                let idx  = self.active_project_idx;
                let proj = &self.projects[idx];
                let mut lines: Vec<&str> = Vec::new();
                if let Some(t) = proj.main_text() { lines.push(t); }
                for task in proj.subs.values() { lines.push(&task.text); }
                if !lines.is_empty() {
                    let text = lines.join("\n");
                    ctx.copy_text(text);
                }
            }

            let paste = ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Paste(s) = e { Some(s.clone()) } else { None }
                })
            });
            if let Some(raw) = paste {
                if raw.len() <= 30 * 1024 {
                    let tasks: Vec<String> = raw
                        .lines()
                        .filter(|l| !l.trim().is_empty())
                        .take(50)
                        .map(|l| l.chars().take(300).collect())
                        .collect();
                    if !tasks.is_empty() {
                        let idx = self.active_project_idx;
                        let s   = self.settings.clone();
                        let mut iter = tasks.into_iter();
                        if self.projects[idx].main.is_empty() {
                            let first   = iter.next().unwrap();
                            let task_id = project::gen_task_id(&self.projects[idx].id);
                            let ts      = project::current_time();
                            let _       = self.sync.record_op(sync::oplog::OpKind::AddTask {
                                project_id: self.projects[idx].id.clone(),
                                task_id:    task_id.clone(),
                                text:       first.clone(),
                                target:     sync::oplog::AddTarget::Main,
                            });
                            self.projects[idx].apply_add_to_main(
                                task_id,
                                project::TaskData {
                                    text: first, routine: None, created_at: ts, order_key: 0.0,
                                    text_edited_at: ts, routine_edited_at: 0, pos_edited_at: 0,
                                    transferred_at: 0, completed_at: 0,
                                },
                                ts,
                            );
                        }
                        for text in iter {
                            let task_id = project::gen_task_id(&self.projects[idx].id);
                            let target  = add_target_for(&s, false);
                            let _       = self.sync.record_op(sync::oplog::OpKind::AddTask {
                                project_id: self.projects[idx].id.clone(),
                                task_id:    task_id.clone(),
                                text:       text.clone(),
                                target,
                            });
                            self.projects[idx].add_task(task_id, text, &s);
                        }
                        self.projects[idx].touch(project::current_time());
                        self.projects[idx].save();
                        self.compact_if_needed(idx);
                    }
                }
            }
        }

        if let Screen::Settings = self.screen {
            let (close, target_h) = settings::draw_settings_ui(
                &ctx, ui, &mut self.settings, &mut self.settings_ui, &mut self.sync, &self.projects,
            );
            let size = vec2(settings::SW, target_h);
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::MaxInnerSize(size));
            if close {
                self.screen = Screen::Main;
                self.last_h = 0.0;
            }
            return;
        }

        if let Screen::Routine = self.screen {
            let action = ui::routine::draw(&ctx, ui, &mut self.routine_ui);
            let size = vec2(ui::routine::RW, self.routine_ui.target_height());
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::MaxInnerSize(size));
            if let ui::routine::CloseAction::Close = action {
                self.commit_routine_editor();
                self.screen = Screen::Main;
                self.last_h = 0.0;
            }
            return;
        }

        let text_w = self.w - 10.0 - 8.0 - 28.0 - 10.0;

        let main_h: f32 = {
            let idx  = self.active_project_idx;
            let text = self.projects[idx].main_text().unwrap_or("—");
            let job  = egui::text::LayoutJob::simple(
                text.to_owned(),
                egui::FontId::proportional(15.0),
                Color32::WHITE,
                text_w,
            );
            let text_h = ctx.fonts_mut(|f| f.layout_job(job).size().y);
            (text_h + 12.0).max(40.0)
        };
		let has_subs = !self.projects[self.active_project_idx].subs.is_empty();

        let h = 12.0 + main_h + 5.0
            + self.projects[self.active_project_idx].subs.len().min(9) as f32 * (ROW - 5.0)
            + if has_subs { 24.0 } else { 19.0 };

        if (h - self.last_h).abs() > 0.5 {
            self.last_h = h;
            let size = vec2(self.w, h);
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::MaxInnerSize(size));
        }

        ui.painter().rect_filled(ui.max_rect(), 10.0, BG);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);

        let bar_rect = egui::Rect::from_min_size(ui.next_widget_position(), vec2(self.w, 12.0));
        let drag = ui.allocate_rect(bar_rect, Sense::drag());
        if drag.dragged() { ctx.send_viewport_cmd(ViewportCommand::StartDrag); }

        let close_center = bar_rect.max - vec2(10.2, 3.1);
        let c = bar_rect.center();

        {
            let (dot_col, label_text) = {
                let p = &self.projects[self.active_project_idx];
                (p.color, p.name.clone())
            };

            let cue_rect = egui::Rect::from_min_size(
                bar_rect.min + vec2(5.0, 2.0),
                vec2(60.0, 16.0),
            );
            let cue_resp = ui.allocate_rect(cue_rect, Sense::click());
            if cue_resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }

            let text_col = if cue_resp.hovered() {
                Color32::from_white_alpha(220)
            } else {
                Color32::from_white_alpha(180)
            };

            let p = ui.painter();
            p.circle_filled(bar_rect.min + vec2(10.0, 10.5), 2.0, dot_col);
            p.text(
                bar_rect.min + vec2(17.0, 10.5),
                egui::Align2::LEFT_CENTER,
                &label_text,
                egui::FontId::proportional(10.5),
                text_col,
            );
            for x in [-6.0f32, 0.0, 6.0] {
                p.circle_filled(c + vec2(x, 4.0), 1.5, Color32::from_white_alpha(35));
            }

            let tab_pressed = ctx.input(|i| i.key_pressed(egui::Key::Tab));
            if tab_pressed {
                ctx.memory_mut(|m| { if let Some(id) = m.focused() { m.surrender_focus(id); } });
            }

            if cue_resp.clicked() || tab_pressed {
                self.project_open = !self.project_open;
                if self.project_open {
                    self.project_keyboard_focus = None;
                    self.project_focus_bias     = 0.5;
                    self.project_focus_bias_target = 0.5;
                    self.project_focus_streak      = 0;
                    self.project_focus_last_dir    = None;
                } else {
                    self.project_adding = false;
                    self.project_buf.clear();
                }
            }

            if !self.project_open {
                self.project_display_order.clear();
            }

            if self.project_open {
                let dropdown_top_y = bar_rect.min.y + 14.0;
                let live_h = ctx.input(|i| i.viewport().inner_rect.map(|r| r.height()))
                    .unwrap_or(self.last_h);
                const FRAME_MARGIN: f32 = 4.0;
                const BOTTOM_GAP:   f32 = 4.0;
                let available_h = (live_h - dropdown_top_y - FRAME_MARGIN * 2.0 - BOTTOM_GAP)
                    .max(40.0);
                let dropdown_pos   = egui::pos2(bar_rect.min.x + 5.0, dropdown_top_y);

                const ROW_H: f32 = 17.0;
                let font       = egui::FontId::proportional(10.5);
                let count_font = egui::FontId::proportional(9.0);
                const COUNT_COL: Color32 = Color32::from_gray(95);

                let count_reserve: f32 = if self.settings.show_task_count { 34.0 } else { 0.0 };
                let row_w: f32 = {
                    let max_label_px = ctx.fonts_mut(|f| {
                        self.projects.iter()
                            .map(|p| f.layout_no_wrap(
                                p.name.clone(), font.clone(), Color32::WHITE,
                            ).size().x)
                            .fold(0.0_f32, f32::max)
                    });
                    {
                        let max_w = self.w - 75.0;
                        if max_w < 150.0 {
                            max_w.max(40.0)
                        } else {
                            (max_label_px + 37.0 + count_reserve).clamp(150.0, max_w)
                        }
                    }
                };

                let project_adding     = self.project_adding;
                let project_need_focus = self.project_need_focus;

                let mut commit_project             = false;
                let mut cancel_project             = false;
                let mut start_adding               = false;
                let mut select_project: Option<usize> = None;
                let mut delete_project: Option<usize> = None;
                let mut open_settings              = false;

                let order_stale = self.project_display_order.len() != self.projects.len()
                    || self.project_display_order.iter()
                        .any(|id| !self.projects.iter().any(|p| &p.id == id));
                if order_stale {
                    self.project_display_order =
                        project_sort::display_order(&self.projects, self.settings.project_sort)
                            .into_iter()
                            .map(|i| self.projects[i].id.clone())
                            .collect();
                }
                let order_idx: Vec<usize> = self.project_display_order.iter()
                    .filter_map(|id| self.projects.iter().position(|p| &p.id == id))
                    .collect();

                let mouse_active = ctx.input(|i|
                    i.pointer.delta() != egui::Vec2::ZERO || i.smooth_scroll_delta != egui::Vec2::ZERO);
                if mouse_active {
                    self.project_focus_is_keyboard = false;
                }

                let n = self.projects.len();
                let arrow_down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));
                let arrow_up   = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
                if arrow_down || arrow_up {
                    self.project_focus_is_keyboard = true;
                    self.project_keyboard_focus = Some(match (self.project_keyboard_focus, arrow_down) {
                        (None, true)     => 0,
                        (None, false)    => n.saturating_sub(1),
                        (Some(i), true)  => (i + 1) % n,
                        (Some(i), false) => (i + n - 1) % n,
                    });
                    const FOCUS_STREAK_MAX: u32 = 5;
                    self.project_focus_streak = if self.project_focus_last_dir == Some(arrow_down) {
                        (self.project_focus_streak + 1).min(FOCUS_STREAK_MAX)
                    } else {
                        1
                    };
                    self.project_focus_last_dir = Some(arrow_down);

                    let pole = if arrow_down { 0.8 } else { 0.2 };
                    let frac = self.project_focus_streak as f32 / FOCUS_STREAK_MAX as f32;
                    self.project_focus_bias_target = 0.5 + (pole - 0.5) * frac;
                }

                if !project_adding && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if let Some(&i) = self.project_keyboard_focus
                        .and_then(|pos| order_idx.get(pos))
                    {
                        select_project = Some(i);
                    }
                }

                let force_sizing_pass = (available_h - self.project_dropdown_h).abs() > 0.5;
                self.project_dropdown_h = available_h;

                const EXP_REACH: f32 = 0.90;
                const EXP_TIME:  f32 = 0.035;

                let n_f = self.projects.len() as f32;
                let content_h = n_f * ROW_H + (n_f - 1.0).max(0.0);
                let max_scroll = (content_h - available_h).max(0.0);

                if self.project_focus_is_keyboard {
                    if let Some(idx) = self.project_keyboard_focus {
                        let dt = ctx.input(|i| i.stable_dt);
                        let t  = 1.0 - (1.0 - EXP_REACH).powf(dt / EXP_TIME);

                        let bias_remaining = self.project_focus_bias_target - self.project_focus_bias;
                        if bias_remaining.abs() < 0.001 {
                            self.project_focus_bias = self.project_focus_bias_target;
                        } else {
                            self.project_focus_bias += t * bias_remaining;
                        }

                        let row_center = idx as f32 * (ROW_H + 1.0) + ROW_H / 2.0;
                        let target = (row_center - self.project_focus_bias * available_h)
                            .clamp(0.0, max_scroll);
                        let remaining = target - self.project_scroll_offset;
                        if remaining.abs() < 1.0 {
                            self.project_scroll_offset = target;
                        } else {
                            self.project_scroll_offset += t * remaining;
                        }
                        if bias_remaining.abs() >= 0.001 || remaining.abs() >= 1.0 {
                            ctx.request_repaint();
                        }
                    }
                }
                self.project_scroll_offset = self.project_scroll_offset.clamp(0.0, max_scroll);

                let area_resp = egui::Area::new(egui::Id::new("project_dropdown"))
                    .fixed_pos(dropdown_pos)
                    .order(egui::Order::Foreground)
                    .sizing_pass(force_sizing_pass)
                    .show(&ctx, |ui| {
                        egui::Frame::new()
                            .fill(Color32::from_rgba_premultiplied(0, 0, 0, 220))
                            .corner_radius(6.0)
                            .inner_margin(egui::Margin::same(4))
                            .show(ui, |ui| {
                                let mut scroll_area = egui::ScrollArea::vertical()
                                    .max_height(available_h)
                                    .auto_shrink([true, true])
                                    .scroll_bar_visibility(
                                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded);
                                if self.project_focus_is_keyboard {
                                    scroll_area = scroll_area.vertical_scroll_offset(self.project_scroll_offset);
                                }
                                scroll_area.show(ui, |ui| {
                                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);

                                        for (pos, &i) in order_idx.iter().enumerate() {
                                            let proj = &self.projects[i];
                                            let is_active = self.active_project_idx == i;

                                            let (rr, _) = ui.allocate_exact_size(
                                                vec2(row_w, ROW_H), Sense::hover());

                                            let name_rect = egui::Rect::from_min_size(
                                                rr.min,
                                                vec2(row_w - 18.0, ROW_H),
                                            );
                                            let name_resp = ui.allocate_rect(name_rect, Sense::click());

                                            let has_cross = self.projects.len() > 1;
                                            let del_rect = egui::Rect::from_min_size(
                                                rr.min + vec2(row_w - 16.5, (ROW_H - 15.0) / 2.0),
                                                vec2(15.0, 15.0),
                                            );
                                            let del_resp = has_cross
                                                .then(|| ui.allocate_rect(del_rect, Sense::click()));

                                            if name_resp.hovered()
                                                || del_resp.as_ref().is_some_and(egui::Response::hovered)
                                            {
                                                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                                            }

                                            let kb_focused = self.project_focus_is_keyboard
                                                && self.project_keyboard_focus == Some(pos);
                                            let mouse_focused = !self.project_focus_is_keyboard
                                                && name_resp.hovered();
                                            let label_col = if is_active || kb_focused || mouse_focused {
                                                Color32::WHITE
                                            } else {
                                                Color32::from_gray(190)
                                            };

                                            ui.painter().circle_filled(
                                                rr.min + vec2(7.0, ROW_H / 2.0),
                                                3.0, proj.color);

                                            let count_text = format!("({})", proj.task_count);
                                            let count_w = if self.settings.show_task_count {
                                                ctx.fonts_mut(|f| f.layout_no_wrap(
                                                    count_text.clone(), count_font.clone(), COUNT_COL,
                                                ).size().x) + 4.0
                                            } else { 0.0 };
                                            let text_avail = row_w - 15.0 - 4.0 - 18.0 - count_w;

                                            let mut job = egui::text::LayoutJob::simple_singleline(
                                                proj.name.clone(), font.clone(), label_col);
                                            job.wrap.max_width         = text_avail;
                                            job.wrap.max_rows          = 1;
                                            job.wrap.overflow_character = Some('…');
                                            let galley = ctx.fonts_mut(|f| f.layout_job(job));
                                            let name_w = galley.size().x;
                                            ui.painter().galley(
                                                rr.min + vec2(15.0, (ROW_H - galley.size().y) / 2.0),
                                                galley, label_col);

                                            if self.settings.show_task_count {
                                                let count_job = egui::text::LayoutJob::simple_singleline(
                                                    count_text, count_font.clone(), COUNT_COL);
                                                let count_galley = ctx.fonts_mut(|f| f.layout_job(count_job));
                                                let x = rr.min.x + 15.0 + name_w + 4.0;
                                                ui.painter().galley(
                                                    egui::pos2(x, rr.min.y + (ROW_H - count_galley.size().y) / 2.0),
                                                    count_galley, COUNT_COL);
                                            }

                                            if name_resp.clicked() {
                                                select_project = Some(i);
                                            }

                                            if let Some(del_resp) = del_resp {
                                                let cross_tint = if del_resp.hovered() {
                                                    Color32::WHITE
                                                } else {
                                                    Color32::from_gray(130)
                                                };
                                                ui.put(del_rect, egui::Image::new(ImageSource::Bytes {
                                                    uri: "bytes://cross.png".into(),
                                                    bytes: CROSS_PNG.into(),
                                                }).fit_to_exact_size(vec2(7.0, 7.0)).tint(cross_tint));
                                                if del_resp.clicked() { delete_project = Some(i); }
                                            }
                                        }

                                        if project_adding {
                                            let mut color_clicked = false;
                                            ui.spacing_mut().item_spacing = vec2(3.0, 0.0);
                                            let te_resp = ui.horizontal(|ui| {
                                                let (cr, cr_resp) = ui.allocate_exact_size(
                                                    vec2(ROW_H, ROW_H), Sense::click());
                                                let base = PROJECT_PALETTE[self.project_new_color_idx];
                                                let col = if cr_resp.hovered() {
                                                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                                                    Color32::from_rgb(
                                                        (base.r() as u16 + 20).min(255) as u8,
                                                        (base.g() as u16 + 20).min(255) as u8,
                                                        (base.b() as u16 + 20).min(255) as u8,
                                                    )
                                                } else { base };
                                                ui.painter().circle_filled(cr.center(), 4.5, col);
                                                if cr_resp.clicked() {
                                                    self.project_new_color_idx =
                                                        (self.project_new_color_idx + 1)
                                                        % PROJECT_PALETTE.len();
                                                    color_clicked = true;
                                                }
                                                let te = egui::TextEdit::singleline(&mut self.project_buf)
                                                    .desired_width(row_w - ROW_H - 3.0 - 6.0)
                                                    .min_size(vec2(0.0, ROW_H))
                                                    .hint_text("Название...")
                                                    .font(font.clone())
                                                    .text_color(Color32::from_gray(210))
                                                    .margin(egui::Margin::symmetric(4, 1));
                                                ui.add(te)
                                            }).inner;

                                            if project_need_focus || color_clicked {
                                                te_resp.request_focus();
                                            }

                                            let enter  = ctx.input(|i| i.key_pressed(egui::Key::Enter));
                                            let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));

                                            if escape {
                                                cancel_project = true;
                                            } else if enter || (te_resp.lost_focus() && !color_clicked) {
                                                commit_project = true;
                                            }
                                        } else {
                                            let (add_rect, add_resp) = ui.allocate_exact_size(
                                                vec2(row_w, ROW_H), Sense::click());
                                            if add_resp.hovered() {
                                                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                                                ui.painter().rect_filled(add_rect, 3.0,
                                                    Color32::from_white_alpha(12));
                                            }
                                            ui.painter().text(
                                                add_rect.min + vec2(3.0, ROW_H / 2.0),
                                                egui::Align2::LEFT_CENTER,
                                                "+",
                                                egui::FontId::proportional(11.5),
                                                Color32::from_gray(140),
                                            );
                                            ui.painter().text(
                                                add_rect.min + vec2(15.0, ROW_H / 2.0),
                                                egui::Align2::LEFT_CENTER,
                                                "Добавить...",
                                                font.clone(),
                                                Color32::from_gray(140),
                                            );
                                            if add_resp.clicked() {
                                                start_adding = true;
                                            }
                                        }

                                        ui.add_space(3.0);
                                        let sep = ui.allocate_exact_size(
                                            vec2(row_w, 1.0), Sense::hover()).0;
                                        ui.painter().hline(sep.x_range(), sep.center().y,
                                            (0.5, Color32::from_white_alpha(25)));
                                        ui.add_space(3.0);

                                        let (set_rect, set_resp) = ui.allocate_exact_size(
                                            vec2(row_w, ROW_H), Sense::click());
                                        if set_resp.hovered() {
                                            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                                            ui.painter().rect_filled(set_rect, 3.0,
                                                Color32::from_white_alpha(12));
                                        }
                                        if set_resp.clicked() { open_settings = true; }
                                        ui.painter().circle_filled(
                                            set_rect.min + vec2(7.0, ROW_H / 2.0),
                                            2.0, Color32::from_gray(160));
                                        ui.painter().text(
                                            set_rect.min + vec2(15.0, ROW_H / 2.0),
                                            egui::Align2::LEFT_CENTER,
                                            "Настройки",
                                            font.clone(),
                                            Color32::from_gray(160),
                                        );
                                    })
                            })
                    });

                self.project_scroll_offset = area_resp.inner.inner.state.offset.y;

                if start_adding {
                    self.project_adding        = true;
                    self.project_need_focus    = true;
                    self.project_new_color_idx = 0;
                }
                if self.project_need_focus && !start_adding {
                    self.project_need_focus = false;
                }
                if commit_project {
                    let name = mem::take(&mut self.project_buf);
                    if !name.is_empty() {
                        let color = PROJECT_PALETTE[self.project_new_color_idx];
                        let ts    = project::current_time();
                        let mut p = project::LoadedProject::new(project::gen_id(), name, color, ts);
                        p.order_key = self.projects.iter()
                            .map(|proj| proj.order_key)
                            .fold(0.0, f64::max) + 1000.0;
                        p.save();
                        let _ = self.sync.record_op(sync::oplog::OpKind::CreateProject {
                            project_id: p.id.clone(),
                            name:       p.name.clone(),
                            color:      p.color_hex.clone(),
                            created_at: ts,
                        });
                        self.projects.push(p);
                        let new_idx = self.projects.len() - 1;
                        self.switch_to_project(new_idx);
                    }
                    self.project_new_color_idx = 0;
                    self.project_adding        = false;
                    self.project_open          = false;
                }
                if cancel_project {
                    self.project_buf.clear();
                    self.project_adding        = false;
                    self.project_new_color_idx = 0;
                }
                if let Some(i) = select_project {
                    if self.switch_to_project(i) {
                        self.project_open   = false;
                        self.project_adding = false;
                        self.project_buf.clear();
                    }
                }
                if open_settings {
                    self.project_open   = false;
                    self.project_adding = false;
                    self.project_buf.clear();
                    self.screen = Screen::Settings;
                    let size = vec2(settings::SW, settings::SH_GENERAL);
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
                    ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(size));
                    ctx.send_viewport_cmd(ViewportCommand::MaxInnerSize(size));
                }

                if !self.project_adding
                    && ctx.input(|i| i.pointer.any_click())
                    && !area_resp.response.hovered()
                    && !cue_resp.hovered()
                {
                    self.project_open = false;
                }

                if let Some(i) = delete_project {
                    let name = self.projects[i].name.clone();
                    if confirm_delete(&name) {
                        let project_id = self.projects[i].id.clone();
                        let ts         = project::current_time();
                        if self.project_loader_rx.is_some() {
                            self.deleted_during_load.insert(project_id.clone());
                        }
                        let _          = self.sync.record_op(sync::oplog::OpKind::DeleteProject {
                            project_id: project_id.clone(),
                        });
                        self.sync.tombstones.add_project(&project_id, ts, &self.sync.identity.device_id);
                        self.projects[i].delete_file();
                        self.projects.remove(i);

                        if i < self.active_project_idx {
                            self.active_project_idx -= 1;
                        } else if i == self.active_project_idx {
                            if self.active_project_idx >= self.projects.len() {
                                self.active_project_idx = self.projects.len().saturating_sub(1);
                            }
                        }

                        if let Some(p) = self.projects.get(self.active_project_idx) {
                            self.settings.last_project_id = Some(p.id.clone());
                            self.settings.save();
                        }

                        self.project_open   = false;
                        self.project_adding = false;
                        self.project_buf.clear();
                    }
                }
            }
        }

        let close_resp = ui.allocate_rect(
            egui::Rect::from_center_size(close_center, vec2(12.0, 7.0)),
            Sense::click(),
        );
        let close_col = if close_resp.hovered() {
            Color32::from_rgb(255, 80, 80)
        } else {
            Color32::from_rgb(220, 50, 50)
        };
        if close_resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
        ui.painter().circle_filled(close_center, 3.15, close_col);
        if close_resp.clicked() { 
            if let Some(outer) = ctx.input(|i| i.viewport().outer_rect) {
                self.settings.last_pos = Some([outer.min.x, outer.min.y]);
                self.settings.save();
            }
            ctx.send_viewport_cmd(ViewportCommand::Close); 
        }

        let divider_y = ui.next_widget_position().y + main_h;
        let drag_pointer = if self.dragging_task.is_some() {
            ctx.input(|i| i.pointer.interact_pos())
        } else {
            None
        };
        let dragged_is_active = self.dragging_task
            .and_then(|i| self.projects[self.active_project_idx].subs.get_index(i))
            .is_none_or(|(_, t)| project::is_active_task(t));
        let dragging_above_divider =
            drag_pointer.map_or(false, |p| p.y < divider_y) && dragged_is_active;

        ui.allocate_ui_with_layout(vec2(self.w, main_h), Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
            let (btn_alloc, _) = ui.allocate_exact_size(vec2(25.0, 25.0), Sense::hover());
            let btn_shifted = btn_alloc.translate(vec2(0.2, -2.2));
            let tick_resp = ui.put(btn_shifted,
                egui::Button::image(ImageSource::Bytes {
                    uri: "bytes://tick.png".into(),
                    bytes: TICK_PNG.into(),
                })
                .fill(Color32::from_rgb(34, 197, 94))
                .corner_radius(6.0)
                .min_size(vec2(25.0, 25.0)),
            );
            if tick_resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            if tick_resp.clicked() {
                let idx = self.active_project_idx;
                if let Some(task_id) = self.projects[idx].main.keys().next().cloned() {
                    self.complete_task_local(idx, task_id);
                } else if let Some(pos) = self.projects[idx].subs.iter()
                    .position(|(_, t)| project::is_active_task(t))
                {
                    let task_id    = self.projects[idx].subs.get_index(pos).unwrap().0.clone();
                    let project_id = self.projects[idx].id.clone();
                    let ts         = project::current_time();
                    let _ = self.sync.record_op(sync::oplog::OpKind::PromoteTask {
                        project_id, task_id: task_id.clone(),
                    });
                    self.projects[idx].apply_promote_task(&task_id, ts);
                    self.projects[idx].touch(ts);
                    self.projects[idx].save();
                }
            }
            ui.add_space(8.0);
            let avail = ui.available_rect_before_wrap();
            let dragged_preview: Option<(String, bool)> = if dragging_above_divider {
                self.dragging_task.and_then(|i| {
                    self.projects[self.active_project_idx].subs.get_index(i)
                        .map(|(_, t)| (t.text.clone(), t.routine.is_some()))
                })
            } else {
                None
            };
            let text_str: &str = match &dragged_preview {
                Some((t, _)) => t.as_str(),
                None => self.projects[self.active_project_idx].main_text().unwrap_or("—"),
            };
            let mut job = egui::text::LayoutJob::simple(
                text_str.to_owned(),
                egui::FontId::proportional(15.0),
                Color32::WHITE,
                avail.width() - 10.0,
            );
            let is_routine = match &dragged_preview {
                Some((_, has_routine)) => *has_routine,
                None => self.projects[self.active_project_idx].main
                    .values().next().map_or(false, |t| t.routine.is_some()),
            };
            if is_routine && self.settings.highlight_routines {
                job.sections[0].format.underline = Stroke::new(1.0, ROUTINE_UNDERLINE);
            }
            let galley = ctx.fonts_mut(|f| f.layout_job(job));
            let pos = avail.min + vec2(10.0, (avail.height() - galley.size().y) / 2.0 - 1.0);
            ui.painter().galley(pos, galley, Color32::WHITE);
            ui.allocate_exact_size(avail.size(), Sense::hover());
        });

        ui.painter().hline(0.0..=self.w, divider_y, (0.5, SEP));
        ui.add_space(6.0);

        // ── sub tasks ────────────────────────────────────────────────────

        let proj_ref = &self.projects[self.active_project_idx];
        let mut display_order: Vec<(usize, String, bool, bool)> = proj_ref.subs.values()
            .enumerate()
            .map(|(real_i, t)| (real_i, t.text.clone(), project::is_active_task(t), t.routine.is_some()))
            .collect();
        if self.settings.group_inactive_at_end {
            let order_keys: Vec<f64> = proj_ref.subs.values().map(|t| t.order_key).collect();
            let created_ats: Vec<u64> = proj_ref.subs.values().map(|t| t.created_at).collect();
            display_order.sort_by(|a, b| {
                (!a.2).cmp(&!b.2)
                    .then_with(|| order_keys[a.0].partial_cmp(&order_keys[b.0])
                        .unwrap_or(std::cmp::Ordering::Equal))
                    .then_with(|| created_ats[a.0].cmp(&created_ats[b.0]))
            });
        }

        let mut edit_just_finished = false;

        if !display_order.is_empty() {
            let (mut promote, mut delete, mut open_routine) = (None::<usize>, None::<usize>, None::<usize>);
            let mut start_drag: Option<usize> = None;
            let mut start_edit: Option<usize> = None;
            let (mut commit_edit, mut cancel_edit) = (false, false);
            let list_locked = self.editing_task.is_some() || self.dragging_task.is_some();

            let scroll_h = display_order.len().min(9) as f32 * (ROW - 5.0);
            let mut row_rects: Vec<(usize, egui::Rect)> = Vec::with_capacity(display_order.len());
            let default_item_spacing_y = ui.spacing().item_spacing.y;
            let default_interact_size  = ui.spacing().interact_size;
            let mut prev_was_dragged = false;
            let mut scroll_viewport: Option<egui::Rect> = None;
            egui::ScrollArea::vertical()
                .max_height(scroll_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    scroll_viewport = Some(ui.clip_rect());
                    if let Some(pointer) = drag_pointer {
                        if self.dragging_task.is_some() {
                            let viewport = ui.clip_rect();
                            let mut delta_y = 0.0_f32;

                            delta_y += ctx.input(|i| i.smooth_scroll_delta.y);

                            const EDGE_ZONE: f32 = 20.0;
                            const MAX_EDGE_SPEED: f32 = 300.0;
                            const EDGE_SCROLL_MULTIPLIER: f32 = 1.0;
                            let dt = ctx.input(|i| i.stable_dt);
                            if pointer.y < viewport.top() + EDGE_ZONE {
                                let depth = ((viewport.top() + EDGE_ZONE) - pointer.y).clamp(0.0, EDGE_ZONE);
                                delta_y += (depth / EDGE_ZONE) * MAX_EDGE_SPEED * EDGE_SCROLL_MULTIPLIER * dt;
                            } else if pointer.y > viewport.bottom() - EDGE_ZONE {
                                let depth = (pointer.y - (viewport.bottom() - EDGE_ZONE)).clamp(0.0, EDGE_ZONE);
                                delta_y -= (depth / EDGE_ZONE) * MAX_EDGE_SPEED * EDGE_SCROLL_MULTIPLIER * dt;
                            }

                            if delta_y != 0.0 {
                                ui.scroll_with_delta(vec2(0.0, delta_y));
                            }
                        }
                    }

                    for (real_i, task_text, is_active, has_routine) in display_order.iter() {
                        let real_i      = *real_i;
                        let is_active   = *is_active;
                        let has_routine = *has_routine;
                        let is_editing  = self.editing_task == Some(real_i);
                        let is_dragged_row = self.dragging_task == Some(real_i);

                        let row_h = if is_dragged_row { 0.0 } else { ROW - 5.0 };

                        ui.spacing_mut().item_spacing.y =
                            if is_dragged_row || prev_was_dragged { 0.0 } else { default_item_spacing_y };
                        prev_was_dragged = is_dragged_row;
                        ui.spacing_mut().interact_size =
                            if is_dragged_row { egui::vec2(0.0, 0.0) } else { default_interact_size };

                        let row_rect = egui::Rect::from_min_size(
                            ui.cursor().min, vec2(self.w, row_h));
                        let row_hovered = ui.rect_contains_pointer(row_rect);
                        let show_buttons = row_hovered && !list_locked;

                        let row_resp = ui.push_id(real_i, |ui| {
                        ui.allocate_ui_with_layout(
                            vec2(self.w, row_h),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                if is_editing {
                                    ui.add_space(7.0);
                                    let (chk_rect, chk_resp) = ui.allocate_exact_size(
                                        vec2(15.0, 15.0), Sense::click());
                                    let chk_tint = if chk_resp.hovered() {
                                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                                        Color32::WHITE
                                    } else {
                                        Color32::from_gray(130)
                                    };
                                    ui.put(chk_rect, egui::Image::new(ImageSource::Bytes {
                                        uri: "bytes://tick_small.png".into(),
                                        bytes: TICK_SMALL_PNG.into(),
                                    }).fit_to_exact_size(vec2(8.0, 8.0)).tint(chk_tint));

                                    ui.add_space(6.0);
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.add_space(10.0);
                                        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
                                        let enter  = ctx.input(|i| i.key_pressed(egui::Key::Enter));

                                        let r = ui.add(
                                            egui::TextEdit::singleline(&mut self.edit_buf)
                                                .desired_width(self.w - 36.0)
                                                .text_color(Color32::from_gray(200))
                                                .font(egui::FontId::proportional(13.0)),
                                        );
                                        if self.edit_need_focus {
                                            r.request_focus();
                                            self.edit_need_focus = false;
                                        }

                                        let window_focused = ctx.input(|i| i.focused);
                                        let lost = r.lost_focus() || !window_focused;

                                        if escape {
                                            cancel_edit = true;
                                        } else if enter || lost || chk_resp.clicked() {
                                            commit_edit = true;
                                        }
                                    });
                                } else {
                                    let anim_id = ui.id().with("btns_anim");
                                    let t = ctx.animate_bool_with_time(anim_id, show_buttons, 0.15);
                                    let t = if is_dragged_row { 0.0 } else { t };

                                    ui.add_space(7.0 * t);
                                    let (btn_rect, btn_resp) = ui.allocate_exact_size(
                                        vec2(15.0 * t, 15.0 * t), Sense::click());
                                    let btn_hovered = btn_resp.hovered() && show_buttons;
                                    if btn_hovered { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                                    let btn_base = if btn_hovered { 255 } else { 231 };
                                    ui.put(btn_rect, egui::Image::new(ImageSource::Bytes {
                                        uri: "bytes://cross.png".into(),
                                        bytes: CROSS_PNG.into(),
                                    }).fit_to_exact_size(vec2(8.0 * t, 8.0 * t))
                                        .tint(Color32::from_rgba_unmultiplied(btn_base, btn_base, btn_base, (255.0 * t) as u8)));
                                    ui.put(btn_rect, egui::Image::new(ImageSource::Bytes {
                                        uri: "bytes://cross_light.png".into(),
                                        bytes: CROSS_LIGHT_PNG.into(),
                                    }).fit_to_exact_size(vec2(8.0 * t, 8.0 * t))
                                        .tint(Color32::from_white_alpha(if btn_hovered { (255.0 * t) as u8 } else { 0 })));
                                    if show_buttons && btn_resp.clicked() { delete = Some(real_i); }

                                    ui.add_space(4.0 * t);
                                    let (clk_rect, clk_resp) = ui.allocate_exact_size(
                                        vec2(15.0 * t, 15.0 * t), Sense::click());
                                    let clk_hovered = clk_resp.hovered() && show_buttons;
                                    if clk_hovered { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                                    let clk_base = if clk_hovered { 255 } else { 231 };
                                    ui.put(clk_rect, egui::Image::new(ImageSource::Bytes {
                                        uri: "bytes://clock.png".into(),
                                        bytes: CLOCK_PNG.into(),
                                    }).fit_to_exact_size(vec2(8.0 * t, 8.0 * t))
                                        .tint(Color32::from_rgba_unmultiplied(clk_base, clk_base, clk_base, (255.0 * t) as u8)));
                                    ui.put(clk_rect, egui::Image::new(ImageSource::Bytes {
                                        uri: "bytes://clock_light.png".into(),
                                        bytes: CLOCK_LIGHT_PNG.into(),
                                    }).fit_to_exact_size(vec2(8.0 * t, 8.0 * t))
                                        .tint(Color32::from_white_alpha(if clk_hovered { (255.0 * t) as u8 } else { 0 })));
                                    if show_buttons && clk_resp.clicked() { open_routine = Some(real_i); }

                                    ui.add_space(4.0 * t);
                                    let (pen_rect, pen_resp) = ui.allocate_exact_size(
                                        vec2(15.0 * t, 15.0 * t), Sense::click());
                                    let pen_hovered = pen_resp.hovered() && show_buttons;
                                    if pen_hovered { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                                    let pen_base = if pen_hovered { 255 } else { 231 };
                                    ui.put(pen_rect, egui::Image::new(ImageSource::Bytes {
                                        uri: "bytes://pencil.png".into(),
                                        bytes: PENCIL_PNG.into(),
                                    }).fit_to_exact_size(vec2(8.0 * t, 8.0 * t))
                                        .tint(Color32::from_rgba_unmultiplied(pen_base, pen_base, pen_base, (255.0 * t) as u8)));
                                    ui.put(pen_rect, egui::Image::new(ImageSource::Bytes {
                                        uri: "bytes://pencil_light.png".into(),
                                        bytes: PENCIL_LIGHT_PNG.into(),
                                    }).fit_to_exact_size(vec2(8.0 * t, 8.0 * t))
                                        .tint(Color32::from_white_alpha(if pen_hovered { (255.0 * t) as u8 } else { 0 })));
                                    if show_buttons && pen_resp.clicked() {
                                        start_edit = Some(real_i);
                                    }

                                    ui.add_space(6.0);
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.add_space(10.0);
                                        let text_color = if is_dragged_row {
                                            Color32::TRANSPARENT
                                        } else if is_active {
                                            Color32::from_gray(200)
                                        } else {
                                            Color32::from_gray(90)
                                        };
                                        let draggable = (is_active || !self.settings.group_inactive_at_end)
                                            && !list_locked;
                                        let sense = if draggable { Sense::drag() } else { Sense::hover() };
                                        let mut fmt = egui::text::TextFormat {
                                            font_id: egui::FontId::proportional(
                                                if is_dragged_row { 1.0 } else { 13.0 }),
                                            color: text_color,
                                            ..Default::default()
                                        };
                                        if has_routine && is_active && !is_dragged_row
                                            && self.settings.highlight_routines {
                                            fmt.underline = Stroke::new(1.0, ROUTINE_UNDERLINE);
                                        }
                                        let mut job = egui::text::LayoutJob::default();
                                        job.append(task_text.as_str(), 0.0, fmt);
                                        let label_r = ui.add(
                                            egui::Label::new(job)
                                                .truncate()
                                                .selectable(false)
                                                .sense(sense),
                                        );
                                        if draggable && label_r.hovered() {
                                            ctx.set_cursor_icon(egui::CursorIcon::AllScroll);
                                        }
                                        if draggable && label_r.drag_started() {
                                            start_drag = Some(real_i);
                                        }
                                    });
                                }
                            },
                        ).response
                        });
                        if !is_dragged_row {
                            row_rects.push((real_i, row_resp.inner.rect));
                        }
                    }

                    if self.dragging_task.is_some() {
                        ui.spacing_mut().item_spacing.y = default_item_spacing_y;
                        ui.allocate_exact_size(vec2(self.w, ROW - 5.0), Sense::hover());
                    }
                });

            edit_just_finished = commit_edit || cancel_edit;
            if edit_just_finished {
                if let Some(i) = self.editing_task {
                    if commit_edit {
                        let new_text = self.edit_buf.trim().to_string();
                        let idx = self.active_project_idx;
                        let old_text = self.projects[idx].subs.get_index(i).map(|(_, t)| t.text.clone());
                        if !new_text.is_empty() && old_text.as_deref() != Some(new_text.as_str()) {
                            if let Some((task_id, _)) = self.projects[idx].subs.get_index(i) {
                                let task_id = task_id.clone();
                                let ts = project::current_time();
                                let _ = self.sync.record_op(sync::oplog::OpKind::EditTask {
                                    project_id: self.projects[idx].id.clone(),
                                    task_id:    task_id.clone(),
                                    text:       new_text.clone(),
                                });
                                self.projects[idx].apply_edit_task(&task_id, &new_text, ts);
                                self.projects[idx].touch(ts);
                            }
                            self.projects[idx].save();
                        }
                    }
                }
                self.editing_task = None;
                self.edit_buf.clear();
                ctx.memory_mut(|m| { if let Some(id) = m.focused() { m.surrender_focus(id); } });
            }

            let idx = self.active_project_idx;

            if let Some(i) = start_edit {
                self.editing_task   = Some(i);
                self.edit_buf       = self.projects[idx].subs.get_index(i)
                    .map(|(_, t)| t.text.clone()).unwrap_or_default();
                self.edit_need_focus = true;
            }
            if let Some(i) = start_drag {
                self.dragging_task = Some(i);
            }

            if let Some(dragged_real_i) = self.dragging_task {
                let pointer_released = ctx.input(|i| i.pointer.any_released());
                match drag_pointer {
                    None => {
                        if pointer_released { self.dragging_task = None; }
                    }
                    Some(pointer) if dragging_above_divider => {
                        let _ = pointer;
                        if pointer_released {
                            promote = Some(dragged_real_i);
                            self.dragging_task = None;
                        }
                    }
                    Some(pointer) => {
                        let mut insert_before: Option<usize> = None;
                        let mut line_y = row_rects.last().map(|(_, r)| r.bottom())
                            .unwrap_or(divider_y);
                        for (r_real_i, rect) in row_rects.iter() {
                            if pointer.y < rect.center().y {
                                insert_before = Some(*r_real_i);
                                line_y = rect.top();
                                break;
                            }
                        }
                        if let Some(viewport) = scroll_viewport {
                            line_y = line_y.clamp(viewport.top(), viewport.bottom());
                        }
                        ui.painter().hline(0.0..=self.w, line_y, (0.41, SEP));
                        if pointer_released {
                            let ts = project::current_time();
                            if let Some((task_id, order_key)) =
                                self.projects[idx].reorder_sub(dragged_real_i, insert_before, ts)
                            {
                                let _ = self.sync.record_op(sync::oplog::OpKind::MoveTask {
                                    project_id: self.projects[idx].id.clone(),
                                    task_id,
                                    order_key,
                                });
                                self.projects[idx].touch(ts);
                                self.projects[idx].save();
                                self.compact_if_needed(idx);
                            }
                            self.dragging_task = None;
                        }
                    }
                }
            }

            if let Some(i) = promote {
                if let Some((task_id, _)) = self.projects[idx].subs.get_index(i) {
                    let task_id = task_id.clone();
                    let ts      = project::current_time();
                    let _ = self.sync.record_op(sync::oplog::OpKind::PromoteTask {
                        project_id: self.projects[idx].id.clone(),
                        task_id:    task_id.clone(),
                    });
                    self.projects[idx].apply_promote_task(&task_id, ts);
                    self.projects[idx].touch(ts);
                    self.projects[idx].save();
                }
            }
            if let Some(i) = open_routine {
                let idx = self.active_project_idx;
                let loaded = self.projects[idx].subs.get_index(i)
                    .map(|(id, t)| (id.clone(), t.text.clone(), t.routine.clone()));
                if let Some((task_id, task_name, routine)) = loaded {
                    self.routine_ui.load(task_id, task_name, routine.as_ref());
                }
                self.screen = Screen::Routine;
                let size = vec2(ui::routine::RW, self.routine_ui.target_height());
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
                ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(size));
                ctx.send_viewport_cmd(ViewportCommand::MaxInnerSize(size));
            }
            if let Some(i) = delete {
                let is_active_routine = self.projects[idx].subs.get_index(i)
                    .map(|(_, t)| t.routine.is_some() && project::is_active_task(t))
                    .unwrap_or(false);

                if is_active_routine {
                    let task_id = self.projects[idx].subs.get_index(i).unwrap().0.clone();
                    self.complete_task_local(idx, task_id);
                } else {
                    if let Some((task_id, _)) = self.projects[idx].subs.get_index(i) {
                        let task_id    = task_id.clone();
                        let project_id = self.projects[idx].id.clone();
                        let ts         = project::current_time();
                        let _          = self.sync.record_op(sync::oplog::OpKind::DeleteTask {
                            project_id: project_id.clone(), task_id: task_id.clone(),
                        });
                        self.sync.tombstones.add_task(&task_id, &project_id, ts, &self.sync.identity.device_id);
                    }
                    self.projects[idx].delete_sub(i);
                    self.projects[idx].touch(project::current_time());
                }
                self.projects[idx].save();
            }
        }

        // ── add row ──────────────────────────────────────────────────────
        if has_subs { ui.add_space(-4.0); } else { ui.add_space(-8.0); };
        ui.allocate_ui_with_layout(vec2(self.w, 28.0), Layout::left_to_right(Align::Center), |ui| {
            ui.add_space(4.0);
            if self.adding {
                let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
                let enter  = ctx.input(|i| i.key_pressed(egui::Key::Enter));

                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.buf)
                        .desired_width(self.w - 24.0)
                        .hint_text("Новое задание...")
                        .text_color(Color32::from_gray(200)),
                );
                if self.need_focus {
                    r.request_focus();
                    self.need_focus = false;
                }

                let window_focused = ctx.input(|i| i.focused);
                let lost   = r.lost_focus() || !window_focused;
                let commit = (enter || lost) && !escape;
                let cancel = escape;

                if commit || cancel {
                    if commit && !self.buf.is_empty() {
                        let text    = mem::take(&mut self.buf);
                        let s       = self.settings.clone();
                        let idx     = self.active_project_idx;
                        let task_id = project::gen_task_id(&self.projects[idx].id);
                        let target  = add_target_for(&s, self.projects[idx].main.is_empty());
                        let _       = self.sync.record_op(sync::oplog::OpKind::AddTask {
                            project_id: self.projects[idx].id.clone(),
                            task_id:    task_id.clone(),
                            text:       text.clone(),
                            target,
                        });
                        self.projects[idx].add_task(task_id, text, &s);
                        self.projects[idx].touch(project::current_time());
                        self.projects[idx].save();
                        self.compact_if_needed(idx);
                    } else {
                        self.buf.clear();
                    }
                    self.adding = false;
                    ctx.memory_mut(|m| { if let Some(id) = m.focused() { m.surrender_focus(id); } });
                }
            } else {
                let global_enter = !edit_just_finished
                    && ctx.input(|i| i.key_pressed(egui::Key::Enter));

                let add_btn = ui.add(
                    egui::Button::new(
                        RichText::new("+ Добавить...")
                            .size(12.0)
                            .color(Color32::from_white_alpha(60)),
                    )
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE),
                );
                if add_btn.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                if add_btn.clicked() || global_enter {
                    self.adding = true;
                    self.need_focus = true;
                }
            }
        });

        // ── resize handles ───────────────────────────────────────────────
        const EDGE: f32 = 6.0;
        let left_rect  = egui::Rect::from_min_size(egui::pos2(0.0, 0.0),            vec2(EDGE, h));
        let right_rect = egui::Rect::from_min_size(egui::pos2(self.w - EDGE, 0.0),  vec2(EDGE, h));

        let left_resp  = ui.allocate_rect(left_rect,  Sense::drag());
        let right_resp = ui.allocate_rect(right_rect, Sense::drag());

        if left_resp.hovered()  || left_resp.dragged()  {
            ctx.set_cursor_icon(egui::CursorIcon::ResizeWest);
        }
        if right_resp.hovered() || right_resp.dragged() {
            ctx.set_cursor_icon(egui::CursorIcon::ResizeEast);
        }

        if right_resp.dragged() {
            let delta = right_resp.drag_delta().x;
            self.w = (self.w + delta).max(MIN_W);
            let size = vec2(self.w, h);
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::MaxInnerSize(size));
        }

        if left_resp.dragged() {
            let delta   = left_resp.drag_delta().x;
            let old_w   = self.w;
            self.w      = (self.w - delta).max(MIN_W);
            let x_shift = old_w - self.w;
            if let Some(outer) = ctx.input(|i| i.viewport().outer_rect) {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(
                    egui::pos2(outer.min.x + x_shift, outer.min.y),
                ));
            }
            let size = vec2(self.w, h);
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(size));
            ctx.send_viewport_cmd(ViewportCommand::MaxInnerSize(size));
        }

        {
            let is_dragging = right_resp.dragged() || left_resp.dragged();
            let drag_key = egui::Id::new("resize_was_dragging");
            let was_dragging: bool = ctx.data(|d| d.get_temp(drag_key).unwrap_or(false));
            ctx.data_mut(|d| d.insert_temp(drag_key, is_dragging));
            if was_dragging && !is_dragging {
                self.settings.last_width = Some(self.w);
                self.settings.save();
            }
        }
    }
}

// ── entry ─────────────────────────────────────────────────────────────────────

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    let _ = std::fs::create_dir_all(app_dir());
    LOG_PATH.set(app_dir().join("debug.log")).ok();
    let _ = std::fs::write(app_dir().join("debug.log"), "");
    let _ = log::set_logger(&FILE_LOGGER).map(|()| log::set_max_level(log::LevelFilter::Warn));
    clog!("=== Cue started ===");

    if let Some(lock) = read_lock() {
        if lock_is_fresh(&lock) && is_alive(lock.pid) {
            focus::focus_pid(lock.pid);
            return Ok(());
        }
    }
    write_lock();

    notify::register_aumid();

    let settings = settings::Settings::load();
    let initial_w = settings.last_width.unwrap_or(W);

    let mut viewport = egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_icon(eframe::icon_data::from_png_bytes(ICON_PNG).unwrap())
        .with_inner_size([initial_w, 100.0])
        .with_min_inner_size([MIN_W, 50.0])
        .with_resizable(false);

    if let Some(pos) = settings.last_pos {
        viewport = viewport.with_position(egui::pos2(pos[0], pos[1]));
    }

    let result = eframe::run_native(
        "Cue",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(App::new(cc, settings)))),
    );

    icon_cache::persist();
    delete_lock();
    result
}