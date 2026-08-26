#![windows_subsystem = "windows"]

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod settings;
pub mod project;
pub mod manifest;
pub mod routine_scheduler;
pub mod notify;
pub mod icon_cache;
pub mod exclusive_bind;
pub mod sync;
pub mod ui;

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

/// Мост между стандартным `log`-фасадом (через который egui шлёт свои
/// внутренние warn!/error!, включая "id clash") и уже существующим
/// файловым логом (debug.log) — без egui-варнингов было физически некуда
/// смотреть: GUI-процесс на Windows не имеет консоли, а без установленного
/// log::Log egui'шные log::warn! просто молча пропадали в никуда.
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
/// Маленькая галочка — подтверждение инлайн-редактирования текста задачи
/// (появляется на месте кнопки-крестика, пока задача редактируется).
static TICK_SMALL_PNG: &[u8] = include_bytes!("../pics/tick_small.png");
/// Карандаш — кнопка входа в режим инлайн-редактирования текста задачи.
static PENCIL_PNG: &[u8] = include_bytes!("../pics/pencil.png");
/// Жёлтые часы — кнопка настройки routine у sub-задачи (сейчас временно
/// использовала CROSS_PNG как заглушку).
static CLOCK_PNG: &[u8] = include_bytes!("../pics/clock.png");
// "_light" версии — забеленные копии тех же трёх иконок (сгенерированы
// скриптом make_light_icons.py, альфа уже запечена в самом файле). Рисуются
// поверх обычной иконки на hover — включаем/выключаем их видимость только
// внешней альфой (0/255), а не наличием/отсутствием отрисовки (см. правило
// про "Widget rect changed id between passes").
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

