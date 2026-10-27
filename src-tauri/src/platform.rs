use crate::model::platform;
use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};
pub struct Platform {
    pub resources: PathBuf,
    windows: Mutex<Option<WindowsHelper>>,
}
struct WindowsHelper {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    sequence: u64,
}
impl Platform {
    pub fn new(resources: PathBuf) -> Self {
        Self {
            resources,
            windows: Mutex::new(None),
        }
    }
    pub async fn run(&self, file: PathBuf, args: &[String]) -> Result<String> {
        let mut cmd = Command::new(file);
        cmd.args(args).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        let output = tokio::time::timeout(Duration::from_secs(7), cmd.output())
            .await
            .context("原生调用超时")??;
        ensure!(
            output.status.success(),
            "原生调用失败：{}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(700)
                .collect::<String>()
        );
        ensure!(output.stdout.len() <= 1024 * 1024, "原生返回数据过大");
        Ok(String::from_utf8(output.stdout)?.trim().into())
    }
    async fn win(&self, action: &str, payload: Value) -> Result<Value> {
        let mut guard = self.windows.lock().await;
        if guard.is_none() {
            let mut command = Command::new("powershell.exe");
            command
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(self.resources.join("native/monitor.ps1"))
                .args(["-Action", "serve"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let mut child = command.spawn()?;
            let input = child.stdin.take().context("无原生输入通道")?;
            let output = BufReader::new(child.stdout.take().context("无原生输出通道")?);
            *guard = Some(WindowsHelper {
                child,
                input,
                output,
                sequence: 0,
            });
        }
        let helper = guard.as_mut().unwrap();
        helper.sequence += 1;
        let id = helper.sequence;
        let mut request = payload;
        request["id"] = serde_json::json!(id);
        request["action"] = action.into();
        let result = async {
            helper
                .input
                .write_all(format!("{request}\n").as_bytes())
                .await?;
            let mut line = String::new();
            helper.output.read_line(&mut line).await?;
            ensure!(line.len() <= 1024 * 1024, "原生返回数据过大");
            let reply: Value = serde_json::from_str(&line)?;
            ensure!(reply["id"] == id, "原生响应序号不匹配");
            if let Some(error) = reply["error"].as_str() {
                bail!("原生调用失败：{}", error)
            }
            Ok(reply["result"].clone())
        };
        match tokio::time::timeout(Duration::from_secs(8), result).await {
            Ok(Ok(v)) => Ok(v),
            r => {
                if let Some(mut h) = guard.take() {
                    let _ = h.child.kill().await;
                }
                match r {
                    Ok(Err(e)) => Err(e),
                    _ => bail!("原生桥超时，已清理"),
                }
            }
        }
    }
    pub async fn available(&self) -> bool {
        match platform() {
            "darwin" => self.resources.join("native/bin/m1ddc").is_file(),
            "win32" => self.resources.join("native/monitor.ps1").is_file(),
            _ => false,
        }
    }
    pub async fn accessibility(&self) -> bool {
        platform() == "win32"
            || self
                .run(
                    self.resources.join("native/bin/mac-hotkey"),
                    &["check".into()],
                )
                .await
                .is_ok_and(|s| s == "true")
    }
    pub async fn list(&self) -> Result<Value> {
        if platform() == "win32" {
            let v = self.win("list", serde_json::json!({})).await?;
            return Ok(if v.is_array() {
                v
            } else if v.is_null() {
                serde_json::json!([])
            } else {
                serde_json::json!([v])
            });
        }
        ensure!(platform() == "darwin", "当前系统不支持 DDC");
        let text = self
            .run(
                self.resources.join("native/bin/m1ddc"),
                &["display".into(), "list".into()],
            )
            .await?;
        let mut monitors = parse_mac_displays(&text);
        for monitor in &mut monitors {
            let id = monitor["id"].as_str().unwrap().to_owned();
            // An unreadable VCP must not hide an otherwise selectable display.
            if let Ok(Ok(value)) = tokio::time::timeout(
                Duration::from_millis(2500),
                self.run(
                    self.resources.join("native/bin/m1ddc"),
                    &["display".into(), id, "get".into(), "input".into()],
                ),
            )
            .await
            {
                if let Some(value) = parse_input(&value) {
                    monitor["currentInput"] = value.into();
                    monitor["readable"] = true.into();
                }
            }
        }
        Ok(monitors.into())
    }
    pub async fn input(&self, id: &str, value: u16) -> Result<()> {
        ensure!(
            !id.is_empty() && !id.contains(['\n', '\r', '\0']),
            "显示器标识无效"
        );
        if platform() == "win32" {
            self.win("set", serde_json::json!({"monitorId":id,"value":value}))
                .await?;
        } else {
            self.run(
                self.resources.join("native/bin/m1ddc"),
                &[
                    "display".into(),
                    id.into(),
                    "set".into(),
                    "input".into(),
                    value.to_string(),
                ],
            )
            .await?;
        }
        Ok(())
    }
    pub async fn inject(&self, host: bool) -> Result<()> {
        let target = if host { "host" } else { "peer" };
        if platform() == "win32" {
            self.win(target, serde_json::json!({})).await?;
        } else {
            self.run(
                self.resources.join("native/bin/mac-hotkey"),
                &[target.into()],
            )
            .await?;
        }
        Ok(())
    }
    pub async fn point(&self, x: f64, y: f64) -> Result<()> {
        ensure!(x.is_finite() && y.is_finite(), "坐标无效");
        if platform() == "win32" {
            self.win(
                "pointer",
                serde_json::json!({"x":x.round() as i32,"y":y.round() as i32}),
            )
            .await?;
        } else {
            self.run(
                self.resources.join("native/bin/mac-hotkey"),
                &["pointer".into(), x.to_string(), y.to_string()],
            )
            .await?;
        }
        Ok(())
    }
    pub async fn screens(&self) -> Result<Value> {
        if platform() == "win32" {
            self.win("screens", serde_json::json!({})).await
        } else {
            Ok(serde_json::from_str(
                &self
                    .run(
                        self.resources.join("native/bin/mac-hotkey"),
                        &["screens".into()],
                    )
                    .await?,
            )?)
        }
    }
    pub async fn close(&self) {
        if let Some(mut h) = self.windows.lock().await.take() {
            let _ = h.child.kill().await;
        }
    }
}

fn parse_mac_displays(output: &str) -> Vec<Value> {
    let regex = regex::Regex::new(r"(?m)^\[(\d+)\][ \t]*(.*?)\s*\(([A-Fa-f0-9-]{36})\)[ \t]*$")
        .expect("static display regex");
    regex
        .captures_iter(output)
        .map(|m| serde_json::json!({"id":&m[3],"name":m[2].trim(),"readable":false}))
        .collect()
}

fn parse_input(output: &str) -> Option<u16> {
    let value = output.trim();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_scan_keeps_names_and_ignores_diagnostic_lines() {
        let output = "DDC probe message\n[1] Lenovo T2224r (12345678-1234-1234-1234-123456789ABC)\n[2] Broken (not-a-uuid)\n";
        let monitors = parse_mac_displays(output);
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0]["name"], "Lenovo T2224r");
        assert_eq!(monitors[0]["readable"], false);
        assert!(monitors[0].get("currentInput").is_none());
    }
    #[test]
    fn input_read_rejects_errors_overflow_and_trailing_output() {
        assert_eq!(parse_input(" 17\n"), Some(17));
        for text in ["", "65536", "-1", "17\nerror", "0x11", "unavailable"] {
            assert_eq!(parse_input(text), None);
        }
    }
}
