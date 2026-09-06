use eframe::egui::{self, Color32, Stroke};
use crate::settings::{Settings, StartupMode};
use crate::project::LoadedProject;

fn bg()           -> Color32 { Color32::from_white_alpha(13) }  // rgba(255,255,255,0.05) — как в settings_concept.html
fn bg_hover()     -> Color32 { Color32::from_white_alpha(13) }
fn text()         -> Color32 { Color32::from_gray(190) }
fn border()       -> Color32 { Color32::from_rgb(0x23, 0x23, 0x23) }
fn border_hover() -> Color32 { Color32::from_rgb(0x65, 0x65, 0x65) }

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{head}...")
    }
}

pub fn draw(ui: &mut egui::Ui, settings: &mut Settings, projects: &[LoadedProject]) -> bool {
    let mut changed = false;

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(22.0);
        ui.add_enabled_ui(settings.startup_mode == StartupMode::Fixed, |ui| {
            let visuals = ui.visuals_mut();
            for w in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.open] {
                w.fg_stroke = Stroke::new(1.0, text());
            }
            visuals.widgets.inactive.weak_bg_fill = bg();
            visuals.widgets.inactive.bg_stroke    = Stroke::new(1.0, border());
            visuals.widgets.hovered.weak_bg_fill  = bg_hover();
            visuals.widgets.hovered.bg_stroke     = Stroke::new(1.0, border_hover());
            visuals.widgets.open.weak_bg_fill     = bg_hover();
            visuals.widgets.open.bg_stroke        = Stroke::new(1.0, border_hover());

            let selected = settings.fixed_project_id.as_deref()
                .and_then(|id| projects.iter().find(|p| p.id == id))
                .or_else(|| projects.first());
            let label = selected.map(|p| p.name.as_str()).unwrap_or("");
            let label = truncate(label, 23);
            let combo = egui::ComboBox::from_id_salt("fixed_project")
                .width(220.0)
                .selected_text(label)
                .icon(|ui, rect, visuals, _is_open| {
                    let rect = egui::Rect::from_center_size(
                        rect.center(),
                        egui::vec2(rect.width() * 0.5, rect.height() * 0.32),
                    );
                    ui.painter().add(egui::Shape::convex_polygon(
                        vec![rect.left_top(), rect.right_top(), rect.center_bottom()],
                        visuals.fg_stroke.color,
                        Stroke::NONE,
                    ));
                })
                .popup_style(egui::style::StyleModifier::new(|style: &mut egui::Style| {
                    style.visuals.window_fill        = Color32::from_rgba_unmultiplied(0x2a, 0x2a, 0x2a, 220);
                    style.visuals.window_stroke       = Stroke::NONE;
                    style.visuals.menu_corner_radius   = 6.0.into();
                    style.spacing.menu_margin         = egui::Margin::same(4);
                }))
                .show_ui(ui, |ui| {
                    const ROW_H: f32 = 17.0;
                    let row_font = egui::FontId::proportional(10.5);
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
                    for p in projects {
                        let is_sel = settings.fixed_project_id.as_deref() == Some(p.id.as_str());
                        let row_w  = ui.available_width();
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(row_w, ROW_H), egui::Sense::click());
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        let col = if is_sel || resp.hovered() { Color32::WHITE } else { text() };
                        ui.painter().circle_filled(
                            rect.min + egui::vec2(7.0, ROW_H / 2.0), 3.0, p.color);
                        let galley = ui.painter().layout_no_wrap(
                            truncate(&p.name, 23), row_font.clone(), col);
                        ui.painter().galley(
                            rect.min + egui::vec2(15.0, (ROW_H - galley.size().y) / 2.0), galley, col);
                        if resp.clicked() {
                            settings.fixed_project_id = Some(p.id.clone());
                            changed = true;
                        }
                    }
                });
            combo.response.on_hover_cursor(egui::CursorIcon::PointingHand);
        });
    });

    changed
}

