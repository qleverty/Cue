use serde::{Deserialize, Serialize};


#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
pub enum NewTaskPos {
    #[default]
    End,
    Beginning,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
pub enum StartupMode {
    #[default]
    LastOpened,
    Fixed,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ProjectSort {
    ByName,
    #[default]
    ByColor,
    ByCreated,
    ByModified,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub new_task_pos:     NewTaskPos,
    #[serde(default)]
    pub new_task_pos_edited_at: u64,
    pub replace_main:     bool,
    #[serde(default)]
    pub replace_main_edited_at: u64,
    #[serde(default)]
    pub last_project_id:  Option<String>,
    #[serde(default)]
    pub startup_mode:     StartupMode,
    #[serde(default)]
    pub fixed_project_id: Option<String>,
    #[serde(default)]
    pub last_width:       Option<f32>,
    #[serde(default)]
    pub last_pos:         Option<[f32; 2]>,
    #[serde(default = "default_group_inactive")]
    pub group_inactive_at_end: bool,
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_highlight_routines")]
    pub highlight_routines: bool,
    #[serde(default)]
    pub show_task_count: bool,
    #[serde(default = "default_delete_spent_routines")]
    pub delete_spent_routines: bool,
    #[serde(default)]
    pub project_sort: ProjectSort,
}

fn default_group_inactive() -> bool { true }
fn default_http_port() -> u16 { crate::server::DEFAULT_PORT }
fn default_highlight_routines() -> bool { true }
fn default_delete_spent_routines() -> bool { true }

impl Settings {
    pub fn apply_new_task_pos(&mut self, value: NewTaskPos, ts: u64) -> bool {
        if ts <= self.new_task_pos_edited_at { return false; }
        self.new_task_pos = value;
        self.new_task_pos_edited_at = ts;
        true
    }

    pub fn apply_replace_main(&mut self, value: bool, ts: u64) -> bool {
        if ts <= self.replace_main_edited_at { return false; }
        self.replace_main = value;
        self.replace_main_edited_at = ts;
        true
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            new_task_pos:     NewTaskPos::End,
            new_task_pos_edited_at: 0,
            replace_main:     false,
            replace_main_edited_at: 0,
            last_project_id:  None,
            startup_mode:     StartupMode::LastOpened,
            fixed_project_id: None,
            last_width:       None,
            last_pos:         None,
            group_inactive_at_end: true,
            http_port: default_http_port(),
            highlight_routines: true,
            show_task_count: false,
            delete_spent_routines: true,
            project_sort: ProjectSort::default(),
        }
    }
}

fn settings_path() -> std::path::PathBuf {
    crate::app_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(settings_path()).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        if let Ok(j) = serde_json::to_string(self) {
            let _ = std::fs::write(settings_path(), j);
        }
    }
}
