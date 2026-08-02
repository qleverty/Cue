use std::collections::HashSet;
use crate::{
    project::{hex_to_color32, LoadedProject, TaskData},
    settings::Settings,
    sync::{
        oplog::{AddTarget, Op, OpKind},
        tombstones::Tombstones,
    },
};

/// Apply one op to the full mutable state.
/// Returns `Some(project_id)` if a project was modified and needs saving.
/// Returns `None` if the op was skipped (dedup / tombstoned) or no project was touched.
pub fn apply_op(
    op:         &Op,
    projects:   &mut Vec<LoadedProject>,
    tombstones: &mut Tombstones,
    settings:   &mut Settings,
    seen:       &mut HashSet<String>,
) -> Option<String> {
    if !seen.insert(op.op_id.clone()) { return None; }

    match &op.kind {
        // ── projects ─────────────────────────────────────────────────────
        OpKind::CreateProject { project_id, name, color, created_at } => {
            if tombstones.deleted_at(project_id).is_some() { return None; }
            if projects.iter().any(|p| &p.id == project_id) { return None; }
            let c = hex_to_color32(color)?;
            let mut p   = LoadedProject::new(project_id.clone(), name.clone(), c, *created_at);
            p.color_hex = color.clone();
            projects.push(p);
            Some(project_id.clone())
        }
        OpKind::DeleteProject { project_id } => {
            if tombstones.deleted_at(project_id).is_some() { return None; }
            tombstones.add_project(project_id, op.ts, &op.device_id);
            if let Some(i) = projects.iter().position(|p| &p.id == project_id) {
                projects[i].delete_file();
                projects.remove(i);
            }
            None // project gone, nothing to save
        }
        OpKind::RenameProject { project_id, name } => {
            if tombstones.deleted_at(project_id).is_some() { return None; }
            let p = projects.iter_mut().find(|p| &p.id == project_id)?;
            p.name = name.clone();
            Some(project_id.clone())
        }
        OpKind::RecolorProject { project_id, color } => {
            if tombstones.deleted_at(project_id).is_some() { return None; }
            let p = projects.iter_mut().find(|p| &p.id == project_id)?;
            let c = hex_to_color32(color)?;
            p.color     = c;
            p.color_hex = color.clone();
            Some(project_id.clone())
        }

        // ── tasks ─────────────────────────────────────────────────────────
        OpKind::AddTask { project_id, task_id, text, target } => {
            if tombstones.deleted_at(project_id)
                .or_else(|| tombstones.deleted_at(task_id)).is_some() { return None; }
            let proj = projects.iter_mut().find(|p| &p.id == project_id)?;
            if proj.main.contains_key(task_id.as_str())
                || proj.subs.contains_key(task_id.as_str()) { return None; }
            let task = TaskData {
                text: text.clone(), routine: None,
                created_at: op.ts, order_key: 0.0,
            };
            match target {
                AddTarget::Main => {
                    if !proj.main.is_empty() {
                        let (old_id, mut old) = proj.main.shift_remove_index(0).unwrap();
                        old.order_key = proj.next_end_key();
                        proj.subs.insert(old_id, old);
                    }
                    proj.main.insert(task_id.clone(), task);
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
            Some(project_id.clone())
        }
        OpKind::DeleteTask { project_id, task_id } => {
            if tombstones.deleted_at(project_id).is_some() { return None; }
            tombstones.add_task(task_id, project_id, op.ts, &op.device_id);
            let proj = projects.iter_mut().find(|p| &p.id == project_id)?;
            proj.subs.shift_remove(task_id.as_str());
            proj.main.shift_remove(task_id.as_str());
            Some(project_id.clone())
        }
        OpKind::CompleteMain { project_id, task_id } => {
            if tombstones.deleted_at(project_id).is_some() { return None; }
            let proj = projects.iter_mut().find(|p| &p.id == project_id)?;
            if !proj.main.contains_key(task_id.as_str()) { return None; }
            // op.ts — момент, когда завершение реально произошло на исходном
            // устройстве; используем его же для проверки "все ли direct-даты
            // рутины уже прошли" (см. раздел 5.2/7.1 плана — намеренное
            // упрощение вместо отдельного сравнения меток).
            proj.complete_main(op.ts);
            Some(project_id.clone())
        }
        OpKind::SetRoutine { project_id, task_id, routine } => {
            if tombstones.deleted_at(project_id)
                .or_else(|| tombstones.deleted_at(task_id)).is_some() { return None; }
            let proj = projects.iter_mut().find(|p| &p.id == project_id)?;
            let task = proj.main.get_mut(task_id.as_str())
                .or_else(|| proj.subs.get_mut(task_id.as_str()))?;
            task.routine = routine.clone();
            Some(project_id.clone())
        }
        OpKind::PromoteTask { project_id, task_id } => {
            if tombstones.deleted_at(project_id).is_some() { return None; }
            let proj = projects.iter_mut().find(|p| &p.id == project_id)?;
            let i = proj.subs.get_index_of(task_id.as_str())?;
            proj.promote_sub(i);
            Some(project_id.clone())
        }
        OpKind::EditTask { project_id, task_id, text } => {
            if tombstones.deleted_at(project_id)
                .or_else(|| tombstones.deleted_at(task_id)).is_some() { return None; }
            let proj = projects.iter_mut().find(|p| &p.id == project_id)?;
            let t = proj.main.get_mut(task_id.as_str())
                .or_else(|| proj.subs.get_mut(task_id.as_str()))?;
            t.text = text.clone();
            Some(project_id.clone())
        }

        // ── settings ─────────────────────────────────────────────────────
        OpKind::SetSharedSetting { key, value } => {
            match key.as_str() {
                "new_task_pos" => if let Ok(v) = serde_json::from_value(value.clone()) {
                    settings.new_task_pos = v; settings.save();
                },
                "replace_main" => if let Ok(v) = serde_json::from_value(value.clone()) {
                    settings.replace_main = v; settings.save();
                },
                _ => {}
            }
            None
        }
    }
}