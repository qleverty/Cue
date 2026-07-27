use eframe::egui::{self, Color32, ImageSource, RichText, ScrollArea, vec2};

const DAY_SHORT: [&str; 7] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];
const DAY_FULL:  [&str; 7] = [
    "Понедельник", "Вторник",  "Среда",
    "Четверг",     "Пятница",  "Суббота", "Воскресенье",
];

pub struct WeekState {
    pub selected_days: [bool; 7],
    pub time_input:    super::time_input::TimeInputState,
    pub entries:       Vec<(usize, String)>,
}

impl Default for WeekState {
    fn default() -> Self {
        Self {
            selected_days: [false; 7],
            time_input:    super::time_input::TimeInputState::default(),
            entries:       Vec::new(),
        }
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut WeekState, list_h: f32) {
    ui.add_space(10.0);

    let gap      = 4.0_f32;
    let btn_w    = (super::RW - 28.0 - gap * 6.0) / 7.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.add_space(14.0);
        for (i, label) in DAY_SHORT.iter().enumerate() {
            let selected = state.selected_days[i];
            let btn = ui.add(
                egui::Button::new(
                    RichText::new(*label)
                        .color(if selected {
                            Color32::WHITE
                        } else {
                            Color32::from_white_alpha(120)
                        })
                        .size(11.0),
                )
                .min_size(vec2(btn_w, 20.0))
                .fill(if selected {
                    Color32::from_white_alpha(35)
                } else {
                    Color32::from_white_alpha(8)
                }),
            );
            if btn.clicked() {
                state.selected_days[i] = !state.selected_days[i];
            }
            if btn.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        }
    });

    ui.add_space(6.0);

    ui.horizontal(|ui| {
        ui.add_space(14.0);
        super::time_input::time_input(ui, &mut state.time_input);
        ui.add_space(5.0);
        let any_selected = state.selected_days.iter().any(|&d| d);
        let can_add      = any_selected && state.time_input.is_valid();
        let btn = ui.add_enabled(
            can_add,
            egui::Button::new(
                RichText::new("+").color(Color32::from_white_alpha(180)).size(14.0),
            )
            .min_size(vec2(24.0, 0.0)),
        );
        if btn.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        if btn.clicked() {
            if let Some(time) = state.time_input.to_time_string() {
                for (i, &sel) in state.selected_days.iter().enumerate() {
                    if sel { state.entries.push((i, time.clone())); }
                }
                state.entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
                state.selected_days = [false; 7];
                state.time_input.clear();
            }
        }
    });

    ui.add_space(8.0);

    if state.entries.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new("Нет дней").color(Color32::from_white_alpha(30)).size(11.5));
        });
    } else {
        ScrollArea::vertical().max_height(list_h - 7.0).show(ui, |ui| {
            ui.add_space(-4.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let mut remove = None;
            for (i, (day_idx, time)) in state.entries.iter().enumerate() {
                let label = format!("{} · {}", DAY_FULL[*day_idx], time);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(&label)
                            .color(Color32::from_white_alpha(160))
                            .size(11.5),
                    );
                    let row = ui.max_rect();
                    let del_rect = egui::Rect::from_min_size(
                        egui::pos2(row.right() - 29.0, row.top() + (row.height() - 15.0) / 2.0),
                        vec2(15.0, 15.0),
                    );
                    let x = ui.allocate_rect(del_rect, egui::Sense::click());
                    let tint = if x.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        Color32::WHITE
                    } else {
                        Color32::from_gray(130)
                    };
                    ui.put(del_rect, egui::Image::new(ImageSource::Bytes {
                        uri: "bytes://cross.png".into(),
                        bytes: crate::CROSS_PNG.into(),
                    }).fit_to_exact_size(vec2(8.0, 8.0)).tint(tint));
                    if x.clicked() { remove = Some(i); }
                });
                ui.add_space(2.0);
            }
            if let Some(i) = remove { state.entries.remove(i); }
        });
    }
}
