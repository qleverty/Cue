#[cfg(windows)]
mod imp {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::OnceLock;

    const AUMID: &str =
        r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe";

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
            let path = crate::app_dir().join("notify_toast_daemon.ps1");
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
            eprintln!("[cue_daemon] не удалось запустить powershell: {e}");
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn send(_title: &str, _body: &str, _color_hex: &str) {}
}

pub use imp::send;
