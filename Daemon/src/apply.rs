use std::collections::HashSet;
use crate::{
    oplog::{AddTarget, Op, OpKind},
    project::{is_valid_hex_color, LoadedProject, TaskData},
    settings::Settings,
    tombstones::Tombstones,
};

pub(crate) fn find_task_project(projects: &[LoadedProject], hint_id: &str, task_id: &str) -> Option<usize> {
    if let Some(i) = projects.iter().position(|p| p.id == hint_id) {
        if projects[i].main.contains_key(task_id) || projects[i].subs.contains_key(task_id) {
            return Some(i);
        }
    }
    projects.iter().position(|p| p.main.contains_key(task_id) || p.subs.contains_key(task_id))
}

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
            if !is_valid_hex_color(color) { return Vec::new(); }
            let mut p = LoadedProject::new(project_id.clone(), name.clone(), color.clone(), *created_at);
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
            Vec::new()
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
            if !is_valid_hex_color(color) { return Vec::new(); }
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
                    proj.subs.insert(task_id.clone(), t);
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
        OpKind::CompactProjectsOrder { order } => {
            let changed = crate::project::apply_compact_projects_order(projects, order, op.ts);
            println!("[cue_daemon] COMPACT_PROJECTS_ORDER listed={} changed={}", order.len(), changed.len());
            changed
        }
        OpKind::CompactOrder { project_id, order } => {
            if tombstones.deleted_at(project_id).is_some() { return Vec::new(); }
            let Some(p) = projects.iter_mut().find(|p| &p.id == project_id) else { return Vec::new(); };
            let changed = p.apply_compact_order(order, op.ts);
            println!("[cue_daemon] COMPACT_ORDER project={project_id} listed={} changed={changed}", order.len());
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

            let (final_text, final_text_ts, final_routine, final_routine_ts,
                 final_order_key, final_pos_ts, final_created_at, existing_transferred_at) =
                if let Some(idx) = existing {
                    let cur = projects[idx].main.get(task_id.as_str())
                        .or_else(|| projects[idx].subs.get(task_id.as_str()));
                    let Some(cur) = cur else { return Vec::new(); };
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

            if existing_transferred_at >= op.ts { return Vec::new(); }

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
