//! Ubuntu DDC uses ddcutil and resolves stable connector/EDID IDs to the current
//! I²C bus on every operation. Never persist an ephemeral bus or X11 resource ID.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn x11_session(kind: Option<&str>, wayland: bool, display: bool) -> bool {
    !wayland && kind != Some("wayland") && display
}
pub fn ensure_x11() -> Result<()> {
    ensure!(
        x11_session(std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var_os("WAYLAND_DISPLAY").is_some(),
            std::env::var_os("DISPLAY").is_some()),
        "当前键鼠共享仅支持 Ubuntu X11 会话；请注销并在登录界面选择 Ubuntu on Xorg。Wayland 可使用仅切换显示器模式，但暂不支持键鼠共享"
    );
    Ok(())
}
pub fn ddcutil() -> Option<PathBuf> {
    ["/usr/bin/ddcutil", "/usr/local/bin/ddcutil"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}
#[derive(Debug)]
pub struct Display {
    pub id: String,
    pub name: String,
    pub bus: Option<u32>,
}
fn connector_name(value: &str) -> &str {
    value
        .split_once('-')
        .filter(|(card, _)| {
            card.strip_prefix("card")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .map_or(value, |(_, connector)| connector)
}
pub fn stable_id(connector: &str, edid: &[u8]) -> String {
    format!(
        "drm:{}:{}",
        connector_name(connector),
        &hex::encode(Sha256::digest(edid))[..24]
    )
}
fn edid_name(edid: &[u8]) -> Option<String> {
    edid.get(54..126)?.chunks_exact(18).find_map(|block| {
        if block[..5] != [0, 0, 0, 0xfc, 0] {
            return None;
        }
        let name = String::from_utf8_lossy(&block[5..])
            .trim_matches(['\0', '\n', '\r', ' '])
            .to_owned();
        (!name.is_empty()).then_some(name)
    })
}
pub fn displays_at(root: &Path) -> Result<Vec<Display>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(root).context("无法读取 Linux DRM 显示设备")? {
        let entry = entry?;
        let path = entry.path();
        if std::fs::read_to_string(path.join("status"))
            .unwrap_or_default()
            .trim()
            != "connected"
        {
            continue;
        }
        let connector = entry.file_name().to_string_lossy().into_owned();
        // Internal panels are selected separately through RandR, not DDC input switching.
        if ["eDP", "LVDS"]
            .iter()
            .any(|prefix| connector_name(&connector).starts_with(prefix))
        {
            continue;
        }
        let edid = std::fs::read(path.join("edid")).unwrap_or_default();
        let bus = std::fs::canonicalize(path.join("ddc")).ok().and_then(|p| {
            p.file_name()?
                .to_str()?
                .strip_prefix("i2c-")?
                .parse::<u32>()
                .ok()
        });
        let name = edid_name(&edid).unwrap_or_else(|| connector_name(&connector).to_owned());
        found.push(Display {
            id: stable_id(&connector, &edid),
            name,
            bus,
        });
    }
    found.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(found)
}
pub fn displays() -> Result<Vec<Display>> {
    displays_at(Path::new("/sys/class/drm"))
}
fn number(value: &str) -> Option<u16> {
    if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix('x')) {
        u16::from_str_radix(hex, 16).ok()
    } else {
        value.parse().ok()
    }
}
pub fn parse_vcp(output: &str) -> Option<u16> {
    output.lines().find_map(|line| {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.first() != Some(&"VCP") || !matches!(parts.get(1), Some(&"60") | Some(&"0x60")) {
            return None;
        }
        match parts.as_slice() {
            // ddcutil 2.x defines input source as a simple non-continuous VCP.
            [_, _, "SNC", value] => number(value).filter(|v| *v <= 255),
            [_, _, "C", current, _] => number(current),
            _ => None,
        }
    })
}

/// Must be called only in the child's pre_exec hook. No allocation or locks.
#[cfg(target_os = "linux")]
pub fn arm_parent_death_signal(owner: u32) -> std::io::Result<()> {
    unsafe {
        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::getppid() != owner as libc::pid_t {
            libc::_exit(1);
        }
    }
    Ok(())
}

impl crate::platform::Platform {
    async fn linux_vcp(&self, id: &str, value: Option<u16>) -> Result<Option<u16>> {
        let display = displays()?
            .into_iter()
            .find(|d| d.id == id)
            .context("所选显示器已断开或接口发生变化，请重新扫描")?;
        let bus = display
            .bus
            .context("该接口未提供 I²C/DDC 通道；请检查显卡驱动、扩展坞及接线")?;
        let tool = ddcutil().context("未安装 ddcutil，请运行 sudo apt install ddcutil")?;
        let mut args = vec!["--bus".into(), bus.to_string(), "--terse".into()];
        if let Some(value) = value {
            args.extend(["setvcp".into(), "60".into(), value.to_string()]);
        } else {
            args.extend(["getvcp".into(), "60".into()]);
        }
        let output = self.run(tool, &args).await.map_err(|error| anyhow::anyhow!(
            "DDC 操作失败：{error:#}；请确认显示器开启 DDC/CI，i2c-dev 已加载且当前用户可访问 /dev/i2c-*（通常需加入 i2c 组并重新登录）；应用不会请求 root 权限"))?;
        if value.is_some() {
            Ok(None)
        } else {
            Ok(parse_vcp(&output))
        }
    }
    pub(crate) async fn linux_input(&self, id: &str, value: u16) -> Result<()> {
        self.linux_vcp(id, Some(value)).await?;
        Ok(())
    }
    pub(crate) async fn linux_current_input(&self, id: &str) -> Result<Option<u16>> {
        self.linux_vcp(id, None).await
    }
    pub(crate) async fn linux_list(&self) -> Result<Value> {
        let mut monitors = Vec::new();
        for display in displays()? {
            let mut monitor = json!({"id":display.id,"name":display.name,"readable":false});
            if let Ok(Some(value)) = self.linux_current_input(&display.id).await {
                monitor["readable"] = true.into();
                monitor["currentInput"] = value.into();
            }
            monitors.push(monitor);
        }
        Ok(monitors.into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wayland_with_xwayland_is_never_treated_as_native_x11() {
        assert!(x11_session(Some("x11"), false, true));
        assert!(x11_session(None, false, true));
        assert!(!x11_session(Some("wayland"), false, true));
        assert!(!x11_session(Some("x11"), true, true));
        assert!(!x11_session(None, false, false));
    }
    #[test]
    fn vcp_parses_only_successful_input_source_responses() {
        assert_eq!(parse_vcp("VCP 60 SNC x11"), Some(17));
        assert_eq!(parse_vcp("VCP 60 C 17 18"), Some(17));
        assert_eq!(parse_vcp("VCP 60 C x0011 x0012"), Some(17));
        assert_eq!(
            parse_vcp("diagnostic\nVCP 0x60 C 0x000f 0x0012\n"),
            Some(15)
        );
        for line in [
            "VCP 60 ERR",
            "VCP 10 C 17 100",
            "VCP 60 C 65536 0",
            "17",
            "VCP 60 SNC x100",
        ] {
            assert_eq!(parse_vcp(line), None);
        }
    }
    #[test]
    fn monitor_identity_survives_card_and_bus_renumbering() {
        assert_eq!(
            stable_id("card0-DP-1", b"same edid"),
            stable_id("card2-DP-1", b"same edid")
        );
        assert_ne!(
            stable_id("card0-DP-1", b"first"),
            stable_id("card0-DP-1", b"second")
        );
        assert_ne!(stable_id("card0-DP-1", b""), stable_id("card0-DP-2", b""));
    }
    #[cfg(unix)]
    #[test]
    fn drm_resolves_current_bus_and_keeps_unreadable_monitors_selectable() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let connector = root.path().join("card0-DP-1");
        std::fs::create_dir(&connector).unwrap();
        std::fs::write(connector.join("status"), "connected\n").unwrap();
        std::fs::write(connector.join("edid"), b"edid").unwrap();
        let old = displays_at(root.path()).unwrap();
        assert_eq!(old[0].bus, None);
        let bus = root.path().join("i2c-9");
        std::fs::create_dir(&bus).unwrap();
        symlink(&bus, connector.join("ddc")).unwrap();
        let new = displays_at(root.path()).unwrap();
        assert_eq!(new[0].id, old[0].id);
        assert_eq!(new[0].bus, Some(9));
        std::fs::write(connector.join("status"), "disconnected\n").unwrap();
        assert!(displays_at(root.path()).unwrap().is_empty());
    }
}
