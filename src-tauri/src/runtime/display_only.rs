use super::*;

// No remote identity is available in standalone mode. Never store a made-up
// peer ID in display_device_id; it remains the full-sharing routing authority.
fn toggle_target(config: &Config, current: Option<u16>, last: Option<&str>) -> bool {
    match current {
        Some(value) if Some(value) == config.local_input => false,
        Some(value) if Some(value) == config.peer_input => true,
        _ => last == Some("peer"),
    }
}
impl Runtime {
    // Called with the lifecycle lock held, before any engine or network setup.
    pub(super) async fn start_display_only(&self) -> Result<()> {
        let state = self.snapshot().await;
        ensure!(
            state.config.mode == OperationMode::DisplayOnly,
            "当前不是仅切换显示器模式"
        );
        state.config.ready()?;
        ensure!(
            self.platform.available().await,
            "原生显示器控制组件未准备，请检查安装包"
        );
        self.discovery.lock().await.take();
        self.transfers.suspend().await;
        {
            let mut i = self.inner.lock().await;
            i.state.control_disconnected();
            i.state.listening = false;
            i.state.reset_display();
            i.state.running = true;
            i.state.last_error = None;
        }
        self.publish();
        self.log("info", "仅切换显示器已启用：不建立局域网连接，不启动 Deskflow，不同步键鼠。输入源指令由本机发送，能否往返切换取决于显示器和连接链路。实际画面未验证。" ).await;
        Ok(())
    }
    pub(super) async fn switch_display_only(&self, local: Option<bool>) -> Result<()> {
        // Button, tray, shortcut and stop must share the same lifecycle guard.
        // Repeated hotkeys while a DDC call is pending are discarded, not queued.
        let Ok(_lifecycle) = self.start_stop.try_lock() else {
            bail!("正在处理其他操作，请稍候再切换");
        };
        let state = self.snapshot().await;
        ensure!(
            state.config.mode == OperationMode::DisplayOnly && state.running,
            "请先启用仅切换显示器"
        );
        state.config.ready()?;
        let local = match local {
            Some(local) => local,
            None => {
                let read = self
                    .platform
                    .current_input(&state.config.display_id)
                    .await
                    .ok()
                    .flatten();
                if read.is_none() && state.display_target.is_none() {
                    self.log(
                        "info",
                        "当前输入源无法读取，首次快捷键将请求切到另一输入；画面请以实际显示为准",
                    )
                    .await;
                }
                toggle_target(&state.config, read, state.display_target.as_deref())
            }
        };
        let value = if local {
            state.config.local_input
        } else {
            state.config.peer_input
        }
        .context("输入源未配置")?;
        {
            let mut i = self.inner.lock().await;
            i.state.switching = true;
            i.state.display_target = None;
            i.state.display_device_id = None;
            i.state.display_status = "uncertain".into();
        }
        self.publish();
        // The only device effect in this path. No network RPC, injection or
        // cursor movement, even if an old pairing/engine configuration exists.
        let result = self.platform.input(&state.config.display_id, value).await;
        {
            let mut i = self.inner.lock().await;
            i.state.switching = false;
            if result.is_ok() {
                i.state.display_target = Some(if local { "local" } else { "peer" }.into());
                i.state.display_status = "commanded".into();
                i.state.last_error = None;
            } else {
                i.state.last_error = result.as_ref().err().map(|e| e.to_string());
            }
        }
        self.publish();
        self.trace("displayOnly.input", json!({"local":local,"value":value,"accepted":result.is_ok(),"pictureVerified":false,"error":result.as_ref().err().map(|e|e.to_string())})).await;
        result?;
        self.log(
            "info",
            "输入源指令已从本机发送；键鼠未转移，画面请以显示器实际显示为准",
        )
        .await;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toggle_prefers_read_value_then_last_command_and_handles_unknown() {
        let config = Config {
            local_input: Some(15),
            peer_input: Some(17),
            ..Config::default()
        };
        assert!(!toggle_target(&config, Some(15), Some("peer")));
        assert!(toggle_target(&config, Some(17), Some("local")));
        assert!(toggle_target(&config, None, Some("peer")));
        assert!(!toggle_target(&config, None, Some("local")));
        assert!(!toggle_target(&config, None, None));
        assert!(!toggle_target(&config, Some(99), None));
    }
    #[cfg(target_os = "macos")]
    async fn standalone_fixture() -> (tempfile::TempDir, Arc<Runtime>) {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "client");
        runtime.stop().await.unwrap();
        {
            let mut i = runtime.inner.lock().await;
            // Deliberately leave stale full-sharing settings and no engine,
            // no permission, no peer, and an unavailable retained screen.
            let config = Config {
                mode: OperationMode::DisplayOnly,
                role: "client".into(),
                display_id: "fake-monitor".into(),
                local_input: Some(15),
                peer_input: Some(17),
                deskflow_path: "/definitely/missing/deskflow-core".into(),
                retained_display_id: "disconnected".into(),
                windows_secure_input: true,
                ..Config::default()
            };
            i.store.value.config = config.clone();
            i.state.config = config;
            i.state.dependencies.accessibility = false;
            i.state.dependencies.deskflow = false;
        }
        let helper = runtime.platform.resources.join("native/bin/m1ddc");
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        // Isolated mock executable: records DDC requests to this temporary
        // directory only. Never opens IOKit, USB or a real display.
        std::fs::write(
            &helper,
            r#"#!/bin/sh
base="$(dirname "$0")"
if [ "$3" = "get" ]; then
  if [ -f "$base/current" ]; then cat "$base/current"; else exit 1; fi
else
  echo "$*" >> "$base/writes"
  if [ -f "$base/fail" ]; then exit 1; fi
  echo "$5" > "$base/current"
fi
"#,
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        (directory, runtime)
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn standalone_start_never_needs_pairing_engine_permissions_or_an_available_network_port()
    {
        let (_directory, runtime) = standalone_fixture().await;
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = occupied.local_addr().unwrap().port();
        runtime.inner.lock().await.state.config.control_port = port;
        runtime.start(None).await.unwrap();
        let s = runtime.snapshot().await;
        assert!(s.running);
        assert!(!s.connected && !s.listening && s.peer.is_none());
        assert_eq!(s.engine, "stopped");
        assert!(s.active_device_id.is_none());
        assert!(runtime.network.lock().await.is_none());
        assert!(runtime.engine.lock().await.is_none());
        assert!(runtime.link.lock().await.is_none());
        assert!(runtime.discovery.lock().await.is_none());
        assert_eq!(runtime.shortcut().await.as_deref(), Some("control+alt+F10"));
        assert!(runtime.start(None).await.is_err());
        runtime.stop().await.unwrap();
        assert!(runtime.shortcut().await.is_none());
        assert!(runtime.switch_to(true).await.is_err());
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn standalone_switches_and_hotkey_toggle_only_write_ddc_and_failure_can_be_retried() {
        let (_directory, runtime) = standalone_fixture().await;
        runtime.start(None).await.unwrap();
        runtime.switch_to(false).await.unwrap();
        let s = runtime.snapshot().await;
        assert_eq!(s.display_target.as_deref(), Some("peer"));
        assert!(s.active_device_id.is_none() && s.display_device_id.is_none());
        runtime.toggle().await.unwrap();
        assert_eq!(
            runtime.snapshot().await.display_target.as_deref(),
            Some("local")
        );
        let bin = runtime.platform.resources.join("native/bin");
        // Physical input changed independently: current VCP wins over history.
        std::fs::write(bin.join("current"), "17").unwrap();
        runtime.toggle().await.unwrap();
        assert_eq!(
            runtime.snapshot().await.display_target.as_deref(),
            Some("local")
        );
        std::fs::remove_file(bin.join("current")).unwrap();
        runtime.toggle().await.unwrap();
        assert_eq!(
            runtime.snapshot().await.display_target.as_deref(),
            Some("peer")
        );
        std::fs::write(bin.join("fail"), "").unwrap();
        assert!(runtime.switch_to(true).await.is_err());
        let failed = runtime.snapshot().await;
        assert_eq!(failed.display_status, "uncertain");
        assert!(
            failed.display_target.is_none() && !failed.switching && failed.last_error.is_some()
        );
        std::fs::remove_file(bin.join("fail")).unwrap();
        runtime.switch_to(true).await.unwrap();
        assert_eq!(runtime.snapshot().await.display_status, "commanded");
        assert!(runtime.snapshot().await.last_error.is_none());
        assert!(runtime.engine.lock().await.is_none() && runtime.link.lock().await.is_none());
        let writes = std::fs::read_to_string(bin.join("writes")).unwrap();
        assert_eq!(writes.lines().count(), 6);
        assert!(writes
            .lines()
            .all(|line| line.starts_with("display fake-monitor set input ")));
        runtime.stop().await.unwrap();
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn mode_change_is_blocked_while_running_and_keeps_full_sharing_settings() {
        let (_directory, runtime) = standalone_fixture().await;
        runtime.start(None).await.unwrap();
        let mut sharing = runtime.snapshot().await.config;
        sharing.mode = OperationMode::Sharing;
        assert!(runtime.save(sharing.clone()).await.is_err());
        runtime.switch_to(false).await.unwrap();
        runtime.stop().await.unwrap();
        runtime.save(sharing.clone()).await.unwrap();
        let s = runtime.snapshot().await;
        assert_eq!(s.config, sharing);
        assert!(s.display_target.is_none() && s.display_device_id.is_none());
        assert_eq!(s.display_status, "uninitialized");
        // Full sharing still validates the existing missing engine path rather
        // than accidentally entering the standalone path.
        assert!(runtime.start(None).await.is_err());
        assert!(!runtime.snapshot().await.running);
    }
}
