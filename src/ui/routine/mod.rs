use eframe::egui::{self, Align2, Color32, FontId, Sense, Shape, Stroke, ViewportCommand, vec2};
use crate::BG;

pub const RW: f32 = 254.0;
pub const RH: f32 = 320.0;

pub struct RoutineUiState {
    pub task_id:   String,
    pub task_name: String,
}

impl Default for RoutineUiState {
    fn default() -> Self {
        Self { task_id: String::new(), task_name: String::new() }
    }
}

pub fn draw(ctx: &egui::Context, ui: &mut egui::Ui, state: &RoutineUiState) -> bool {
    ui.painter().rect_filled(ui.max_rect(), 10.0, BG);
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);

    let bar_rect = egui::Rect::from_min_size(ui.next_widget_position(), vec2(RW, 12.0));
    let drag = ui.allocate_rect(bar_rect, Sense::drag());
    if drag.dragged() { ctx.send_viewport_cmd(ViewportCommand::StartDrag); }

    let mut close = false;

    let close_center = bar_rect.max - vec2(11.0, 2.0);
    let close_r = ui.allocate_rect(
        egui::Rect::from_center_size(close_center, vec2(14.0, 10.0)),
        Sense::click(),
    );
    let close_col = if close_r.hovered() {
        Color32::from_rgb(255, 80, 80)
    } else {
        Color32::from_rgb(220, 50, 50)
    };
    {
        let c   = close_center;
        let tip = c + vec2( 4.0,  0.0);
        let top = c + vec2(-2.5, -3.8);
        let bot = c + vec2(-2.5,  3.8);
        ui.painter().add(Shape::convex_polygon(vec![tip, top, bot], close_col, Stroke::NONE));
    }
    if close_r.clicked() { close = true; }

    let c = bar_rect.center();
    for x in [-6.0f32, 0.0, 6.0] {
        ui.painter().circle_filled(c + vec2(x, 4.0), 1.5, Color32::from_white_alpha(35));
    }

    {
        let p = ui.painter();
        p.circle_filled(bar_rect.min + vec2(10.0, 10.5), 2.0, Color32::from_rgb(220, 180, 40));

        let max_chars = 28;
        let title = if state.task_name.chars().count() > max_chars {
            let truncated: String = state.task_name.chars().take(max_chars).collect();
            format!("{}…", truncated.trim_end())
        } else {
            state.task_name.clone()
        };

        p.text(
            bar_rect.min + vec2(17.0, 10.5),
            Align2::LEFT_CENTER,
            &title,
            FontId::proportional(10.5),
            Color32::from_white_alpha(180),
        );
    }

    close
}
