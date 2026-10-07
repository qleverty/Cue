use std::time::Instant;

use eframe::egui::{self, Color32, ImageSource, RichText, Sense, vec2};

use crate::sync::{
    discovery,
    server::PairingRequest,
    peers::PeerEntry,
    PeerStatus,
    SyncHandle,
};

// ── assets ────────────────────────────────────────────────────────────────────

static DESKTOP_PNG: &[u8] = include_bytes!("../../../pics/desktop.png");

const DEVICE_NAME_MAX_CHARS: usize = 32;

const PORT_MIN: u16 = 1024;

const ERROR_RED: Color32 = Color32::from_rgb(220, 50, 50);

const LIST_MAX_ROWS: f32 = 3.0;
const PEER_ROW_H:    f32 = 38.0;
const FOUND_ROW_H:   f32 = 28.0;

// ── state ─────────────────────────────────────────────────────────────────────

pub enum ScanState {
    Idle,
    Scanning { started_at: Instant },
    Results(Vec<discovery::DiscoveredPeer>),
    Empty,
}

impl Default for ScanState {
    fn default() -> Self { Self::Idle }
}

pub struct SyncPanelState {
    pub scan_state:    ScanState,
    device_name_buf:   String,
    name_initialized:  bool,
    port_buf:          String,
    port_initialized:  bool,
    port_dirty:        bool,
    content_bottom:    f32,
}

impl SyncPanelState {
    pub fn window_height(&self) -> f32 {
        if self.content_bottom > 0.0 {
            self.content_bottom.ceil()
        } else {
            crate::settings::SH_SYNC
        }
    }
}

impl Default for SyncPanelState {
    fn default() -> Self {
        Self {
            scan_state:       ScanState::default(),
            device_name_buf:  String::new(),
            name_initialized: false,
            port_buf:         String::new(),
            port_initialized: false,
            port_dirty:       false,
            content_bottom:   0.0,
        }
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

pub fn draw(
    ui:    &mut egui::Ui,
    state: &mut SyncPanelState,
    sync:  &mut SyncHandle,
    settings: &mut crate::settings::Settings,
) -> bool {
    ui.ctx().request_repaint_after(std::time::Duration::from_secs(30));
    if !state.name_initialized {
        state.device_name_buf = sync.identity.device_name.clone();
        state.name_initialized = true;
    }
    if !state.port_initialized {
        state.port_buf = sync.shared.port().to_string();
        state.port_initialized = true;
    }

    let frame = egui::Frame::new()
        .inner_margin(egui::Margin { left: 14, right: 14, top: 14, bottom: 14 })
        .show(ui, |ui| {
            draw_pairing_banner(ui, sync);
            draw_this_device(ui, state, sync, settings);
            sep(ui);
            draw_peers(ui, sync);
            sep(ui);
            draw_discovery(ui, state, sync);
        });
    state.content_bottom = frame.response.rect.bottom() - ui.max_rect().top();

    false
}

// ── sections ──────────────────────────────────────────────────────────────────

fn draw_pairing_banner(ui: &mut egui::Ui, sync: &mut SyncHandle) {
    let pending: Vec<PairingRequest> = sync.shared.pending_pairings.lock().unwrap().clone();
    if pending.iter().any(|r| r.waiting()) {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    }
    let Some(req) = pending.iter().find(|r| !r.waiting()).cloned() else { return; };

    let mut accept = false;
    let mut reject = false;

    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(59, 130, 246, 31))
        .stroke(egui::Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(59, 130, 246, 64),
        ))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("{} хочет подключиться", req.device_name))
                    .size(11.5)
                    .color(Color32::from_white_alpha(166)),
            );
            ui.add_space(7.0);
            ui.horizontal(|ui| {
                if btn(ui, "Принять",   true).clicked()  { accept = true; }
                if btn(ui, "Отклонить", false).clicked() { reject = true; }
            });
        });

    ui.add_space(10.0);

    if accept { accept_pairing(sync, &req); }
    if reject { reject_pairing(sync, &req.device_id); }
}

