use eframe::egui::{self, Color32, RichText};

pub fn draw(ui: &mut egui::Ui) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(RichText::new("Placeholder")
            .color(Color32::from_white_alpha(80)).size(13.0));
    });
}