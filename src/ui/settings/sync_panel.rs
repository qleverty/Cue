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

/// Ограничение на длину имени устройства (в символах — char_limit у egui
/// считает именно символы, не байты, так что кириллица не режется криво).
const DEVICE_NAME_MAX_CHARS: usize = 32;

/// Допустимый диапазон порта HTTP-сервера синка. Ниже 1024 — привилегированные
/// порты; порт discovery (UDP) занимать нельзя.
const PORT_MIN: u16 = 1024;

/// Красный ошибок — тот же, что у кнопки закрытия окна (main.rs, settings.rs).
const ERROR_RED: Color32 = Color32::from_rgb(220, 50, 50);

/// Списки (подключённые / найденные) показывают до стольких строк, дальше —
/// скролл. Высота строк фиксированная, чтобы "ровно три" было ровно тремя.
const LIST_MAX_ROWS: f32 = 3.0;
const PEER_ROW_H:    f32 = 38.0;
const FOUND_ROW_H:   f32 = 28.0;

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
    /// Редактируемое поле порта (только цифры).
    port_buf:          String,
    port_initialized:  bool,
    /// Поле порта правили с тех пор, как оно последний раз "применялось":
    /// красный цвет ошибки при этом гаснет сразу, не дожидаясь Enter.
    port_dirty:        bool,
    /// Низ содержимого вкладки относительно верха окна (для авто-высоты).
    content_bottom:    f32,
}

