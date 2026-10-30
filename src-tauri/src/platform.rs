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
        let tool = file
            .file_name()
            .unwrap_or(file.as_os_str())
            .to_string_lossy();
        let invocation = format!("{tool} {}", args.join(" ")).trim().to_owned();
        let mut cmd = Command::new(file);
        cmd.args(args).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        let output = tokio::time::timeout(Duration::from_secs(7), cmd.output())
            .await
            .with_context(|| format!("原生调用超时：{invocation}"))?
            .map_err(|error| anyhow::anyhow!("无法启动原生工具：{invocation}；{error}"))?;
        ensure!(
            output.status.success(),
            "原生调用失败：{invocation}；退出状态={}；stderr={}；stdout={}",
            output.status,
            // m1ddc reports some I2C errors on stdout rather than stderr.
            native_output_detail(&output.stderr),
            native_output_detail(&output.stdout),
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
                bail!(
                    "原生调用失败：Windows 操作={action}；{}",
                    native_output_detail(error.as_bytes())
                )
            }
            Ok(reply["result"].clone())
        };
        let deadline = if action == "get" {
            Duration::from_millis(2500)
        } else {
            Duration::from_secs(8)
        };
        match tokio::time::timeout(deadline, result).await {
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
    pub async fn lock_session(&self) -> Result<()> {
        #[cfg(windows)]
        {
            #[link(name = "user32")]
            unsafe extern "system" {
                fn LockWorkStation() -> i32;
            }
            ensure!(
                unsafe { LockWorkStation() } != 0,
                "Windows 未接受锁屏请求：{}",
                std::io::Error::last_os_error()
            );
            return Ok(());
        }
        #[cfg(not(windows))]
        bail!("仅支持锁定 Windows 接收端")
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
    pub async fn current_input(&self, id: &str) -> Result<Option<u16>> {
        ensure!(
            !id.is_empty() && !id.contains(['\n', '\r', '\0']),
            "显示器标识无效"
        );
        if platform() == "win32" {
            let value = self.win("get", serde_json::json!({"monitorId":id})).await?;
            return Ok(value.as_u64().and_then(|v| u16::try_from(v).ok()));
        }
        ensure!(platform() == "darwin", "当前系统不支持 DDC");
        let output = tokio::time::timeout(
            Duration::from_millis(2500),
            self.run(
                self.resources.join("native/bin/m1ddc"),
                &["display".into(), id.into(), "get".into(), "input".into()],
            ),
        )
        .await
        .context("读取显示器输入源超时")??;
        Ok(parse_input(&output))
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

fn native_output_detail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let detail: String = text.trim().chars().take(700).collect();
    if detail.is_empty() {
        "未提供输出".into()
    } else {
        detail
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
    #[cfg(unix)]
    #[tokio::test]
    async fn native_failure_keeps_ddc_error_written_to_stdout() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let helper = directory.path().join("fake-ddc");
        std::fs::write(
            &helper,
            "#!/bin/sh\necho 'I2C communication failed'\nexit 2\n",
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let platform = Platform::new(directory.path().into());
        let error = platform.run(helper, &[]).await.unwrap_err();
        assert!(error.to_string().contains("I2C communication failed"));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn silent_native_failure_keeps_tool_operation_and_exit_status() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let helper = directory.path().join("fake-ddc");
        std::fs::write(&helper, "#!/bin/sh\nprintf '  '\nprintf '  ' >&2\nexit 7\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let platform = Platform::new(directory.path().into());
        let args = ["display", "external", "set", "input", "17"].map(String::from);
        let error = platform.run(helper, &args).await.unwrap_err().to_string();
        assert!(error.contains("fake-ddc display external set input 17"));
        assert!(error.contains("退出状态=exit status: 7"));
        assert!(error.contains("stderr=未提供输出；stdout=未提供输出"));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn native_failure_keeps_both_output_streams() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let helper = directory.path().join("fake-ddc");
        std::fs::write(
            &helper,
            "#!/bin/sh\necho 'DDC bus unavailable'\necho 'helper failed' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let platform = Platform::new(directory.path().into());
        let error = platform.run(helper, &[]).await.unwrap_err().to_string();
        assert!(error.contains("stderr=helper failed；stdout=DDC bus unavailable"));
    }
    #[test]
    fn native_error_details_are_nonempty_lossy_and_bounded() {
        assert_eq!(native_output_detail(b" \n\t"), "未提供输出");
        assert!(native_output_detail(&[0xff]).contains('\u{fffd}'));
        assert_eq!(
            native_output_detail("错".repeat(701).as_bytes())
                .chars()
                .count(),
            700
        );
    }
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
