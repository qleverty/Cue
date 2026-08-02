use serde::{Deserialize, Serialize};
use eframe::egui::{
    self, Color32, RichText, Sense, ViewportCommand, vec2,
};
use super::BG;

pub const SW: f32 = 300.0;

pub const SH_GENERAL:  f32 = 226.0;
pub const SH_PROJECTS: f32 = 160.0;
pub const SH_SYNC:     f32 = 310.0;

// ── Settings data ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
pub enum NewTaskPos {
    #[default]
    End,
    Beginning,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub new_task_pos:     NewTaskPos,
    pub replace_main:     bool,
    #[serde(default)]
    pub last_project_id:  Option<String>,
    #[serde(default)]
    pub last_width:       Option<f32>,
    #[serde(default)]
    pub last_pos:         Option<[f32; 2]>,
    /// Режим показа subs: true — неактивные рутины визуально едут в конец
    /// списка (группировка active/inactive чисто display-time, физический
    /// порядок в файле не трогается); false — "плоский" режим, порядок в UI
    /// = физический порядок, неактивные просто тусклые на своём месте.
    /// См. обсуждение 2026-08-02.
    #[serde(default = "default_group_inactive")]
    pub group_inactive_at_end: bool,
}

fn default_group_inactive() -> bool { true }

impl Default for Settings {
    fn default() -> Self {
        Self {
            new_task_pos:     NewTaskPos::End,
            replace_main:     false,
            last_project_id:  None,
            last_width:       None,
            last_pos:         None,
            group_inactive_at_end: true,
        }
    }
}

fn settings_path() -> std::path::PathBuf {
    super::app_dir().join("settings.json")
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

// ── UI state ──────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Projects,
    Sync,
}

#[derive(Default)]
pub struct SettingsUiState {
    pub tab:        SettingsTab,
    pub sync_panel: crate::ui::settings::sync_panel::SyncPanelState,
}

impl SettingsUiState {
    pub fn target_height(&self) -> f32 {
        match self.tab {
            SettingsTab::General  => SH_GENERAL,
            SettingsTab::Projects => SH_PROJECTS,
            SettingsTab::Sync     => SH_SYNC,
        }
    }
}

// ── Draw ──────────────────────────────────────────────────────────────────────

/// Returns `(close, target_height)`.
/// `target_height` is the desired inner window height for the active tab —
/// the caller can forward it to `ViewportCommand::InnerSize` (Step 7).
pub fn draw_settings_ui(
    ctx:      &egui::Context,
    ui:       &mut egui::Ui,
    settings: &mut Settings,
    state:    &mut SettingsUiState,
    sync:     &mut crate::sync::SyncHandle,
) -> (bool, f32) {
    let mut close = false;

    ui.painter().rect_filled(ui.max_rect(), 10.0, BG);
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);

    // ── titlebar ──────────────────────────────────────────────────────────────

    let bar_rect = egui::Rect::from_min_size(
        ui.next_widget_position(), vec2(SW, 12.0));
    let drag = ui.allocate_rect(bar_rect, Sense::drag());
    if drag.dragged() { ctx.send_viewport_cmd(ViewportCommand::StartDrag); }

    let close_center = bar_rect.max - vec2(11.0, 2.0);
    let close_r = ui.allocate_rect(
        egui::Rect::from_center_size(close_center, vec2(14.0, 10.0)),
        Sense::click());
    let close_col = if close_r.hovered() {
        Color32::from_rgb(255, 80, 80)
    } else {
        Color32::from_rgb(220, 50, 50)
    };
    {
        let c   = close_center;
        let tip = c + vec2( 4.0,  0.0);
        let top = c + vec2(-2.5, -3.8);
        let bot = c + vec2(-2.5,  3.8);
        ui.painter().add(egui::Shape::convex_polygon(
            vec![tip, top, bot],
            close_col,
            egui::Stroke::NONE,
        ));
    }
    if close_r.clicked() { close = true; }
    if close_r.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }

    let c = bar_rect.center();
    for x in [-6.0f32, 0.0, 6.0] {
        ui.painter().circle_filled(
            c + vec2(x, 4.0), 1.5, Color32::from_white_alpha(35));
    }

    {
        let p = ui.painter();
        p.circle_filled(bar_rect.min + vec2(10.0, 10.5), 2.0, Color32::from_gray(160));
        p.text(
            bar_rect.min + vec2(17.0, 10.5),
            egui::Align2::LEFT_CENTER,
            "Настройки",
            egui::FontId::proportional(10.5),
            Color32::from_white_alpha(180),
        );
    }

    // ── tab bar ───────────────────────────────────────────────────────────────

    ui.add_space(9.5);
    let tabs = [
        ("Основные",      SettingsTab::General),
        ("Проекты",       SettingsTab::Projects),
        ("Синхронизация", SettingsTab::Sync),
    ];
    let font_id = egui::FontId::proportional(12.0);
    let total_w: f32 = tabs.iter().map(|(label, _)|
        ui.painter().layout_no_wrap(label.to_string(), font_id.clone(), Color32::WHITE).size().x
    ).sum();
    let gap = (SW - total_w) / (tabs.len() as f32 + 1.0);
    ui.horizontal(|ui| {
        ui.add_space(gap);
        for (label, tab) in &tabs {
            let active = state.tab == *tab;
            let color  = if active {
                Color32::from_white_alpha(220)
            } else {
                Color32::from_white_alpha(90)
            };
            let resp = ui.add(
                egui::Label::new(RichText::new(*label).color(color).size(12.0))
                    .sense(Sense::click())
                    .selectable(false),
            );
            if resp.clicked() { state.tab = tab.clone(); }
            if resp.hovered() && !active {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                // Overdraw with brighter color on hover (inactive tabs only —
                // the active tab is already at full brightness).
                ui.painter().text(
                    resp.rect.center(),
                    egui::Align2::CENTER_CENTER,
                    *label,
                    egui::FontId::proportional(12.0),
                    Color32::from_white_alpha(160),
                );
            } else if resp.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            ui.add_space(gap);
        }
    });

    ui.add_space(6.0);
    let y = ui.next_widget_position().y;
    ui.painter().hline(0.0..=SW, y, (0.5, crate::SEP));
    ui.add_space(1.0);

    // ── tab content ───────────────────────────────────────────────────────────

    match state.tab {
        SettingsTab::General  =>
            { crate::ui::settings::general::draw(ui, settings); }
        SettingsTab::Projects =>
            { crate::ui::settings::projects::draw(ui); }
        SettingsTab::Sync     =>
            { crate::ui::settings::sync_panel::draw(ui, &mut state.sync_panel, sync); }
    }

    (close, state.target_height())
}