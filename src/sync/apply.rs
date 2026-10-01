use std::collections::HashSet;
use crate::{
    project::{hex_to_color32, LoadedProject, TaskData},
    settings::Settings,
    sync::{
        oplog::{AddTarget, Op, OpKind},
        tombstones::Tombstones,
    },
};

/// Индекс проекта, где сейчас реально лежит task_id — сначала пробуем
/// hint_id (адрес на момент отправки опа), если не нашли там — ищем по
/// всем загруженным проектам: задача могла уехать (TransferTask) уже
/// куда-то ещё, пока этот оп летел по сети. project_id в опе после этого
/// — просто подсказка-маршрутизация, не авторитетный адрес.
pub(crate) fn find_task_project(projects: &[LoadedProject], hint_id: &str, task_id: &str) -> Option<usize> {
    if let Some(i) = projects.iter().position(|p| p.id == hint_id) {
        if projects[i].main.contains_key(task_id) || projects[i].subs.contains_key(task_id) {
            return Some(i);
        }
    }
    projects.iter().position(|p| p.main.contains_key(task_id) || p.subs.contains_key(task_id))
}

/// Apply one op to the full mutable state.
/// Returns project_id'ы, которые были задеты и нуждаются в сохранении на
/// диск — обычно один, но TransferTask трогает сразу два (откуда и куда).
///
/// Заодно двигает `last_edited` задетых проектов на `op.ts` — но только для
/// опов, которые считаются правкой содержимого проекта (см. `counts_as_edit`).
pub fn apply_op(
    op:         &Op,
    projects:   &mut Vec<LoadedProject>,
    tombstones: &mut Tombstones,
    settings:   &mut Settings,
    seen:       &mut HashSet<String>,
) -> Vec<String> {
    let dirty = apply_op_inner(op, projects, tombstones, settings, seen);
    if counts_as_edit(&op.kind) {
        for id in &dirty {
            if let Some(p) = projects.iter_mut().find(|p| &p.id == id) {
                p.touch(op.ts);
            }
        }
    }
    dirty
}

/// Какие опы считаются «изменением проекта» для сортировки по дате
/// изменения. НЕ считаются: создание (дата = created_at), удаление проекта,
/// MoveProject (порядок в списке проектов — не содержимое проекта),
/// CompactOrder (служебная перенумерация) и настройки.
fn counts_as_edit(kind: &OpKind) -> bool {
    matches!(kind,
        OpKind::AddTask { .. }      | OpKind::DeleteTask { .. }
        | OpKind::CompleteTask { .. } | OpKind::SetRoutine { .. }
        | OpKind::PromoteTask { .. }  | OpKind::EditTask { .. }
        | OpKind::MoveTask { .. }     | OpKind::TransferTask { .. }
        | OpKind::RenameProject { .. } | OpKind::RecolorProject { .. })
}