/// Цвет подчёркивания текста задач-рутин (в main и активных subs) — тот же,
/// что у жёлтого индикатора в шапке окна редактора рутины
/// (см. ui/routine/mod.rs:150).
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
    project_adding:        bool,
    project_buf:           String,
    project_need_focus:    bool,
    project_new_color_idx: usize,
    /// Момент последней проверки планировщика рутин (routine_scheduler::local_now()).
    /// См. Cue_Routines_Implementation_Plan.txt, Этап 5.
    last_routine_tick:     u64,
    /// Момент последнего обновления cue.lock (см. write_lock()) — обновляется
    /// периодически, пока Cue жива, чтобы демон мог отличить реально
    /// работающую Cue от давно рухнувшей сессии с тем же PID.
    last_lock_refresh:     u64,
    /// Some — Ветка Б холодного старта: ждём батч от потока-загрузчика
    /// (весь список проектов с диска). None — Ветка А, поток ничего не
    /// пришлёт, ждать нечего. Этап 6 (мёрж) читает и опустошает это поле.
    project_loader_rx:     Option<std::sync::mpsc::Receiver<Vec<project::LoadedProject>>>,
    /// Временный список id, удалённых (локально или синхронно от пира)
    /// ПОКА ещё ждём батч от потока-загрузчика — чтобы мёрж-блок ниже не
    /// воскресил их обратно. Наполняется только пока project_loader_rx —
    /// Some; очищается сразу после успешного мёржа батча (окно закрылось).
    deleted_during_load:    std::collections::HashSet<String>,
    /// real_i (индекс в subs) задачи, которая сейчас редактируется инлайн
    /// (карандаш). None — никто не редактируется. Пока Some — весь список
    /// subs залочен для остальных кнопок (крестик/routine/карандаш).
    editing_task:            Option<usize>,
    edit_buf:                String,
    edit_need_focus:         bool,
    /// real_i перетаскиваемой (drag & drop) sub-задачи. Пока Some — весь
    /// список залочен точно так же, как при редактировании (list_locked),
    /// а строка-источник рисуется как невидимый плейсхолдер.
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

        // ── манифест + условия (i)/(ii) ─────────────────────────────────────
        // См. Cue_Старт_Приложения_План.txt, шаг 3.
        let manifest = manifest::load();

        // (i) Тот же самый дешёвый чек, что уже использует sync::SyncHandle::init
        // (mod.rs:168) для решения "бутстрапить или нет". Дублируем его здесь,
        // не меняя сигнатуру sync::init — см. обсуждение в плане, syscall дешевле
        // правки интерфейса.
        let ops_path       = app_dir().join("ops.ndjson");
        let ops_ndjson_empty = std::fs::metadata(&ops_path).map(|m| m.len() == 0).unwrap_or(true);

        // (ii) Манифеста нет вовсе, либо он есть, но пуст (например, юзер
        // вручную удалил файл, или это первый запуск этой версии кода).
        let manifest_missing_or_empty = manifest.is_empty();

        // Любое из двух условий → полная синхронная загрузка (Ветка А).
        // Оба ложны (обычный случай, повседневная работа) → Ветка Б —
        // манифестные заглушки + один активный проект синхронно, остальное
        // едет отдельным потоком.
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
            // Всегда, не только когда manifest_missing_or_empty — все проекты
            // и так честно загружены в память в этот момент, лишнего чтения
            // диска это не добавляет, а закрывает случай "манифест не пуст,
            // но с дырой/лишней записью" (не только полностью пустой).
            clog!("[start] rebuilding manifest from {} loaded projects", loaded.len());
            manifest::rebuild_from(&loaded);

            let last_id = settings.last_project_id.clone();
            active_idx = last_id.as_deref()
                .and_then(|id| loaded.iter().position(|p| p.id == id))
                .unwrap_or(0);
            projects         = loaded;
            project_loader_rx = None; // поток ничего не пришлёт — see need_load_projects ниже
        } else {
            clog!("[start] Ветка Б (partial load) — манифест содержит {} проектов", manifest.len());
            let last_id = settings.last_project_id.clone();
            let active  = project::load_active_with_fallback(&manifest, last_id.as_deref());
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
                        created_at: 0, // временная заглушка — см. project.rs, load_active_with_fallback
                        loaded:     false,
                        main_edited_at: 0,
                    }
                })
                .collect();
            built.push(active);
            active_idx = built.len() - 1; // именно что вставили последним — активный, реальный

            projects          = built;
            project_loader_rx = Some(project_batch_rx);
        }

        // Поток "cue-routine": иконки — всегда; полное чтение всех файлов
        // проектов — только если Ветка Б (Ветка А уже загрузила всё сама
        // синхронно, повторное чтение было бы работой без цели). Флаг и tx
        // захватываются в move-замыкании по значению, вычислены выше.
        let need_load_projects = !full_sync_load;
        std::thread::Builder::new()
            .name("cue-routine".into())
            .spawn(move || {
                icon_cache::load();
                if need_load_projects {
                    let all = project::load_all_projects();
                    let _   = project_batch_tx.send(all);
                }
            })
            .expect("spawn routine thread");

        let sync = sync::SyncHandle::init(&mut projects, cc.egui_ctx.clone());

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
            project_adding:        false,
            project_buf:           String::new(),
            project_need_focus:    false,
            project_new_color_idx: 0,
            last_routine_tick:     0, // 0 → первый тик в update() сработает сразу
            last_lock_refresh:     0, // 0 → первое обновление лока в update() сработает сразу
            project_loader_rx,
            deleted_during_load:   std::collections::HashSet::new(),
            editing_task:          None,
            edit_buf:              String::new(),
            edit_need_focus:       false,
            dragging_task:         None,
        };

        app
    }

    /// Переключает активный проект. Если целевой проект — ещё заглушка
    /// (loaded: false, Ветка Б холодного старта, батч ещё не подъехал) —
    /// синхронно, на месте клика, пытается дочитать именно этот один файл.
    /// См. Cue_Мёрж_Батча_И_Битые_Файлы.txt, "СЦЕНАРИЙ: КЛИК НА ПРОЕКТ,
    /// КОТОРОГО ЕЩЁ НЕТ В self.projects С ЗАГРУЖЕННЫМИ ЗАДАЧАМИ".
    fn switch_to_project(&mut self, idx: usize) {
        if idx == self.active_project_idx { return; }

        if !self.projects[idx].loaded {
            match project::load_one(&self.projects[idx].id) {
                Some(real) => {
                    self.projects[idx] = real;
                }
                None => {
                    // Любая ошибка чтения — файл битый/недоступен/лок.
                    // Фантомно убрать из ОЗУ: НЕ логируем как удаление, НЕ
                    // вызываем delete_file(), НЕ шлём DeleteProject op —
                    // юзер ничего не просил удалять, файл может просто
                    // временно быть недоступен (например пир как раз
                    // дописывает его через sync).
                    clog!("[switch] project {} unreadable — removing phantom placeholder", self.projects[idx].id);
                    self.projects.remove(idx);
                    if idx < self.active_project_idx {
                        self.active_project_idx -= 1;
                    }
                    if self.projects.is_empty() {
                        self.projects.push(project::create_default_project());
                        self.active_project_idx = 0;
                    }
                    self.settings.last_project_id =
                        Some(self.projects[self.active_project_idx].id.clone());
                    self.settings.save();
                    return; // idx-проекта больше нет — переключение на него отменено
                }
            }
        }

        self.active_project_idx = idx;
        self.settings.last_project_id = Some(self.projects[idx].id.clone());
        self.settings.save();
    }

    /// Коммитит текущее состояние окна редактора рутины в модель/оплог/диск —
    /// но только если оно реально отличается от того, что было при open()
    /// (иначе просто открыл посмотреть и закрыл — не спамим SetRoutine).
    /// Не трогает self.screen — вызывающий код сам решает, что дальше
    /// (закрыть окно редактора или продолжить закрытие всего приложения).
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
        self.projects[idx].save();
    }

    /// Тик планировщика рутин — см. Cue_Routines_Implementation_Plan.txt,
    /// Этап 5. Вызывается из ui() не чаще, чем раз в TICK_INTERVAL_SECS
    /// (ui() и так гарантированно зовётся минимум раз в секунду благодаря
    /// ctx.request_repaint_after(1 сек) выше).
    fn routine_tick(&mut self) {
        const TICK_INTERVAL_SECS: u64 = 5;
        const LOCK_REFRESH_INTERVAL_SECS: u64 = 15 * 60;

        let now = routine_scheduler::local_now();

        if now >= self.last_lock_refresh + LOCK_REFRESH_INTERVAL_SECS {
            self.last_lock_refresh = now;
            write_lock();
        }

        if now < self.last_routine_tick + TICK_INTERVAL_SECS { return; }
        self.last_routine_tick = now;

        for proj in &mut self.projects {
            let mut changed = false;

            // main: рутина там в норме уже active=true (см. раздел 1 плана —
            // "не может быть неактивной и в main"), но проверяем защитно,
            // без репозиционирования (main — всегда 0/1 элемент). Раньше эта
            // ветка не выставляла `changed`, из-за чего активация здесь не
            // сохранялась на диск — теперь это реальный путь (например,
            // после применения входящего SetRoutine с сети), так что
            // пишем наравне с subs.
            for task in proj.main.values_mut() {
                if let Some(routine) = task.routine.as_mut() {
                    if !routine.active {
                        if let Some(occ) = routine_scheduler::due_occurrence(routine, now) {
                            routine.active = true;
                            routine.last_triggered_at = now;
                            routine_scheduler::prune_expired_direct(routine, now);
                            routine_scheduler::on_activated(&proj.name, &task.text);
                            if now.saturating_sub(occ) <= routine_scheduler::NOTIFY_WINDOW_SECS {
                                notify::send(&task.text, &proj.name, &proj.color_hex);
                            }
                            changed = true;
                        }
                    }
                }
            }

            // subs: флипаем active на месте, без перемещения по IndexMap —
            // order_key и физическая позиция задачи не меняются никогда при
            // активации. Группировка active/inactive для показа считается
            // отдельно, на лету, при отрисовке (см. display_order в update()).
            for task in proj.subs.values_mut() {
                let Some(routine) = task.routine.as_mut() else { continue };
                if !routine.active {
                    if let Some(occ) = routine_scheduler::due_occurrence(routine, now) {
                        routine.active = true;
                        routine.last_triggered_at = now;
                        routine_scheduler::prune_expired_direct(routine, now);
                        routine_scheduler::on_activated(&proj.name, &task.text);
                        if now.saturating_sub(occ) <= routine_scheduler::NOTIFY_WINDOW_SECS {
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

        // ── Alt+F4 / закрытие всего окна: если в этот момент открыт редактор
        // рутины — успеть закоммитить его состояние перед выходом. Не
        // мешает крашу/убийству через диспетчер задач — там этот код просто
        // не успевает выполниться, что и требуется.
        if ctx.input(|i| i.viewport().close_requested()) {
            if matches!(self.screen, Screen::Routine) {
                self.commit_routine_editor();
            }
            // If the engine thread hasn't finished loading ops.ndjson yet,
            // finish it right here so anything recorded in the meantime
            // (including the SetRoutine commit right above, if any) actually
            // makes it to disk before the process exits. No-op almost always —
            // the engine thread has typically done this long before the user
            // gets around to closing the window.
            self.sync.flush_oplog_before_exit();
        }

        // ── merge in historical op_ids once the background oplog load
        // (if any) finishes — see OplogState/ensure_oplog_ready ─────────────
        self.sync.poll_oplog_ready();

        // ── drain incoming sync ops (written by engine thread) ────────────
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

                    // Узкий фикс: если оп касается проекта, который у нас
                    // всё ещё манифестная заглушка (loaded: false, Ветка Б
                    // холодного старта) — синхронно догрузить его ПЕРЕД
                    // apply_op. Иначе apply_op честно нашёл бы проект в
                    // self.projects и замутировал бы пустые main/subs
                    // заглушки как будто это настоящий пустой проект — а
                    // последующий save() затёр бы реальный файл на диске,
                    // потеряв всё, что там было. Если чтение не удалось
                    // (файл реально битый, не просто "ещё не загружен") —
                    // фантомно убираем заглушку (тот же путь, что и в
                    // switch_to_project при клике) и НЕ применяем оп —
                    // ровно то же самое, что случилось бы, если бы проекта
                    // не было в self.projects вовсе.
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
                                        continue; // проекта больше нет — этот оп не применяем
                                    }
                                }
                            }
                        }
                    }

                    if let Some(pid) = sync::apply::apply_op(
                        &op, &mut self.projects,
                        &mut self.sync.tombstones,
                        &mut self.settings,
                        &mut self.sync.seen_ops,
                    ) { dirty.insert(pid); }
                }
            }
            for pid in &dirty {
                if let Some(p) = self.projects.iter().find(|p| p.id == *pid) {
                    p.save();
                }
            }
            // A synced DeleteProject may have shrunk `projects` — guard the active index.
            if self.projects.is_empty() {
                self.projects.push(project::create_default_project());
                self.active_project_idx = 0;
                self.settings.last_project_id = Some(self.projects[0].id.clone());
                self.settings.save();
            } else if self.active_project_idx >= self.projects.len() {
                self.active_project_idx = self.projects.len() - 1;
                self.settings.last_project_id = Some(self.projects[self.active_project_idx].id.clone());
                self.settings.save();
            }
        }
        // ── drain project-loader batch (Ветка Б холодного старта) ──────────
        // Поток-загрузчик, если он был запущен (project_loader_rx == Some),
        // шлёт РОВНО ОДИН батч и завершается — не цикл, один try_recv() в
        // кадр достаточно, в отличие от ops_rx выше. Правило мёржа — см.
        // Cue_Мёрж_Батча_И_Битые_Файлы.txt.
        if let Some(rx) = self.project_loader_rx.take() {
            match rx.try_recv() {
                Ok(batch) => {
                    for incoming in batch {
                        // Этап 7: фильтр ДО матча по self.projects — если id
                        // был удалён (локально или синхронно от пира) пока мы
                        // ждали этот батч, не воскрешаем его. Один проход,
                        // без промежуточного "вставили → удалили".
                        if self.deleted_during_load.contains(&incoming.id) {
                            continue;
                        }
                        match self.projects.iter().position(|p| p.id == incoming.id) {
                            Some(idx) if self.projects[idx].loaded => {
                                // Уже есть живые данные — полный скип, не
                                // трогаем ничего из батча для этого id.
                            }
                            Some(idx) => {
                                // Была заглушка (loaded: false) — полное
                                // доверие батчу целиком.
                                self.projects[idx] = incoming;
                            }
                            None => {
                                // Не было даже заглушки в манифесте —
                                // доверяем батчу целиком.
                                self.projects.push(incoming);
                            }
                        }
                    }
                    // Окно закрылось — сет своё дело сделал, дальше он не
                    // нужен: project_loader_rx уже None (взяли через .take()
                    // выше), новых батчей не будет, воскрешать больше нечему.
                    self.deleted_during_load.clear();
                    // Перестроить манифест из self.projects БЕЗУСЛОВНО, один
                    // раз (это разовое, не per-frame событие — сам приход
                    // батча случается ровно один раз за запуск). Закрывает
                    // случай, когда id вернулся в self.projects через push
                    // выше (ветка None), в обход save()/upsert_entry — иначе
                    // манифест на диске так и остался бы не знать о нём.
                    manifest::rebuild_from(&self.projects);
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Поток ещё не дочитал диск — вернуть receiver на место,
                    // попробовать снова в следующем кадре.
                    self.project_loader_rx = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Поток упал/запаниковал, ничего не прислав. Не блокируем
                    // программу навсегда — просто оставляем заглушки
                    // заглушками (тот же исход, что и "битый файл" при клике,
                    // Этап 8, только сразу для всех разом, а не по одному).
                }
            }
        }

        // Fallback: wake egui periodically in case no sync activity.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        // ── тик планировщика рутин (Cue_Routines_Implementation_Plan.txt,
        //    Этап 5) ────────────────────────────────────────────────────
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
                            let task_id = project::gen_id();
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
                                    text_edited_at: ts, routine_edited_at: 0,
                                },
                                ts,
                            );
                        }
                        for text in iter {
                            let task_id = project::gen_id();
                            let target  = add_target_for(&s, false);
                            let _       = self.sync.record_op(sync::oplog::OpKind::AddTask {
                                project_id: self.projects[idx].id.clone(),
                                task_id:    task_id.clone(),
                                text:       text.clone(),
                                target,
                            });
                            self.projects[idx].add_task(task_id, text, &s);
                        }
                        self.projects[idx].save();
                    }
                }
            }
        }

        if let Screen::Settings = self.screen {
            let (close, target_h) = settings::draw_settings_ui(
                &ctx, ui, &mut self.settings, &mut self.settings_ui, &mut self.sync,
            );
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(
                vec2(settings::SW, target_h),
            ));
            if close {
                self.screen = Screen::Main;
                self.last_h = 0.0;
            }
            return;
        }

        if let Screen::Routine = self.screen {
            let action = ui::routine::draw(&ctx, ui, &mut self.routine_ui);
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(
                vec2(ui::routine::RW, self.routine_ui.target_height()),
            ));
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
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(self.w, h)));
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

            if cue_resp.clicked() {
                self.project_open = !self.project_open;
                if !self.project_open {
                    self.project_adding = false;
                    self.project_buf.clear();
                }
            }

            if self.project_open {
                let dropdown_top_y = bar_rect.min.y + 14.0;
                let available_h    = (self.last_h - dropdown_top_y - 10.0).max(40.0);
                let dropdown_pos   = egui::pos2(bar_rect.min.x + 5.0, dropdown_top_y);

                const ROW_H: f32 = 17.0;
                let font = egui::FontId::proportional(10.5);

                let row_w: f32 = {
                    let max_label_px = ctx.fonts_mut(|f| {
                        self.projects.iter()
                            .map(|p| f.layout_no_wrap(
                                p.name.clone(), font.clone(), Color32::WHITE,
                            ).size().x)
                            .fold(0.0_f32, f32::max)
                    });
                    (max_label_px + 37.0).clamp(150.0, self.w - 16.0)
                };
                let text_avail = row_w - 15.0 - 4.0 - 18.0;

                let project_adding     = self.project_adding;
                let project_need_focus = self.project_need_focus;

                let mut commit_project             = false;
                let mut cancel_project             = false;
                let mut start_adding               = false;
                let mut select_project: Option<usize> = None;
                let mut delete_project: Option<usize> = None;
                let mut open_settings              = false;

                let area_resp = egui::Area::new(egui::Id::new("project_dropdown"))
                    .fixed_pos(dropdown_pos)
                    .order(egui::Order::Foreground)
                    .show(&ctx, |ui| {
                        egui::Frame::new()
                            .fill(Color32::from_rgba_premultiplied(0, 0, 0, 220))
                            .corner_radius(6.0)
                            .inner_margin(egui::Margin::same(4))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(available_h)
                                    .auto_shrink([true, true])
                                    .scroll_bar_visibility(
                                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                                    .show(ui, |ui| {
                                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);

                                        for (i, proj) in self.projects.iter().enumerate() {
                                            let is_active = self.active_project_idx == i;

                                            let (rr, rr_resp) = ui.allocate_exact_size(
                                                vec2(row_w, ROW_H), Sense::hover());
                                            if rr_resp.hovered() {
                                                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                                            }

                                            // Белый — если проект активен ИЛИ наведён курсором
                                            // (фоновую плашку-подсветку убрали — при таких низких
                                            // альфах она была практически незаметна; вместо неё
                                            // сигналит сам цвет текста).
                                            let label_col = if is_active || rr_resp.hovered() {
                                                Color32::WHITE
                                            } else {
                                                Color32::from_gray(190)
                                            };

                                            ui.painter().circle_filled(
                                                rr.min + vec2(7.0, ROW_H / 2.0),
                                                3.0, proj.color);

                                            let mut job = egui::text::LayoutJob::simple_singleline(
                                                proj.name.clone(), font.clone(), label_col);
                                            job.wrap.max_width         = text_avail;
                                            job.wrap.max_rows          = 1;
                                            job.wrap.overflow_character = Some('…');
                                            let galley = ctx.fonts_mut(|f| f.layout_job(job));
                                            ui.painter().galley(
                                                rr.min + vec2(15.0, (ROW_H - galley.size().y) / 2.0),
                                                galley, label_col);

                                            let name_rect = egui::Rect::from_min_size(
                                                rr.min,
                                                vec2(row_w - 18.0, ROW_H),
                                            );
                                            let name_resp = ui.allocate_rect(name_rect, Sense::click());
                                            if name_resp.hovered() {
                                                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                                            }
                                            if name_resp.clicked() {
                                                select_project = Some(i);
                                            }

                                            if self.projects.len() > 1 {
                                                let del_rect = egui::Rect::from_min_size(
                                                    rr.min + vec2(row_w - 16.5, (ROW_H - 15.0) / 2.0),
                                                    vec2(15.0, 15.0),
                                                );
                                                let del_resp = ui.allocate_rect(del_rect, Sense::click());
                                                let cross_tint = if del_resp.hovered() {
                                                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
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
                                    });
                            });
                    });

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
                        let p     = project::LoadedProject::new(project::gen_id(), name, color, ts);
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
                    self.switch_to_project(i);
                    self.project_open   = false;
                    self.project_adding = false;
                    self.project_buf.clear();
                }
                if open_settings {
                    self.project_open   = false;
                    self.project_adding = false;
                    self.project_buf.clear();
                    self.screen = Screen::Settings;
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(
                        vec2(settings::SW, settings::SH_GENERAL),
                    ));
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

        // ── main task ────────────────────────────────────────────────────
        // Y-координата разделителя main/subs нужна ЗАРАНЕЕ (до отрисовки
        // main), чтобы понять, где сейчас курсор при перетаскивании (выше
        // разделителя — "готовим promote", ниже — "готовим вставку в
        // subs"), и подменить текст main-задачи превью перетаскиваемой.
        // main_h — уже известная фиксированная высота main-блока, поэтому
        // divider_y можно посчитать до, а не после его отрисовки.
        let divider_y = ui.next_widget_position().y + main_h;
        let drag_pointer = if self.dragging_task.is_some() {
            ctx.input(|i| i.pointer.interact_pos())
        } else {
            None
        };
        let dragging_above_divider = drag_pointer.map_or(false, |p| p.y < divider_y);

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
                    let now = routine_scheduler::local_now();
                    let _ = self.sync.record_op(sync::oplog::OpKind::CompleteTask {
                        project_id: self.projects[idx].id.clone(),
                        task_id:    task_id.clone(),
                    });
                    self.projects[idx].complete_task(&task_id, now, project::current_time());
                    self.projects[idx].save();
                } else if let Some(pos) = self.projects[idx].subs.iter()
                    .position(|(_, t)| project::is_active_task(t))
                {
                    // main пуст (прочерк) — но есть свободная активная
                    // задача/рутина в subs. Клик по прочерку теперь сам
                    // проверяет это и продвигает её, вместо того чтобы
                    // просто ничего не делать.
                    let task_id    = self.projects[idx].subs.get_index(pos).unwrap().0.clone();
                    let project_id = self.projects[idx].id.clone();
                    let ts         = project::current_time();
                    let _ = self.sync.record_op(sync::oplog::OpKind::PromoteTask {
                        project_id, task_id: task_id.clone(),
                    });
                    self.projects[idx].apply_promote_task(&task_id, ts);
                    self.projects[idx].save();
                }
            }
            ui.add_space(8.0);
            let avail = ui.available_rect_before_wrap();
            // Пока задачу тащат выше разделителя — вместо обычного текста
            // main показываем ПРЕВЬЮ: текст перетаскиваемой задачи (и её
            // routine-статус — для подчёркивания ниже), как будет выглядеть
            // после promote при отпускании именно здесь.
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
            // Превью печатается тем же максимально белым цветом, что и
            // обычный main-текст — никакой прозрачности (раньше была,
            // убрали по просьбе).
            let mut job = egui::text::LayoutJob::simple(
                text_str.to_owned(),
                egui::FontId::proportional(15.0),
                Color32::WHITE,
                avail.width() - 10.0,
            );
            // Жёлтое подчёркивание — если задача является рутиной. Либо
            // текущая main-задача (обычный режим), либо перетаскиваемая
            // задача в preview-режиме (её будущий статус после promote).
            let is_routine = match &dragged_preview {
                Some((_, has_routine)) => *has_routine,
                None => self.projects[self.active_project_idx].main
                    .values().next().map_or(false, |t| t.routine.is_some()),
            };
            if is_routine {
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
        // Реальные индексы в subs, отсортированные под тумблер
        // group_inactive_at_end. Физический порядок в IndexMap НЕ трогаем —
        // это чисто display-time сортировка.
        // task_text/is_active идут рядом, чтобы не дёргать subs повторно
        // в цикле отрисовки.
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
        // иначе (тумблер выключен) — оставляем физический порядок как есть

        // Если в этом кадре инлайн-редактирование текста задачи (карандаш)
        // было закоммичено/отменено по Enter — тем же нажатием НЕ должно
        // ещё и активироваться поле "Добавить..." ниже (см. global_enter).
        let mut edit_just_finished = false;

        if !display_order.is_empty() {
            let (mut promote, mut delete, mut open_routine) = (None::<usize>, None::<usize>, None::<usize>);
            // Так же, как promote/delete/open_routine — НЕ мутируем self
            // напрямую внутри цикла отрисовки. Если начать драг или
            // редактирование прямо посреди цикла, а суммарная высота списка
            // из-за этого поменяется (пропадёт/появится строка), egui может
            // тем же кадром перезапустить internal pass (auto_shrink) — и
            // тот pass увидит self.dragging_task/editing_task УЖЕ
            // изменённым, тогда как первый pass успел отрисовать часть строк
            // до изменения. Структура между двумя pass'ами одного кадра
            // расходится → "Widget rect changed id between passes" (см. баг
            // с красными квадратами, который мы уже один раз чинили).
            // Откладывая мутацию до конца цикла, гарантируем, что
            // self.dragging_task/editing_task не меняются в течение ВСЕГО
            // текущего кадра — вступают в силу только со следующего.
            let mut start_drag: Option<usize> = None;
            let mut start_edit: Option<usize> = None;
            let (mut commit_edit, mut cancel_edit) = (false, false);
            // Пока какая-то задача редактируется ИЛИ перетаскивается — весь
            // список залочен для остальных кнопок (не даём кликать по
            // чужим крестикам/routine/карандашу/тексту, пока идёт правка
            // или drag).
            let list_locked = self.editing_task.is_some() || self.dragging_task.is_some();

            // Высота ScrollArea — по ПОЛНОМУ числу строк, ВСЕГДА (не по
            // "фактически видимых"). Раньше вычитали одну строку, пока идёт
            // драг, но раз строка теперь не пропускается, а просто рисуется
            // с высотой 0 (см. ниже) — реальная суммарная высота контента и
            // не меняется, значит и scroll_h не должен.
            let scroll_h = display_order.len().min(9) as f32 * (ROW - 5.0);
            // Реальные экранные прямоугольники строк (в display-порядке) —
            // нужны после цикла, чтобы по Y координате курсора найти щель
            // под белую полоску-индикатор и вычислить target для reorder_sub.
            let mut row_rects: Vec<(usize, egui::Rect)> = Vec::with_capacity(display_order.len());
            // Запоминаем исходный вертикальный item_spacing — его тоже нужно
            // обнулять вокруг перетаскиваемой строки (см. ниже), иначе даже
            // при высоте 0 между соседними строками остаётся фиксированный
            // зазор ScrollArea, который сам по себе не зависит от высоты
            // конкретного виджета — отсюда и "неполная" пустая строка.
            let default_item_spacing_y = ui.spacing().item_spacing.y;
            let default_interact_size  = ui.spacing().interact_size;
            let mut prev_was_dragged = false;
            // Видимая (обрезанная) область скролла — нужна и здесь (авто-скролл
            // у края), и позже, после цикла (чтобы не рисовать белую полоску
            // выше/ниже реальной зоны отрисовки subs — над разделителем или под
            // списком, в зоне кнопки "Добавить...").
            let mut scroll_viewport: Option<egui::Rect> = None;
            egui::ScrollArea::vertical()
                .max_height(scroll_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    scroll_viewport = Some(ui.clip_rect());
                    // Ручной скролл во время драга — и колёсиком, и у края списка.
                    // Штатную обработку колёсика ScrollArea НЕ трогаем (работает
                    // как обычно вне драга); добавляем СВОЙ путь, включаем только
                    // пока dragging_task активен, чтобы не задвоить скролл.
                    if let Some(pointer) = drag_pointer {
                        if self.dragging_task.is_some() {
                            let viewport = ui.clip_rect();
                            let mut delta_y = 0.0_f32;

                            // Колёсико — читаем вручную и применяем сами (штатный
                            // путь ScrollArea, судя по всему, что-то блокирует
                            // во время активного Sense::drag() на дочернем
                            // виджете). Без множителя — у каждого своя скорость
                            // колёсика в ОС, подстроить универсально всё равно
                            // не выйдет, так что просто передаём как есть.
                            delta_y += ctx.input(|i| i.smooth_scroll_delta.y);

                            // Автоскролл у края видимой области — скорость растёт
                            // линейно от 0 (на границе зоны) до максимума (у
                            // самого края), в обе стороны. EDGE_SCROLL_MULTIPLIER —
                            // сюда крутить, если скорость авто-скролла у края
                            // ощущается медленно/быстро.
                            const EDGE_ZONE: f32 = 20.0;
                            const MAX_EDGE_SPEED: f32 = 300.0; // px/сек, база
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

                        // Перетаскиваемая строка НЕ пропускается циклом (никакого
                        // `continue`!) — вызовы push_id/allocate_ui_with_layout
                        // происходят КАЖДЫЙ кадр для КАЖДОЙ строки, идентично.
                        // "Схлопывание" делаем через высоту 0.0 у ЭТОЙ строки —
                        // тот же приём, что и анимация кнопок (меняется только
                        // число в vec2(), не факт вызова). Раньше строка просто
                        // пропускалась через `continue`, что меняло СУММАРНУЮ
                        // высоту контента ScrollArea между кадрами — а именно
                        // это, судя по всему, и провоцирует egui перезапустить
                        // internal pass (auto_shrink пересчитывает размер), из-за
                        // чего мутация self.dragging_task — даже отложенная после
                        // цикла — всё равно оказывалась видна "второму" pass'у
                        // того же кадра. Если суммарная высота контента вообще
                        // никогда не меняется, пересчитывать нечего — переезжать
                        // между pass'ами нечему.
                        let row_h = if is_dragged_row { 0.0 } else { ROW - 5.0 };

                        // item_spacing тоже обнуляем — и ПЕРЕД этой строкой (если
                        // она сама перетаскивается), и ПЕРЕД следующей (если
                        // предыдущей была перетаскиваемая) — иначе зазор остаётся
                        // с одной из двух сторон. Это чисто стилевая правка
                        // (spacing не создаёт виджетов и не потребляет Id), так что
                        // безопасна даже будучи завязанной на is_dragged_row.
                        ui.spacing_mut().item_spacing.y =
                            if is_dragged_row || prev_was_dragged { 0.0 } else { default_item_spacing_y };
                        prev_was_dragged = is_dragged_row;
                        // interact_size — минимальный размер, который egui может
                        // подставлять под интерактивные виджеты, даже если явно
                        // просишь меньше (например, у allocate_exact_size/
                        // allocate_ui_with_layout есть свой floor на этот счёт).
                        // Раз строка всё равно не кликабельна, пока перетаскивается
                        // (list_locked), занулить его для неё безопасно и по
                        // смыслу, и по структуре (тоже просто стиль, не виджет).
                        ui.spacing_mut().interact_size =
                            if is_dragged_row { egui::vec2(0.0, 0.0) } else { default_interact_size };

                        // Прямоугольник ВСЕЙ строки — вычисляем до отрисовки, чтобы
                        // без лага в кадр знать, наведён ли курсор именно на строку
                        // целиком (а не только когда текст под указателем).
                        let row_rect = egui::Rect::from_min_size(
                            ui.cursor().min, vec2(self.w, row_h));
                        let row_hovered = ui.rect_contains_pointer(row_rect);
                        // Кнопки видны только на hover обычной строки; если строка
                        // сама редактируется — у неё всегда своя галочка (см. ниже);
                        // если залочена из-за редактирования ДРУГОЙ строки — кнопки
                        // не показываем вовсе, они всё равно нерабочие сейчас.
                        let show_buttons = row_hovered && !list_locked;

                        // push_id — обязательный паттерн egui для виджетов в цикле:
                        // без него авто-Id внутри строки строятся по счётчику вызовов
                        // ui.put/allocate_exact_size, а этот счётчик "плывёт" между
                        // кадрами из-за show_buttons (разное число реально нарисованных
                        // виджетов на разных строках/кадрах) — отсюда и ID-клэш с
                        // красными предупреждениями egui. push_id(real_i, ...) солит
                        // все Id внутри строки стабильным индексом задачи и полностью
                        // убирает возможность коллизии.
                        let row_resp = ui.push_id(real_i, |ui| {
                        ui.allocate_ui_with_layout(
                            vec2(self.w, row_h),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                if is_editing {
                                    // ── режим редактирования этой строки ───────
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
                                    // ── обычный режим строки ───────────────────
                                    // ВАЖНО: все три ui.put()-виджета вызываются
                                    // БЕЗУСЛОВНО каждый раз — структура виджетов
                                    // должна быть идентичной на каждом internal
                                    // pass'е egui (multi-pass layout, см. auto_shrink
                                    // у ScrollArea), иначе разное число виджетов
                                    // между passes/кадрами сбивает автогенерируемые
                                    // Id ("Widget rect changed id between passes").
                                    // Появление/исчезновение — только через ПЛАВНОЕ
                                    // изменение размера/отступа (t) и альфы тинта,
                                    // а не наличие/отсутствие ui.put — количество
                                    // вызовов остаётся неизменным на любом pass'е.
                                    let anim_id = ui.id().with("btns_anim");
                                    let t = ctx.animate_bool_with_time(anim_id, show_buttons, 0.15);
                                    // Для перетаскиваемой строки схлопывание должно быть
                                    // МГНОВЕННЫМ (список сразу смыкается), а не за 150мс, как
                                    // обычное появление/исчезновение кнопок на hover — эту
                                    // анимацию трогать не хотим, она отдельно нравится. Поэтому
                                    // ctx.animate_bool_with_time всё равно вызываем каждый кадр
                                    // (чтобы не задеть саму механику анимации у остальных строк),
                                    // но РЕЗУЛЬТАТ для этой конкретной строки перебиваем на
                                    // жёсткий 0.0 — визуально мгновенно, при этом ничего не
                                    // меняя в структуре вызовов ниже (по-прежнему используется
                                    // одна и та же переменная `t`).
                                    let t = if is_dragged_row { 0.0 } else { t };
                                    // Забеливание на hover — через alpha-compositing (умножение
                                    // тинтом не может выбелить цветной пиксель, только притемнить
                                    // или оставить как есть). Базовый слой — оригинальные цвета
                                    // иконки. Поверх — ТОТ ЖЕ PNG ещё раз с белым тинтом; ВАЖНО:
                                    // этот второй ui.put() вызывается БЕЗУСЛОВНО каждый кадр,
                                    // видимость регулируется ТОЛЬКО его альфой (0 = невидим на вид,
                                    // но структурно всё равно нарисован) — а не наличием/
                                    // отсутствием вызова, иначе снова ловим "Widget rect changed
                                    // id between passes" (тот самый баг с красными квадратами).
                                    // Забеленная версия рисуется поверх обычной иконки на
                                    // hover — теперь это отдельный PNG-файл (cross_light.png
                                    // и т.п.), где нужная альфа уже запечена в самом файле
                                    // (см. make_light_icons.py), так что программно регулируем
                                    // только "показать/спрятать" через альфу 0/255 — БЕЗУСЛОВНО
                                    // рисуется каждый кадр, меняется только эта альфа, а не
                                    // наличие вызова (см. правило про "Widget rect changed id
                                    // between passes").

                                    ui.add_space(7.0 * t);
                                    let (btn_rect, btn_resp) = ui.allocate_exact_size(
                                        vec2(15.0 * t, 15.0 * t), Sense::click());
                                    let btn_hovered = btn_resp.hovered() && show_buttons;
                                    if btn_hovered { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                                    // Обычное состояние — не полная яркость (231/255), на hover —
                                    // полная (255) плюс поверх ещё и light-иконка (см. ниже).
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
                                        // Неактивная рутина — тусклый текст, не кликабельна
                                        // для promote (нельзя протолкнуть в main, пока не
                                        // сработало расписание). Перетаскиваемая (is_dragged_row)
                                        // строка — невидимый текст-плейсхолдер: место
                                        // зарезервировано, но сам текст сейчас "летит" за
                                        // курсором в виде превью на main/белой полоски.
                                        let text_color = if is_dragged_row {
                                            Color32::TRANSPARENT
                                        } else if is_active {
                                            Color32::from_gray(200)
                                        } else {
                                            Color32::from_gray(90)
                                        };
                                        // Sense::drag() — клик по тексту больше НЕ promote'ит
                                        // (это делает только перетаскивание выше разделителя),
                                        // поэтому click() тут не нужен, только drag.
                                        let sense = if is_active && !list_locked {
                                            Sense::drag()
                                        } else {
                                            Sense::hover()
                                        };
                                        // Подчёркивание — отдельный, независимый от text_color
                                        // сигнал "это рутина", только у активных строк
                                        // (тусклые/неактивные не подчёркиваем). Через
                                        // LayoutJob/TextFormat, а не
                                        // RichText::underline() — иначе цвет линии слипается
                                        // с цветом текста.
                                        let mut fmt = egui::text::TextFormat {
                                            // Прозрачность не уменьшает размер текста — а нам
                                            // нужно, чтобы перетаскиваемая строка не держала
                                            // высоту вообще (как и кнопки выше). Схлопываем сам
                                            // шрифт до почти нуля именно для неё.
                                            font_id: egui::FontId::proportional(
                                                if is_dragged_row { 1.0 } else { 13.0 }),
                                            color: text_color,
                                            ..Default::default()
                                        };
                                        if has_routine && is_active && !is_dragged_row {
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
                                        // Курсор-"перемещение" (не палец) — подсказывает,
                                        // что тут именно drag, а не клик; клик по тексту
                                        // больше НИЧЕГО не делает — promote теперь только
                                        // через перетаскивание выше разделителя.
                                        if is_active && !list_locked && label_r.hovered() {
                                            ctx.set_cursor_icon(egui::CursorIcon::AllScroll);
                                        }
                                        if is_active && !list_locked && label_r.drag_started() {
                                            start_drag = Some(real_i);
                                        }
                                    });
                                }
                            },
                        ).response
                        });
                        // Саму строку по-прежнему рисуем безусловно (структура вызовов
                        // не меняется — это важно для стабильности Id, см. комментарии
                        // выше). А вот в row_rects её не кладём, если это перетаскиваемая
                        // строка — она физически всё ещё "здесь" (высота 0, но позиция
                        // никуда не делась), и если её оставить как кандидата для щели,
                        // рядом с реальной соседней щелью появляется почти совпадающий,
                        // но не идентичный дубликат — отсюда "две линии в один пиксель".
                        if !is_dragged_row {
                            row_rects.push((real_i, row_resp.inner.rect));
                        }
                    }
                });

            edit_just_finished = commit_edit || cancel_edit;
            if edit_just_finished {
                if let Some(i) = self.editing_task {
                    if commit_edit {
                        let new_text = self.edit_buf.trim().to_string();
                        let idx = self.active_project_idx;
                        let old_text = self.projects[idx].subs.get_index(i).map(|(_, t)| t.text.clone());
                        // Пустой текст или текст не поменялся — не шлём EDIT_TASK,
                        // просто выходим из режима редактирования.
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

            // Применяем отложенные мутации ТОЛЬКО теперь, когда весь цикл
            // отрисовки для этого кадра уже полностью завершён — см.
            // комментарий у объявления start_drag/start_edit выше.
            if let Some(i) = start_edit {
                self.editing_task   = Some(i);
                self.edit_buf       = self.projects[idx].subs.get_index(i)
                    .map(|(_, t)| t.text.clone()).unwrap_or_default();
                self.edit_need_focus = true;
            }
            if let Some(i) = start_drag {
                self.dragging_task = Some(i);
            }

            // ── drag & drop: щель под курсором, белая полоска, коммит по отпусканию ──
            if let Some(dragged_real_i) = self.dragging_task {
                let pointer_released = ctx.input(|i| i.pointer.any_released());
                match drag_pointer {
                    None => {
                        // Не смогли получить позицию курсора этим кадром (обычно
                        // случается только на самом первом кадре drag_started,
                        // до следующего кадра) — просто ничего не делаем.
                        if pointer_released { self.dragging_task = None; }
                    }
                    Some(pointer) if dragging_above_divider => {
                        // Курсор выше разделителя — при отпускании работает
                        // РОВНО тот же путь, что и обычный клик по задаче
                        // (promote), просто источник события другой.
                        let _ = pointer;
                        if pointer_released {
                            promote = Some(dragged_real_i);
                            self.dragging_task = None;
                        }
                    }
                    Some(pointer) => {
                        // Курсор ниже разделителя — ищем ближайшую щель между
                        // строками (в display-порядке) по Y координате курсора:
                        // первая строка, чей центр ниже курсора — вставляем
                        // перед ней; если такой нет — курсор ниже всех строк,
                        // вставляем в самый конец (разрешено явно).
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
                        // Линия не должна залезать выше разделителя или ниже
                        // реально отрисованной зоны subs (там уже начинается
                        // "Добавить...") — такое может произойти, если ближайшая
                        // щель приходится на строку, физически прокрученную за
                        // пределы видимой области. Прижимаем к границам видимого
                        // viewport'а ScrollArea.
                        if let Some(viewport) = scroll_viewport {
                            line_y = line_y.clamp(viewport.top(), viewport.bottom());
                        }
                        ui.painter().hline(0.0..=self.w, line_y, (0.41, SEP));
                        if pointer_released {
                            self.projects[idx].reorder_sub(dragged_real_i, insert_before);
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
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(
                    vec2(ui::routine::RW, self.routine_ui.target_height()),
                ));
            }
            if let Some(i) = delete {
                let is_active_routine = self.projects[idx].subs.get_index(i)
                    .map(|(_, t)| t.routine.is_some() && project::is_active_task(t))
                    .unwrap_or(false);

                if is_active_routine {
                    // Крестик по активной рутине = "сделал досрочно, без
                    // main" — первый слой брони, не удаление. Задача не
                    // двигается: остаётся ровно там же, физически.
                    let task_id    = self.projects[idx].subs.get_index(i).unwrap().0.clone();
                    let project_id = self.projects[idx].id.clone();
                    let now        = routine_scheduler::local_now();
                    let _          = self.sync.record_op(sync::oplog::OpKind::CompleteTask {
                        project_id, task_id: task_id.clone(),
                    });
                    self.projects[idx].complete_task(&task_id, now, project::current_time());
                } else {
                    // Обычная задача, либо уже неактивная рутина — реальное
                    // удаление, как раньше.
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
                        let task_id = project::gen_id();
                        let target  = add_target_for(&s, self.projects[idx].main.is_empty());
                        let _       = self.sync.record_op(sync::oplog::OpKind::AddTask {
                            project_id: self.projects[idx].id.clone(),
                            task_id:    task_id.clone(),
                            text:       text.clone(),
                            target,
                        });
                        self.projects[idx].add_task(task_id, text, &s);
                        self.projects[idx].save();
                    } else {
                        self.buf.clear();
                    }
                    self.adding = false;
                    ctx.memory_mut(|m| { if let Some(id) = m.focused() { m.surrender_focus(id); } });
                }
            } else {
                // Не даём Enter'у активировать "Добавить...", если этим же Enter'ом
                // только что закоммитили/отменили инлайн-редактирование текста
                // задачи (карандаш) — иначе один Enter делает сразу два дела.
                // editing_task к этому моменту уже сброшен в None выше по кадру,
                // поэтому проверяем именно edit_just_finished, а не editing_task.
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
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(self.w, h)));
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
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(self.w, h)));
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
    let _ = std::fs::create_dir_all(app_dir());
    // Init file logger before anything else — GUI apps have no console on Windows.
    LOG_PATH.set(app_dir().join("debug.log")).ok();
    // Truncate log on each run so it doesn't grow forever during debugging.
    let _ = std::fs::write(app_dir().join("debug.log"), "");
    // Подключаем FileLogger к стандартному log-фасаду — без этого egui
    // молча глотал бы свои внутренние warn! (в т.ч. "id clash"), и красные
    // квадраты на экране были бы без единого объяснения, откуда они.
    let _ = log::set_logger(&FILE_LOGGER).map(|()| log::set_max_level(log::LevelFilter::Warn));
    clog!("=== Cue started ===");

    if let Some(lock) = read_lock() {
        if lock_is_fresh(&lock) && is_alive(lock.pid) {
            focus::focus_pid(lock.pid);
            return Ok(());
        }
    }
    write_lock();

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