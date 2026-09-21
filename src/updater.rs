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
    let (host, path) = split_url(url)?;

    let root_store = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string()).ok()?;
    let conn = rustls::ClientConnection::new(Arc::new(config), server_name).ok()?;

    let sock = TcpStream::connect((host, 443)).ok()?;
    sock.set_read_timeout(Some(TIMEOUT)).ok()?;
    sock.set_write_timeout(Some(TIMEOUT)).ok()?;

    let mut tls = rustls::StreamOwned::new(conn, sock);
    write!(
        tls,
        "GET /{path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: cue-updater\r\nConnection: close\r\n\r\n"
    ).ok()?;

    let mut raw = Vec::new();
    tls.read_to_end(&mut raw).ok()?;

    let sep = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..sep]).ok()?;
    let body = raw[sep + 4..].to_vec();

    let mut lines = head.lines();
    let status_line = lines.next()?;
    let status: u16 = status_line.split_whitespace().nth(1)?.parse().ok()?;

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
                current = headers.get("location")?.clone();
            }
            _ => return None,
        }
    }
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

fn find_update_url(body: &[u8], current: (u32, u32, u32)) -> Option<String> {
    let releases: serde_json::Value = serde_json::from_slice(body).ok()?;
    let releases = releases.as_array()?;

    let upper = (current.0, current.1 + 1, 0);
    let mut best: Option<((u32, u32, u32), String)> = None;

    for rel in releases {
        let tag = rel.get("tag_name")?.as_str()?;
        let Some(v) = parse_version(tag) else { continue };
        if v <= current || v >= upper { continue; }
        if best.as_ref().is_some_and(|(bv, _)| v <= *bv) { continue; }

        let assets = rel.get("assets").and_then(|a| a.as_array())?;
        let Some(asset) = assets.iter().find(|a|
            a.get("name").and_then(|n| n.as_str()) == Some(ASSET_NAME)
        ) else { continue };
        let Some(dl_url) = asset.get("browser_download_url").and_then(|u| u.as_str()) else { continue };

        best = Some((v, dl_url.to_string()));
    }

    best.map(|(_, url)| url)
}

pub fn check_and_stage_update() {
    let Ok(exe) = std::env::current_exe() else { return };

    let mut dl  = exe.as_os_str().to_owned();
    dl.push(".cueextradl");
    let dl_path = std::path::PathBuf::from(dl);

    let mut upd = exe.as_os_str().to_owned();
    upd.push(".cueextraupd");
    let upd_path = std::path::PathBuf::from(upd);

    let _ = std::fs::remove_file(&dl_path);
    let _ = std::fs::remove_file(&upd_path);

    let Some(current) = parse_version(env!("CARGO_PKG_VERSION")) else { return };

    let Some(body) = fetch(&format!("https://api.github.com/repos/{REPO}/releases")) else { return };
    let Some(url) = find_update_url(&body, current) else { return };
    let Some(bytes) = fetch(&url) else { return };

    if std::fs::write(&dl_path, bytes).is_err() { return; }
    let _ = std::fs::rename(&dl_path, &upd_path);
}
