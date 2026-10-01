use eframe::egui::{self, Color32, RichText};
use crate::settings::{ProjectSort, Settings};
use super::general::toggle_label;

/// Вкладка «Проекты»: сортировка списка проектов. Настройка локальная (не
/// синкается), применяется при следующем открытии списка проектов.
pub fn draw(ui: &mut egui::Ui, settings: &mut Settings) -> bool {
    ui.add_space(14.0);
    ui.visuals_mut().selection.bg_fill = Color32::from_rgb(86, 111, 146);

    let mut changed = false;

    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(RichText::new("Сортировка списка проектов:")
            .color(Color32::from_white_alpha(90)).size(11.0));
    });
    ui.add_space(6.0);

    let options = [
        (ProjectSort::ByName,     "По названию"),
        (ProjectSort::ByColor,    "По цвету"),
        (ProjectSort::ByCreated,  "По дате создания"),
        (ProjectSort::ByModified, "По дате изменения"),
    ];
    for (i, (value, label)) in options.iter().enumerate() {
        if i > 0 { ui.add_space(4.0); }
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let radio_clicked = ui.add(egui::RadioButton::new(
                settings.project_sort == *value, "")).clicked();
            ui.add_space(4.0);
            let label_clicked = toggle_label(ui, label);
            if (radio_clicked || label_clicked) && settings.project_sort != *value {
                settings.project_sort = *value;
                changed = true;
            }
        });
    }

    if changed { settings.save(); }
    changed
}
