use eframe::egui::{self, Color32, RichText, ScrollArea, TextEdit, vec2};
use super::date_picker;

pub struct DirectState {
    pub date:      date_picker::DatePickerState,
    pub time_buf:  String,
    pub entries:   Vec<String>,
}

impl Default for DirectState {
    fn default() -> Self {
        Self {
            date:     date_picker::DatePickerState::default(),
            time_buf: String::new(),
            entries:  Vec::new(),
        }
    }
}

const MONTHS: [&str; 12] = [
    "янв", "фев", "мар", "апр", "май", "июн",
    "июл", "авг", "сен", "окт", "ноя", "дек",
];

fn format_entry(date: &str, time: &str) -> String {
    let parts: Vec<&str> = date.splitn(3, '.').collect();
    if parts.len() == 3 {
        if let (Ok(d), Ok(m), Ok(y)) = (
            parts[0].parse::<u32>(),
            parts[1].parse::<usize>(),
            parts[2].parse::<u32>(),
        ) {
            if m >= 1 && m <= 12 {
                return format!("{} {} {} · {}", d, MONTHS[m - 1], y, time);
            }
        }
    }
    format!("{} · {}", date, time)
}

pub fn draw(ui: &mut egui::Ui, state: &mut DirectState) {
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        ui.add_space(14.0);
        date_picker::show(ui, &mut state.date);
        ui.add_space(5.0);
        ui.add(
            TextEdit::singleline(&mut state.time_buf)
                .hint_text("чч:мм")
                .desired_width(46.0)
                .font(egui::TextStyle::Small),
        );
        ui.add_space(5.0);
        let can_add = state.date.selected.is_some() && !state.time_buf.is_empty();
        let btn = ui.add_enabled(
            can_add,
            egui::Button::new(RichText::new("+").color(Color32::from_white_alpha(180)).size(14.0))
                .min_size(vec2(24.0, 0.0)),
        );
        if btn.clicked() {
            let date_str = state.date.selected.clone().unwrap_or_default();
            state.entries.push(format_entry(&date_str, &state.time_buf));
            state.date.selected = None;
            state.time_buf.clear();
        }
    });

    ui.add_space(8.0);

    let entry_h = 20.0;
    let max_visible = 5;
    let scroll_h = (state.entries.len().min(max_visible) as f32) * entry_h;

    if state.entries.is_empty() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(
                RichText::new("Нет дат")
                    .color(Color32::from_white_alpha(30))
                    .size(11.5),
            );
        });
        ui.add_space(4.0);
    } else {
        ScrollArea::vertical()
            .max_height(scroll_h)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let mut remove = None;
                for (i, entry) in state.entries.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.label(
                            RichText::new(entry)
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
