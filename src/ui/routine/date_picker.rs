use eframe::egui::{self, Align2, Color32, FontId, ImageSource, Order, RichText, Sense, vec2};

pub const POPUP_W: f32 = 200.0;

static ARROW_PNG: &[u8] = include_bytes!("../../../pics/arrow.png");

const MONTHS_LONG: [&str; 12] = [
    "Январь", "Февраль", "Март",      "Апрель",
    "Май",    "Июнь",    "Июль",      "Август",
    "Сентябрь", "Октябрь", "Ноябрь", "Декабрь",
];
pub const MONTHS_SHORT: [&str; 12] = [
    "янв", "фев", "мар", "апр", "май", "июн",
    "июл", "авг", "сен", "окт", "ноя", "дек",
];
const DOW: [&str; 7] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];

const INNER_W:    f32 = POPUP_W - 16.0;
const CELL_W:     f32 = INNER_W / 7.0;
const CELL_H:     f32 = 20.0;
const BTN_OFFSET: f32 = 49.0;

pub struct DatePickerState {
    pub open:       bool,
    pub selected:   Option<(u32, usize, i32)>,
    pub view_year:  i32,
    pub view_month: usize,
}

impl Default for DatePickerState {
    fn default() -> Self {
        let (_, m, y) = today();
        Self { open: false, selected: None, view_year: y, view_month: m }
    }
}

/// "Сегодня" — через local_now() (местное время с поправкой на часовой
/// пояс, см. routine_scheduler.rs), а не сырой SystemTime::now(). См.
/// обсуждение 2026-08-05: на сыром UTC пикер глубокой ночью (когда
/// локальная дата уже перевалила за полночь, а UTC — ещё нет) показывал
/// "сегодня" вчерашним днём, и DIRECT-рутины сохранялись на сутки раньше
/// нужного — из-за чего либо мгновенно активировались, либо вовсе не
/// присылали уведомление (разрыв now/occ вылетал за NOTIFY_WINDOW_SECS).
fn today() -> (u32, usize, i32) {
    let days = crate::routine_scheduler::local_now() / 86400;
    let (y, m, d) = crate::routine_scheduler::civil_from_days(days as i64);
    (d, (m - 1) as usize, y)
}

fn days_in_month(month: usize, year: i32) -> u32 {
    let days = [31u32, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if month == 1 && (year % 4 == 0 && year % 100 != 0 || year % 400 == 0) { 29 }
    else { days[month] }
}

fn first_weekday(month: usize, year: i32) -> usize {
    let t = [0i32, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 2 { year - 1 } else { year };
    ((y + y/4 - y/100 + y/400 + t[month] + 1 + 6).rem_euclid(7)) as usize
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
        egui::Button::new(RichText::new(&label).color(color).size(11.5)).fill(Color32::from_white_alpha(17))
            .min_size(vec2(110.0, 22.0))
            .sense(Sense::click()),
    );
	
	if btn.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }

    if btn.clicked() {
        state.open = !state.open;
        if state.open {
            let (_, m, y) = state.selected.unwrap_or_else(today);
            state.view_month = m;
            state.view_year  = y;
        }
    }

    if !state.open { return; }

    let popup_pos = egui::pos2(btn.rect.right() - POPUP_W, btn.rect.top());

    let area = egui::Area::new(egui::Id::new("date_picker_popup"))
        .fixed_pos(popup_pos)
        .order(Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_premultiplied(0, 0, 0, 220))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(8))
                .stroke(egui::Stroke::new(0.5, Color32::from_white_alpha(18)))
                .show(ui, |ui| {
                    let (td, tm, ty) = today();
                    draw_header(ui, state, tm, ty);
                    ui.add_space(4.0);
                    draw_dow_row(ui);
                    ui.add_space(2.0);
                    draw_grid(ui, state, td, tm, ty);
                });
        });

    if area.response.clicked_elsewhere() { state.open = false; }
}

