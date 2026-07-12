use eframe::egui::{self, Color32, RichText, Sense, ViewportCommand, vec2};
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

    ui.add_space(16.0);

    let mut close = false;

    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let back = ui.add(
            egui::Label::new(
                RichText::new("← Назад").color(Color32::from_white_alpha(90)).size(11.5)
            ).sense(Sense::click())
        );
        if back.clicked() { close = true; }
        ui.add_space(8.0);
        ui.label(
            RichText::new(&state.task_name)
                .color(Color32::from_white_alpha(45))
                .size(11.0)
        );
    });

    ui.add_space(8.0);
    let y = ui.next_widget_position().y;
    ui.painter().hline(0.0..=RW, y, (0.5, crate::SEP));
    ui.add_space(12.0);

    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(
            RichText::new("Здесь будет редактор расписания")
                .color(Color32::from_white_alpha(40))
                .size(12.0),
        );
    });

    close
}
