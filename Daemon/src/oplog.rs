use serde::{Deserialize, Serialize};
use std::path::Path;

// ── Op types ─────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AddTarget { Main, End, Beginning }

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "op", content = "payload")]
pub enum OpKind {
    #[serde(rename = "CREATE_PROJECT")]
    CreateProject  { project_id: String, name: String, color: String, created_at: u64 },
    #[serde(rename = "DELETE_PROJECT")]
    DeleteProject  { project_id: String },
    #[serde(rename = "RENAME_PROJECT")]
    RenameProject  { project_id: String, name: String },
    #[serde(rename = "RECOLOR_PROJECT")]
    RecolorProject { project_id: String, color: String },
    #[serde(rename = "MOVE_PROJECT")]
    MoveProject    { project_id: String, order_key: f64 },
    #[serde(rename = "ADD_TASK")]
    AddTask        { project_id: String, task_id: String, text: String, target: AddTarget },
    #[serde(rename = "DELETE_TASK")]
    DeleteTask     { project_id: String, task_id: String },
    #[serde(rename = "COMPLETE_TASK")]
    CompleteTask   { project_id: String, task_id: String, #[serde(default)] routine_edited_at: u64 },
    #[serde(rename = "PROMOTE_TASK")]
    PromoteTask    { project_id: String, task_id: String },
    #[serde(rename = "EDIT_TASK")]
    EditTask       { project_id: String, task_id: String, text: String },
    #[serde(rename = "MOVE_TASK")]
    MoveTask       { project_id: String, task_id: String, order_key: f64 },
    #[serde(rename = "COMPACT_ORDER")]
    CompactOrder   { project_id: String, order: Vec<String> },
    #[serde(rename = "COMPACT_PROJECTS_ORDER")]
    CompactProjectsOrder { order: Vec<String> },
    #[serde(rename = "SET_ROUTINE")]
    SetRoutine     { project_id: String, task_id: String, routine: Option<crate::project::Routine> },
    #[serde(rename = "TRANSFER_TASK")]
    TransferTask   {
        task_id:           String,
        from_project_id:   String,
        to_project_id:     String,
        target:            AddTarget,
        text:              String,
        text_edited_at:    u64,
        routine:           Option<crate::project::Routine>,
        routine_edited_at: u64,
        order_key:         f64,
        pos_edited_at:     u64,
        created_at:        u64,
    },
    #[serde(rename = "SET_SHARED_SETTING")]
    SetSharedSetting { key: String, value: serde_json::Value },
}

impl OpKind {
    #[allow(dead_code)]
    pub fn project_id(&self) -> Option<&str> {
        match self {
            OpKind::CreateProject  { project_id, .. } => Some(project_id),
            OpKind::DeleteProject  { project_id }     => Some(project_id),
            OpKind::RenameProject  { project_id, .. } => Some(project_id),
            OpKind::RecolorProject { project_id, .. } => Some(project_id),
            OpKind::MoveProject    { project_id, .. } => Some(project_id),
            OpKind::AddTask        { project_id, .. } => Some(project_id),
            OpKind::DeleteTask     { project_id, .. } => Some(project_id),
            OpKind::CompleteTask   { project_id, .. } => Some(project_id),
            OpKind::PromoteTask    { project_id, .. } => Some(project_id),
            OpKind::EditTask       { project_id, .. } => Some(project_id),
            OpKind::MoveTask       { project_id, .. } => Some(project_id),
            OpKind::CompactOrder   { project_id, .. } => Some(project_id),
            OpKind::SetRoutine     { project_id, .. } => Some(project_id),
            OpKind::TransferTask   { from_project_id, .. } => Some(from_project_id),
            OpKind::CompactProjectsOrder { .. }       => None,
            OpKind::SetSharedSetting { .. }           => None,
        }
    }

    #[allow(dead_code)]
    pub fn task_id(&self) -> Option<&str> {
        match self {
            OpKind::DeleteTask     { task_id, .. } => Some(task_id),
            OpKind::CompleteTask   { task_id, .. } => Some(task_id),
            OpKind::PromoteTask    { task_id, .. } => Some(task_id),
            OpKind::EditTask       { task_id, .. } => Some(task_id),
            OpKind::MoveTask       { task_id, .. } => Some(task_id),
            OpKind::SetRoutine     { task_id, .. } => Some(task_id),
            OpKind::TransferTask   { task_id, .. } => Some(task_id),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Op {
    pub op_id:     String,
    pub device_id: String,
    pub seq:       u64,
    pub ts:        u64,
    #[serde(flatten)]
    pub kind:      OpKind,
}

pub fn read_existing(dir: &Path) -> Vec<Op> {
    std::fs::read_to_string(dir.join("ops.ndjson")).ok()
        .map(|s| s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default()
}
