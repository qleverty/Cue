use eframe::egui::{self, Align2, Color32, FontId, RichText, ScrollArea, Sense, TextEdit, vec2};

pub struct MonthState {
    pub selected_days: [bool; 31],
    pub time_buf:      String,
    pub entries:       Vec<(u32, String)>,
}

impl Default for MonthState {
    fn default() -> Self {
        Self {
            selected_days: [false; 31],
            time_buf:      String::new(),
            entries:       Vec::new(),
        }
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut MonthState) {
    ui.add_space(10.0);

    let cols      = 7_usize;
    let rows      = 5_usize;
    let gap       = 3.0_f32;
    let inner_w   = super::RW - 28.0;
    let cell_w    = (inner_w - gap * (cols as f32 - 1.0)) / cols as f32;
    let cell_h    = cell_w * 0.85;
    let grid_h    = rows as f32 * cell_h + (rows as f32 - 1.0) * gap;

    ui.horizontal(|ui| {
        ui.add_space(14.0);
        let (grid, _) = ui.allocate_exact_size(vec2(inner_w, grid_h), Sense::hover());

        for day in 1u32..=31 {
            let i    = (day - 1) as usize;
            let col  = (i % cols) as f32;
            let row  = (i / cols) as f32;
            let cell = egui::Rect::from_min_size(
                egui::pos2(
                    grid.min.x + col * (cell_w + gap),
                    grid.min.y + row * (cell_h + gap),
                ),
                vec2(cell_w, cell_h),
            );

            let selected = state.selected_days[i];
            let id       = ui.id().with(day);
            let resp     = ui.interact(cell, id, Sense::click());

            let (bg, text_col) = if selected {
                (Color32::from_white_alpha(35), Color32::WHITE)
            } else if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                (Color32::from_white_alpha(15), Color32::from_white_alpha(200))
            } else {
                (Color32::from_white_alpha(8), Color32::from_white_alpha(140))
            };

            ui.painter().rect_filled(cell, 3.0, bg);
            ui.painter().text(
                cell.center(), Align2::CENTER_CENTER,
                day.to_string(),
                FontId::proportional(10.5),
                text_col,
            );

            if resp.clicked() {
                state.selected_days[i] = !state.selected_days[i];
            }
        }
    });

    ui.add_space(6.0);

    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.add(
            TextEdit::singleline(&mut state.time_buf)
                .hint_text("чч:мм")
                .desired_width(46.0)
                .font(egui::TextStyle::Small),
        );
        ui.add_space(5.0);
        let any_selected = state.selected_days.iter().any(|&d| d);
        let can_add      = any_selected && !state.time_buf.is_empty();
        let btn = ui.add_enabled(
            can_add,
            egui::Button::new(
                RichText::new("+").color(Color32::from_white_alpha(180)).size(14.0),
            )
            .min_size(vec2(24.0, 0.0)),
        );
        if btn.clicked() {
            for (i, &sel) in state.selected_days.iter().enumerate() {
                if sel {
                    state.entries.push(((i + 1) as u32, state.time_buf.clone()));
                }
            }
            state.entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            state.selected_days = [false; 31];
            state.time_buf.clear();
        }
    });

    ui.add_space(8.0);

    if state.entries.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(
                RichText::new("Нет дат")
                    .color(Color32::from_white_alpha(30))
                    .size(11.5),
            );
        });
    } else {
        let entry_h     = 20.0;
        let max_visible = 5;
        let scroll_h    = (state.entries.len().min(max_visible) as f32) * entry_h;

        ScrollArea::vertical().max_height(scroll_h).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let mut remove = None;
            for (i, (day, time)) in state.entries.iter().enumerate() {
                let label = format!("{} числа · {}", day, time);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(&label)
                            .color(Color32::from_white_alpha(160))
                            .size(11.5),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                        if x.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    });
                });
                ui.add_space(2.0);
            }
            if let Some(i) = remove { state.entries.remove(i); }
        });
    }
}
