const RUN_VALUE_NAME: &str = "CueDaemon";

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;

    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const KEY_READ:  u32 = 0x0002_0019;
    const KEY_WRITE: u32 = 0x0002_0006;
    const REG_SZ:    u32 = 1;
    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegCreateKeyExW(
            hKey: isize, lpSubKey: *const u16, Reserved: u32, lpClass: *const u16,
            dwOptions: u32, samDesired: u32, lpSecurityAttributes: *const c_void,
            phkResult: *mut isize, lpdwDisposition: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            hKey: isize, lpValueName: *const u16, Reserved: u32,
            dwType: u32, lpData: *const u8, cbData: u32,
        ) -> i32;
        fn RegDeleteValueW(hKey: isize, lpValueName: *const u16) -> i32;
        fn RegQueryValueExW(
            hKey: isize, lpValueName: *const u16, lpReserved: *mut u32,
            lpType: *mut u32, lpData: *mut u8, lpcbData: *mut u32,
        ) -> i32;
        fn RegCloseKey(hKey: isize) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    fn open_run_key(access: u32) -> Option<isize> {
        let sub = wide(RUN_KEY);
        let mut key: isize = 0;
        let mut disposition: u32 = 0;
        let rc = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER, sub.as_ptr(), 0, std::ptr::null(), 0,
                access, std::ptr::null(), &mut key, &mut disposition,
            )
        };
        if rc == 0 { Some(key) } else { None }
    }

    pub fn is_registered() -> bool {
        let Some(key) = open_run_key(KEY_READ) else { return false; };
        let name = wide(super::RUN_VALUE_NAME);
        let mut buf = [0u8; 1024];
        let mut len = buf.len() as u32;
        let rc = unsafe {
            RegQueryValueExW(key, name.as_ptr(), std::ptr::null_mut(),
                std::ptr::null_mut(), buf.as_mut_ptr(), &mut len)
        };
        unsafe { RegCloseKey(key); }
        rc == 0
    }

    pub fn register(exe_path: &std::path::Path) {
        let Some(key) = open_run_key(KEY_WRITE) else { return; };
        let name  = wide(super::RUN_VALUE_NAME);
        let value = wide(&format!("\"{}\"", exe_path.display()));
        unsafe {
            RegSetValueExW(key, name.as_ptr(), 0, REG_SZ,
                value.as_ptr() as *const u8, (value.len() * 2) as u32);
            RegCloseKey(key);
        }
    }

    pub fn unregister() {
        let Some(key) = open_run_key(KEY_WRITE) else { return; };
        let name = wide(super::RUN_VALUE_NAME);
        unsafe {
            RegDeleteValueW(key, name.as_ptr());
            RegCloseKey(key);
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    pub fn is_registered() -> bool { false }
    pub fn register(_exe_path: &std::path::Path) {}
    pub fn unregister() {}
}

pub fn set_enabled(enabled: bool) {
    if enabled {
        ensure_registered_and_running();
    } else {
        imp::unregister();
    }
}

fn ensure_registered_and_running() {
    if !imp::is_registered() {
        imp::register(&crate::daemon_updater::daemon_exe_path());
    }
    if crate::daemon_liveness::running_pid().is_none() {
        crate::daemon_updater::spawn_daemon();
    }
}

pub fn reconcile(settings: &crate::settings::Settings) {
    if settings.background_daemon {
        ensure_registered_and_running();
    }
}
