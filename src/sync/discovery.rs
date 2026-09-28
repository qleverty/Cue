use std::net::UdpSocket;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

use super::DeviceType;

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DiscoveredPeer {
    pub device_id:   String,
    pub device_name: String,
    pub device_type: DeviceType,
    pub ip:          String,
    /// Порт HTTP-сервера этого устройства (из самого discovery-сообщения).
    pub port:        u16,
    seen_at:         u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DiscoveryKind { Ping, Pong }

#[derive(Serialize, Deserialize)]
struct DiscoveryMsg {
    kind:        DiscoveryKind,
    device_id:   String,
    device_name: String,
    device_type: DeviceType,
    /// Порт HTTP-сервера отправителя. Из UDP-пакета его не узнать (там виден
    /// только IP), поэтому передаём явно. У старых клиентов поля нет.
    #[serde(default = "super::server::default_port")]
    port:        u16,
}

pub type DiscoveredList = Arc<Mutex<Vec<DiscoveredPeer>>>;

pub const UDP_PORT: u16      = 52683;
const PEER_TTL:     u64      = 10;
const READ_TIMEOUT: Duration = Duration::from_millis(250);
/// Один UDP-broadcast легко теряется (Wi-Fi power-save, ARP, фаервол в момент
/// первого запуска) — на один запрос шлём серию из нескольких PING-ов.
const PING_BURST:     u8       = 3;
const PING_BURST_GAP: Duration = Duration::from_millis(500);

// ── public API ────────────────────────────────────────────────────────────────

pub struct Discovery {
    pub discovered: DiscoveredList,
    /// None — сокет ещё не забинжен (идёт retry) либо был потерян и не
    /// восстановился; Some(true) — забинжен и слушает нормально.
    pub ready:      Arc<Mutex<bool>>,
    /// Текст последней ошибки bind, для отображения в UI. Сбрасывается в
    /// None сразу после успешного bind.
    pub bind_error: Arc<Mutex<Option<String>>>,
    ping_tx: mpsc::SyncSender<()>,
}

impl Discovery {
    pub fn send_ping(&self) {
        self.ping_tx.try_send(()).ok();
    }

    /// Текст для UI при клике "Найти устройства": None — можно продолжать
    /// как обычно (или показывать "устройства не найдены"), Some(msg) —
    /// показать msg красным вместо результата поиска.
    pub fn error_for_ui(&self) -> Option<String> {
        if *self.ready.lock().unwrap() { None } else { self.bind_error.lock().unwrap().clone() }
    }
}

pub fn start(
    our_device_id:   String,
    our_device_name: Arc<RwLock<String>>,
    local_ip:        Option<String>,
    our_device_type: DeviceType,
    our_http_port:   Arc<AtomicU16>,
) -> Discovery {
    let discovered: DiscoveredList = Arc::new(Mutex::new(Vec::new()));
    let discovered_clone           = Arc::clone(&discovered);
    let (ping_tx, ping_rx)         = mpsc::sync_channel::<()>(1);
    let broadcast                  = subnet_broadcast(local_ip.as_deref());
    let ready:      Arc<Mutex<bool>>           = Arc::new(Mutex::new(false));
    let bind_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let ready_clone      = Arc::clone(&ready);
    let bind_error_clone = Arc::clone(&bind_error);

    std::thread::Builder::new()
        .name("cue-discovery".into())
        .spawn(move || run(
            our_device_id, our_device_name, our_device_type, our_http_port,
            discovered_clone, ping_rx, broadcast, ready_clone, bind_error_clone,
        ))
        .expect("spawn discovery thread");

    Discovery { discovered, ready, bind_error, ping_tx }
}

pub fn current(list: &DiscoveredList) -> Vec<DiscoveredPeer> {
    let now = crate::project::current_time();
    list.lock().unwrap()
        .iter()
        .filter(|p| now.saturating_sub(p.seen_at) <= PEER_TTL)
        .cloned()
        .collect()
}

// ── listener loop ─────────────────────────────────────────────────────────────

fn run(
    our_id:     String,
    our_name:   Arc<RwLock<String>>,
    our_type:   DeviceType,
    our_port:   Arc<AtomicU16>,
    list:       DiscoveredList,
    ping_rx:    mpsc::Receiver<()>,
    broadcast:  String,
    ready:      Arc<Mutex<bool>>,
    bind_error: Arc<Mutex<Option<String>>>,
) {
    const BIND_RETRY_INTERVAL: Duration = Duration::from_secs(15);

    let sock = loop {
        match bind_socket() {
            Some(s) => break s,
            None => {
                *bind_error.lock().unwrap() = Some(format!("Порт {UDP_PORT} занят другим приложением"));
                crate::clog!("[discovery] bind failed, retry in {BIND_RETRY_INTERVAL:?}");
                std::thread::sleep(BIND_RETRY_INTERVAL);
            }
        }
    };
    *ready.lock().unwrap()      = true;
    *bind_error.lock().unwrap() = None;

    let mut buf = [0u8; 512];
    let mut burst_left: u8 = 0;
    let mut next_send      = Instant::now();
    loop {
        if ping_rx.try_recv().is_ok() {
            burst_left = PING_BURST;
            next_send  = Instant::now();
        }
        if burst_left > 0 && Instant::now() >= next_send {
            burst_left -= 1;
            next_send   = Instant::now() + PING_BURST_GAP;
            let msg = DiscoveryMsg {
                kind:        DiscoveryKind::Ping,
                device_id:   our_id.clone(),
                device_name: our_name.read().unwrap().clone(),
                device_type: our_type,
                port:        our_port.load(Ordering::Relaxed),
            };
            if let Ok(bytes) = serde_json::to_vec(&msg) {
                // Broadcast-адрес пересчитываем на каждый PING: наш IP мог
                // смениться после старта. Не получилось определить — берём
                // тот, что посчитали при запуске.
                let bc = super::get_lan_ip()
                    .map(|ip| subnet_broadcast(Some(&ip)))
                    .unwrap_or_else(|| broadcast.clone());
                // subnet_broadcast() считает сеть как /24; в сети с другой
                // маской он промахнётся — поэтому дополнительно шлём и на
                // 255.255.255.255 (ходит через одну "главную" сетевую карту).
                let mut targets = vec![bc];
                if targets[0] != "255.255.255.255" { targets.push("255.255.255.255".to_owned()); }
                for t in targets {
                    match sock.send_to(&bytes, format!("{t}:{UDP_PORT}")) {
                        Ok(_)  => crate::clog!("[discovery] sent PING → {t}"),
                        Err(e) => crate::clog!("[discovery] send_ping to {t} failed: {e}"),
                    }
                }
            }
        }

        let (len, src) = match sock.recv_from(&mut buf) {
            Ok(r)  => r,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                   || e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => {
                crate::clog!("[discovery] recv_from error: {e}");
                continue;
            }
        };

        let msg: DiscoveryMsg = match serde_json::from_slice(&buf[..len]) {
            Ok(m)  => m,
            Err(_) => continue,
        };

        let src_ip = src.ip().to_string();

        match msg.kind {
            DiscoveryKind::Ping => {
                crate::clog!("[discovery] got PING from {} @ {src_ip}", msg.device_id);

                let pong = DiscoveryMsg {
                    kind:        DiscoveryKind::Pong,
                    device_id:   our_id.clone(),
                    device_name: our_name.read().unwrap().clone(),
                    device_type: our_type,
                    port:        our_port.load(Ordering::Relaxed),
                };
                if let Ok(bytes) = serde_json::to_vec(&pong) {
                    let _ = sock.send_to(&bytes, src);
                }

                if msg.device_id != our_id {
                    upsert(&list, msg.device_id, msg.device_name, msg.device_type, src_ip, msg.port);
                }
            }
            DiscoveryKind::Pong => {
                crate::clog!("[discovery] got PONG from {} @ {src_ip}", msg.device_id);

                if msg.device_id != our_id {
                    upsert(&list, msg.device_id, msg.device_name, msg.device_type, src_ip, msg.port);
                }
            }
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn subnet_broadcast(local_ip: Option<&str>) -> String {
    let ip = match local_ip {
        Some(s) => s,
        None    => return "255.255.255.255".to_owned(),
    };
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() == 4 {
        format!("{}.{}.{}.255", parts[0], parts[1], parts[2])
    } else {
        "255.255.255.255".to_owned()
    }
}

fn bind_socket() -> Option<UdpSocket> {
    let addr = format!("0.0.0.0:{UDP_PORT}");
    let sock = UdpSocket::bind(&addr).map_err(|e| {
        crate::clog!("[discovery] bind failed on {addr}: {e}");
    }).ok()?;
    if let Err(e) = sock.set_broadcast(true) {
        crate::clog!("[discovery] set_broadcast failed: {e}");
        return None;
    }
    let _ = sock.set_read_timeout(Some(READ_TIMEOUT));
    Some(sock)
}

fn upsert(list: &DiscoveredList, id: String, name: String, device_type: DeviceType, ip: String, port: u16) {
    let now = crate::project::current_time();
    let mut guard = list.lock().unwrap();
    match guard.iter_mut().find(|p| p.device_id == id) {
        Some(existing) => {
            existing.device_name = name;
            existing.device_type = device_type;
            existing.ip          = ip;
            existing.port        = port;
            existing.seen_at     = now;
        }
        None => guard.push(DiscoveredPeer {
            device_id: id,
            device_name: name,
            device_type,
            ip,
            port,
            seen_at: now,
        }),
    }
}
