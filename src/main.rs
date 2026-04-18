#![windows_subsystem = "windows"]

use eframe::egui::{
    self, Align, Color32, ImageSource, Layout, RichText,
    Sense, Stroke, Ui, ViewportCommand, vec2,
};
use serde::{Deserialize, Serialize};
use std::mem;

const W: f32 = 260.0;
const ROW: f32 = 28.0;
const BG:  Color32 = Color32::from_rgba_premultiplied(9, 9, 11, 222);
const SEP: Color32 = Color32::from_rgba_premultiplied(255, 255, 255, 10);

static TICK_PNG:  &[u8] = include_bytes!("../pics/tick.png");
static CROSS_PNG: &[u8] = include_bytes!("../pics/cross.png");
static ICON_PNG:  &[u8] = include_bytes!("../icon.png");

// ── data ─────────────────────────────────────────────────────────────────────

fn data_path() -> std::path::PathBuf {
    std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.join("tasks.json")))
        .unwrap_or_else(|| "tasks.json".into())
}

#[derive(Serialize, Deserialize, Default)]
struct Tasks {
    main: String,
    subs: Vec<String>,
}

impl Tasks {
    fn load() -> Self {
        std::fs::read_to_string(data_path()).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save(&self) {
        if self.main.is_empty() && self.subs.is_empty() {
            let _ = std::fs::remove_file(data_path());
        } else if let Ok(j) = serde_json::to_string(self) {
            let _ = std::fs::write(data_path(), j);
        }
    }
}

// ── app ──────────────────────────────────────────────────────────────────────

struct App {
    t:             Tasks,
    adding:        bool,
    need_focus:    bool,
    focus_add_btn: bool,
    buf:           String,
    last_h:        f32,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut vis = egui::Visuals::dark();
        vis.panel_fill    = Color32::TRANSPARENT;
        vis.window_fill   = Color32::TRANSPARENT;
        vis.window_stroke = Stroke::NONE;
        cc.egui_ctx.set_visuals(vis);

        Self { t: Tasks::load(), adding: false, need_focus: false, focus_add_btn: false, buf: String::new(), last_h: 0.0 }
    }
}