fn apply_op_inner(
    op:         &Op,
    projects:   &mut Vec<LoadedProject>,
    tombstones: &mut Tombstones,
    settings:   &mut Settings,
    seen:       &mut HashSet<String>,
) -> Vec<String> {
    if !seen.insert(op.op_id.clone()) { return Vec::new(); }

    match &op.kind {
        // ── projects ─────────────────────────────────────────────────────
        OpKind::CreateProject { project_id, name, color, created_at } => {
            if tombstones.deleted_at(project_id).is_some() { return Vec::new(); }
            if projects.iter().any(|p| &p.id == project_id) { return Vec::new(); }
            let Some(c) = hex_to_color32(color) else { return Vec::new(); };
            let mut p   = LoadedProject::new(project_id.clone(), name.clone(), c, *created_at);
            p.color_hex = color.clone();
            // "В конец" — единственный вариант размещения нового проекта в
            // v2 (сортировки "Произвольный" ещё нет), поэтому order_key не
            // приходит по сети — каждое устройство ставит его само,
            // независимо, тем же приёмом, что и next_end_key() у задач.
            p.order_key = projects.iter().map(|p| p.order_key).fold(0.0, f64::max) + 1000.0;
            projects.push(p);
            vec![project_id.clone()]
        }
        OpKind::DeleteProject { project_id } => {
            if tombstones.deleted_at(project_id).is_some() { return Vec::new(); }
            tombstones.add_project(project_id, op.ts, &op.device_id);
            if let Some(i) = projects.iter().position(|p| &p.id == project_id) {
                projects[i].delete_file();
                projects.remove(i);
            }
            Vec::new() // project gone, nothing to save
        }
        OpKind::RenameProject { project_id, name } => {
            if tombstones.deleted_at(project_id).is_some() { return Vec::new(); }
            let Some(p) = projects.iter_mut().find(|p| &p.id == project_id) else { return Vec::new(); };
            if op.ts <= p.name_edited_at { return Vec::new(); }
            p.name = name.clone();
            p.name_edited_at = op.ts;
            vec![project_id.clone()]
        }
        OpKind::RecolorProject { project_id, color } => {
            if tombstones.deleted_at(project_id).is_some() { return Vec::new(); }
            let Some(p) = projects.iter_mut().find(|p| &p.id == project_id) else { return Vec::new(); };
            if op.ts <= p.color_edited_at { return Vec::new(); }
            let Some(c) = hex_to_color32(color) else { return Vec::new(); };
            p.color     = c;
            p.color_hex = color.clone();
            p.color_edited_at = op.ts;
            vec![project_id.clone()]
        }
        OpKind::MoveProject { project_id, order_key } => {
            if tombstones.deleted_at(project_id).is_some() { return Vec::new(); }
            let Some(p) = projects.iter_mut().find(|p| &p.id == project_id) else { return Vec::new(); };
            if op.ts <= p.order_key_edited_at { return Vec::new(); }
            p.order_key = *order_key;
            p.order_key_edited_at = op.ts;
            vec![project_id.clone()]
        }

        // ── tasks ─────────────────────────────────────────────────────────
        OpKind::AddTask { project_id, task_id, text, target } => {
            if tombstones.deleted_at(project_id)
                .or_else(|| tombstones.deleted_at(task_id)).is_some() { return Vec::new(); }
            let Some(proj) = projects.iter_mut().find(|p| &p.id == project_id) else { return Vec::new(); };
            if proj.main.contains_key(task_id.as_str())
                || proj.subs.contains_key(task_id.as_str()) { return Vec::new(); }
            let task = TaskData {
                text: text.clone(), routine: None,
                created_at: op.ts, order_key: 0.0,
                text_edited_at: op.ts, routine_edited_at: 0, pos_edited_at: 0,
                transferred_at: 0, completed_at: 0,
            };
            match target {
                AddTarget::Main => {
                    proj.apply_add_to_main(task_id.clone(), task, op.ts);
                }
                AddTarget::End => {
                    let mut t = task; t.order_key = proj.next_end_key();
                    proj.subs.insert(task_id.clone(), t); // физический конец
                }
                AddTarget::Beginning => {
                    let mut t = task; t.order_key = proj.next_beg_key();
                    proj.subs.shift_insert(0, task_id.clone(), t);
                }
            }
            vec![project_id.clone()]
        }
        OpKind::DeleteTask { project_id, task_id } => {
            if tombstones.deleted_at(project_id).is_some() { return Vec::new(); }
            tombstones.add_task(task_id, project_id, op.ts, &op.device_id);
            // По task_id, не строго по project_id — задача могла уехать
            // (TransferTask) в другой проект до того, как это удаление
            // сюда долетело; project_id тут просто подсказка-адрес.
            let Some(idx) = find_task_project(projects, project_id, task_id) else { return Vec::new(); };
            let real_id = projects[idx].id.clone();
            projects[idx].subs.shift_remove(task_id.as_str());
            projects[idx].main.shift_remove(task_id.as_str());
            vec![real_id]
        }
        OpKind::CompleteTask { project_id, task_id, routine_edited_at } => {
            if tombstones.deleted_at(project_id).is_some() { return Vec::new(); }
            let Some(idx) = find_task_project(projects, project_id, task_id) else { return Vec::new(); };
            let real_id = projects[idx].id.clone();
            // op.ts используется и для чистки direct-дат, и для
            // main_edited_at/LWW-гейта — точного локального времени
            // исходного устройства всё равно нет (намеренное упрощение, см.
            // project.rs). Удаление исчерпанной задачи сюда НЕ входит —
            // если оно нужно, отправитель присылает отдельный DeleteTask.
            if projects[idx].complete_task(task_id, op.ts, op.ts, *routine_edited_at).is_none() { return Vec::new(); }
            vec![real_id]
        }
        OpKind::SetRoutine { project_id, task_id, routine } => {
            if tombstones.deleted_at(project_id)
                .or_else(|| tombstones.deleted_at(task_id)).is_some() { return Vec::new(); }
            let Some(idx) = find_task_project(projects, project_id, task_id) else { return Vec::new(); };
            let real_id = projects[idx].id.clone();
            if !projects[idx].apply_set_routine(task_id, routine.clone(), op.ts) { return Vec::new(); }
            vec![real_id]
        }
        OpKind::PromoteTask { project_id, task_id } => {
            if tombstones.deleted_at(project_id).is_some() { return Vec::new(); }
            let Some(idx) = find_task_project(projects, project_id, task_id) else { return Vec::new(); };
            let real_id = projects[idx].id.clone();
            if !projects[idx].apply_promote_task(task_id, op.ts) { return Vec::new(); }
            vec![real_id]
        }
        OpKind::EditTask { project_id, task_id, text } => {
            if tombstones.deleted_at(project_id)
                .or_else(|| tombstones.deleted_at(task_id)).is_some() { return Vec::new(); }
            let Some(idx) = find_task_project(projects, project_id, task_id) else { return Vec::new(); };
            let real_id = projects[idx].id.clone();
            if !projects[idx].apply_edit_task(task_id, text, op.ts) { return Vec::new(); }
            vec![real_id]
        }
        OpKind::MoveTask { project_id, task_id, order_key } => {
            if tombstones.deleted_at(project_id)
                .or_else(|| tombstones.deleted_at(task_id)).is_some() { return Vec::new(); }
            let Some(idx) = find_task_project(projects, project_id, task_id) else { return Vec::new(); };
            let real_id = projects[idx].id.clone();
            if !projects[idx].apply_move_task(task_id, *order_key, op.ts) { return Vec::new(); }
            vec![real_id]
        }
        OpKind::CompactOrder { project_id, order } => {
            if tombstones.deleted_at(project_id).is_some() { return Vec::new(); }
            let Some(p) = projects.iter_mut().find(|p| &p.id == project_id) else { return Vec::new(); };
            let known = order.iter().filter(|id| p.subs.contains_key(id.as_str())).count();
            let changed = p.apply_compact_order(order, op.ts);
            crate::clog!(
                "[apply] COMPACT_ORDER project={project_id} listed={} known={known} changed={changed}",
                order.len()
            );
            if !changed { return Vec::new(); }
            vec![project_id.clone()]
        }
        OpKind::TransferTask {
            task_id, from_project_id, to_project_id, target,
            text, text_edited_at, routine, routine_edited_at,
            order_key, pos_edited_at, created_at,
        } => {
            if tombstones.deleted_at(task_id).is_some() { return Vec::new(); }
            if tombstones.deleted_at(to_project_id).is_some() { return Vec::new(); }

            let existing = find_task_project(projects, from_project_id, task_id);

            // Мёржим по каждому полю отдельно против того, что реально уже
            // лежит на найденной копии (если нашлась) — TransferTask несёт
            // снепшот, а не авторитетную перезапись: пока он летел, кто-то
            // мог независимо отредактировать текст/рутину/позицию с более
            // свежим штампом, и это не должно потеряться при переносе.
            let (final_text, final_text_ts, final_routine, final_routine_ts,
                 final_order_key, final_pos_ts, final_created_at, existing_transferred_at) =
                if let Some(idx) = existing {
                    let cur = projects[idx].main.get(task_id.as_str())
                        .or_else(|| projects[idx].subs.get(task_id.as_str()));
                    let Some(cur) = cur else { return Vec::new(); }; // не должно случиться — find_task_project уже подтвердил наличие
                    (
                        if *text_edited_at >= cur.text_edited_at { text.clone() } else { cur.text.clone() },
                        (*text_edited_at).max(cur.text_edited_at),
                        if *routine_edited_at >= cur.routine_edited_at { routine.clone() } else { cur.routine.clone() },
                        (*routine_edited_at).max(cur.routine_edited_at),
                        if *pos_edited_at >= cur.pos_edited_at { *order_key } else { cur.order_key },
                        (*pos_edited_at).max(cur.pos_edited_at),
                        cur.created_at.min(*created_at),
                        cur.transferred_at,
                    )
                } else {
                    (text.clone(), *text_edited_at, routine.clone(), *routine_edited_at,
                     *order_key, *pos_edited_at, *created_at, 0)
                };

            // Гонка "куда едет" — не про поля, а про сам факт переноса:
            // выигрывает перенос с более поздним ts, независимо от порядка
            // доставки. Если задачу раньше никогда не переносили,
            // existing_transferred_at=0, и любой перенос выигрывает.
            if existing_transferred_at >= op.ts { return Vec::new(); }

            // completed_at в снепшоте не едет (оно локальная производная от
            // CompleteTask) — переносим то, что уже лежит на найденной копии.
            let existing_completed_at = existing
                .and_then(|idx| projects[idx].main.get(task_id.as_str())
                    .or_else(|| projects[idx].subs.get(task_id.as_str())))
                .map_or(0, |t| t.completed_at);

            let mut touched = Vec::new();
            if let Some(idx) = existing {
                let from_real_id = projects[idx].id.clone();
                projects[idx].subs.shift_remove(task_id.as_str());
                projects[idx].main.shift_remove(task_id.as_str());
                touched.push(from_real_id);
            }

            let Some(dest) = projects.iter_mut().find(|p| &p.id == to_project_id) else { return touched; };
            let new_task = TaskData {
                text: final_text, routine: final_routine,
                created_at: final_created_at, order_key: final_order_key,
                text_edited_at: final_text_ts, routine_edited_at: final_routine_ts,
                pos_edited_at: final_pos_ts, transferred_at: op.ts,
                completed_at: existing_completed_at,
            };
            match target {
                AddTarget::Main => { dest.apply_add_to_main(task_id.clone(), new_task, op.ts); }
                AddTarget::End | AddTarget::Beginning => {
                    // order_key уже явно задан в снепшоте (пережил мёрж выше) —
                    // в отличие от AddTask, тут не пересчитываем через
                    // next_end_key()/next_beg_key() заново: у AddTask
                    // End/Beginning — просто чуть более грубая версия того
                    // же самого, а тут уже есть точное значение, ему и верим.
                    dest.subs.insert(task_id.clone(), new_task);
                }
            }
            touched.push(to_project_id.clone());
            touched
        }

        // ── settings ─────────────────────────────────────────────────────
        OpKind::SetSharedSetting { key, value } => {
            let mut applied = false;
            match key.as_str() {
                "new_task_pos" => if let Ok(v) = serde_json::from_value(value.clone()) {
                    applied = settings.apply_new_task_pos(v, op.ts);
                },
                "replace_main" => if let Ok(v) = serde_json::from_value(value.clone()) {
                    applied = settings.apply_replace_main(v, op.ts);
                },
                _ => {}
            }
            if applied { settings.save(); }
            Vec::new()
        }
    }
}
#[cfg(test)]
mod touch_tests {
    use super::*;
    use eframe::egui::Color32;