fn arrow_btn(ui: &mut egui::Ui, rect: egui::Rect, flipped: bool) -> egui::Response {
    let id   = ui.id().with(rect.min.x.to_bits()).with(rect.min.y.to_bits());
    let resp = ui.interact(rect, id, Sense::click());
    let tint = if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        Color32::WHITE
    } else {
        Color32::from_gray(130)
    };
    egui::Image::new(ImageSource::Bytes {
        uri:   "bytes://arrow.png".into(),
        bytes: ARROW_PNG.into(),
    })
    .fit_to_exact_size(vec2(1.0, 1.0))
    .tint(tint)
	.uv(if flipped {
		egui::Rect::from_min_max(egui::pos2(1.0, 0.0), egui::pos2(0.0, 1.0))
	} else {
		egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
	})
    .paint_at(ui, rect);
    resp
}

fn draw_arrow_static(ui: &mut egui::Ui, rect: egui::Rect, flipped: bool, tint: Color32) {
    let uv = if flipped {
        egui::Rect::from_min_max(egui::pos2(1.0, 0.0), egui::pos2(0.0, 1.0))
    } else {
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
    };
    egui::Image::new(ImageSource::Bytes {
        uri:   "bytes://arrow.png".into(),
        bytes: ARROW_PNG.into(),
    })
    .fit_to_exact_size(vec2(1.0, 1.0))
    .tint(tint)
    .uv(uv)
    .paint_at(ui, rect);
}

fn draw_header(ui: &mut egui::Ui, state: &mut DatePickerState, today_m: usize, today_y: i32) {
    let title      = format!("{} {}", MONTHS_LONG[state.view_month], state.view_year);
    let at_minimum = state.view_year == today_y && state.view_month == today_m;
    let btn_w      = 14.0;
    let (row, _)   = ui.allocate_exact_size(vec2(INNER_W, 15.0), Sense::hover());
    let cx         = row.min.x + INNER_W / 2.0;

    let left  = egui::Rect::from_center_size(egui::pos2(cx - BTN_OFFSET, row.center().y), vec2(btn_w, btn_w));
    let right = egui::Rect::from_center_size(egui::pos2(cx + BTN_OFFSET, row.center().y), vec2(btn_w, btn_w));

	if at_minimum {
		draw_arrow_static(ui, left, true, Color32::from_gray(80));
	} else if arrow_btn(ui, left, true).clicked() {
		if state.view_month == 0 { state.view_month = 11; state.view_year -= 1; }
		else { state.view_month -= 1; }
	}

    if arrow_btn(ui, right, false).clicked() {
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

fn draw_grid(ui: &mut egui::Ui, state: &mut DatePickerState, today_d: u32, today_m: usize, today_y: i32) {
    let offset     = first_weekday(state.view_month, state.view_year);
    let total_days = days_in_month(state.view_month, state.view_year);
    let (grid, _)  = ui.allocate_exact_size(vec2(INNER_W, 6.0 * CELL_H), Sense::hover());

    for i in 0..(6 * 7usize) {
        if i < offset { continue; }
        let day = (i - offset + 1) as u32;
        if day > total_days { continue; }

        let is_past = state.view_year == today_y && state.view_month == today_m && day < today_d;

        let col  = (i % 7) as f32;
        let row  = (i / 7) as f32;
        let cell = egui::Rect::from_min_size(
            egui::pos2(grid.min.x + col * CELL_W, grid.min.y + row * CELL_H),
            vec2(CELL_W, CELL_H),
        );

        if is_past {
            ui.allocate_rect(cell, Sense::hover());
            ui.painter().text(cell.center(), Align2::CENTER_CENTER, day.to_string(),
                FontId::proportional(11.0), Color32::from_white_alpha(80));
            continue;
        }

        let is_selected = state.selected == Some((day, state.view_month, state.view_year));
        let is_today    = day == today_d && state.view_month == today_m && state.view_year == today_y;
        let resp        = ui.allocate_rect(cell, Sense::click());

        let text_col = if is_selected {
            Color32::WHITE
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            ui.painter().rect_filled(cell.shrink(2.0), 3.0, Color32::from_white_alpha(12));
            Color32::from_white_alpha(220)
        } else if is_today {
            Color32::from_rgb(220, 180, 40)
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
