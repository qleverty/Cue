use std::time::Instant;

use eframe::egui::{self, Color32, RichText, Sense, vec2};

use crate::sync::{
    discovery,
    server::{PairingRequest, PORT},
    peers::PeerEntry,
    PeerStatus,
    SyncHandle,
};

// ── state ─────────────────────────────────────────────────────────────────────

pub enum ScanState {
    Idle,
    /// Active scan; `started_at` drives the dot animation.
    Scanning { started_at: Instant },
    Results(Vec<discovery::DiscoveredPeer>),
    Empty,
}

impl Default for ScanState {
    fn default() -> Self { Self::Idle }
}

pub struct SyncPanelState {
    pub scan_state:    ScanState,
    /// Mirrors `sync.identity.device_name` for the editable name field.
    device_name_buf:   String,
    name_initialized:  bool,
}

impl Default for SyncPanelState {
    fn default() -> Self {
        Self {
            scan_state:       ScanState::default(),
            device_name_buf:  String::new(),
            name_initialized: false,
        }
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

pub fn draw(
    ui:    &mut egui::Ui,
    state: &mut SyncPanelState,
    sync:  &mut SyncHandle,
) -> bool {
    if !state.name_initialized {
        state.device_name_buf = sync.identity.device_name.clone();
        state.name_initialized = true;
    }

    egui::Frame::new()
        .inner_margin(egui::Margin { left: 14, right: 14, top: 14, bottom: 4 })
        .show(ui, |ui| {
            draw_pairing_banner(ui, sync);
            draw_this_device(ui, state, sync);
            sep(ui);
            draw_peers(ui, sync);
            sep(ui);
            draw_discovery(ui, state, sync);
            sep(ui);
            draw_sync_now(ui, sync);
        });

    false
}

// ── sections ──────────────────────────────────────────────────────────────────

fn draw_pairing_banner(ui: &mut egui::Ui, sync: &mut SyncHandle) {
    let pending: Vec<PairingRequest> = sync.shared.pending_pairings.lock().unwrap().clone();
    let Some(req) = pending.first().cloned() else { return; };

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

fn draw_this_device(ui: &mut egui::Ui, state: &mut SyncPanelState, sync: &mut SyncHandle) {
    block_title(ui, "Это устройство");

    ui.horizontal(|ui| {
        // Icon placeholder — replaced with a PNG in a later step.
        let (rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
        ui.painter().rect_filled(rect, 6.0, Color32::from_white_alpha(15));
        ui.add_space(10.0);

        ui.vertical(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut state.device_name_buf)
                    .font(egui::FontId::proportional(13.0))
                    .text_color(Color32::from_white_alpha(210))
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
            );
            if resp.lost_focus() {
                let trimmed = state.device_name_buf.trim().to_owned();
                if !trimmed.is_empty() {
                    *sync.shared.device_name.write().unwrap() = trimmed.clone();
                    sync.identity.device_name                 = trimmed;
                    sync.identity.save(&crate::app_dir());
                } else {
                    // Revert to saved name if the field was cleared.
                    state.device_name_buf = sync.identity.device_name.clone();
                }
            }

            let ip_text = match &sync.local_ip {
                Some(ip) => format!("{ip} · порт {PORT}"),
                None     => format!("порт {PORT}"),
            };
            ui.label(
                RichText::new(ip_text)
                    .size(10.0)
                    .color(Color32::from_white_alpha(72)),
            );
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
                .color(Color32::from_white_alpha(51)),
        );
    } else {
        for (i, peer) in peers.iter().enumerate() {
            let status = statuses.get(&peer.device_id).cloned().unwrap_or_default();
            let (dot_color, meta_text, meta_err) = peer_display(peer, &status);
            let is_last = i == peers.len() - 1;
            let mut disconnect = false;

            ui.horizontal(|ui| {
                // Status dot
                let dot_pos = ui.next_widget_position() + vec2(3.0, 11.0);
                ui.allocate_exact_size(vec2(8.0, 22.0), Sense::hover());
                ui.painter().circle_filled(dot_pos, 3.0, dot_color);
                ui.add_space(4.0);

                ui.vertical(|ui| {
                    ui.add_space(3.0);
                    ui.label(
                        RichText::new(&peer.device_name)
                            .size(12.5)
                            .color(Color32::from_white_alpha(184)),
                    );
                    let meta_color = if meta_err {
                        Color32::from_rgba_unmultiplied(220, 60, 60, 180)
                    } else {
                        Color32::from_white_alpha(72)
                    };
                    ui.label(RichText::new(&meta_text).size(10.0).color(meta_color));
                    ui.add_space(3.0);
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                        // Overdraw with brighter color on hover.
                        ui.painter().text(
                            dis.rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Отключить",
                            egui::FontId::proportional(10.0),
                            Color32::from_rgba_unmultiplied(255, 80, 80, 217),
                        );
                    }
                    if dis.clicked() { disconnect = true; }
                });
            });

            if disconnect { to_remove = Some(peer.device_id.clone()); }

            if !is_last {
                let y = ui.next_widget_position().y;
                ui.painter().hline(0.0..=crate::settings::SW, y, (0.5, crate::SEP));
            }
        }
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

    // Evaluate transition before borrowing scan_state for drawing.
    let should_finish = if let ScanState::Scanning { started_at } = &state.scan_state {
        started_at.elapsed().as_secs_f32() >= 1.8
    } else {
        false
    };
    if should_finish {
        let found = crate::sync::discovery::current(&sync.shared.discovered.discovered);
        // Filter out already-paired peers.
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
            ui.ctx().request_repaint();
            let elapsed = started_at.elapsed().as_secs_f32();
            let pulse   = (elapsed * std::f32::consts::TAU).sin() * 0.5 + 0.5;
            let alpha   = (64.0 + pulse * 128.0) as u8;
            ui.horizontal(|ui| {
                let dot_pos = ui.next_widget_position() + vec2(3.0, 7.0);
                ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
                ui.painter().circle_filled(
                    dot_pos, 2.5,
                    Color32::from_rgba_unmultiplied(59, 130, 246, alpha),
                );
                ui.label(
                    RichText::new("Поиск устройств...")
                        .size(10.5)
                        .color(Color32::from_white_alpha(64)),
                );
            });
        }

        ScanState::Results(found) => {
            for peer in found {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&peer.device_name)
                            .size(12.5)
                            .color(Color32::from_white_alpha(153)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if btn(ui, "Подключить", true).clicked() {
                            send_pairing_request(sync, peer);
                        }
                    });
                });
            }
        }

        ScanState::Empty => {
            ui.label(
                RichText::new("Устройств не найдено")
                    .size(10.5)
                    .color(Color32::from_white_alpha(64)),
            );
        }
    }
}