    fn proj(id: &str, created: u64) -> LoadedProject {
        LoadedProject::new(id.into(), id.into(), Color32::WHITE, created)
    }

    fn op(n: u32, ts: u64, kind: OpKind) -> Op {
        Op { op_id: format!("op{n}"), device_id: "dev".into(), seq: n as u64, ts, kind }
    }

    /// Применяет оп к одному проекту "p" (created_at = 100) и возвращает его
    /// last_edited после применения.
    fn last_edited_after(kinds: Vec<(u64, OpKind)>) -> u64 {
        let mut projects = vec![proj("p", 100)];
        let mut tombs = Tombstones::load(std::path::Path::new("/nonexistent-cue-test-dir"));
        let mut settings = Settings::default();
        let mut seen = HashSet::new();
        for (i, (ts, kind)) in kinds.into_iter().enumerate() {
            apply_op(&op(i as u32, ts, kind), &mut projects, &mut tombs, &mut settings, &mut seen);
        }
        projects[0].last_edited
    }

    fn add_task(task: &str) -> OpKind {
        OpKind::AddTask {
            project_id: "p".into(), task_id: task.into(),
            text: "t".into(), target: AddTarget::End,
        }
    }

    #[test]
    fn task_ops_move_last_edited_to_op_ts() {
        assert_eq!(last_edited_after(vec![(500, add_task("t1"))]), 500);
    }

