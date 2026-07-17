use eframe::egui::{self, TextEdit, vec2};

pub fn time_input(ui: &mut egui::Ui, buf: &mut String) -> egui::Response {
    let resp = ui.add_sized(
        vec2(46.0, 22.0),
        TextEdit::singleline(buf)
            .hint_text("чч:мм")
            .font(egui::TextStyle::Small),
    );

    if resp.changed() {
        let digits: String = buf.chars()
            .filter(|c| c.is_ascii_digit())
            .take(4)
            .collect();
        let formatted = match digits.len() {
            0 | 1 | 2 => digits,
            _          => format!("{}:{}", &digits[..2], &digits[2..]),
        };
        if formatted != *buf {
            *buf = formatted;
        }
    }

    resp
}
