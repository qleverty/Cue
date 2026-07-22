use eframe::egui::{self, Color32, RichText, ScrollArea, vec2};
use super::date_picker;

pub struct DirectEntry {
    pub day:   u32,
    pub month: usize,
    pub year:  i32,
    pub time:  String,
}

impl DirectEntry {
    pub fn display(&self) -> String {
        format!("{} {} {} · {}", self.day, date_picker::MONTHS_SHORT[self.month], self.year, self.time)
    }

    pub fn to_routine_string(&self) -> String {
        format!("{:04}-{:02}-{:02} {}", self.year, self.month + 1, self.day, self.time)
    }
}

pub struct DirectState {
    pub date:       date_picker::DatePickerState,
    pub time_input: super::time_input::TimeInputState,
    pub entries:    Vec<DirectEntry>,
}

impl Default for DirectState {
    fn default() -> Self {
        Self {
            date:       date_picker::DatePickerState::default(),
            time_input: super::time_input::TimeInputState::default(),
            entries:    Vec::new(),
        }
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut DirectState, avail_h: f32) {
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        ui.add_space(14.0);
        date_picker::show(ui, &mut state.date);
        ui.add_space(5.0);
        super::time_input::time_input(ui, &mut state.time_input);
        ui.add_space(5.0);
        let can_add = state.date.selected.is_some() && state.time_input.is_valid();
        let btn = ui.add_enabled(
            can_add,
            egui::Button::new(RichText::new("+").color(Color32::from_white_alpha(180)).size(14.0))
                .min_size(vec2(24.0, 0.0)),
        );
        if btn.clicked() {
            if let (Some((d, m, y)), Some(time)) = (state.date.selected, state.time_input.to_time_string()) {
                state.entries.push(DirectEntry { day: d, month: m, year: y, time });
                state.entries.sort_by(|a, b| (a.year, a.month, a.day, &a.time).cmp(&(b.year, b.month, b.day, &b.time)));
                state.date.selected = None;
                state.time_input.clear();
            }
        }
    });

    ui.add_space(8.0);

    let fixed   = 10.0 + 22.0 + 8.0;
    let list_h  = (avail_h - fixed).max(0.0);

    if state.entries.is_empty() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new("Нет дат").color(Color32::from_white_alpha(30)).size(11.5));
        });
    } else {
        ScrollArea::vertical().max_height(list_h).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let mut remove = None;
                for (i, entry) in state.entries.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.label(
                            RichText::new(entry.display())
                                .color(Color32::from_white_alpha(160))
                                .size(11.5),
                        );
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.add_space(14.0);
                                let x = ui.add(
                                    egui::Label::new(
                                        RichText::new("×")
                                            .color(Color32::from_white_alpha(60))
                                            .size(13.0),
                                    )
                                    .sense(egui::Sense::click()),
                                );
                                if x.clicked() { remove = Some(i); }
                                if x.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                            },
                        );
                    });
                    ui.add_space(2.0);
                }
                if let Some(i) = remove { state.entries.remove(i); }
            });
    }
}
