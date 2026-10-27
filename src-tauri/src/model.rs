use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "unsupported"
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub device_name: String,
    pub role: String,
    pub peer_address: String,
    #[serde(default)]
    pub fallback_ips: Vec<std::net::IpAddr>,
    pub control_port: u16,
    pub deskflow_port: u16,
    pub deskflow_path: String,
    pub display_id: String,
    pub local_input: Option<u16>,
    pub peer_input: Option<u16>,
    pub shortcut: String,
    pub clipboard: bool,
    pub clipboard_megabytes: u8,
    pub layout_side: String,
    pub cross_screen_video: String,
    pub retained_display_id: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            device_name: hostname::get()
                .unwrap_or_default()
                .to_string_lossy()
                .chars()
                .take(48)
                .collect(),
            role: "host".into(),
            peer_address: String::new(),
            fallback_ips: vec![],
            // Preview can coexist with the installed stable application.
            control_port: 49729,
            deskflow_port: 24801,
            deskflow_path: String::new(),
            display_id: String::new(),
            local_input: None,
            peer_input: None,
            shortcut: "control+alt+F10".into(),
            clipboard: true,
            clipboard_megabytes: 4,
            layout_side: "off".into(),
            cross_screen_video: "follow".into(),
            retained_display_id: String::new(),
        }
    }
}
fn clean(s: &str, max: usize) -> bool {
    s.chars().count() <= max && !s.contains(['\r', '\n', '\0'])
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            clean(&self.device_name, 48) && !self.device_name.trim().is_empty(),
            "设备名称无效"
        );
        ensure!(
            ["host", "client"].contains(&self.role.as_str()),
            "键鼠角色无效"
        );
        ensure!(
            clean(&self.peer_address, 253) && !self.peer_address.chars().any(char::is_whitespace),
            "提供端地址无效"
        );
        ensure!(self.fallback_ips.len() <= 8, "备用 IP 最多填写 8 个");
        ensure!(
            self.control_port >= 1024
                && self.deskflow_port >= 1024
                && self.control_port != self.deskflow_port,
            "端口无效或重复"
        );
        ensure!(
            clean(&self.display_id, 512) && clean(&self.deskflow_path, 4096),
            "显示器或引擎路径无效"
        );
        ensure!(
            self.local_input.is_none() || self.local_input != self.peer_input,
            "两端输入源不能相同"
        );
        ensure!(
            (1..=32).contains(&self.clipboard_megabytes),
            "剪贴板大小无效"
        );
        ensure!(
            regex::Regex::new(r"^control\+alt\+(shift\+)?F([1-9]|1[0-7])$")?
                .is_match(&self.shortcut),
            "快捷键格式无效"
        );
        ensure!(
            ["off", "left", "right", "top", "bottom"].contains(&self.layout_side.as_str()),
            "布局无效"
        );
        ensure!(
            ["keep", "follow"].contains(&self.cross_screen_video.as_str()),
            "画面模式无效"
        );
        ensure!(
            self.retained_display_id.len() <= 128
                && self
                    .retained_display_id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "独立屏幕无效"
        );
        Ok(())
    }
    pub fn ready(&self) -> Result<()> {
        self.validate()?;
        ensure!(
            !self.display_id.is_empty() && self.local_input.is_some() && self.peer_input.is_some(),
            "请先配置显示器和两端输入源"
        );
        ensure!(platform() != "unsupported", "当前仅支持 macOS 和 Windows");
        ensure!(
            !(platform() == "win32" && self.role == "host" && self.shortcut.ends_with("F12")),
            "F12 是 Windows 保留键，请选择 F10 或 F9"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Pin {
    pub id: uuid::Uuid,
    pub fingerprint: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: uuid::Uuid,
    pub name: String,
    pub platform: String,
    pub fingerprint: String,
    pub role: String,
    pub local_input: u16,
    pub peer_input: u16,
    pub shortcut: String,
    pub clipboard: bool,
    pub clipboard_megabytes: u8,
    pub deskflow_port: u16,
    pub layout_version: u8,
    pub layout_side: String,
    pub cross_screen_video: String,
    pub retained_display: bool,
}
impl Identity {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            ["darwin", "win32"].contains(&self.platform.as_str())
                && ["host", "client"].contains(&self.role.as_str()),
            "对端平台或角色无效"
        );
        ensure!(
            self.layout_version == 1 && valid_fingerprint(&self.fingerprint),
            "对端协议或证书身份无效"
        );
        let c = Config {
            device_name: self.name.clone(),
            role: self.role.clone(),
            local_input: Some(self.local_input),
            peer_input: Some(self.peer_input),
            shortcut: self.shortcut.clone(),
            clipboard_megabytes: self.clipboard_megabytes,
            deskflow_port: self.deskflow_port,
            layout_side: self.layout_side.clone(),
            cross_screen_video: self.cross_screen_video.clone(),
            ..Config::default()
        };
        c.validate()
    }
}
pub fn valid_fingerprint(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_injection_and_collision() {
        let mut c = Config::default();
        c.peer_address = "pc\n[security]".into();
        assert!(c.validate().is_err());
        c.peer_address = "pc.local".into();
        c.control_port = c.deskflow_port;
        assert!(c.validate().is_err());
    }
    #[test]
    fn accepts_hostname_and_ip_fallback() {
        let c = Config {
            peer_address: "pc.local".into(),
            fallback_ips: vec!["192.168.1.5".parse().unwrap()],
            ..Config::default()
        };
        c.validate().unwrap();
    }
}
