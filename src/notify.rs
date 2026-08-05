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
//   - Toast без AUMID (App User Model ID) Windows тихо проглатывает, без
//     ошибок. Регистрировать свой AUMID (ярлык в Start Menu через COM
//     IShellLink/IPropertyStore) — отдельная, более объёмная задача на
//     будущее. Пока используем уже существующий системный AUMID самого
//     PowerShell — общеизвестный трюк, работает из коробки, минус один:
//     подпись в шапке тоста будет "Windows PowerShell", а не "Cue".
//     Иконка и текст при этом полностью наши (см. AUMID ниже —
//     единственное место, которое придётся поменять, когда сделаем
//     свой ярлык).

#[cfg(windows)]
mod imp {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::OnceLock;

    /// Системный AUMID PowerShell — общеизвестный трюк для показа тостов
    /// без регистрации собственного AUMID. ЕДИНСТВЕННОЕ место, которое
    /// нужно будет поменять, когда сделаем свой ярлык в Start Menu —
    /// больше нигде в модуле ничего трогать не придётся.
    const AUMID: &str =
        r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe";

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
        let script = script_path();

        let result = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                   "-WindowStyle", "Hidden", "-File"])
            .arg(script)
            .arg("-AumId").arg(AUMID)
            .arg("-Title").arg(title)
            .arg("-Body").arg(body)
            .arg("-IconPath").arg(&icon)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();

        if let Err(e) = result {
            crate::clog!("[notify] не удалось запустить powershell: {e}");
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn send(_title: &str, _body: &str, _color_hex: &str) {
        // На остальных ОС уведомлений пока нет вовсе — рутина всё равно
        // активируется молча, как и раньше (см. обсуждение 2026-08-02).
    }
}

pub use imp::send;
