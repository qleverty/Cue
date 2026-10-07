#![windows_subsystem = "windows"]

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod apply;
mod cue_liveness;
mod cursors;
mod engine;
mod exclusive_bind;
mod icon_cache;
mod identity;
mod manifest;
mod notify;
mod oplog;
mod peers;
mod project;
mod routine_scheduler;
mod server;
mod settings;
mod tombstones;

pub(crate) static ICON_PNG: &[u8] = include_bytes!("../icon.png");

pub fn app_dir() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("Cue")
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| std::path::PathBuf::from(h).join(".config"))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
            })
            .join("cue")
    }
}

// ── daemon.lock ──────────────────────────────────────────────────────────────

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct DaemonLockData { pid: u32, time: u64 }

fn daemon_lock_path() -> std::path::PathBuf { app_dir().join("daemon.lock") }

fn read_daemon_lock() -> Option<DaemonLockData> {
    serde_json::from_str(&std::fs::read_to_string(daemon_lock_path()).ok()?).ok()
}

fn write_daemon_lock() {
    let d = DaemonLockData { pid: std::process::id(), time: project::current_time() };
    if let Ok(j) = serde_json::to_string(&d) { let _ = std::fs::write(daemon_lock_path(), j); }
}

#[allow(dead_code)]
fn delete_daemon_lock() { let _ = std::fs::remove_file(daemon_lock_path()); }

fn daemon_lock_is_fresh(l: &DaemonLockData) -> bool {
    project::current_time().saturating_sub(l.time) < 3600
}

#[cfg(target_os = "windows")]
fn daemon_is_alive(pid: u32) -> bool {
    type HANDLE = *mut u8;
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> HANDLE;
        fn CloseHandle(h: HANDLE) -> i32;
    }
    unsafe {
        let h = OpenProcess(0x00100000, 0, pid);
        if h.is_null() { return false; }
        CloseHandle(h);
        true
    }
}
#[cfg(target_os = "linux")]
fn daemon_is_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn daemon_is_alive(_: u32) -> bool { false }

const LOCK_REFRESH_INTERVAL_SECS: u64 = 15 * 60;

// ── routine tick ──────────────────────────────────────────────────────────────

const TICK_INTERVAL_SECS: u64 = 60;

fn tick(projects: &mut Vec<project::LoadedProject>, cue_was_running: &mut bool) {
    let cue_running = cue_liveness::cue_is_running();

    if cue_running {
        if !*cue_was_running {
            println!("[cue_daemon] Cue жива, пропускаю тик");
        }
        *cue_was_running = true;
        return;
    }

    if *cue_was_running {
        println!("[cue_daemon] Cue закрылась, перечитываю проекты с диска");
        *projects = project::load_all_projects();
    }
    *cue_was_running = false;

    let now = routine_scheduler::local_now();
    let utc = project::current_time();

    for p in projects.iter_mut() {
        let mut changed = false;

        for (id, task) in p.main.iter_mut().chain(p.subs.iter_mut()) {
            let Some(routine) = task.routine.as_mut() else { continue };
            if routine.active { continue }
            let Some(ago) = routine_scheduler::due_secs_ago(routine, task.completed_at, now, utc) else { continue };

            routine.active = true;
            routine.last_triggered_at = now;
            routine_scheduler::prune_expired_direct(routine, now);
            changed = true;

            println!(
                "  ACTIVATED: [{}] {} — задача '{}' (task_id={id}), ago={ago}",
                p.name, p.color_hex, task.text
            );

            if ago <= routine_scheduler::NOTIFY_WINDOW_SECS {
                notify::send(&task.text, &p.name, &p.color_hex);
            }
        }

        if changed {
            p.save();
            println!("[cue_daemon] сохранён проект {} ({})", p.name, p.id);
        }
    }
}

fn main() {
    println!("[cue_daemon] запущен, pid={}", std::process::id());

    if let Some(lock) = read_daemon_lock() {
        if daemon_lock_is_fresh(&lock) && daemon_is_alive(lock.pid) {
            println!("[cue_daemon] другой cue_daemon уже запущен (pid={}), выхожу", lock.pid);
            return;
        }
    }
    write_daemon_lock();

    notify::register_aumid();

    // ── identity-gate ────────────────────────────────────────────────────
	
    let dir = app_dir();
    let identity = loop {
        if let Some(id) = identity::DeviceIdentity::load(&dir) { break id; }
        println!("[cue_daemon] identity.json ещё нет, жду 300с");
        std::thread::sleep(std::time::Duration::from_secs(300));
    };
    println!("[cue_daemon] identity найдена, device_id={}", identity.device_id);

    let (ping_tx, ping_rx) = std::sync::mpsc::sync_channel::<()>(1);

    let shared = std::sync::Arc::new(server::SharedState {
        device_id:        identity.device_id.clone(),
        device_name:      identity.device_name.clone(),
        peers:            std::sync::RwLock::new(peers::Peers::load(&dir)),
        oplog_path:       dir.join("ops.ndjson"),
        pending_pairings: std::sync::Mutex::new(server::load_pending_pairings(&dir)),
        pending_outgoing: std::sync::Mutex::new(std::collections::HashMap::new()),
        http_port:        std::sync::atomic::AtomicU16::new(server::DEFAULT_PORT),
        ping_tx,
    });
    server::start(std::sync::Arc::clone(&shared));

    // ── engine (pull-side) ───────────────────────────────────────
	
    let cursors = std::sync::Arc::new(std::sync::Mutex::new(cursors::Cursors::load(&dir)));
    let (ops_tx, ops_rx) = std::sync::mpsc::channel::<Vec<oplog::Op>>();
    engine::start(std::sync::Arc::clone(&shared), std::sync::Arc::clone(&cursors), ops_tx, ping_rx);

    let mut tombstones = tombstones::Tombstones::load(&dir);
    let mut settings   = settings::Settings::load();
    let mut seen: std::collections::HashSet<String> =
        oplog::read_existing(&dir).into_iter().map(|op| op.op_id).collect();

    let mut projects = project::load_all_projects();
    println!("[cue_daemon] прочитано проектов: {}", projects.len());
    manifest::rebuild_from(&projects);

    let mut cue_was_running    = cue_liveness::cue_is_running();
    let mut last_lock_refresh  = project::current_time();

    loop {
        tick(&mut projects, &mut cue_was_running);

        while let Ok(ops) = ops_rx.try_recv() {
            let mut device_id = String::new();
            let mut max_seq   = 0u64;
            for op in &ops {
                device_id = op.device_id.clone();
                max_seq   = max_seq.max(op.seq);
                let dirty = apply::apply_op(op, &mut projects, &mut tombstones, &mut settings, &mut seen);
                for id in &dirty {
                    if let Some(p) = projects.iter().find(|p| &p.id == id) {
                        p.save();
                    }
                }
            }
            if !device_id.is_empty() {
                cursors.lock().unwrap().set(&device_id, max_seq);
            }
        }

        let now = project::current_time();
        if now >= last_lock_refresh + LOCK_REFRESH_INTERVAL_SECS {
            last_lock_refresh = now;
            write_daemon_lock();
        }

        std::thread::sleep(std::time::Duration::from_secs(TICK_INTERVAL_SECS));
    }
}
