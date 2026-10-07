use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{
    cursors::Cursors,
    oplog::Op,
    peers::{DeviceType, PollUpdate},
    server::{SharedState, PROTO_VER},
};

const SYNC_INTERVAL: Duration = Duration::from_secs(30);
const HTTP_TIMEOUT:  Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);


enum PullError { Revoked, Unavailable }

pub fn start(
    state:   Arc<SharedState>,
    cursors: Arc<Mutex<Cursors>>,
    ops_tx:  mpsc::Sender<Vec<Op>>,
    ping_rx: mpsc::Receiver<()>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("cue-daemon-engine".into())
        .spawn(move || run(state, cursors, ops_tx, ping_rx))
        .expect("spawn daemon engine thread")
}

fn run(
    state:   Arc<SharedState>,
    cursors: Arc<Mutex<Cursors>>,
    ops_tx:  mpsc::Sender<Vec<Op>>,
    ping_rx: mpsc::Receiver<()>,
) {
    if !crate::cue_liveness::cue_is_running() {
        if pull_all(&state, &cursors, &ops_tx).is_err() { return; }
    }
    loop {
        match ping_rx.recv_timeout(SYNC_INTERVAL) {
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {
                if crate::cue_liveness::cue_is_running() { continue; }
                if pull_all(&state, &cursors, &ops_tx).is_err() { break; }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn mark_incompatible(state: &SharedState, id: &str, now_ts: u64) {
    state.peers.write().unwrap().apply_poll(id, PollUpdate {
        contact_ts: Some(now_ts), revoked: Some(false), incompatible: Some(true),
    });
}

fn mark_revoked(state: &SharedState, id: &str, now_ts: u64) {
    state.peers.write().unwrap().apply_poll(id, PollUpdate {
        contact_ts: Some(now_ts), revoked: Some(true), incompatible: Some(false),
    });
}

fn pull_all(
    state:   &SharedState,
    cursors: &Mutex<Cursors>,
    ops_tx:  &mpsc::Sender<Vec<Op>>,
) -> Result<(), ()> {
    let peers = state.peers.read().unwrap().all().to_vec();
    let mut answered = false;

    for peer in peers {
        if crate::cue_liveness::cue_is_running() { break; }

        let Some(ip) = peer.ip_hint else { continue; };
        let now_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

        let hello_url = format!("http://{ip}:{}/hello", peer.port);
        #[derive(serde::Deserialize)]
        struct Hello {
            #[serde(default)] device_id:   String,
            #[serde(default)] proto_ver:   u32,
            #[serde(default)] proto_vers:  Vec<u32>,
            device_name: String,
            #[serde(default)] device_type: DeviceType,
        }

        let hello = match http_get(&hello_url, HTTP_TIMEOUT) {
            Ok(body) => match serde_json::from_str::<Hello>(&body) {
                Ok(h) if h.device_id != peer.device_id => { continue; }
                Ok(h)  => h,
                Err(_) => {
                    answered = true;
                    mark_incompatible(state, &peer.device_id, now_ts);
                    continue;
                }
            },
            Err(PullError::Revoked) => {
                answered = true;
                mark_revoked(state, &peer.device_id, now_ts);
                continue;
            }
            Err(PullError::Unavailable) => { continue; }
        };
        answered = true;

        let peer_understands = hello.proto_ver == PROTO_VER || hello.proto_vers.contains(&PROTO_VER);
        if !peer_understands {
            mark_incompatible(state, &peer.device_id, now_ts);
            continue;
        }

        {
            let mut peers = state.peers.write().unwrap();
            if let Some(p) = peers.list_mut().find(|p| p.device_id == peer.device_id) {
                if p.device_name != hello.device_name { p.device_name = hello.device_name.clone(); }
                if p.device_type != hello.device_type { p.device_type = hello.device_type; }
            }
            peers.apply_poll(&peer.device_id, PollUpdate { incompatible: Some(false), ..Default::default() });
        }

        let since = cursors.lock().unwrap().get(&peer.device_id);
        let url = format!("http://{ip}:{}/1/ops?since={since}&token={}&port={}", peer.port, peer.token, state.port());

        match http_get(&url, HTTP_TIMEOUT) {
            Ok(body) => {
                state.peers.write().unwrap().apply_poll(&peer.device_id, PollUpdate {
                    contact_ts: Some(now_ts), revoked: Some(false), incompatible: Some(false),
                });
                if body.trim().is_empty() { continue; }

                let ops: Vec<Op> = body.lines()
                    .filter_map(|l| serde_json::from_str::<Op>(l).ok())
                    .collect();
                if ops.is_empty() { continue; }

                if ops_tx.send(ops).is_err() { return Err(()); }
            }
            Err(PullError::Revoked) => {
                mark_revoked(state, &peer.device_id, now_ts);
            }
            Err(PullError::Unavailable) => {}
        }
    }

    if answered {
        state.peers.read().unwrap().save();
    }
    Ok(())
}

// ── minimal HTTP/1.0 ──────────────────────────────────────────────────────────

fn http_get(url: &str, timeout: Duration) -> Result<String, PullError> {
    let (host_port, path_query) = parse_url(url).map_err(|_| PullError::Unavailable)?;
    let mut stream = connect(host_port, timeout).map_err(|_| PullError::Unavailable)?;
    write!(stream, "GET {path_query} HTTP/1.0\r\nHost: {host_port}\r\nConnection: close\r\n\r\n")
        .map_err(|_| PullError::Unavailable)?;

    let mut raw = String::new();
    stream.read_to_string(&mut raw).map_err(|_| PullError::Unavailable)?;

    let status_line = raw.lines().next().unwrap_or("");
    if status_line.contains(" 403 ") || status_line.ends_with(" 403") {
        return Err(PullError::Revoked);
    }
    if !status_line.contains(" 200 ") && !status_line.ends_with(" 200") {
        return Err(PullError::Unavailable);
    }
    Ok(raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("").to_owned())
}

fn parse_url(url: &str) -> std::io::Result<(&str, String)> {
    let rest = url.strip_prefix("http://")
        .ok_or_else(|| std::io::Error::other("url must start with http://"))?;
    let (host_port, tail) = rest.split_once('/').unwrap_or((rest, ""));
    Ok((host_port, format!("/{tail}")))
}

fn connect(host_port: &str, timeout: Duration) -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect(host_port)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    Ok(stream)
}