fn draw_sync_now(ui: &mut egui::Ui, sync: &mut SyncHandle) {
    let last_sync = sync.shared.peers.read().unwrap()
        .all().iter()
        .filter_map(|p| p.last_synced_at)
        .max();

    let time_text = match last_sync {
        Some(ts) => format!("Синхронизирован {}", format_ago(ts)),
        None     => "Нет данных о синхронизации".to_owned(),
    };

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(time_text)
                .size(10.5)
                .color(Color32::from_white_alpha(64)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if btn(ui, "Синхронизировать", false).clicked() {
                sync.shared.ping_tx.try_send(()).ok();
            }
        });
    });
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn peer_display(peer: &PeerEntry, status: &PeerStatus) -> (Color32, String, bool) {
    if status.revoked {
        return (Color32::from_rgb(200, 45, 45), "Отключён с той стороны".to_owned(), true);
    }
    if status.error {
        return (Color32::from_white_alpha(46), "Недоступен".to_owned(), true);
    }
    let meta = peer.last_synced_at
        .map(format_ago)
        .unwrap_or_else(|| "ожидание…".to_owned());
    if status.online {
        (Color32::from_rgb(34, 197, 94), meta, false)
    } else {
        (Color32::from_white_alpha(46), meta, false)
    }
}

fn format_ago(ts: u64) -> String {
    let now  = crate::project::current_time();
    let diff = now.saturating_sub(ts);
    match diff {
        0..=59       => "только что".to_owned(),
        60..=3599    => format!("{} мин назад",  diff / 60),
        3600..=86399 => format!("{} ч назад",    diff / 3600),
        _            => format!("{} дн назад",   diff / 86400),
    }
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
        RichText::new(text.to_uppercase())
            .size(10.0)
            .color(Color32::from_white_alpha(64)),
    );
    ui.add_space(6.0);
}

/// Variant of `block_title` without bottom spacing — used in horizontal rows.
fn block_title_inline(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(10.0)
            .color(Color32::from_white_alpha(64)),
    );
}

