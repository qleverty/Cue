use std::net::UdpSocket;
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::Duration;
use serde::{Deserialize, Serialize};

use super::DeviceType;

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DiscoveredPeer {
    pub device_id:   String,
    pub device_name: String,
    pub device_type: DeviceType,
    pub ip:          String,
    seen_at:         u64,
}

/// Wire-формат UDP discovery-пакета — JSON целиком (раньше был самодельный
/// plaintext "CUE_PING id name", в который было некуда безопасно дописать
/// новое поле: имя забирало "всё до конца строки"). Обратную совместимость
/// с этим старым форматом сознательно не сохраняем.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DiscoveryKind { Ping, Pong }

#[derive(Serialize, Deserialize)]
struct DiscoveryMsg {
    kind:        DiscoveryKind,
    device_id:   String,
    device_name: String,
    device_type: DeviceType,
}

pub type DiscoveredList = Arc<Mutex<Vec<DiscoveredPeer>>>;

const UDP_PORT:     u16      = 52683;
const PEER_TTL:     u64      = 10;
const READ_TIMEOUT: Duration = Duration::from_millis(500);

// ── public API ────────────────────────────────────────────────────────────────

pub struct Discovery {
    pub discovered: DiscoveredList,
    /// Send () to trigger a discovery ping broadcast from the listener socket.
    ping_tx: mpsc::SyncSender<()>,
}

impl Discovery {
    pub fn send_ping(&self) {
        self.ping_tx.try_send(()).ok();
    }
}

pub fn start(
    our_device_id:   String,
    our_device_name: Arc<RwLock<String>>,
    local_ip:        Option<String>,
    our_device_type: DeviceType,
) -> Discovery {
    let discovered: DiscoveredList = Arc::new(Mutex::new(Vec::new()));
    let discovered_clone           = Arc::clone(&discovered);
    let (ping_tx, ping_rx)         = mpsc::sync_channel::<()>(1);
    let broadcast                  = subnet_broadcast(local_ip.as_deref());

    std::thread::Builder::new()
        .name("cue-discovery".into())
        .spawn(move || run(our_device_id, our_device_name, our_device_type, discovered_clone, ping_rx, broadcast))
        .expect("spawn discovery thread");

    Discovery { discovered, ping_tx }
}

/// Read the current non-expired discovered peers.
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
    our_id:    String,
    our_name:  Arc<RwLock<String>>,
    our_type:  DeviceType,
    list:      DiscoveredList,
    ping_rx:   mpsc::Receiver<()>,
    broadcast: String,
) {
    let sock = match bind_socket() {
        Some(s) => s,
        None    => return,
    };

    let mut buf = [0u8; 512];
    loop {
        // Check if a ping was requested — send from this socket so replies
        // come back to port 52683 where we're already listening.
        if ping_rx.try_recv().is_ok() {
            let msg = DiscoveryMsg {
                kind:        DiscoveryKind::Ping,
                device_id:   our_id.clone(),
                device_name: our_name.read().unwrap().clone(),
                device_type: our_type,
            };
            if let Ok(bytes) = serde_json::to_vec(&msg) {
                match sock.send_to(&bytes, format!("{broadcast}:{UDP_PORT}")) {
                    Ok(_)  => crate::clog!("[discovery] sent PING"),
                    Err(e) => crate::clog!("[discovery] send_ping failed: {e}"),
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

                // Reply with PONG from this same socket (port 52683).
                let pong = DiscoveryMsg {
                    kind:        DiscoveryKind::Pong,
                    device_id:   our_id.clone(),
                    device_name: our_name.read().unwrap().clone(),
                    device_type: our_type,
                };
                if let Ok(bytes) = serde_json::to_vec(&pong) {
                    let _ = sock.send_to(&bytes, src);
                }

                if msg.device_id != our_id {
                    upsert(&list, msg.device_id, msg.device_name, msg.device_type, src_ip);
                }
            }
            DiscoveryKind::Pong => {
                crate::clog!("[discovery] got PONG from {} @ {src_ip}", msg.device_id);

                if msg.device_id != our_id {
                    upsert(&list, msg.device_id, msg.device_name, msg.device_type, src_ip);
                }
            }
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Derive the subnet broadcast from a local IP, assuming /24.
/// Falls back to 255.255.255.255 if the IP can't be parsed.
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

fn upsert(list: &DiscoveredList, id: String, name: String, device_type: DeviceType, ip: String) {
    let now = crate::project::current_time();
    let mut guard = list.lock().unwrap();
    match guard.iter_mut().find(|p| p.device_id == id) {
        Some(existing) => {
            existing.device_name = name;
            existing.device_type = device_type;
            existing.ip          = ip;
            existing.seen_at     = now;
        }
        None => guard.push(DiscoveredPeer {
            device_id: id,
            device_name: name,
            device_type,
            ip,
            seen_at: now,
        }),
    }
}