fn draw_this_device(
    ui: &mut egui::Ui,
    state: &mut SyncPanelState,
    sync: &mut SyncHandle,
    settings: &mut crate::settings::Settings,
) {
    block_title(ui, "Это устройство");

    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
        ui.painter().rect_filled(rect, 6.0, Color32::from_white_alpha(15));
        let icon_size = vec2(16.0, 16.0);
        let icon_rect = egui::Rect::from_center_size(rect.center(), icon_size);
        ui.put(icon_rect, egui::Image::new(ImageSource::Bytes {
            uri: "bytes://desktop.png".into(),
            bytes: DESKTOP_PNG.into(),
        }).fit_to_exact_size(icon_size).tint(Color32::from_white_alpha(200)));
        ui.add_space(10.0);

        ui.vertical(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut state.device_name_buf)
                    .font(egui::FontId::proportional(13.0))
                    .text_color(Color32::from_white_alpha(210))
                    .frame(egui::Frame::NONE)
                    .char_limit(DEVICE_NAME_MAX_CHARS)
                    .desired_width(f32::INFINITY),
            );
            if resp.lost_focus() {
                let trimmed = state.device_name_buf.trim().to_owned();
                if !trimmed.is_empty() {
                    *sync.shared.device_name.write().unwrap() = trimmed.clone();
                    sync.identity.device_name                 = trimmed;
                    sync.identity.save(&crate::app_dir());
                } else {
                    state.device_name_buf = sync.identity.device_name.clone();
                }
            }

            let our_port = sync.shared.port();
            let ip_text: Option<String> = sync.local_ip.as_ref().map(|ip| format!("{ip} ·"));
            ui.horizontal(|ui| {
                if let Some(text) = ip_text {
                    ui.label(
                        RichText::new(text)
                            .size(10.0)
                            .color(Color32::from_white_alpha(90)),
                    );
                    ui.add_space(4.0);
                }
                let failed = sync.shared.server_bind_failed.load(std::sync::atomic::Ordering::Relaxed);
                let port_color = if failed && !state.port_dirty {
                    ERROR_RED
                } else {
                    Color32::from_white_alpha(130)
                };
                let port_resp = ui.add(
                    egui::TextEdit::singleline(&mut state.port_buf)
                        .font(egui::FontId::proportional(10.0))
                        .text_color(port_color)
                        .frame(egui::Frame::NONE)
                        .char_limit(5)
                        .desired_width(34.0),
                );
                if port_resp.changed() { state.port_dirty = true; }
                state.port_buf.retain(|c| c.is_ascii_digit());
                port_resp.clone().on_hover_text("Стандартный: 24684");
                if port_resp.lost_focus() {
                    if let Ok(p) = state.port_buf.parse::<u16>() {
                        if p >= PORT_MIN && p != discovery::UDP_PORT && p != our_port {
                            apply_port(sync, settings, p);
                        }
                    }
                    state.port_buf   = sync.shared.port().to_string();
                    state.port_dirty = false;
                }
            });
        });
    });
}

fn draw_peers(ui: &mut egui::Ui, sync: &mut SyncHandle) {
    block_title(ui, "Подключённые устройства");

    let peers    = sync.shared.peers.read().unwrap().all().to_vec();
    let statuses = sync.shared.sync_status.lock().unwrap().peer_statuses.clone();
    let mut to_remove: Option<String> = None;

    if peers.is_empty() {
        ui.label(
            RichText::new("Нет подключённых устройств")
                .size(11.5)
                .color(Color32::from_white_alpha(90)),
        );
    } else {
        egui::ScrollArea::vertical()
            .id_salt("sync_peers")
            .max_height(PEER_ROW_H * LIST_MAX_ROWS)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for peer in peers.iter() {
                    let status = statuses.get(&peer.device_id).cloned().unwrap_or_default();
                    let (status_color, meta_text) = peer_display(peer, &status);
                    let mut disconnect = false;

                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), PEER_ROW_H),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let dis = ui.add(
                                egui::Label::new(
                                    RichText::new("Отключить")
                                        .size(10.0)
                                        .color(Color32::from_rgba_unmultiplied(255, 80, 80, 115)),
                                )
                                .sense(Sense::click())
                                .selectable(false),
                            );
                            if dis.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                ui.painter().text(
                                    dis.rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "Отключить",
                                    egui::FontId::proportional(10.0),
                                    Color32::from_rgba_unmultiplied(255, 80, 80, 217),
                                );
                            }
                            if dis.clicked() { disconnect = true; }
                            ui.add_space(10.0);

                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.vertical(|ui| {
                                    ui.add_space(5.0);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&peer.device_name)
                                                .size(12.5)
                                                .color(Color32::from_white_alpha(184)),
                                        )
                                        .truncate(),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&meta_text).size(10.0).color(status_color),
                                        )
                                        .truncate(),
                                    );
                                });
                            });
                        },
                    );

                    if disconnect { to_remove = Some(peer.device_id.clone()); }
                }
            });
    }

    if let Some(id) = to_remove {
        sync.shared.peers.write().unwrap().remove(&id);
    }
}

