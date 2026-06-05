#![windows_subsystem = "windows"]

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod settings;
pub mod project;
pub mod sync;

use eframe::egui::{
    self, Align, Color32, ImageSource, Layout, RichText,
    Sense, Stroke, Ui, ViewportCommand, vec2,
};
use serde::{Deserialize, Serialize};
use std::mem;

pub const W:     f32   = 254.0;
pub const MIN_W: f32   = 180.0;
pub const ROW:   f32   = 28.0;
pub const BG:  Color32 = Color32::from_rgba_premultiplied(9, 9, 11, 222);
pub const SEP: Color32 = Color32::from_rgba_premultiplied(255, 255, 255, 10);

static TICK_PNG:  &[u8] = include_bytes!("../pics/tick.png");
static CROSS_PNG: &[u8] = include_bytes!("../pics/cross.png");
static ICON_PNG:  &[u8] = include_bytes!("../icon.png");

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
    now.saturating_sub(l.time) < 86400
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

enum Screen { Main, Settings }

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
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut vis = egui::Visuals::dark();
        vis.panel_fill    = Color32::TRANSPARENT;
        vis.window_fill   = Color32::TRANSPARENT;
        vis.window_stroke = Stroke::NONE;
        cc.egui_ctx.set_visuals(vis);

        let _ = std::fs::create_dir_all(project::projects_dir());
        let mut settings = settings::Settings::load();
        let mut projects = project::load_all_projects();

        if projects.is_empty() {
            projects.push(project::create_default_project());
        }

        let sync = sync::SyncHandle::init(&mut projects);

        let last_id    = settings.last_project_id.clone();
        let active_idx = last_id.as_deref()
            .and_then(|id| projects.iter().position(|p| p.id == id))
            .unwrap_or(0);

        let actual_id = projects[active_idx].id.clone();
        if last_id.as_deref() != Some(&actual_id) {
            settings.last_project_id = Some(actual_id);
            settings.save();
        }

        let initial_w = settings.last_width.unwrap_or(W);

        let mut app = Self {
            settings,
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
        };

        if app.settings.reset_on_startup {
            let idx = app.active_project_idx;
            app.projects[idx].main.clear();
            app.projects[idx].subs.clear();
        }

        app
    }

    fn switch_to_project(&mut self, idx: usize) {
        if idx == self.active_project_idx { return; }
        self.active_project_idx = idx;
        self.settings.last_project_id = Some(self.projects[idx].id.clone());
        self.settings.save();
    }
}

// ── render ───────────────────────────────────────────────────────────────────

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] { [0.0; 4] }


    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

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
                            let first = iter.next().unwrap();
                            self.projects[idx].main.insert(
                                project::gen_id(),
                                project::TaskData {
                                    text:       first,
                                    active:     true,
                                    schedule:   None,
                                    created_at: project::current_time(),
                                    order_key:  0.0,
                                },
                            );
                        }
                        for text in iter {
                            self.projects[idx].add_task(text, &s);
                        }
                        if !self.settings.reset_on_startup {
                            self.projects[idx].save();
                        }
                    }
                }
            }
        }

        if let Screen::Settings = self.screen {
            if settings::draw_settings_ui(&ctx, ui, &mut self.settings) {
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
                            .fill(Color32::from_rgba_premultiplied(0, 0, 0, 204))
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
                                                ui.painter().rect_filled(rr, 3.0,
                                                    Color32::from_white_alpha(12));
                                            } else if is_active {
                                                ui.painter().rect_filled(rr, 3.0,
                                                    Color32::from_white_alpha(8));
                                            }

                                            let label_col = if is_active {
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
                                                }).fit_to_exact_size(vec2(8.0, 8.0)).tint(cross_tint));
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
                        let p = project::LoadedProject::new(
                            project::gen_id(), name, color, project::current_time(),
                        );
                        p.save();
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
                        vec2(settings::SW, settings::SH),
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
                self.projects[idx].complete_main();
                if !self.settings.reset_on_startup { self.projects[idx].save(); }
            }
            ui.add_space(8.0);
            let avail    = ui.available_rect_before_wrap();
            let text_str = self.projects[self.active_project_idx].main_text().unwrap_or("—");
            let job = egui::text::LayoutJob::simple(
                text_str.to_owned(),
                egui::FontId::proportional(15.0),
                Color32::WHITE,
                avail.width() - 10.0,
            );
            let galley = ctx.fonts_mut(|f| f.layout_job(job));
            let pos = avail.min + vec2(10.0, (avail.height() - galley.size().y) / 2.0 - 1.0);
            ui.painter().galley(pos, galley, Color32::WHITE);
            ui.allocate_exact_size(avail.size(), Sense::hover());
        });

        let y = ui.next_widget_position().y;
        ui.painter().hline(0.0..=self.w, y, (0.5, SEP));
        ui.add_space(6.0);

        // ── sub tasks ────────────────────────────────────────────────────
        let sub_texts: Vec<String> = self.projects[self.active_project_idx]
            .subs.values()
            .map(|t| t.text.clone())
            .collect();

        if !sub_texts.is_empty() {
            let (mut promote, mut delete) = (None::<usize>, None::<usize>);

            let scroll_h = sub_texts.len().min(9) as f32 * (ROW - 5.0);
            egui::ScrollArea::vertical()
                .max_height(scroll_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, task_text) in sub_texts.iter().enumerate() {
                        ui.allocate_ui_with_layout(
                            vec2(self.w, ROW - 5.0),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                ui.add_space(7.0);
                                let (btn_rect, btn_resp) = ui.allocate_exact_size(
                                    vec2(15.0, 15.0), Sense::click());
                                let cross_tint = if btn_resp.hovered() {
                                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                                    Color32::WHITE
                                } else {
                                    Color32::from_gray(130)
                                };
                                ui.put(btn_rect, egui::Image::new(ImageSource::Bytes {
                                    uri: "bytes://cross.png".into(),
                                    bytes: CROSS_PNG.into(),
                                }).fit_to_exact_size(vec2(8.0, 8.0)).tint(cross_tint));
                                if btn_resp.clicked() { delete = Some(i); }

                                ui.add_space(6.0);
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.add_space(10.0);
                                    let label_r = ui.add(
                                        egui::Label::new(
                                            RichText::new(task_text.as_str())
                                                .color(Color32::from_gray(200))
                                                .size(13.0),
                                        )
                                        .truncate()
                                        .sense(Sense::click()),
                                    );
                                    if label_r.hovered() {
                                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                    if label_r.clicked() { promote = Some(i); }
                                });
                            },
                        );
                    }
                });

            let idx = self.active_project_idx;
            if let Some(i) = promote {
                self.projects[idx].promote_sub(i);
                if !self.settings.reset_on_startup { self.projects[idx].save(); }
            }
            if let Some(i) = delete {
                self.projects[idx].delete_sub(i);
                if !self.settings.reset_on_startup { self.projects[idx].save(); }
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
                        let text = mem::take(&mut self.buf);
                        let s    = self.settings.clone();
                        let idx  = self.active_project_idx;
                        self.projects[idx].add_task(text, &s);
                        if !s.reset_on_startup { self.projects[idx].save(); }
                    } else {
                        self.buf.clear();
                    }
                    self.adding = false;
                    ctx.memory_mut(|m| { if let Some(id) = m.focused() { m.surrender_focus(id); } });
                }
            } else {
                let global_enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));

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
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    );

    delete_lock();
    result
}