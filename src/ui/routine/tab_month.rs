use eframe::egui::{self, Color32, RichText, ScrollArea, Sense, vec2};

pub struct MonthState {
    pub selected_days: [bool; 31],
    pub time_input:    super::time_input::TimeInputState,
    pub entries:       Vec<(u32, String)>,
}

impl Default for MonthState {
    fn default() -> Self {
        Self {
            selected_days: [false; 31],
            time_input:    super::time_input::TimeInputState::default(),
            entries:       Vec::new(),
        }
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut MonthState, avail_h: f32) {
    ui.add_space(10.0);

    let gap   = 4.0_f32;
    let btn_h = 18.0_f32;
    let rows: [(usize, usize); 3] = [(0, 11), (11, 21), (21, 31)];

    for (ri, &(start, end)) in rows.iter().enumerate() {
        let count = end - start;
        let btn_w = (super::RW - 28.0 - gap * (count as f32 - 1.0)) / count as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.add_space(14.0);
            for i in start..end {
                let day      = (i + 1) as u32;
                let selected = state.selected_days[i];
                let btn = ui.add(
                    egui::Button::new(
                        RichText::new(day.to_string())
                            .color(if selected { Color32::WHITE } else { Color32::from_white_alpha(120) })
                            .size(10.1),
                    )
                    .min_size(vec2(btn_w, btn_h))
                    .fill(if selected { Color32::from_white_alpha(35) } else { Color32::from_white_alpha(8) }),
                );
                if btn.clicked() { state.selected_days[i] = !state.selected_days[i]; }
            }
        });
        if ri < 2 { ui.add_space(gap); }
    }

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
        if btn.clicked() {
            if let Some(time) = state.time_input.to_time_string() {
                for (i, &sel) in state.selected_days.iter().enumerate() {
                    if sel { state.entries.push(((i + 1) as u32, time.clone())); }
                }
                state.entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
                state.selected_days = [false; 31];
                state.time_input.clear();
            }
        }
    });

    ui.add_space(8.0);

    if state.entries.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new("Нет дат").color(Color32::from_white_alpha(30)).size(11.5));
        });
    } else {
        let fixed  = 10.0 + 3.0 * 18.0 + 2.0 * 4.0 + 6.0 + 22.0 + 8.0;
        let list_h = (avail_h - fixed).max(0.0);
        ScrollArea::vertical().max_height(list_h).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let mut remove = None;
            for (i, (day, time)) in state.entries.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(RichText::new(format!("{} числа · {}", day, time))
                        .color(Color32::from_white_alpha(160)).size(11.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(14.0);
                        let x = ui.add(egui::Label::new(
                            RichText::new("×").color(Color32::from_white_alpha(60)).size(13.0)
                        ).sense(Sense::click()));
                        if x.clicked() { remove = Some(i); }
                        if x.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    });
                });
                ui.add_space(2.0);
            }
            if let Some(i) = remove { state.entries.remove(i); }
        });
    }
}