    #[test]
    fn rename_and_recolor_count_as_edit() {
        assert_eq!(last_edited_after(vec![
            (300, OpKind::RenameProject { project_id: "p".into(), name: "x".into() }),
        ]), 300);
        assert_eq!(last_edited_after(vec![
            (400, OpKind::RecolorProject { project_id: "p".into(), color: "#112233".into() }),
        ]), 400);
    }

    #[test]
    fn move_project_and_compact_order_do_not_count() {
        assert_eq!(last_edited_after(vec![
            (900, OpKind::MoveProject { project_id: "p".into(), order_key: 5.0 }),
        ]), 100);
        assert_eq!(last_edited_after(vec![
            (900, OpKind::CompactOrder { project_id: "p".into(), order: vec![] }),
        ]), 100);
    }

    #[test]
    fn late_old_op_does_not_roll_last_edited_back() {
        assert_eq!(last_edited_after(vec![
            (800, add_task("t1")),
            (200, add_task("t2")),
        ]), 800);
    }

    #[test]
    fn rejected_op_does_not_touch() {
        // Задача для несуществующего проекта: apply ничего не задел.
        let mut projects = vec![proj("p", 100)];
        let mut tombs = Tombstones::load(std::path::Path::new("/nonexistent-cue-test-dir"));
        let mut settings = Settings::default();
        let mut seen = HashSet::new();
        let kind = OpKind::AddTask {
            project_id: "other".into(), task_id: "t".into(),
            text: "t".into(), target: AddTarget::End,
        };
        let dirty = apply_op(&op(1, 999, kind), &mut projects, &mut tombs, &mut settings, &mut seen);
        assert!(dirty.is_empty());
        assert_eq!(projects[0].last_edited, 100);
    }
}
