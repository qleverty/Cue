use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DiscoveredPeer {
    pub device_id:   String,
    pub device_name: String,
    pub ip:          String,
    /// Unix timestamp of last received PONG — used for TTL eviction.
    seen_at:         u64,
}

pub type DiscoveredList = Arc<Mutex<Vec<DiscoveredPeer>>>;

const UDP_PORT:     u16      = 52683;
const PEER_TTL:     u64      = 10;     // seconds
const READ_TIMEOUT: Duration = Duration::from_secs(2);

// ── public API ────────────────────────────────────────────────────────────────

/// Spawn the listener thread. Returns the shared discovered-peer list.
pub fn start(
    our_device_id:   String,
    our_device_name: Arc<std::sync::RwLock<String>>,
) -> DiscoveredList {
    let discovered: DiscoveredList = Arc::new(Mutex::new(Vec::new()));
    let discovered_clone = Arc::clone(&discovered);

    std::thread::Builder::new()
        .name("cue-discovery".into())
        .spawn(move || run(our_device_id, our_device_name, discovered_clone))
        .expect("spawn discovery thread");

    discovered
}

/// Broadcast a `CUE_PING` on the LAN so other devices respond with `CUE_PONG`.
pub fn send_ping(device_id: &str, device_name: &str) {
    match UdpSocket::bind("0.0.0.0:0") {
        Ok(sock) => {
            if sock.set_broadcast(true).is_err() {
                crate::clog!("[discovery] set_broadcast failed");
                return;
            }
            let msg = format!("CUE_PING {} {}", device_id, device_name);
            match sock.send_to(msg.as_bytes(), format!("255.255.255.255:{UDP_PORT}")) {
                Ok(_)  => crate::clog!("[discovery] sent CUE_PING"),
                Err(e) => crate::clog!("[discovery] send_ping failed: {e}"),
            }
        }
        Err(e) => crate::clog!("[discovery] bind for ping failed: {e}"),
    }
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
    our_id:   String,
    our_name: Arc<std::sync::RwLock<String>>,
    list:     DiscoveredList,
) {
    let sock = match bind_socket() {
        Some(s) => s,
        None    => return,
    };

    let mut buf = [0u8; 512];
    loop {
        let (len, src) = match sock.recv_from(&mut buf) {
            Ok(r)  => r,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                   || e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => {
                crate::clog!("[discovery] recv_from error: {e}");
                continue;
            }
        };

        let msg = match std::str::from_utf8(&buf[..len]) {
            Ok(s)  => s.trim(),
            Err(_) => continue,
        };

        let src_ip = src.ip().to_string();

        if let Some(rest) = msg.strip_prefix("CUE_PING ") {
            let (peer_id, peer_name) = match split_id_name(rest) {
                Some(v) => v,
                None    => continue,
            };
            crate::clog!("[discovery] got CUE_PING from {peer_id} @ {src_ip}");

            // Reply with our PONG directly to the sender.
            let our_name_str = our_name.read().unwrap().clone();
            let pong = format!("CUE_PONG {our_id} {our_name_str}");
            let _ = sock.send_to(pong.as_bytes(), src);

            // Also register them as discovered (they're alive).
            if peer_id != our_id {
                upsert(&list, peer_id, peer_name, src_ip);
            }

        } else if let Some(rest) = msg.strip_prefix("CUE_PONG ") {
            let (peer_id, peer_name) = match split_id_name(rest) {
                Some(v) => v,
                None    => continue,
            };
            crate::clog!("[discovery] got CUE_PONG from {peer_id} @ {src_ip}");

            if peer_id != our_id {
                upsert(&list, peer_id, peer_name, src_ip);
            }
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn bind_socket() -> Option<UdpSocket> {
    let addr = format!("0.0.0.0:{UDP_PORT}");
    let sock = UdpSocket::bind(&addr).map_err(|e| {
        crate::clog!("[discovery] bind failed on {addr}: {e}");
    }).ok()?;

    if let Err(e) = sock.set_broadcast(true) {
        crate::clog!("[discovery] set_broadcast failed: {e}");
        return None;
    }
    // SO_REUSEADDR is set by default on most platforms via std.
    let _ = sock.set_read_timeout(Some(READ_TIMEOUT));
    Some(sock)
}

/// Parse `"<device_id> <device_name with spaces>"` — id is the first token.
fn split_id_name(s: &str) -> Option<(String, String)> {
    let mut parts = s.splitn(2, ' ');
    let id   = parts.next()?.trim().to_owned();
    let name = parts.next().unwrap_or("Unknown").trim().to_owned();
    if id.is_empty() { return None; }
    Some((id, name))
}

fn upsert(list: &DiscoveredList, id: String, name: String, ip: String) {
    let now  = crate::project::current_time();
    let mut guard = list.lock().unwrap();
    match guard.iter_mut().find(|p| p.device_id == id) {
        Some(existing) => {
            existing.device_name = name;
            existing.ip          = ip;
            existing.seen_at     = now;
        }
        None => guard.push(DiscoveredPeer {
            device_id:   id,
            device_name: name,
            ip,
            seen_at:     now,
        }),
    }
}