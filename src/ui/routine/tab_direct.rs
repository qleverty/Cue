use eframe::egui::{self, Color32, ImageSource, RichText, ScrollArea, vec2};
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

    /// Обратный парсер: "YYYY-MM-DD HH:MM" -> DirectEntry. None при любом
    /// несовпадении формата — такие записи молча пропускаются при load_from.
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.splitn(2, ' ');
        let date = parts.next()?;
        let time = parts.next()?.to_string();
        let mut d = date.splitn(3, '-');
        let year:  i32   = d.next()?.parse().ok()?;
        let month: usize = d.next()?.parse::<usize>().ok()?.checked_sub(1)?;
        let day:   u32   = d.next()?.parse().ok()?;
        Some(Self { day, month, year, time })
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

impl DirectState {
    /// Полностью перезаписывает entries из строк модели (или очищает, если
    /// пусто). Также сбрасывает виджеты ввода даты/времени — это открытие
    /// окна "с нуля", а не мердж с тем, что было введено на экране.
    pub fn load_from(&mut self, entries: &[String]) {
        self.entries = entries.iter().filter_map(|s| DirectEntry::parse(s)).collect();
        self.entries.sort_by(|a, b| (a.year, a.month, a.day, &a.time).cmp(&(b.year, b.month, b.day, &b.time)));
        self.date       = date_picker::DatePickerState::default();
        self.time_input = super::time_input::TimeInputState::default();
    }

    pub fn to_strings(&self) -> Vec<String> {
        self.entries.iter().map(DirectEntry::to_routine_string).collect()
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut DirectState, list_h: f32) {
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
        if btn.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
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

    if state.entries.is_empty() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new("Нет дат").color(Color32::from_white_alpha(30)).size(11.5));
        });
    } else {
        ScrollArea::vertical().max_height(list_h - 7.0).show(ui, |ui| {
                ui.add_space(-4.0);
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
