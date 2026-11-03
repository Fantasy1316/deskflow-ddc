//! XDG autostart, current user only. Preserve actual disabled state written by
//! GNOME startup settings and quote paths using Desktop Entry Exec rules.
use anyhow::{ensure, Context, Result};
use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
fn path() -> Result<PathBuf> {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .context("无法定位当前用户的 XDG 配置目录")?;
    Ok(root.join("autostart/Deskflow DDC.desktop"))
}
fn active(text: &str) -> bool {
    let mut section = false;
    let mut enabled = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            section = line == "[Desktop Entry]";
            continue;
        }
        if !section {
            continue;
        }
        if line == "Hidden=true" || line == "X-GNOME-Autostart-enabled=false" {
            return false;
        }
        if line.starts_with("Exec=") && line.len() > 5 {
            enabled = true;
        }
    }
    enabled
}
fn entry(executable: &Path) -> Result<String> {
    let value = executable.to_str().context("登录启动路径不是 UTF-8")?;
    ensure!(
        // This serializes a Linux Desktop Entry, including in tests hosted on
        // Windows where Path::is_absolute uses different filesystem rules.
        value.starts_with('/') && !value.contains(['\r', '\n', '\0']),
        "登录启动路径无效"
    );
    let mut escaped = String::new();
    for c in value.chars() {
        match c {
            '%' => escaped.push_str("%%"),
            // Exec quoting is parsed after Desktop Entry string unescaping.
            '\\' => escaped.push_str("\\\\\\\\"),
            '"' | '$' | '`' => {
                escaped.push_str("\\\\");
                escaped.push(c);
            }
            _ => escaped.push(c),
        }
    }
    Ok(format!("[Desktop Entry]\nType=Application\nName=Deskflow DDC\nExec=\"{escaped}\" --autostart\nTerminal=false\nStartupNotify=false\nX-GNOME-Autostart-enabled=true\n"))
}
#[cfg(target_os = "linux")]
pub(super) fn enabled() -> Result<bool> {
    match std::fs::read_to_string(path()?) {
        Ok(text) => Ok(active(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).context("无法读取 Ubuntu 登录启动项"),
    }
}
#[cfg(target_os = "linux")]
pub(super) fn set_enabled(wanted: bool) -> Result<()> {
    let path = path()?;
    if wanted {
        crate::store::atomic_write(&path, entry(&std::env::current_exe()?)?.as_bytes())
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).context("无法关闭 Ubuntu 登录启动项"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_desktop_environment_disabling_autostart() {
        let text = entry(Path::new("/opt/Deskflow DDC/deskflow-ddc")).unwrap();
        assert!(active(&text));
        assert!(!active(&(text.clone() + "Hidden=true\n")));
        assert!(!active(
            &(text.clone() + "X-GNOME-Autostart-enabled=false\n")
        ));
        assert!(active(&(text + "[Other]\nHidden=true\n")));
    }
    #[test]
    fn desktop_exec_quotes_spaces_and_escapes_field_codes() {
        let text = entry(Path::new("/opt/Deskflow DDC/100%/deskflow-ddc")).unwrap();
        assert!(text.contains("Exec=\"/opt/Deskflow DDC/100%%/deskflow-ddc\" --autostart\n"));
        assert!(entry(Path::new("/opt/app\nHidden=true")).is_err());
        assert!(entry(Path::new("relative/app")).is_err());
        assert!(entry(Path::new(r"C:\Program Files\deskflow-ddc.exe")).is_err());
    }
}
