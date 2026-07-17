use eframe::egui::{self, Align2, Color32, FontId, ImageSource, Order, RichText, Sense, vec2};

pub const POPUP_W: f32 = 200.0;

static CROSS_PNG: &[u8] = include_bytes!("../../../pics/cross.png");

const MONTHS_LONG:  [&str; 12] = [
    "Январь", "Февраль", "Март",      "Апрель",
    "Май",    "Июнь",    "Июль",      "Август",
    "Сентябрь", "Октябрь", "Ноябрь", "Декабрь",
];
pub const MONTHS_SHORT: [&str; 12] = [
    "янв", "фев", "мар", "апр", "май", "июн",
    "июл", "авг", "сен", "окт", "ноя", "дек",
];
const DOW: [&str; 7] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];

const INNER_W:    f32  = POPUP_W - 16.0;
const CELL_W:     f32  = INNER_W / 7.0;
const CELL_H:     f32  = 20.0;
const BTN_OFFSET: f32  = 52.0;

pub struct DatePickerState {
    pub open:       bool,
    pub selected:   Option<(u32, usize, i32)>,
    pub view_year:  i32,
    pub view_month: usize,
}

impl Default for DatePickerState {
    fn default() -> Self {
        Self { open: false, selected: None, view_year: 2025, view_month: 5 }
    }
}

fn days_in_month(month: usize, year: i32) -> u32 {
    let days = [31u32, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if month == 1 && (year % 4 == 0 && year % 100 != 0 || year % 400 == 0) { 29 }
    else { days[month] }
}

fn first_weekday(month: usize, year: i32) -> usize {
    let t = [0i32, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 2 { year - 1 } else { year };
    let dow = (y + y/4 - y/100 + y/400 + t[month] + 1).rem_euclid(7);
    ((dow + 6).rem_euclid(7)) as usize
}

pub fn show(ui: &mut egui::Ui, state: &mut DatePickerState) {
    let label = state.selected
        .map(|(d, m, y)| format!("{} {} {}", d, MONTHS_SHORT[m], y))
        .unwrap_or_else(|| "дд.мм.гггг".into());

    let color = if state.selected.is_some() {
        Color32::from_white_alpha(200)
    } else {
        Color32::from_white_alpha(80)
    };

    let btn = ui.add(
        egui::Button::new(RichText::new(&label).color(color).size(11.5))
            .min_size(vec2(110.0, 22.0))
            .sense(Sense::click()),
    );
    if btn.clicked() { state.open = !state.open; }

    if !state.open { return; }

    let popup_pos = egui::pos2(btn.rect.right() - POPUP_W, btn.rect.top());

    let area = egui::Area::new(egui::Id::new("date_picker_popup"))
        .fixed_pos(popup_pos)
        .order(Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_premultiplied(0, 0, 0, 204))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(8))
                .stroke(egui::Stroke::new(0.5, Color32::from_white_alpha(18)))
                .show(ui, |ui| {
                    draw_header(ui, state);
                    ui.add_space(4.0);
                    draw_dow_row(ui);
                    ui.add_space(2.0);
                    draw_grid(ui, state);
                });
        });

    if area.response.clicked_elsewhere() { state.open = false; }
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
    let title    = format!("{} {}", MONTHS_LONG[state.view_month], state.view_year);
    let btn_w    = 15.0;
    let (row, _) = ui.allocate_exact_size(vec2(INNER_W, 15.0), Sense::hover());
    let cx       = row.min.x + INNER_W / 2.0;

    let left  = egui::Rect::from_center_size(egui::pos2(cx - BTN_OFFSET, row.center().y), vec2(btn_w, btn_w));
    let right = egui::Rect::from_center_size(egui::pos2(cx + BTN_OFFSET, row.center().y), vec2(btn_w, btn_w));

    if cross_at(ui, left).clicked() {
        if state.view_month == 0 { state.view_month = 11; state.view_year -= 1; }
        else { state.view_month -= 1; }
    }
    if cross_at(ui, right).clicked() {
        if state.view_month == 11 { state.view_month = 0; state.view_year += 1; }
        else { state.view_month += 1; }
    }

    ui.painter().text(row.center(), Align2::CENTER_CENTER, &title,
        FontId::proportional(11.5), Color32::WHITE);
}

fn draw_dow_row(ui: &mut egui::Ui) {
    let (row, _) = ui.allocate_exact_size(vec2(INNER_W, 13.0), Sense::hover());
    for (i, day) in DOW.iter().enumerate() {
        ui.painter().text(
            egui::pos2(row.min.x + CELL_W * i as f32 + CELL_W / 2.0, row.center().y),
            Align2::CENTER_CENTER, day,
            FontId::proportional(10.0), Color32::from_white_alpha(55),
        );
    }
}

fn draw_grid(ui: &mut egui::Ui, state: &mut DatePickerState) {
    let offset     = first_weekday(state.view_month, state.view_year);
    let total_days = days_in_month(state.view_month, state.view_year);
    let (grid, _)  = ui.allocate_exact_size(vec2(INNER_W, 6.0 * CELL_H), Sense::hover());

    for i in 0..(6 * 7usize) {
        if i < offset { continue; }
        let day = (i - offset + 1) as u32;
        if day > total_days { continue; }

        let col  = (i % 7) as f32;
        let row  = (i / 7) as f32;
        let cell = egui::Rect::from_min_size(
            egui::pos2(grid.min.x + col * CELL_W, grid.min.y + row * CELL_H),
            vec2(CELL_W, CELL_H),
        );

        let is_selected = state.selected == Some((day, state.view_month, state.view_year));
        let resp        = ui.allocate_rect(cell, Sense::click());

        let text_col = if is_selected {
            ui.painter().rect_filled(cell.shrink(2.0), 3.0, Color32::from_white_alpha(40));
            Color32::WHITE
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            ui.painter().rect_filled(cell.shrink(2.0), 3.0, Color32::from_white_alpha(12));
            Color32::from_white_alpha(220)
        } else {
            Color32::from_white_alpha(160)
        };

        ui.painter().text(cell.center(), Align2::CENTER_CENTER, day.to_string(),
            FontId::proportional(11.0), text_col);

        if resp.clicked() {
            state.selected = Some((day, state.view_month, state.view_year));
            state.open     = false;
        }
    }
}
