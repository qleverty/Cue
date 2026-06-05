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
///
/// `seen`: caller-maintained set of already-applied op_ids for dedup.
/// Returns `true` if the op actually changed state.
pub fn apply_op(
    op:         &Op,
    projects:   &mut Vec<LoadedProject>,
    tombstones: &mut Tombstones,
    settings:   &mut Settings,
    seen:       &mut HashSet<String>,
) -> bool {
    if !seen.insert(op.op_id.clone()) { return false; }

    match &op.kind {
        // ── projects ─────────────────────────────────────────────────────
        OpKind::CreateProject { project_id, name, color, created_at } => {
            if tombstones.deleted_at(project_id).is_some() { return false; }
            if projects.iter().any(|p| &p.id == project_id) { return false; }
            let Some(c) = hex_to_color32(color) else { return false; };
            let mut p   = LoadedProject::new(project_id.clone(), name.clone(), c, *created_at);
            p.color_hex = color.clone();
            p.save();
            projects.push(p);
        }
        OpKind::DeleteProject { project_id } => {
            if tombstones.deleted_at(project_id).is_some() { return false; }
            tombstones.add_project(project_id, op.ts, &op.device_id);
            if let Some(i) = projects.iter().position(|p| &p.id == project_id) {
                projects[i].delete_file();
                projects.remove(i);
            }
        }
        OpKind::RenameProject { project_id, name } => {
            if tombstones.deleted_at(project_id).is_some() { return false; }
            if let Some(p) = projects.iter_mut().find(|p| &p.id == project_id) {
                p.name = name.clone();
                p.save();
            }
        }
        OpKind::RecolorProject { project_id, color } => {
            if tombstones.deleted_at(project_id).is_some() { return false; }
            if let Some(p) = projects.iter_mut().find(|p| &p.id == project_id) {
                if let Some(c) = hex_to_color32(color) {
                    p.color     = c;
                    p.color_hex = color.clone();
                    p.save();
                }
            }
        }

        // ── tasks ─────────────────────────────────────────────────────────
        OpKind::AddTask { project_id, task_id, text, target } => {
            if tombstones.deleted_at(project_id)
                .or_else(|| tombstones.deleted_at(task_id)).is_some() { return false; }
            let Some(proj) = projects.iter_mut().find(|p| &p.id == project_id) else { return false; };
            // Idempotent: skip if task already present
            if proj.main.contains_key(task_id.as_str()) || proj.subs.contains_key(task_id.as_str()) {
                return false;
            }
            let task = TaskData {
                text: text.clone(), active: true, schedule: None,
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
                    proj.subs.insert(task_id.clone(), t);
                }
                AddTarget::Beginning => {
                    let mut t = task; t.order_key = proj.next_beg_key();
                    proj.subs.shift_insert(0, task_id.clone(), t);
                }
            }
            proj.save();
        }
        OpKind::DeleteTask { project_id, task_id } => {
            if tombstones.deleted_at(project_id).is_some() { return false; }
            tombstones.add_task(task_id, project_id, op.ts, &op.device_id);
            if let Some(proj) = projects.iter_mut().find(|p| &p.id == project_id) {
                proj.subs.shift_remove(task_id.as_str());
                proj.main.shift_remove(task_id.as_str());
                proj.save();
            }
        }
        OpKind::CompleteMain { project_id, task_id } => {
            if tombstones.deleted_at(project_id).is_some() { return false; }
            if let Some(proj) = projects.iter_mut().find(|p| &p.id == project_id) {
                if proj.main.contains_key(task_id.as_str()) {
                    proj.complete_main();
                    proj.save();
                }
            }
        }
        OpKind::PromoteTask { project_id, task_id } => {
            if tombstones.deleted_at(project_id).is_some() { return false; }
            if let Some(proj) = projects.iter_mut().find(|p| &p.id == project_id) {
                if let Some(i) = proj.subs.get_index_of(task_id.as_str()) {
                    proj.promote_sub(i);
                    proj.save();
                }
            }
        }
        OpKind::EditTask { project_id, task_id, text } => {
            if tombstones.deleted_at(project_id)
                .or_else(|| tombstones.deleted_at(task_id)).is_some() { return false; }
            if let Some(proj) = projects.iter_mut().find(|p| &p.id == project_id) {
                let t = proj.main.get_mut(task_id.as_str())
                    .or_else(|| proj.subs.get_mut(task_id.as_str()));
                if let Some(t) = t { t.text = text.clone(); proj.save(); }
            }
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
                _ => return false,
            }
        }
    }
    true
}