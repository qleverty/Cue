use eframe::egui::{self, Color32, FontId, Pos2, Sense, Stroke, vec2};

pub struct TimeInputState {
    pub digits:  [Option<u8>; 4],
    pub cursor:  usize,
    pub focused: bool,
}

impl Default for TimeInputState {
    fn default() -> Self {
        Self { digits: [None; 4], cursor: 0, focused: false }
    }
}

impl TimeInputState {
    fn resolved(&self) -> Option<(u8, u8)> {
        let h = match (self.digits[0], self.digits[1]) {
            (Some(h1), Some(h0)) => h1 * 10 + h0,
            (Some(h1), None)     => h1,
            (None,     Some(h0)) => h0,
            (None,     None)     => return None,
        };
        let m = match (self.digits[2], self.digits[3]) {
            (Some(m1), Some(m0)) => m1 * 10 + m0,
            (Some(m1), None)     => m1,
            (None,     Some(m0)) => m0,
            (None,     None)     => return None,
        };
        if h <= 23 && m <= 59 { Some((h, m)) } else { None }
    }

    pub fn is_valid(&self) -> bool {
        self.resolved().is_some()
    }

    pub fn to_time_string(&self) -> Option<String> {
        self.resolved().map(|(h, m)| format!("{:02}:{:02}", h, m))
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

pub fn time_input(ui: &mut egui::Ui, state: &mut TimeInputState) {
    let w = 46.0_f32;
    let h = 22.0_f32;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());

    let cw  = 7.0_f32;
    let gap = 4.0_f32;
    let cx  = rect.center().x;
    let xs: [f32; 4] = [
        cx - gap - cw * 2.0,
        cx - gap - cw,
        cx + gap,
        cx + gap + cw,
    ];

    if resp.clicked() {
        if !state.focused {
            state.focused = true;
            state.cursor = state.digits
                .iter()
                .position(|d| d.is_none())
                .unwrap_or(4);
        } else {
            if let Some(pos) = resp.interact_pointer_pos() {
                state.cursor = xs
                    .iter()
                    .enumerate()
                    .min_by(|&(_, a), &(_, b)| {
                        (a - pos.x).abs().partial_cmp(&(b - pos.x).abs()).unwrap()
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0);
            }
        }
    }
    if ui.input(|i| !i.focused) { state.focused = false; }
    if ui.input(|i| i.pointer.any_click()) && !resp.clicked() { state.focused = false; }

    if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
        if rect.contains(pos) {
            let icon = if state.focused {
                egui::CursorIcon::Text
            } else {
                egui::CursorIcon::PointingHand
            };
            ui.ctx().set_cursor_icon(icon);
        }
    }

    if state.focused {
        ui.ctx().request_repaint();
        let events = ui.input(|i| i.events.clone());
        for event in &events {
            match event {
                egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => {
                    if state.cursor > 0 {
                        state.cursor -= 1;
                        state.digits[state.cursor] = None;
                    }
                }
                egui::Event::Key { key: egui::Key::Delete, pressed: true, .. } => {
                    if state.cursor < 4 {
                        state.digits[state.cursor] = None;
                    }
                }
                egui::Event::Key { key: egui::Key::ArrowLeft, pressed: true, .. } => {
                    if state.cursor > 0 { state.cursor -= 1; }
                }
                egui::Event::Key { key: egui::Key::ArrowRight, pressed: true, .. } => {
                    if state.cursor < 4 { state.cursor += 1; }
                }
                egui::Event::Key { key: egui::Key::Enter | egui::Key::Space, pressed: true, .. } => {
                    if state.cursor <= 1 {
                        state.cursor = 2;
                    } else {
                        state.focused = false;
                    }
                }
                egui::Event::Text(text) => {
                    for ch in text.chars() {
                        if let Some(d) = ch.to_digit(10) {
                            if state.cursor < 4 {
                                state.digits[state.cursor] = Some(d as u8);
                                state.cursor = (state.cursor + 1).min(4);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let painter  = ui.painter();
    let bg       = if state.focused { Color32::from_white_alpha(22) } else { Color32::from_white_alpha(17) };
    painter.rect_filled(rect, 3.0, bg);
    if state.focused {
        painter.rect_stroke(rect, 3.0, Stroke::new(0.8, Color32::from_white_alpha(70)), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        painter.rect_stroke(rect, 3.0, Stroke::new(0.8, Color32::from_white_alpha(150)), egui::StrokeKind::Inside);
    }

    let font     = FontId::monospace(11.0);
    let cy       = rect.center().y;
    let colon_x  = cx - 3.0;
    let ph_chars = ['ч', 'ч', 'м', 'м'];

    painter.text(
        Pos2::new(colon_x, cy),
        egui::Align2::LEFT_CENTER,
        ":",
        font.clone(),
        Color32::from_white_alpha(120),
    );

    for (i, &x) in xs.iter().enumerate() {
        let (ch, color) = match state.digits[i] {
            Some(d) => ((b'0' + d) as char, Color32::from_white_alpha(200)),
            None    => (ph_chars[i],         Color32::from_white_alpha(80)),
        };
        painter.text(Pos2::new(x, cy), egui::Align2::LEFT_CENTER, ch.to_string(), font.clone(), color);

        if state.focused && state.cursor == i {
            painter.line_segment(
                [Pos2::new(x, cy - 5.5), Pos2::new(x, cy + 5.5)],
                Stroke::new(1.0, Color32::from_white_alpha(150)),
            );
        }
    }
    if state.focused && state.cursor == 4 {
        let x = xs[3] + cw;
        painter.line_segment(
            [Pos2::new(x, cy - 5.5), Pos2::new(x, cy + 5.5)],
            Stroke::new(1.0, Color32::from_white_alpha(150)),
        );
    }
}
