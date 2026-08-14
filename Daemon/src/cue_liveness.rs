use serde::Deserialize;

#[derive(Deserialize)]
struct LockData { pid: u32, time: u64 }

fn lock_path() -> std::path::PathBuf {
    crate::app_dir().join("cue.lock")
}

fn read_lock() -> Option<LockData> {
    serde_json::from_str(&std::fs::read_to_string(lock_path()).ok()?).ok()
}

fn lock_is_fresh(l: &LockData) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now.saturating_sub(l.time) < 3600
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
#[cfg(target_os = "linux")]
fn is_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn is_alive(_: u32) -> bool { false }

pub fn cue_is_running() -> bool {
    match read_lock() {
        Some(l) => lock_is_fresh(&l) && is_alive(l.pid),
        None => false,
    }
}
