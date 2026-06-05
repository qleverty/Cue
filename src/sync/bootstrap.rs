use super::{
    identity::DeviceIdentity,
    oplog::{AddTarget, OpKind, OpLog},
};

/// Runs only once: if ops.ndjson is empty, generates CREATE_PROJECT + ADD_TASK
/// ops from existing project snapshots and assigns order_keys.
pub fn run_if_needed(
    projects: &mut Vec<crate::project::LoadedProject>,
    identity: &DeviceIdentity,
    oplog:    &mut OpLog,
) {
    if oplog.next_seq > 1 { return; }

    for proj in projects.iter_mut() {
        for (i, task) in proj.main.values_mut().enumerate() {
            task.order_key = i as f64 * 1000.0;
        }
        for (i, task) in proj.subs.values_mut().enumerate() {
            task.order_key = i as f64 * 1000.0;
        }
        proj.save();

        let _ = oplog.append(
            OpKind::CreateProject {
                project_id: proj.id.clone(),
                name:       proj.name.clone(),
                color:      proj.color_hex.clone(),
                created_at: proj.created_at,
            },
            &identity.device_id,
            proj.created_at,
        );
        for (task_id, task) in proj.main.iter() {
            let _ = oplog.append(
                OpKind::AddTask {
                    project_id: proj.id.clone(),
                    task_id:    task_id.clone(),
                    text:       task.text.clone(),
                    target:     AddTarget::Main,
                },
                &identity.device_id,
                task.created_at,
            );
        }
        for (task_id, task) in proj.subs.iter() {
            let _ = oplog.append(
                OpKind::AddTask {
                    project_id: proj.id.clone(),
                    task_id:    task_id.clone(),
                    text:       task.text.clone(),
                    target:     AddTarget::End,
                },
                &identity.device_id,
                task.created_at,
            );
        }
    }
}