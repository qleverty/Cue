use eframe::egui::{self, Color32, RichText};
use crate::settings::{NewTaskPos, Settings};

pub fn draw(ui: &mut egui::Ui, settings: &mut Settings) -> bool {
    ui.add_space(14.0);
    ui.visuals_mut().selection.bg_fill = Color32::from_rgb(86, 111, 146);

    let mut changed = false;

    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(RichText::new("При создании новых задач:")
            .color(Color32::from_white_alpha(120)).size(11.0));
    });
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        ui.add_space(14.0);
        if ui.add(egui::RadioButton::new(
            settings.new_task_pos == NewTaskPos::End, "")).clicked()
        {
            settings.new_task_pos = NewTaskPos::End;
            changed = true;
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Перемещать в конец списка")
            .color(Color32::from_gray(190)).size(13.0));
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        if ui.add(egui::RadioButton::new(
            settings.new_task_pos == NewTaskPos::Beginning, "")).clicked()
        {
            settings.new_task_pos = NewTaskPos::Beginning;
            changed = true;
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Перемещать в начало списка")
            .color(Color32::from_gray(190)).size(13.0));
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        if ui.checkbox(&mut settings.replace_main, "").changed() { changed = true; }
        ui.add_space(4.0);
        ui.label(RichText::new("Ставить на место главной задачи")
            .color(Color32::from_gray(190)).size(13.0));
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        if ui.checkbox(&mut settings.group_inactive_at_end, "").changed() { changed = true; }
        ui.add_space(4.0);
        ui.label(RichText::new("Неактивные рутины — в конец списка")
            .color(Color32::from_gray(190)).size(13.0));
    });

    if changed { settings.save(); }

    changed
}