fn btn(ui: &mut egui::Ui, text: &str, primary: bool) -> egui::Response {
    let (fill, text_color) = if primary {
        (
            Color32::from_rgba_unmultiplied(59, 130, 246, 64),
            Color32::from_white_alpha(166),
        )
    } else {
        (
            Color32::from_white_alpha(18),
            Color32::from_white_alpha(115),
        )
    };
    ui.add(
        egui::Button::new(RichText::new(text).size(10.5).color(text_color))
            .fill(fill)
            .stroke(egui::Stroke::NONE)
            .corner_radius(4.0),
    )
}

/// Token is derived deterministically from both device IDs so both sides
/// independently compute the same value — avoids conflicts if both initiate.
fn deterministic_token(id_a: &str, id_b: &str) -> String {
    let (lo, hi) = if id_a < id_b { (id_a, id_b) } else { (id_b, id_a) };
    format!("{lo}:{hi}")
}

fn send_pairing_request(sync: &mut SyncHandle, peer: &discovery::DiscoveredPeer) {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let our_id   = sync.shared.device_id.clone();
    let token    = deterministic_token(&our_id, &peer.device_id);
    let our_name = sync.shared.device_name.read().unwrap().clone();

    // POST /request_sync to the peer in a background thread.
    // NOTE: we do NOT add to trusted_peers yet — only after the peer accepts
    // and we receive /accept_sync will we register them.
    let ip      = peer.ip.clone();
    let peer_id = peer.device_id.clone();
    let port    = crate::sync::server::PORT;
    std::thread::spawn(move || {
        let addr = format!("{ip}:{port}");
        let body = serde_json::json!({
            "device_id":   our_id,
            "device_name": our_name,
            "token":       token,
        }).to_string();
        let req = format!(
            "POST /1/request_sync HTTP/1.0
Host: {addr}
Content-Type: application/json
Content-Length: {}
Connection: close

{}",
            body.len(), body
        );
        match TcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap()),
            Duration::from_secs(5),
        ) {
            Ok(mut stream) => {
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                let _ = stream.write_all(req.as_bytes());
                let mut buf = [0u8; 64];
                let _ = stream.read(&mut buf);
                crate::clog!("[sync_panel] request_sync → {peer_id} ok");
            }
            Err(e) => crate::clog!("[sync_panel] request_sync → {peer_id} FAILED: {e}"),
        }
    });

    // Wake engine for immediate pull attempt.
    sync.shared.ping_tx.try_send(()).ok();
}

fn accept_pairing(sync: &mut SyncHandle, req: &PairingRequest) {
    // Add the initiator to our trusted peers.
    let entry = PeerEntry {
        device_id:      req.device_id.clone(),
        device_name:    req.device_name.clone(),
        token:          req.token.clone(),
        ip_hint:        Some(req.from_ip.clone()),
        last_synced_at: None,
    };
    sync.shared.peers.write().unwrap().add(entry);
    reject_pairing(sync, &req.device_id);
    sync.shared.ping_tx.try_send(()).ok();

    // Notify the initiator so they can add us to their trusted_peers.
    let ip       = req.from_ip.clone();
    let our_id   = sync.shared.device_id.clone();
    let our_name = sync.shared.device_name.read().unwrap().clone();
    let token    = req.token.clone();
    let port     = crate::sync::server::PORT;
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        use std::time::Duration;
        let addr = format!("{ip}:{port}");
        let body = serde_json::json!({
            "device_id":   our_id,
            "device_name": our_name,
            "token":       token,
            "from_ip":     ip,
        }).to_string();
        let http = format!(
            "POST /1/accept_sync HTTP/1.0
Host: {addr}
Content-Type: application/json
Content-Length: {}
Connection: close

{}",
            body.len(), body
        );
        match TcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap()),
            Duration::from_secs(5),
        ) {
            Ok(mut s) => {
                let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
                let _ = s.write_all(http.as_bytes());
                let mut buf = [0u8; 64];
                let _ = s.read(&mut buf);
                crate::clog!("[sync_panel] accept_sync sent to {ip} ok");
            }
            Err(e) => crate::clog!("[sync_panel] accept_sync to {ip} FAILED: {e}"),
        }
    });
}

fn reject_pairing(sync: &mut SyncHandle, device_id: &str) {
    let mut pending = sync.shared.pending_pairings.lock().unwrap();
    pending.retain(|r| r.device_id != device_id);
    crate::sync::server::save_pending_pairings(
        &crate::app_dir(),
        &pending,
    );
}