use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Once};
use std::time::Duration;

const REPO: &str = "qleverty/Cue";
const ASSET_NAME: &str = "win64-cue.exe";
const TIMEOUT: Duration = Duration::from_secs(10);

static PROVIDER_INIT: Once = Once::new();

fn ensure_provider() {
    PROVIDER_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn split_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    Some((host, path))
}

fn fetch_once(url: &str) -> Option<(u16, HashMap<String, String>, Vec<u8>)> {
    ensure_provider();
    let Some((host, path)) = split_url(url) else {
        crate::clog!("[updater] split_url failed for {url}");
        return None;
    };

    let root_store = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let Ok(server_name) = rustls::pki_types::ServerName::try_from(host.to_string()) else {
        crate::clog!("[updater] bad server_name {host}");
        return None;
    };
    let conn = match rustls::ClientConnection::new(Arc::new(config), server_name) {
        Ok(c) => c,
        Err(e) => { crate::clog!("[updater] ClientConnection::new failed: {e}"); return None; }
    };

    let sock = match TcpStream::connect((host, 443)) {
        Ok(s) => s,
        Err(e) => { crate::clog!("[updater] TcpStream::connect({host}:443) failed: {e}"); return None; }
    };
    let _ = sock.set_read_timeout(Some(TIMEOUT));
    let _ = sock.set_write_timeout(Some(TIMEOUT));

    let mut tls = rustls::StreamOwned::new(conn, sock);
    if let Err(e) = write!(
        tls,
        "GET /{path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: cue-updater\r\nConnection: close\r\n\r\n"
    ) {
        crate::clog!("[updater] write GET failed: {e}");
        return None;
    }

    let mut raw = Vec::new();
    if let Err(e) = tls.read_to_end(&mut raw) {
        crate::clog!("[updater] read_to_end failed after {} bytes: {e}", raw.len());
        return None;
    }
    crate::clog!("[updater] {host}/{path} -> {} bytes raw", raw.len());

    let Some(sep) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
        crate::clog!("[updater] no header/body separator found");
        return None;
    };
    let Ok(head) = std::str::from_utf8(&raw[..sep]) else {
        crate::clog!("[updater] head not valid utf8");
        return None;
    };
    let body = raw[sep + 4..].to_vec();

    let mut lines = head.lines();
    let Some(status_line) = lines.next() else { return None; };
    let Some(status) = status_line.split_whitespace().nth(1).and_then(|s| s.parse().ok()) else {
        crate::clog!("[updater] bad status line: {status_line}");
        return None;
    };
    crate::clog!("[updater] status={status} status_line={status_line}");

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }

    Some((status, headers, body))
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    let mut current = url.to_string();
    for _ in 0..5 {
        let (status, headers, body) = fetch_once(&current)?;
        match status {
            200 => return Some(body),
            301 | 302 | 303 | 307 | 308 => {
                let Some(loc) = headers.get("location") else {
                    crate::clog!("[updater] redirect {status} with no Location header");
                    return None;
                };
                crate::clog!("[updater] redirect {status} -> {loc}");
                current = loc.clone();
            }
            _ => {
                crate::clog!("[updater] unexpected status {status} for {current}");
                return None;
            }
        }
    }
    crate::clog!("[updater] too many redirects starting from {url}");
    None
}

fn parse_version(tag: &str) -> Option<(u32, u32, u32)> {
    let s = tag.strip_prefix('v').unwrap_or(tag);
    let mut parts = s.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    let patch: u32 = parts.next()?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some((major, minor, patch))
}

fn find_tags(xml: &str) -> Vec<String> {
    let marker = "/releases/tag/";
    let mut tags = Vec::new();
    let mut rest = xml;
    while let Some(pos) = rest.find(marker) {
        let after = &rest[pos + marker.len()..];
        let end = after.find(['"', '\'']).unwrap_or(after.len());
        tags.push(after[..end].to_string());
        rest = &after[end..];
    }
    tags
}

fn find_update_url(atom: &str, current: (u32, u32, u32)) -> Option<String> {
    let tags = find_tags(atom);
    crate::clog!("[updater] {} tag(s) found in feed, current={:?}", tags.len(), current);

    let upper = (current.0, current.1 + 1, 0);
    let mut best: Option<(u32, u32, u32)> = None;
    let mut best_tag: Option<String> = None;

    for tag in tags {
        let Some(v) = parse_version(&tag) else {
            crate::clog!("[updater] tag {tag} did not parse as version");
            continue;
        };
        crate::clog!("[updater] tag={tag} parsed={:?}", v);
        if v <= current || v >= upper { continue; }
        if best.is_some_and(|bv| v <= bv) { continue; }
        best = Some(v);
        best_tag = Some(tag);
    }

    crate::clog!("[updater] best={:?} tag={:?}", best, best_tag);
    best_tag.map(|tag| format!("https://github.com/{REPO}/releases/download/{tag}/{ASSET_NAME}"))
}

pub fn check_and_stage_update() {
    if cfg!(debug_assertions) {
        crate::clog!("[updater] skipped — debug build");
        return;
    }
    crate::clog!("[updater] check_and_stage_update start, current version={}", env!("CARGO_PKG_VERSION"));
    let Ok(exe) = std::env::current_exe() else {
        crate::clog!("[updater] current_exe() failed");
        return;
    };

    let mut dl  = exe.as_os_str().to_owned();
    dl.push(".cueextradl");
    let dl_path = std::path::PathBuf::from(dl);

    let mut upd = exe.as_os_str().to_owned();
    upd.push(".cueextraupd");
    let upd_path = std::path::PathBuf::from(upd);

    let _ = std::fs::remove_file(&dl_path);
    let _ = std::fs::remove_file(&upd_path);

    let Some(current) = parse_version(env!("CARGO_PKG_VERSION")) else {
        crate::clog!("[updater] failed to parse own CARGO_PKG_VERSION");
        return;
    };

    let Some(body) = fetch(&format!("https://github.com/{REPO}/releases.atom")) else {
        crate::clog!("[updater] releases feed fetch failed entirely");
        return;
    };
    let atom = String::from_utf8_lossy(&body);
    let Some(url) = find_update_url(&atom, current) else {
        crate::clog!("[updater] no matching update found");
        return;
    };
    crate::clog!("[updater] downloading {url}");
    let Some(bytes) = fetch(&url) else {
        crate::clog!("[updater] asset download failed");
        return;
    };
    crate::clog!("[updater] downloaded {} bytes", bytes.len());

    if let Err(e) = std::fs::write(&dl_path, bytes) {
        crate::clog!("[updater] write dl_path failed: {e}");
        return;
    }
    match std::fs::rename(&dl_path, &upd_path) {
        Ok(()) => crate::clog!("[updater] staged update at {upd_path:?}"),
        Err(e) => crate::clog!("[updater] rename to upd_path failed: {e}"),
    }
}
