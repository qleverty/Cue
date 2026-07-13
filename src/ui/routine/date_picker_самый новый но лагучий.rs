use eframe::egui::{self, Align2, Color32, FontId, ImageSource, Order, RichText, Sense, vec2};

pub const POPUP_W: f32 = 210.0;
pub const POPUP_H: f32 = 200.0;
const INNER_W:     f32 = POPUP_W - 16.0;

static CROSS_PNG: &[u8] = include_bytes!("../../../pics/cross.png");

const MONTHS: [&str; 12] = [
    "Январь", "Февраль", "Март",      "Апрель",
    "Май",    "Июнь",    "Июль",      "Август",
    "Сентябрь", "Октябрь", "Ноябрь", "Декабрь",
];
const DOW: [&str; 7] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];

pub struct DatePickerState {
    pub open:       bool,
    pub selected:   Option<String>,
    pub view_year:  i32,
    pub view_month: usize,
}

impl Default for DatePickerState {
    fn default() -> Self {
        Self { open: false, selected: None, view_year: 2025, view_month: 5 }
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

    if btn.clicked() { state.open = !state.open; }

    if state.open {
        let popup_pos = egui::pos2(btn.rect.right() - POPUP_W, btn.rect.top());

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
                        draw_header(ui, state);
                        ui.add_space(6.0);
                        draw_dow_row(ui);
                    });
            });

        if area_resp.response.clicked_elsewhere() { state.open = false; }
    }
}

fn cross_at(ui: &mut egui::Ui, rect: egui::Rect) -> egui::Response {
    let resp = ui.allocate_rect(rect, Sense::click());
    let tint = if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        Color32::WHITE
    } else {
        Color32::from_gray(130)
    };
    ui.put(rect, egui::Image::new(ImageSource::Bytes {
        uri:   "bytes://cross.png".into(),
        bytes: CROSS_PNG.into(),
    }).fit_to_exact_size(vec2(8.0, 8.0)).tint(tint));
    resp
}

fn draw_header(ui: &mut egui::Ui, state: &mut DatePickerState) {
    let title    = format!("{} {}", MONTHS[state.view_month], state.view_year);
    let row_h    = 15.0;
    let btn_w    = 15.0;

    let (row, _) = ui.allocate_exact_size(vec2(INNER_W, row_h), Sense::hover());

    let left_rect  = egui::Rect::from_min_size(row.min, vec2(btn_w, row_h));
    let right_rect = egui::Rect::from_min_size(
        egui::pos2(row.max.x - btn_w, row.min.y), vec2(btn_w, row_h),
    );

    if cross_at(ui, left_rect).clicked() {
        if state.view_month == 0 { state.view_month = 11; state.view_year -= 1; }
        else { state.view_month -= 1; }
    }
    if cross_at(ui, right_rect).clicked() {
        if state.view_month == 11 { state.view_month = 0; state.view_year += 1; }
        else { state.view_month += 1; }
    }

    ui.painter().text(
        row.center(),
        Align2::CENTER_CENTER,
        &title,
        FontId::proportional(11.5),
        Color32::WHITE,
    );
}

fn draw_dow_row(ui: &mut egui::Ui) {
    let cell_w  = INNER_W / DOW.len() as f32;
    let row_h   = 14.0;
    let (row, _) = ui.allocate_exact_size(vec2(INNER_W, row_h), Sense::hover());

    for (i, day) in DOW.iter().enumerate() {
        let center = egui::pos2(
            row.min.x + cell_w * i as f32 + cell_w / 2.0,
            row.center().y,
        );
        ui.painter().text(
            center,
            Align2::CENTER_CENTER,
            day,
            FontId::proportional(10.0),
            Color32::from_white_alpha(55),
        );
    }
}