impl SyncPanelState {
    /// Высота окна для вкладки "Синхронизация": ровно по содержимому.
    pub fn window_height(&self) -> f32 {
        if self.content_bottom > 0.0 {
            self.content_bottom.ceil()
        } else {
            crate::settings::SH_SYNC   // первый кадр, содержимое ещё не измерено
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
    // Окно подгоняется под содержимое: низ рамки (вместе с нижним отступом)
    // и есть нужная высота. Применяет её SettingsUiState::target_height.
    state.content_bottom = frame.response.rect.bottom() - ui.max_rect().top();

    false
}

// ── sections ──────────────────────────────────────────────────────────────────

fn draw_pairing_banner(ui: &mut egui::Ui, sync: &mut SyncHandle) {
    let pending: Vec<PairingRequest> = sync.shared.pending_pairings.lock().unwrap().clone();
    // Заявки во взаимном ожидании (см. server::request_sync) баннером не
    // показываем — ждём accept_sync от второй стороны. Когда окно ожидания
    // истечёт, заявка станет обычной, поэтому просим перерисовку заранее.
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
                    // Revert to saved name if the field was cleared.
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
                // Порт занят другим приложением — сам текст порта краснеет.
                // Как только начинаешь править поле — снова обычный цвет.
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
                    // Показываем реально действующее значение (при неверном вводе — откат).
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
        // До трёх устройств список просто растёт (окно подстраивается под
        // него), дальше — скролл.
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
                            // Зазор между именем и "Отключить".
                            ui.add_space(10.0);

                            // Имя и статус занимают всё оставшееся место и
                            // обрезаются многоточием, а не переносятся (иначе
                            // строка перестала бы быть фиксированной высоты).
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.vertical(|ui| {
                                    // (38 − ~27 высоты двух строк) / 2 — центрируем блок.
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
            // Обычная подпись ровно того же вида и на том же месте, что и
            // "Устройств не найдено"; меняется только число точек.
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
                                // Если это устройство уже прислало НАМ заявку — вместо
                                // встречного "Подключить" (породил бы взаимную заявку)
                                // предлагаем принять существующую.
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
                                ui.add_space(10.0);   // зазор между именем и кнопкой
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

/// Три видимых состояния (цвет самого статус-текста — точку убрали):
/// - revoked (403 от пира)            → красный, "Отвязано"
/// - online (последний пул успешен)   → зелёный, время последнего синка
/// - всё остальное (offline/error/ещё не опрашивали) → серый, "Оффлайн" или
///   "Ожидание…" (только для свежепривязанного устройства, синка ещё не
///   было — единственный случай, когда текст в сером состоянии отличается).
/// status.error намеренно не проверяем отдельно — свежий обрыв соединения
/// визуально неотличим от простого оффлайна, пользователю эта разница не
/// нужна (см. обсуждение).
fn peer_display(peer: &PeerEntry, status: &PeerStatus) -> (Color32, String) {
    if status.revoked {
        return (ERROR_RED, "Отвязано".to_owned());
    }
    if status.incompatible {
        // Мы реально достучались до пира (/hello ответил) — просто не
        // понимаем формат его /ops. Отдельный от "Оффлайн"/"Отвязано" цвет
        // и текст: причина не связь, а версия протокола. Перепроверяется
        // каждый цикл сама — никакого "навсегда заблокирован" тут нет.
        return (Color32::from_rgb(217, 119, 6), "Несовместимо".to_owned());
    }
    if status.online {
        // "online, но ни разу не синхронизировались" физически недостижимо —
        // engine.rs выставляет online и last_synced_at всегда вместе — но
        // .unwrap_or_else оставляем как защитный fallback, а не unwrap().
        let text = peer.last_synced_at
            .map(format_ago)
            .unwrap_or_else(|| "ожидание…".to_owned());
        return (Color32::from_rgb(34, 197, 94), text);
    }
    let text = match peer.last_synced_at {
        Some(_) => "Оффлайн".to_owned(),
        None    => "Ожидание…".to_owned(),
    };
    (Color32::from_white_alpha(90), text)
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
        RichText::new(text)
            .size(10.0)
            .color(Color32::from_white_alpha(90)),
    );
    ui.add_space(6.0);
}

/// Variant of `block_title` without bottom spacing — used in horizontal rows.
fn block_title_inline(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(10.0)
            .color(Color32::from_white_alpha(90)),
    );
}

/// Текстовая "кнопка" в стиле вкладок настроек (settings.rs::draw_settings_ui) —
/// без заливки и рамки вообще, только яркость текста меняется по наведению.
/// primary — акцентные действия (Подключить/Принять) синим, а не белым.
fn btn(ui: &mut egui::Ui, text: &str, primary: bool) -> egui::Response {
    const BLUE:       Color32 = Color32::from_rgb(74, 144, 217);   // #4A90D9
    const BLUE_HOVER: Color32 = Color32::from_rgb(154, 199, 247);  // светлее — подсветка при наведении

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
        // Перерисовываем текст ярче поверх — тот же приём, что у вкладок настроек.
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

/// Отправляет пиру заявку на пейринг (POST /1/request_sync). Никакого секрета
/// в заявке нет — настоящий токен генерирует принимающая сторона при "Принять"
/// и присылает его нам отдельно в /1/accept_sync (см. server::accept_pairing).
/// Мы запоминаем, что отправили заявку: если пир одновременно отправит такую
/// же нам, request_sync распознает взаимность и разрулит её тай-брейком.
fn send_pairing_request(sync: &mut SyncHandle, peer: &discovery::DiscoveredPeer) {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let our_id   = sync.shared.device_id.clone();
    let our_name = sync.shared.device_name.read().unwrap().clone();
    sync.shared.pending_outgoing.lock().unwrap()
        .insert(peer.device_id.clone(), std::time::Instant::now());

    // POST /request_sync to the peer in a background thread.
    // NOTE: we do NOT add to trusted_peers yet — only after the peer accepts
    // and we receive /accept_sync will we register them.
    let ip      = peer.ip.clone();
    let peer_id = peer.device_id.clone();
    let port     = peer.port;                // порт СЕРВЕРА пира (из его discovery)
    let our_port = sync.shared.port();       // наш — чтобы он записал нас верно
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

    // Wake engine for immediate pull attempt.
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

/// Сменить порт HTTP-сервера на ходу: сохраняем в настройки и передаём
/// серверному потоку — он сам перебиндится, а discovery/engine уже читают порт
/// из общего атомика. Если новый порт занят — уведомление и красная пометка в
/// UI появятся так же, как при занятом порте на старте.
fn apply_port(sync: &mut SyncHandle, settings: &mut crate::settings::Settings, port: u16) {
    settings.http_port = port;
    settings.save();
    sync.shared.set_port(port);
}
