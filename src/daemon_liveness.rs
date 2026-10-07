use serde::Deserialize;

#[derive(Deserialize)]
struct DaemonLockData { pid: u32, time: u64 }

fn daemon_lock_path() -> std::path::PathBuf {
    crate::app_dir().join("daemon.lock")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn is_fresh(l: &DaemonLockData) -> bool {
    now().saturating_sub(l.time) < 3600
}

#[cfg(target_os = "windows")]
fn is_alive(pid: u32) -> bool {
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
#[cfg(not(target_os = "windows"))]
fn is_alive(_: u32) -> bool { false }

pub fn running_pid() -> Option<u32> {
    let s = std::fs::read_to_string(daemon_lock_path()).ok()?;
    let lock: DaemonLockData = serde_json::from_str(&s).ok()?;
    if is_fresh(&lock) && is_alive(lock.pid) { Some(lock.pid) } else { None }
}

pub fn daemon_is_running() -> bool {
    running_pid().is_some()
}
