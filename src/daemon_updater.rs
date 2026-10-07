use std::io::Read;
use std::net::TcpStream;
use std::time::Duration;

pub fn daemon_exe_path()     -> std::path::PathBuf { crate::app_dir().join("cue_daemon.exe") }
fn daemon_version_path() -> std::path::PathBuf { crate::app_dir().join("cue_daemon.version") }

static DAEMON_BYTES: &[u8] = include_bytes!("../cue_daemon.exe");

pub fn ensure_daemon_present() {
    if daemon_exe_path().exists() { return; }
    if std::fs::write(daemon_exe_path(), DAEMON_BYTES).is_ok() {
        let _ = std::fs::write(daemon_version_path(), env!("CARGO_PKG_VERSION"));
    }
}

pub fn check_and_update_daemon() {
    ensure_daemon_present();

    let current = std::fs::read_to_string(daemon_version_path()).unwrap_or_default();
    if current.trim() == env!("CARGO_PKG_VERSION") { return; }

    println!("[cue] демон устарел ({current:?} != {:?}), обновляю", env!("CARGO_PKG_VERSION"));

    if let Some(pid) = crate::daemon_liveness::running_pid() {
        if !request_graceful_exit(pid) {
            println!("[cue] демон (pid={pid}) не завершился за 5 попыток, отступаю до следующего запуска");
            return;
        }
    }

    replace_and_relaunch();
}

fn request_graceful_exit(expected_pid: u32) -> bool {
    for attempt in 1..=5 {
        send_shutdown();
        std::thread::sleep(Duration::from_secs(1));
        match crate::daemon_liveness::running_pid() {
            None => return true,
            Some(pid) if pid != expected_pid => return true,
            Some(_) => println!("[cue] демон всё ещё жив, попытка {attempt}/5"),
        }
    }
    false
}

fn send_shutdown() {
    let port = crate::settings::Settings::load().http_port;
    let Ok(mut s) = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(500),
    ) else { return; };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    use std::io::Write;
    let _ = s.write_all(b"GET /1/control?cmd=shutdown HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    let mut buf = [0u8; 64];
    let _ = s.read(&mut buf);
}

fn replace_and_relaunch() {
    let staged = crate::app_dir().join("cue_daemon.exe.cueextraupd");
    if std::fs::write(&staged, DAEMON_BYTES).is_err() {
        println!("[cue] не удалось сохранить новый cue_daemon.exe на диск");
        return;
    }
    if std::fs::rename(&staged, daemon_exe_path()).is_err() {
        println!("[cue] не удалось переименовать cue_daemon.exe.cueextraupd → cue_daemon.exe");
        return;
    }
    let _ = std::fs::write(daemon_version_path(), env!("CARGO_PKG_VERSION"));
    println!("[cue] демон обновлён до {}", env!("CARGO_PKG_VERSION"));

    if crate::settings::Settings::load().background_daemon {
        spawn_daemon();
    }
}

pub fn spawn_daemon() {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let result = std::process::Command::new(daemon_exe_path())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        if let Err(e) = result {
            println!("[cue] не удалось запустить cue_daemon.exe: {e}");
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new(daemon_exe_path()).spawn();
    }
}
