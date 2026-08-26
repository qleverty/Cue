use eframe::egui::{self, Color32, RichText};
use crate::settings::{NewTaskPos, Settings};

pub fn draw(ui: &mut egui::Ui, settings: &mut Settings, sync: &mut crate::sync::SyncHandle) -> bool {
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
            let ts = crate::project::current_time();
            if settings.apply_new_task_pos(NewTaskPos::End, ts) {
                let _ = sync.record_op(crate::sync::oplog::OpKind::SetSharedSetting {
                    key:   "new_task_pos".into(),
                    value: serde_json::to_value(NewTaskPos::End).unwrap_or_default(),
                });
            }
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
            let ts = crate::project::current_time();
            if settings.apply_new_task_pos(NewTaskPos::Beginning, ts) {
                let _ = sync.record_op(crate::sync::oplog::OpKind::SetSharedSetting {
                    key:   "new_task_pos".into(),
                    value: serde_json::to_value(NewTaskPos::Beginning).unwrap_or_default(),
                });
            }
            changed = true;
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Перемещать в начало списка")
            .color(Color32::from_gray(190)).size(13.0));
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        let mut v = settings.replace_main;
        if ui.checkbox(&mut v, "").changed() {
            let ts = crate::project::current_time();
            if settings.apply_replace_main(v, ts) {
                let _ = sync.record_op(crate::sync::oplog::OpKind::SetSharedSetting {
                    key:   "replace_main".into(),
                    value: serde_json::to_value(v).unwrap_or_default(),
                });
            }
            changed = true;
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Ставить на место главной задачи")
            .color(Color32::from_gray(190)).size(13.0));
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        // Не shared — чисто локальное предпочтение отображения, LWW не нужен.
        if ui.checkbox(&mut settings.group_inactive_at_end, "").changed() { changed = true; }
        ui.add_space(4.0);
        ui.label(RichText::new("Неактивные рутины — в конец списка")
            .color(Color32::from_gray(190)).size(13.0));
    });

    if changed { settings.save(); }

    changed
}