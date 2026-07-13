use eframe::egui::{self, Color32, Order, RichText, Sense, vec2};

pub const POPUP_W: f32 = 200.0;
pub const POPUP_H: f32 = 190.0;

pub struct DatePickerState {
    pub open:     bool,
    pub selected: Option<String>,
}

impl Default for DatePickerState {
    fn default() -> Self {
        Self { open: false, selected: None }
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut DatePickerState) {
    let label = state.selected.as_deref().unwrap_or("дд.мм.гггг");
    let color = if state.selected.is_some() {
        Color32::from_white_alpha(200)
    } else {
        Color32::from_white_alpha(80)
    };

    let btn = ui.add(
        egui::Button::new(RichText::new(label).color(color).size(11.5))
            .min_size(vec2(110.0, 22.0))
            .sense(Sense::click()),
    );

    if btn.clicked() {
        state.open = !state.open;
    }

    if state.open {
        let popup_pos = egui::pos2(
            btn.rect.right() - POPUP_W,
            btn.rect.top(),
        );

        let area_resp = egui::Area::new(egui::Id::new("date_picker_popup"))
            .fixed_pos(popup_pos)
            .order(Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_premultiplied(14, 14, 18, 250))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::same(8))
                    .stroke(egui::Stroke::new(0.5, Color32::from_white_alpha(18)))
                    .show(ui, |ui| {
                        ui.set_min_size(vec2(POPUP_W - 16.0, POPUP_H - 16.0));
                        ui.label(
                            RichText::new("скоро здесь будет календарь")
                                .color(Color32::from_white_alpha(30))
                                .size(11.0),
                        );
                    });
            });

        if area_resp.response.clicked_elsewhere() {
            state.open = false;
        }
    }
}
