#[cfg(windows)]
mod imp {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};

    const OWN_AUMID: &str = "Cue.App";
    const DISPLAY_NAME: &str = "Cue";
    const FALLBACK_AUMID: &str =
        r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe";

    static OWN_REGISTERED: AtomicBool = AtomicBool::new(false);

    fn aumid() -> &'static str {
        if OWN_REGISTERED.load(Ordering::Relaxed) { OWN_AUMID } else { FALLBACK_AUMID }
    }

    #[allow(non_snake_case)]
    mod reg {
        use std::ffi::c_void;
        pub const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
        pub const KEY_WRITE: u32 = 0x0002_0006;
        pub const REG_EXPAND_SZ: u32 = 2;
        #[link(name = "advapi32")]
        unsafe extern "system" {
            pub fn RegCreateKeyExW(
                hKey: isize, lpSubKey: *const u16, Reserved: u32, lpClass: *const u16,
                dwOptions: u32, samDesired: u32, lpSecurityAttributes: *const c_void,
                phkResult: *mut isize, lpdwDisposition: *mut u32,
            ) -> i32;
            pub fn RegSetValueExW(
                hKey: isize, lpValueName: *const u16, Reserved: u32,
                dwType: u32, lpData: *const u8, cbData: u32,
            ) -> i32;
            pub fn RegCloseKey(hKey: isize) -> i32;
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    fn set_string(key: isize, name: &str, value: &str) -> bool {
        let name = wide(name);
        let data = wide(value);
        let rc = unsafe {
            reg::RegSetValueExW(
                key, name.as_ptr(), 0, reg::REG_EXPAND_SZ,
                data.as_ptr() as *const u8, (data.len() * 2) as u32,
            )
        };
        rc == 0
    }

    pub fn register() {
        let icon = crate::app_dir().join("cue_toast_icon.png");
        let png  = crate::ICON_PNG;
        let stale = std::fs::metadata(&icon).map_or(true, |m| m.len() != png.len() as u64);
        if stale {
            let _ = std::fs::create_dir_all(crate::app_dir());
            if let Err(e) = std::fs::write(&icon, png) {
                crate::clog!("[notify] не удалось записать иконку для тостов: {e}");
                return;
            }
        }
        let Some(icon_str) = icon.to_str() else { return; };

        let sub = wide(&format!("Software\\Classes\\AppUserModelId\\{OWN_AUMID}"));
        let mut key: isize = 0;
        let mut disposition: u32 = 0;
        let rc = unsafe {
            reg::RegCreateKeyExW(
                reg::HKEY_CURRENT_USER, sub.as_ptr(), 0, std::ptr::null(), 0,
                reg::KEY_WRITE, std::ptr::null(), &mut key, &mut disposition,
            )
        };
        if rc != 0 {
            crate::clog!("[notify] RegCreateKeyExW вернул {rc} — остаёмся на AUMID PowerShell");
            return;
        }
        let ok = set_string(key, "DisplayName", DISPLAY_NAME)
            && set_string(key, "IconUri", icon_str);
        unsafe { reg::RegCloseKey(key); }

        if ok {
            OWN_REGISTERED.store(true, Ordering::Relaxed);
        } else {
            crate::clog!("[notify] не удалось записать значения AUMID — остаёмся на AUMID PowerShell");
        }
    }

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SCRIPT: &str = r#"
param(
    [string]$AumId,
    [string]$Title,
    [string]$Body,
    [string]$IconPath
)

[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null

$xml = New-Object Windows.Data.Xml.Dom.XmlDocument

$toastNode = $xml.CreateElement("toast")
$xml.AppendChild($toastNode) | Out-Null

$visualNode = $xml.CreateElement("visual")
$toastNode.AppendChild($visualNode) | Out-Null

$bindingNode = $xml.CreateElement("binding")
$bindingNode.SetAttribute("template", "ToastGeneric")
$visualNode.AppendChild($bindingNode) | Out-Null

$titleNode = $xml.CreateElement("text")
$titleNode.AppendChild($xml.CreateTextNode($Title)) | Out-Null
$bindingNode.AppendChild($titleNode) | Out-Null

$bodyNode = $xml.CreateElement("text")
$bodyNode.AppendChild($xml.CreateTextNode($Body)) | Out-Null
$bindingNode.AppendChild($bodyNode) | Out-Null

if ($IconPath -and (Test-Path $IconPath)) {
    $imgNode = $xml.CreateElement("image")
    $imgNode.SetAttribute("placement", "appLogoOverride")
    $imgNode.SetAttribute("hint-crop", "circle")
    $imgNode.SetAttribute("src", $IconPath)
    $bindingNode.AppendChild($imgNode) | Out-Null
}

$notifier = [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($AumId)
$toast    = New-Object Windows.UI.Notifications.ToastNotification($xml)
$notifier.Show($toast)
"#;

    fn script_path() -> &'static std::path::Path {
        static PATH: OnceLock<std::path::PathBuf> = OnceLock::new();
        PATH.get_or_init(|| {
            let path = crate::app_dir().join("notify_toast.ps1");
            if !path.exists() {
                let _ = std::fs::create_dir_all(crate::app_dir());
                if let Ok(mut f) = std::fs::File::create(&path) {
                    let _ = f.write_all(SCRIPT.as_bytes());
                }
            }
            path
        })
    }

    pub fn send(title: &str, body: &str, color_hex: &str) {
        let icon = crate::icon_cache::icon_path_for(color_hex);
        send_with_icon_path(title, body, Some(icon));
    }

    pub fn send_no_icon(title: &str, body: &str) {
        send_with_icon_path(title, body, None);
    }

    fn send_with_icon_path(title: &str, body: &str, icon: Option<std::path::PathBuf>) {
        let script = script_path();

        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
               "-WindowStyle", "Hidden", "-File"])
            .arg(script)
            .arg("-AumId").arg(aumid())
            .arg("-Title").arg(title)
            .arg("-Body").arg(body);
        if let Some(icon) = icon {
            cmd.arg("-IconPath").arg(icon);
        }
        let result = cmd.creation_flags(CREATE_NO_WINDOW).spawn();

        if let Err(e) = result {
            crate::clog!("[notify] не удалось запустить powershell: {e}");
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn register() {}

    pub fn send(_title: &str, _body: &str, _color_hex: &str) {
    }

    pub fn send_no_icon(_title: &str, _body: &str) {}
}

pub use imp::send;
pub use imp::register as register_aumid;
pub use imp::send_no_icon;
