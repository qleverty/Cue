// Модуль системных уведомлений (Windows toast).
//
// НЕ претендует на роль "чистой библиотеки, переиспользуемой 1:1 в
// cue_daemon" (в отличие от routine_scheduler.rs) — просто обычный модуль
// самого Cue. Демон, когда до него дойдёт очередь, вполне может
// реализовать отправку уведомлений иначе.
//
// См. обсуждение 2026-08-02 — почему именно так:
//   - Никаких новых крейтов (ни winrt-toast, ни windows-rs) — только
//     std::process::Command, спавним powershell.exe и просим ЕГО дёрнуть
//     WinRT ToastNotification API. Каждый лишний крейт — лишние байты
//     в бинарнике, а тут задача копеечная.
//   - Toast без зарегистрированного AUMID (App User Model ID) Windows
//     тихо проглатывает, без ошибок. Свой AUMID "Cue.App" регистрируется
//     при старте (register_aumid) записью в реестр HKCU\Software\Classes\
//     AppUserModelId\Cue.App (DisplayName + IconUri) — без ярлыка в Start
//     Menu, без COM и без новых крейтов. Иконка лежит в app_dir(), а не
//     рядом с exe, поэтому перенос exe ничего не ломает. Если регистрация
//     по какой-то причине не удалась — откатываемся на системный AUMID
//     PowerShell (подпись в шапке будет "Windows PowerShell", но тосты
//     продолжат показываться).
//   - Если реестровая регистрация на какой-то версии Windows не даст
//     тост — запасной план: ярлык в Start Menu (IShellLink/IPropertyStore).

#[cfg(windows)]
mod imp {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Наш собственный AUMID. МЕНЯТЬ НЕЛЬЗЯ после релиза: по нему Windows
    /// хранит пользовательские настройки уведомлений и историю.
    const OWN_AUMID: &str = "Cue.App";
    /// Подпись в шапке тоста.
    const DISPLAY_NAME: &str = "Cue";
    /// Запасной вариант: системный AUMID PowerShell — общеизвестный трюк
    /// для показа тостов без собственной регистрации.
    const FALLBACK_AUMID: &str =
        r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe";

    /// true, если register() успешно записал наш AUMID в реестр.
    static OWN_REGISTERED: AtomicBool = AtomicBool::new(false);

    /// Какой AUMID передавать в CreateToastNotifier.
    fn aumid() -> &'static str {
        if OWN_REGISTERED.load(Ordering::Relaxed) { OWN_AUMID } else { FALLBACK_AUMID }
    }

    // Сырой FFI на advapi32.dll — по той же причине, что kernel32 в
    // routine_scheduler.rs: библиотека и так есть в любом Windows-процессе,
    // новых зависимостей в Cargo.toml не появляется.
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

    /// UTF-16 с нулевым терминатором для Win32.
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

    /// Регистрирует AUMID "Cue.App" (имя + иконка) в HKCU. Идемпотентно,
    /// права администратора не нужны. Вызывать один раз при старте, до
    /// первого send(). Иконка — тот же icon.png, что у окна, кладётся в
    /// app_dir() при первом запуске (или если размер не совпал).
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

    /// Не показывать окно консоли при спавне powershell.exe.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// PS-скрипт для показа тоста. Параметры — именованные (-AumId,
    /// -Title, -Body, -IconPath), приходят как ОТДЕЛЬНЫЕ аргументы
    /// процесса (см. send() ниже) — не склеены в текст скрипта, поэтому
    /// произвольный текст задачи (кавычки, амперсанды, что угодно) не
    /// может сломать сам скрипт или впрыснуть в него команды.
    //
    // XML тоста собирается через DOM (CreateElement/SetAttribute/
    // CreateTextNode), а не строковой склейкой тегов — так текст
    // автоматически экранируется, и им нельзя сломать структуру XML
    // (аналог параметризованных запросов вместо конкатенации в SQL).
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

    /// Путь к самому .ps1-скрипту (тоже пишется лениво, один раз за
    /// процесс — -File надёжнее квотинга через -Command с хвостовыми
    /// аргументами).
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

    /// Без -IconPath вообще — флаг не передаётся, не просто передаётся
    /// пустым значением (меньше риск странностей в биндинге параметров
    /// PowerShell). $IconPath останется $null по умолчанию, скрипт это уже
    /// учитывает (`if ($IconPath -and ...)`). Временный эксперимент —
    /// проверить, годится ли системная иконка по умолчанию, прежде чем
    /// городить отдельный ассет специально под предупреждения.
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
        // На остальных ОС уведомлений пока нет вовсе — рутина всё равно
        // активируется молча, как и раньше (см. обсуждение 2026-08-02).
    }

    pub fn send_no_icon(_title: &str, _body: &str) {}
}

pub use imp::send;
pub use imp::register as register_aumid;
pub use imp::send_no_icon;
