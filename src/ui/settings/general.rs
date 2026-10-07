use eframe::egui::{self, Color32, RichText};
use crate::settings::{NewTaskPos, Settings, StartupMode};
use crate::project::LoadedProject;

pub(super) const SECTION_TITLE_SIZE: f32 = 10.0;

pub(super) fn first_section_title(ui: &mut egui::Ui, text: &str) {
    ui.horizontal_top(|ui| {
        ui.set_min_height(ui.spacing().interact_size.y);
        ui.add_space(14.0);
        ui.label(RichText::new(text)
            .color(Color32::from_white_alpha(90)).size(SECTION_TITLE_SIZE));
    });
}

pub(super) fn toggle_label(ui: &mut egui::Ui, text: &str) -> bool {
    let base  = Color32::from_gray(190);
    let hover = Color32::from_gray(230);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(), egui::FontId::proportional(13.0), Color32::PLACEHOLDER,
    );
    let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
    ui.painter().galley(rect.min, galley, if response.hovered() { hover } else { base });
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

pub fn draw(
    ui:       &mut egui::Ui,
    settings: &mut Settings,
    sync:     &mut crate::sync::SyncHandle,
    projects: &[LoadedProject],
) -> bool {
    ui.visuals_mut().selection.bg_fill = Color32::from_rgb(86, 111, 146);

    let mut changed = false;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
        ui.add_space(14.0);

        first_section_title(ui, "При создании новых задач:");
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let radio_clicked = ui.add(egui::RadioButton::new(
                settings.new_task_pos == NewTaskPos::End, "")).clicked();
            ui.add_space(4.0);
            let label_clicked = toggle_label(ui, "Перемещать в конец списка");
            if radio_clicked || label_clicked {
                let ts = crate::project::current_time();
                if settings.apply_new_task_pos(NewTaskPos::End, ts) {
                    let _ = sync.record_op(crate::sync::oplog::OpKind::SetSharedSetting {
                        key:   "new_task_pos".into(),
                        value: serde_json::to_value(NewTaskPos::End).unwrap_or_default(),
                    });
                }
                changed = true;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let radio_clicked = ui.add(egui::RadioButton::new(
                settings.new_task_pos == NewTaskPos::Beginning, "")).clicked();
            ui.add_space(4.0);
            let label_clicked = toggle_label(ui, "Перемещать в начало списка");
            if radio_clicked || label_clicked {
                let ts = crate::project::current_time();
                if settings.apply_new_task_pos(NewTaskPos::Beginning, ts) {
                    let _ = sync.record_op(crate::sync::oplog::OpKind::SetSharedSetting {
                        key:   "new_task_pos".into(),
                        value: serde_json::to_value(NewTaskPos::Beginning).unwrap_or_default(),
                    });
                }
                changed = true;
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let mut v = settings.replace_main;
            let checkbox_clicked = ui.checkbox(&mut v, "").changed();
            ui.add_space(4.0);
            let label_clicked = toggle_label(ui, "Ставить на место главной задачи");
            if checkbox_clicked || label_clicked {
                if label_clicked { v = !settings.replace_main; }
                let ts = crate::project::current_time();
                if settings.apply_replace_main(v, ts) {
                    let _ = sync.record_op(crate::sync::oplog::OpKind::SetSharedSetting {
                        key:   "replace_main".into(),
                        value: serde_json::to_value(v).unwrap_or_default(),
                    });
                }
                changed = true;
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new("При запуске:")
                .color(Color32::from_white_alpha(90)).size(SECTION_TITLE_SIZE));
        });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let radio_clicked = ui.add(egui::RadioButton::new(
                settings.startup_mode == StartupMode::LastOpened, "")).clicked();
            ui.add_space(4.0);
            let label_clicked = toggle_label(ui, "Открывать последний проект");
            if radio_clicked || label_clicked {
                settings.startup_mode = StartupMode::LastOpened;
                changed = true;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let radio_clicked = ui.add(egui::RadioButton::new(
                settings.startup_mode == StartupMode::Fixed, "")).clicked();
            ui.add_space(4.0);
            let label_clicked = toggle_label(ui, "Всегда открывать:");
            if radio_clicked || label_clicked {
                settings.startup_mode = StartupMode::Fixed;
                changed = true;
            }
        });
        ui.add_space(2.0);
        if crate::ui::settings::projects_dropdown::draw(ui, settings, projects) {
            changed = true;
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new("Прочее:")
                .color(Color32::from_white_alpha(90)).size(SECTION_TITLE_SIZE));
        });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let mut v = settings.group_inactive_at_end;
            let checkbox_clicked = ui.checkbox(&mut v, "").changed();
            ui.add_space(4.0);
            let label_clicked = toggle_label(ui, "Неактивные рутины в конце списка");
            if checkbox_clicked || label_clicked {
                settings.group_inactive_at_end = if label_clicked { !settings.group_inactive_at_end } else { v };
                changed = true;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let mut v = settings.highlight_routines;
            let checkbox_clicked = ui.checkbox(&mut v, "").changed();
            ui.add_space(4.0);
            let label_clicked = toggle_label(ui, "Подсветка рутинных задач");
            if checkbox_clicked || label_clicked {
                settings.highlight_routines = if label_clicked { !settings.highlight_routines } else { v };
                changed = true;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let mut v = settings.show_task_count;
            let checkbox_clicked = ui.checkbox(&mut v, "").changed();
            ui.add_space(4.0);
            let label_clicked = toggle_label(ui, "Число задач в списке проектов");
            if checkbox_clicked || label_clicked {
                settings.show_task_count = if label_clicked { !settings.show_task_count } else { v };
                changed = true;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let mut v = settings.delete_spent_routines;
            let checkbox_clicked = ui.checkbox(&mut v, "").changed();
            ui.add_space(4.0);
            let label_clicked = toggle_label(ui, "Удалять истёкшие рутины");
            if checkbox_clicked || label_clicked {
                settings.delete_spent_routines = if label_clicked { !settings.delete_spent_routines } else { v };
                changed = true;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let mut v = settings.background_daemon;
            let checkbox_clicked = ui.checkbox(&mut v, "").changed();
            ui.add_space(4.0);
            let label_clicked = toggle_label(ui, "Работа в фоновом режиме");
            if checkbox_clicked || label_clicked {
                settings.background_daemon = if label_clicked { !settings.background_daemon } else { v };
                crate::autostart::set_enabled(settings.background_daemon);
                changed = true;
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(format!("Version: {}", env!("CARGO_PKG_VERSION")))
                .color(Color32::from_white_alpha(45)).size(SECTION_TITLE_SIZE));
        });
        ui.add_space(6.0);
        });

    if changed { settings.save(); }

    changed
}