// ── render ───────────────────────────────────────────────────────────────────

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] { [0.0; 4] }

    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        let text_w = W - 10.0 - 8.0 - 28.0 - 10.0;

        let main_h: f32 = {
            let text = if self.t.main.is_empty() { "—" } else { &self.t.main };
            let job = egui::text::LayoutJob::simple(
                text.to_owned(),
                egui::FontId::proportional(15.0),
                Color32::WHITE,
                text_w,
            );
            let text_h = ctx.fonts_mut(|f| f.layout_job(job).size().y);
            (text_h + 20.0).max(46.0)
        };

        let h = 12.0 + main_h + 5.0 + self.t.subs.len().min(9) as f32 * ROW + 28.0;

        if (h - self.last_h).abs() > 0.5 {
            self.last_h = h;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(W, h)));
        }

        ui.painter().rect_filled(ui.max_rect(), 10.0, BG);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);

        // ── drag bar ─────────────────────────────────────────────────────
        let bar_rect = egui::Rect::from_min_size(ui.next_widget_position(), vec2(W, 12.0));
        let drag = ui.allocate_rect(bar_rect, Sense::drag());
        if drag.dragged() { ctx.send_viewport_cmd(ViewportCommand::StartDrag); }

        let close_center = bar_rect.max - vec2(11.0, 2.0);
        let c = bar_rect.center();

        {
            let p = ui.painter();
            p.circle_filled(bar_rect.min + vec2(10.0, 10.5), 2.0, Color32::from_rgb(59, 130, 246));
            p.text(
                bar_rect.min + vec2(17.0, 10.5),
                egui::Align2::LEFT_CENTER,
                "Cue",
                egui::FontId::proportional(10.5),
                Color32::from_white_alpha(180),
            );
            for x in [-6.0f32, 0.0, 6.0] {
                p.circle_filled(c + vec2(x, 4.0), 1.5, Color32::from_white_alpha(35));
            }
        }

        let close_resp = ui.allocate_rect(
            egui::Rect::from_center_size(close_center, vec2(10.0, 5.0)),
            Sense::click(),
        );
        let close_col = if close_resp.hovered() { Color32::from_rgb(255, 80, 80) } else { Color32::from_rgb(220, 50, 50) };
        ui.painter().circle_filled(close_center, 3.8, close_col);
        if close_resp.clicked() { ctx.send_viewport_cmd(ViewportCommand::Close); }

        // ── main task ────────────────────────────────────────────────────
        ui.allocate_ui_with_layout(vec2(W, main_h), Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(10.0);
            if ui.add(
                egui::Button::image(ImageSource::Bytes {
                    uri: "bytes://tick.png".into(),
                    bytes: TICK_PNG.into(),
                })
                .fill(Color32::from_rgb(34, 197, 94))
                .corner_radius(6.0)
                .min_size(vec2(28.0, 28.0)),
            ).clicked() {
                self.t.main = if self.t.subs.is_empty() {
                    String::new()
                } else {
                    self.t.subs.remove(0)
                };
                self.t.save();
            }
            ui.add_space(8.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(if self.t.main.is_empty() { "—" } else { &self.t.main })
                            .color(Color32::WHITE)
                            .size(15.0),
                    ).wrap(),
                );
            });
        });

        let y = ui.next_widget_position().y;
        ui.painter().hline(0.0..=W, y, (0.5, SEP));
        ui.add_space(5.0);

        // ── sub tasks ────────────────────────────────────────────────────
        if !self.t.subs.is_empty() {
            let (mut promote, mut delete) = (None, None);

            let scroll_h = self.t.subs.len().min(9) as f32 * ROW;
            egui::ScrollArea::vertical()
                .max_height(scroll_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, task) in self.t.subs.iter().enumerate() {
                        ui.allocate_ui_with_layout(
                            vec2(W, ROW),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                ui.add_space(10.0);
                                let (btn_rect, btn_resp) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::click());
                                let btn_col = if btn_resp.hovered() { Color32::from_rgb(255, 80, 80) } else { Color32::from_rgb(239, 68, 68) };
                                ui.painter().circle_filled(btn_rect.center(), 8.0, btn_col);
                                ui.put(btn_rect, egui::Image::new(ImageSource::Bytes {
                                    uri: "bytes://cross.png".into(),
                                    bytes: CROSS_PNG.into(),
                                }).fit_to_exact_size(vec2(8.0, 8.0)));
                                if btn_resp.clicked() { delete = Some(i); }

                                ui.add_space(6.0);
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.add_space(10.0);
                                    let label_r = ui.add(
                                        egui::Label::new(
                                            RichText::new(task.as_str())
                                                .color(Color32::from_gray(140))
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

            if let Some(i) = promote {
                if self.t.main.is_empty() {
                    self.t.main = self.t.subs.remove(i);
                } else {
                    mem::swap(&mut self.t.main, &mut self.t.subs[i]);
                }
                self.t.save();
            }
            if let Some(i) = delete { self.t.subs.remove(i); self.t.save(); }
        }

        // ── add row ──────────────────────────────────────────────────────
        ui.allocate_ui_with_layout(vec2(W, 28.0), Layout::left_to_right(Align::Center), |ui| {
            ui.add_space(12.0);
            if self.adding {
                let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
                let enter  = ctx.input(|i| i.key_pressed(egui::Key::Enter));

                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.buf)
                        .desired_width(W - 24.0)
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
                        if self.t.main.is_empty() {
                            self.t.main = text;
                        } else {
                            self.t.subs.push(text);
                        }
                        self.t.save();
                        if enter { self.focus_add_btn = true; }
                    } else {
                        self.buf.clear();
                    }
                    self.adding = false;
                    ctx.memory_mut(|m| { if let Some(id) = m.focused() { m.surrender_focus(id); } });
                }
            } else {
                let add_btn = ui.add(
                    egui::Button::new(
                        RichText::new("+ Добавить...")
                            .size(12.0)
                            .color(Color32::from_white_alpha(60)),
                    )
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE),
                );
                if self.focus_add_btn {
                    add_btn.request_focus();
                    self.focus_add_btn = false;
                }
                let enter_on_btn = add_btn.has_focus()
                    && ctx.input(|i| i.key_pressed(egui::Key::Enter));
                if add_btn.clicked() || enter_on_btn {
                    self.adding = true;
                    self.need_focus = true;
                }
            }
        });
    }
}

// ── entry ─────────────────────────────────────────────────────────────────────

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "cue",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_decorations(false)
                .with_transparent(true)
                .with_always_on_top()
                .with_icon(eframe::icon_data::from_png_bytes(ICON_PNG).unwrap())
                .with_inner_size([W, 100.0])
                .with_min_inner_size([W, 50.0])
                .with_resizable(false),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}