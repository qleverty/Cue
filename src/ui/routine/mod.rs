pub mod date_picker;
pub mod tab_direct;
pub mod tab_week;

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Shape, Stroke, ViewportCommand, vec2};
use crate::BG;

pub const RW: f32 = 254.0;

pub const RH_DIRECT: f32 = 340.0;
pub const RH_WEEK:   f32 = 300.0;
pub const RH_MONTH:  f32 = 300.0;

#[derive(PartialEq, Clone, Copy)]
pub enum RoutineTab { Direct, Week, Month }

pub struct RoutineUiState {
    pub task_id:   String,
    pub task_name: String,
    pub tab:       RoutineTab,
    pub direct:    tab_direct::DirectState,
    pub week:      tab_week::WeekState,
}

impl Default for RoutineUiState {
    fn default() -> Self {
        Self {
            task_id:   String::new(),
            task_name: String::new(),
            tab:       RoutineTab::Direct,
            direct:    tab_direct::DirectState::default(),
            week:      tab_week::WeekState::default(),
        }
    }
}

impl RoutineUiState {
    pub fn target_height(&self) -> f32 {
        match self.tab {
            RoutineTab::Direct => RH_DIRECT,
            RoutineTab::Week   => RH_WEEK,
            RoutineTab::Month  => RH_MONTH,
        }
    }
}

pub fn draw(ctx: &egui::Context, ui: &mut egui::Ui, state: &mut RoutineUiState) -> bool {
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

    ui.add_space(9.8);
    let tabs = [
        ("Дата",  RoutineTab::Direct),
        ("Неделя",  RoutineTab::Week),
        ("Месяц",   RoutineTab::Month),
    ];
    let font_id = egui::FontId::proportional(12.0);
    let total_w: f32 = tabs.iter().map(|(label, _)|
        ui.painter().layout_no_wrap(label.to_string(), font_id.clone(), Color32::WHITE).size().x
    ).sum();
    let gap = (RW - total_w) / (tabs.len() as f32 + 1.0);
    ui.horizontal(|ui| {
        ui.add_space(gap);
        for (label, tab) in &tabs {
            let active = state.tab == *tab;
            let color = if active {
                Color32::from_white_alpha(220)
            } else {
                Color32::from_white_alpha(90)
            };
            let resp = ui.add(
                egui::Label::new(RichText::new(*label).color(color).size(12.0))
                    .sense(Sense::click()),
            );
            if resp.clicked() { state.tab = tab.clone(); }
            ui.add_space(gap);
        }
    });
    ui.add_space(6.0);
    let y = ui.next_widget_position().y;
    ui.painter().hline(14.0..=(RW - 14.0), y, (0.5, crate::SEP));
    ui.add_space(1.0);

    match state.tab {
        RoutineTab::Direct => tab_direct::draw(ui, &mut state.direct),
        RoutineTab::Week   => tab_week::draw(ui, &mut state.week),
        RoutineTab::Month  => {}
    }

    let y = ui.next_widget_position().y;
    ui.painter().hline(14.0..=(RW - 14.0), y, (0.5, crate::SEP));

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        let save = ui.add(
            egui::Button::new(
                RichText::new("Сохранить")
                    .color(Color32::from_white_alpha(180))
                    .size(12.0),
            )
            .min_size(vec2(RW - 28.0, 22.0)),
        );
        if save.clicked() { close = true; }
    });
    ui.add_space(8.0);

    close
}
