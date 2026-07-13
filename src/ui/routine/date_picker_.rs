use eframe::egui::{self, Color32, ImageSource, Order, RichText, Sense, vec2};

pub const POPUP_W: f32 = 200.0;
pub const POPUP_H: f32 = 190.0;

static CROSS_PNG: &[u8] = include_bytes!("../../../pics/cross.png");

const MONTHS: [&str; 12] = [
    "Январь", "Февраль", "Март",     "Апрель",
    "Май",    "Июнь",    "Июль",     "Август",
    "Сентябрь", "Октябрь", "Ноябрь", "Декабрь",
];

pub struct DatePickerState {
    pub open:        bool,
    pub selected:    Option<String>,
    pub view_year:   i32,
    pub view_month:  usize,
}

impl Default for DatePickerState {
    fn default() -> Self {
        let now = chrono_free_today();
        Self {
            open:       false,
            selected:   None,
            view_year:  now.0,
            view_month: now.1,
        }
    }
}

fn chrono_free_today() -> (i32, usize) {
    (2025, 6)
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
                        draw_header(ui, state);
                    });
            });

        if area_resp.response.clicked_elsewhere() {
            state.open = false;
        }
    }
}

fn cross_btn(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::click());
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
    let title = format!("{} {}", MONTHS[state.view_month], state.view_year);
    let inner_w = POPUP_W - 16.0;

    let font_id = egui::FontId::proportional(11.5);
    let title_w = ui.painter()
        .layout_no_wrap(title.clone(), font_id.clone(), Color32::WHITE)
        .size().x;

    let btn_w    = 15.0;
    let gap      = (inner_w - title_w - btn_w * 2.0) / 4.0;

    ui.horizontal(|ui| {
        ui.add_space(gap);
        if cross_btn(ui).clicked() {
            if state.view_month == 0 {
                state.view_month = 11;
                state.view_year -= 1;
            } else {
                state.view_month -= 1;
            }
        }
        ui.add_space(gap);
        ui.label(RichText::new(&title).color(Color32::WHITE).size(11.5));
        ui.add_space(gap);
        if cross_btn(ui).clicked() {
            if state.view_month == 11 {
                state.view_month = 0;
                state.view_year += 1;
            } else {
                state.view_month += 1;
            }
        }
    });
}