fn draw_discovery(ui: &mut egui::Ui, state: &mut SyncPanelState, sync: &mut SyncHandle) {
    let scanning = matches!(state.scan_state, ScanState::Scanning { .. });

    ui.horizontal(|ui| {
        block_title_inline(ui, "Найти устройства");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if btn(ui, "Сканировать", false).clicked() && !scanning {
                sync.shared.discovered.send_ping();
                state.scan_state = ScanState::Scanning { started_at: Instant::now() };
            }
        });
    });
    ui.add_space(4.0);

    let should_finish = if let ScanState::Scanning { started_at } = &state.scan_state {
        started_at.elapsed().as_secs_f32() >= 1.8
    } else {
        false
    };
    if should_finish {
        let found = crate::sync::discovery::current(&sync.shared.discovered.discovered);
        let paired_ids: std::collections::HashSet<_> = sync.shared.peers
            .read().unwrap()
            .all().iter()
            .map(|p| p.device_id.clone())
            .collect();
        let filtered: Vec<_> = found.into_iter()
            .filter(|p| !paired_ids.contains(&p.device_id))
            .collect();
        state.scan_state = if filtered.is_empty() {
            ScanState::Empty
        } else {
            ScanState::Results(filtered)
        };
    }

    match &state.scan_state {
        ScanState::Idle => {}

        ScanState::Scanning { started_at } => {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
            let dots = 1 + ((started_at.elapsed().as_millis() / 400) % 3) as usize;
            ui.label(
                RichText::new(format!("Поиск устройств{}", ".".repeat(dots)))
                    .size(10.5)
                    .color(Color32::from_white_alpha(90)),
            );
        }

        ScanState::Results(found) => {
            egui::ScrollArea::vertical()
                .id_salt("sync_found")
                .max_height(FOUND_ROW_H * LIST_MAX_ROWS)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for peer in found {
                        ui.allocate_ui_with_layout(
                            vec2(ui.available_width(), FOUND_ROW_H),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                let incoming = sync.shared.pending_pairings.lock().unwrap()
                                    .iter()
                                    .find(|r| r.device_id == peer.device_id && !r.waiting())
                                    .cloned();
                                match incoming {
                                    Some(req) => {
                                        if btn(ui, "Принять", true).clicked() {
                                            accept_pairing(sync, &req);
                                        }
                                    }
                                    None => {
                                        if btn(ui, "Подключить", true).clicked() {
                                            send_pairing_request(sync, peer);
                                        }
                                    }
                                }
                                ui.add_space(10.0);
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&peer.device_name)
                                                .size(12.5)
                                                .color(Color32::from_white_alpha(153)),
                                        )
                                        .truncate(),
                                    );
                                });
                            },
                        );
                    }
                });
        }

        ScanState::Empty => {
            if let Some(err) = sync.shared.discovered.error_for_ui() {
                ui.label(
                    RichText::new(err)
                        .size(10.5)
                        .color(ERROR_RED),
                );
            } else {
                ui.label(
                    RichText::new("Устройств не найдено")
                        .size(10.5)
                        .color(Color32::from_white_alpha(90)),
                );
            }
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn peer_display(peer: &PeerEntry, status: &PeerStatus) -> (Color32, String) {
    if status.revoked {
        return (ERROR_RED, "Отвязано".to_owned());
    }
    if status.incompatible {
        return (Color32::from_rgb(217, 119, 6), "Несовместимо".to_owned());
    }
    if status.online {
        let text = match peer.last_synced_at {
            Some(ts) => format!("Онлайн ({})", format_ago(ts)),
            None     => "Онлайн".to_owned(),
        };
        return (Color32::from_rgb(34, 197, 94), text);
    }
    let text = match peer.last_synced_at {
        Some(ts) => format!("Оффлайн ({})", format_ago(ts)),
        None     => "Ожидание…".to_owned(),
    };
    (Color32::from_white_alpha(90), text)
}

fn format_ago(ts: u64) -> String {
    ago_text(crate::project::current_time().saturating_sub(ts))
}

fn ago_text(diff: u64) -> String {
    if diff < 60 {
        return "только что".to_owned();
    }
    let (d, h, m) = (diff / 86_400, diff % 86_400 / 3_600, diff % 3_600 / 60);
    let mut parts: Vec<String> = Vec::with_capacity(3);
    if d > 0 { parts.push(format!("{d}д")); }
    if h > 0 { parts.push(format!("{h}ч")); }
    if m > 0 { parts.push(format!("{m}м")); }
    format!("{} назад", parts.join(", "))
}

fn sep(ui: &mut egui::Ui) {
    ui.add_space(12.0);
    let y  = ui.next_widget_position().y;
    let x0 = ui.next_widget_position().x;
    let x1 = x0 + ui.available_width();
    ui.painter().hline(x0..=x1, y, (0.5, crate::SEP));
    ui.add_space(12.0);
}

fn block_title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(10.0)
            .color(Color32::from_white_alpha(90)),
    );
    ui.add_space(6.0);
}

fn block_title_inline(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(10.0)
            .color(Color32::from_white_alpha(90)),
    );
}

fn btn(ui: &mut egui::Ui, text: &str, primary: bool) -> egui::Response {
    const BLUE:       Color32 = Color32::from_rgb(74, 144, 217);
    const BLUE_HOVER: Color32 = Color32::from_rgb(154, 199, 247);

    let (color, hover_color) = if primary {
        (BLUE, BLUE_HOVER)
    } else {
        (Color32::from_white_alpha(90), Color32::from_white_alpha(160))
    };

    let resp = ui.add(
        egui::Label::new(RichText::new(text).color(color).size(10.5))
            .sense(Sense::click())
            .selectable(false),
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().text(
            resp.rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(10.5),
            hover_color,
        );
    }
    resp
}

fn send_pairing_request(sync: &mut SyncHandle, peer: &discovery::DiscoveredPeer) {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let our_id   = sync.shared.device_id.clone();
    let our_name = sync.shared.device_name.read().unwrap().clone();
    sync.shared.pending_outgoing.lock().unwrap()
        .insert(peer.device_id.clone(), std::time::Instant::now());

    let ip      = peer.ip.clone();
    let peer_id = peer.device_id.clone();
    let port     = peer.port;
    let our_port = sync.shared.port();
    std::thread::spawn(move || {
        let addr = format!("{ip}:{port}");
        let body = serde_json::json!({
            "device_id":   our_id,
            "device_name": our_name,
            "device_type": "desktop",
            "port":        our_port,
        }).to_string();
        let req = format!(
            "POST /1/request_sync HTTP/1.0\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        );
        match TcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap()),
            Duration::from_secs(5),
        ) {
            Ok(mut stream) => {
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = stream.write_all(req.as_bytes());
                let mut buf = [0u8; 64];
                let _ = stream.read(&mut buf);
                crate::clog!("[sync_panel] request_sync → {peer_id} ok");
            }
            Err(e) => crate::clog!("[sync_panel] request_sync → {peer_id} FAILED: {e}"),
        }
    });

    sync.shared.ping_tx.try_send(()).ok();
}

fn accept_pairing(sync: &mut SyncHandle, req: &PairingRequest) {
    crate::sync::server::accept_pairing(&sync.shared, req);
}

fn reject_pairing(sync: &mut SyncHandle, device_id: &str) {
    let mut pending = sync.shared.pending_pairings.lock().unwrap();
    pending.retain(|r| r.device_id != device_id);
    crate::sync::server::save_pending_pairings(
        &crate::app_dir(),
        &pending,
    );
}

fn apply_port(sync: &mut SyncHandle, settings: &mut crate::settings::Settings, port: u16) {
    settings.http_port = port;
    settings.save();
    sync.shared.set_port(port);
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ago_text_formats() {
        assert_eq!(ago_text(0), "только что");
        assert_eq!(ago_text(59), "только что");
        assert_eq!(ago_text(60), "1м назад");
        assert_eq!(ago_text(59 * 60 + 59), "59м назад");
        assert_eq!(ago_text(3_600), "1ч назад");
        assert_eq!(ago_text(3_600 + 60), "1ч, 1м назад");
        assert_eq!(ago_text(2 * 3_600 + 5 * 60), "2ч, 5м назад");
        assert_eq!(ago_text(86_400), "1д назад");
        assert_eq!(ago_text(86_400 + 300), "1д, 5м назад");
        assert_eq!(ago_text(86_400 + 2 * 3_600 + 3 * 60), "1д, 2ч, 3м назад");
        assert_eq!(ago_text(120 * 86_400), "120д назад");
        assert_eq!(ago_text(12 * 86_400 + 23 * 3_600 + 59 * 60 + 59), "12д, 23ч, 59м назад");
    }

    fn peer(ts: Option<u64>) -> PeerEntry {
        PeerEntry {
            device_id: "a".into(), device_name: "n".into(), token: "t".into(),
            ip_hint: None, port: 1, last_synced_at: ts, revoked: false,
            incompatible: false, device_type: Default::default(),
        }
    }
    fn st(online: bool, revoked: bool, incompatible: bool) -> PeerStatus {
        PeerStatus { online, error: false, revoked, incompatible }
    }

    #[test]
    fn display_priority_and_texts() {
        let now = crate::project::current_time();
        assert_eq!(peer_display(&peer(Some(now)), &st(true, true, true)).1, "Отвязано");
        assert_eq!(peer_display(&peer(Some(now)), &st(false, false, true)).1, "Несовместимо");
        assert_eq!(peer_display(&peer(Some(now)), &st(true, false, false)).1, "Онлайн (только что)");
        assert_eq!(peer_display(&peer(Some(now - 180)), &st(false, false, false)).1, "Оффлайн (3м назад)");
        assert_eq!(peer_display(&peer(None), &st(false, false, false)).1, "Ожидание…");
        assert_eq!(peer_display(&peer(None), &st(true, false, false)).1, "Онлайн");
    }
}
