use super::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SwitchRecord {
    pub token: uuid::Uuid,
    pub target: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreRequest {
    pub token: uuid::Uuid,
}

fn resolve_target(state: &Snapshot, requested: Option<String>) -> Result<String> {
    let peer = &state.peer.as_ref().context("另一台电脑尚未连接")?.id;
    let target = match requested {
        Some(target) => target,
        None => match state.display_device_id.as_deref() {
            Some(source) if source == state.device_id => peer.clone(),
            Some(source) if source == peer => state.device_id.clone(),
            _ => bail!(
                "无法查询显示器当前输入；请先用明确目标的切换按钮选择画面，切换完成后再使用切换快捷键"
            ),
        },
    };
    ensure!(
        target == state.device_id || &target == peer,
        "无效的切换目标"
    );
    Ok(target)
}

fn validate_keyboard_owner(state: &Snapshot, target: &str) -> Result<()> {
    ensure!(
        !state.emergency_stopping
            && state.engine == "ready"
            && state.active_device_id.as_deref() == Some(target),
        "未确认目标键鼠归属，或键鼠已发生新的跨屏，请重试"
    );
    Ok(())
}

fn record_completed_switch(state: &mut Snapshot, target: &str) -> Result<()> {
    validate_keyboard_owner(state, target)?;
    state.output_switch = Some(SwitchRecord {
        token: uuid::Uuid::new_v4(),
        target: target.into(),
    });
    state.display_device_id = Some(target.into());
    state.last_display_device_id = Some(target.into());
    state.display_status = "commanded".into();
    state.display_source = None;
    Ok(())
}

fn validate_retained_output(config: &Config, outputs: &Value) -> Result<()> {
    if config.retained_display_id.is_empty() {
        return Ok(());
    }
    let selected = outputs
        .as_array()
        .and_then(|values| values.iter().find(|v| v["id"] == config.output_id));
    let cursor_id = selected
        .and_then(|v| v["cursorId"].as_str())
        .unwrap_or(&config.output_id);
    ensure!(
        cursor_id != config.retained_display_id,
        "共用输出不能同时用作独立屏幕，请选择另一块始终可用的屏幕"
    );
    Ok(())
}

async fn prepare_outputs<F, Fut>(target: &str, source: &str, mut action: F) -> Result<()>
where
    F: FnMut(String, &'static str) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    action(target.into(), "enable").await?;
    // A provider-side injected switch hotkey can wake a sleeping Windows
    // source, so transfer input BEFORE putting the original output to sleep.
    action(target.into(), "keyboard").await?;
    action(source.into(), "disable").await?;
    action(source.into(), "confirm").await
}

impl Runtime {
    pub(super) async fn check_output_selection(&self, config: &Config) -> Result<()> {
        validate_retained_output(config, &self.platform.output_list().await?)?;
        self.platform.capture_output(&config.output_id).await?;
        Ok(())
    }

    pub async fn check_output(&self) -> Result<()> {
        let s = self.snapshot().await;
        ensure!(
            !s.running && s.config.display_method == DisplayMethod::Signal,
            "请停止共享并保存自动选源设置后再检测"
        );
        let result = self.check_output_selection(&s.config).await;
        self.inner.lock().await.state.output_check = Some(json!({
            "id":s.config.output_id,"available":result.is_ok(),
            "detail":result.as_ref().err().map(|e|format!("{e:#}")).unwrap_or_else(|| "本机输出接口可用，未断开输出；自动选源与回切仍需实测".into())
        }));
        self.publish();
        result.map(|_| ())
    }

    pub(super) async fn restore_local_output(&self) -> Result<()> {
        let mut guard = self.output_lease.lock().await;
        let had_lease = guard.is_some();
        let result = if let Some(lease) = guard.as_mut() {
            // Keep the guardian and its captured baseline on failure so the
            // next request performs a real recovery instead of reporting success.
            lease.restore().await.context("恢复显示输出失败")
        } else {
            Ok(())
        };
        if result.is_ok() {
            *guard = None;
        }
        self.inner.lock().await.state.local_output_recovery_pending = result.is_err();
        drop(guard);
        self.publish();
        if had_lease && result.is_ok() {
            self.trace("output.restored", json!({"pictureVerified":false}))
                .await;
        }
        result
    }

    pub(super) async fn local_output_action(&self, operation: &str) -> Result<()> {
        let s = self.snapshot().await;
        ensure!(
            s.config.display_method == DisplayMethod::Signal,
            "本机未启用自动选源兜底"
        );
        match operation {
            "enable" => {
                self.restore_local_output().await?;
                // Presence is a readiness check only; never picture verification.
                self.platform.capture_output(&s.config.output_id).await?;
            }
            "disable" => {
                let mut guard = self.output_lease.lock().await;
                if let Some(lease) = guard.as_mut() {
                    // Explicit buttons may reassert the same target after a
                    // physical input change. Re-arm the pre-commit recovery deadline.
                    return lease.command("disable").await;
                }
                let mut lease = crate::output_guard::Lease::start(
                    &self.platform.resources,
                    &s.config.output_id,
                )
                .await?;
                // Keep the lease even on partial failure; callers restore both
                // endpoints, and dropping this Runtime delivers EOF independently.
                let result = lease.command("disable").await;
                *guard = Some(lease);
                result?;
            }
            "confirm" => {
                self.output_lease
                    .lock()
                    .await
                    .as_mut()
                    .context("输出已恢复，不能保留旧切换")?
                    .command("confirm")
                    .await?;
            }
            "restore" => self.restore_local_output().await?,
            _ => bail!("不支持的显示输出指令"),
        }
        self.trace(
            "output.operation",
            json!({"operation":operation,"output":s.config.output_id,"pictureVerified":false}),
        )
        .await;
        Ok(())
    }

    async fn output_action(&self, link: &Link, device: &str, operation: &str) -> Result<()> {
        ensure!(self.current(link), "共享会话已结束");
        if device == self.snapshot().await.device_id {
            self.local_output_action(operation).await
        } else {
            link.session
                .request("output-action", json!(operation), Duration::from_secs(15))
                .await
                .map(|_| ())
        }
    }

    pub(super) async fn restore_outputs(&self, link: &Link) -> Result<()> {
        let (local, remote) = tokio::join!(
            self.restore_local_output(),
            link.session
                .request("output-action", json!("restore"), Duration::from_secs(15))
        );
        // Both restorations are attempted even if one fails.
        match (local, remote) {
            (Ok(()), Ok(_)) => Ok(()),
            (local, remote) => bail!(
                "恢复显示输出：本机={}；对端={}。请检查原输出，必要时使用显示器实体按键恢复",
                local
                    .err()
                    .map(|e| format!("{e:#}"))
                    .unwrap_or_else(|| "已请求恢复".into()),
                remote
                    .err()
                    .map(|e| format!("{e:#}"))
                    .unwrap_or_else(|| "已请求恢复".into())
            ),
        }
    }

    pub(super) async fn command_output_switch(
        &self,
        link: &Arc<Link>,
        requested: Option<String>,
    ) -> Result<()> {
        let _command = self
            .commands
            .try_lock()
            .map_err(|_| anyhow::anyhow!("正在切换，请稍后"))?;
        let s = self.snapshot().await;
        ensure!(
            self.current(link) && s.config.role == "host" && s.engine == "ready",
            "两端共享尚未就绪"
        );
        ensure!(
            self.display_recovery.lock().await.pending().is_none(),
            "刚唤醒或显示连接发生变化，正在恢复输出并重新连接，请稍后重试"
        );
        let target = resolve_target(&s, requested)?;
        let protection = self.switch_protection.enter(Some(&target))?;
        let source = if target == s.device_id {
            s.peer.as_ref().unwrap().id.clone()
        } else {
            s.device_id.clone()
        };
        self.command_pending.store(true, Ordering::Release);
        let result: Result<()> = async {
            {
                let _effects = self.effects.lock().await;
                self.inner.lock().await.state.switching = true;
                self.publish_control(link, None).await;
            }
            // First restore the target, then transfer input, and only then
            // remove the original source. Never inject a hotkey after sleep.
            prepare_outputs(&target, &source, |device, operation| {
                let target = target.clone();
                async move {
                    let stage = match operation {
                        "enable" => SwitchStage::RestoringTarget,
                        "keyboard" => SwitchStage::TransferringInput,
                        "disable" => SwitchStage::DisablingSource,
                        _ => SwitchStage::Committing,
                    };
                    self.switch_stage(link, stage).await?;
                    let result = if operation == "keyboard" {
                        self.inner.lock().await.state.switching = false;
                        self.keyboard_switch(link, device, None).await
                    } else {
                        let _effects = self.effects.lock().await;
                        ensure!(self.current(link), "共享会话已结束");
                        let mut i = self.inner.lock().await;
                        if operation == "disable" || operation == "confirm" {
                            validate_keyboard_owner(&i.state, &target)?;
                        }
                        i.state.switching = true;
                        drop(i);
                        self.output_action(link, &device, operation).await
                    };
                    result.with_context(|| format!("{}失败", stage.label()))
                }
            })
            .await?;
            {
                let _effects = self.effects.lock().await;
                ensure!(self.current(link), "共享会话已结束");
                let mut i = self.inner.lock().await;
                record_completed_switch(&mut i.state, &target)?;
                i.state.switching = false;
                i.state.switch_progress = None;
            }
            Ok(())
        }
        .await;
        if result.is_err() && self.current(link) {
            let _ = self.switch_stage(link, SwitchStage::RestoringOutputs).await;
            let restored = self.restore_outputs(link).await;
            if self.current(link) {
                if let Some(from) = &s.active_device_id {
                    let _effects = self.effects.lock().await;
                    if self.snapshot().await.active_device_id.as_ref() != Some(from) {
                        let restored = self.inject_wait(link, from).await.is_ok();
                        self.inner.lock().await.state.active_device_id =
                            restored.then(|| from.clone());
                    }
                }
            }
            if self.current(link) {
                let mut i = self.inner.lock().await;
                i.state.reset_display();
                i.state.switching = false;
                i.state.switch_progress = None;
                i.state.display_status = "uncertain".into();
                i.state.display_source = None;
                if restored.is_err() {
                    i.state.output_switch = Some(SwitchRecord {
                        token: uuid::Uuid::new_v4(),
                        target: target.clone(),
                    });
                }
            }
            if let Err(error) = restored {
                self.log("error", &format!("{error:#}")).await;
            }
        }
        self.command_pending.store(false, Ordering::Release);
        if self.current(link) {
            self.settle_display(&protection, Some(link), &target, result.is_ok())
                .await;
        }
        if self.current(link) {
            let mut i = self.inner.lock().await;
            i.state.switching = false;
            i.state.switch_progress = None;
            drop(protection);
        } else {
            drop(protection);
        }
        self.publish_control(link, result.as_ref().err().map(|e| format!("{e:#}")))
            .await;
        if result.is_ok() {
            self.trace("output.completed", json!({"target":target,"keyboardConfirmed":true,"outputCommandsAccepted":true,"pictureVerified":false})).await;
            self.log("success", "已完成输出与键鼠切换，无需手动确认；若显示器未自动选源，可重试或恢复输出。软件无法直接读取实际输入源。").await;
        }
        result
    }

    pub async fn restore_output(&self, request: RestoreRequest) -> Result<()> {
        let link = self
            .link
            .lock()
            .await
            .clone()
            .context("另一台电脑尚未连接")?;
        let s = self.snapshot().await;
        ensure!(
            s.config.display_method == DisplayMethod::Signal,
            "本机未启用自动选源兜底"
        );
        if s.config.role != "host" {
            return link
                .session
                .request(
                    "restore-output",
                    json!({"token":request.token}),
                    Duration::from_secs(20),
                )
                .await
                .map(|_| ());
        }
        let _command = self
            .commands
            .try_lock()
            .map_err(|_| anyhow::anyhow!("正在切换，请稍后"))?;
        let _effects = self.effects.lock().await;
        ensure!(self.current(&link), "共享会话已结束");
        let completed = self
            .snapshot()
            .await
            .output_switch
            .context("没有可恢复的输出切换")?;
        ensure!(
            completed.token == request.token,
            "切换记录已更新，请使用最新的恢复按钮"
        );
        self.switch_stage(&link, SwitchStage::RestoringOutputs)
            .await?;
        self.command_pending.store(true, Ordering::Release);
        let result = self.restore_outputs(&link).await;
        self.command_pending.store(false, Ordering::Release);
        ensure!(self.current(&link), "共享会话已结束");
        {
            let mut i = self.inner.lock().await;
            // Retain recovery controls if either endpoint failed to restore.
            i.state.switch_progress = None;
            i.state.display_device_id = None;
            i.state.last_display_device_id = None;
            i.state.display_status = "uncertain".into();
            i.state.display_source = None;
            if result.is_ok() {
                i.state.output_switch = None;
            }
        }
        self.trace(
            "output.recovery",
            json!({"target":completed.target,"restored":result.is_ok(),"pictureVerified":false}),
        )
        .await;
        self.publish_control(&link, result.as_ref().err().map(|e| format!("{e:#}")))
            .await;
        if result.is_ok() {
            self.log("info", "两端输出已请求恢复；当前实际输入未知，请用明确目标的按钮重试或使用显示器实体按键。").await;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn stopped_and_disconnected_output_recovery_survives_session_and_config_resets() {
        for role in ["host", "client"] {
            for cleanup in ["stop", "disconnect", "emergency"] {
                let dir = tempfile::tempdir().unwrap();
                let runtime = super::super::pointer_bridge::tests::fixture(dir.path(), role);
                {
                    let mut i = runtime.inner.lock().await;
                    i.state.config.display_method = DisplayMethod::Signal;
                    i.state.config.output_id = "original-output".into();
                    i.state.output_switch = Some(SwitchRecord {
                        token: uuid::Uuid::new_v4(),
                        target: "receiver".into(),
                    });
                }
                *runtime.output_lease.lock().await =
                    Some(crate::output_guard::tests::retrying_lease(dir.path(), 2).await);
                match cleanup {
                    "stop" => assert!(runtime.stop().await.is_err()),
                    "emergency" => assert!(runtime.emergency_stop().await.is_err()),
                    _ => runtime.cleanup_session(1, 1).await,
                }
                let stopped = runtime.snapshot().await;
                assert!(stopped.local_output_recovery_pending);
                assert_eq!(
                    serde_json::to_value(&stopped).unwrap()["localOutputRecoveryPending"],
                    true
                );
                assert!(!stopped.connected);
                assert_eq!(stopped.running, cleanup == "disconnect");
                assert!(!stopped.emergency_stopping);
                assert!(stopped.output_switch.is_none());
                assert!(runtime.output_lease.lock().await.is_some());

                // Pairing, wake and display settings reset routing, but cannot
                // discard the old guardian's captured baseline or retry entry.
                {
                    let mut i = runtime.inner.lock().await;
                    i.state.pairing_changed(protocol::new_pairing());
                    i.state.config.output_id = "replacement-output".into();
                    i.state.config.display_method = DisplayMethod::Ddc;
                    i.state.reset_display();
                }
                // The renderer and tray both dispatch Stop for local recovery.
                // It works without a peer and never launches a native helper.
                assert!(runtime.stop().await.is_err());
                assert!(runtime.snapshot().await.local_output_recovery_pending);
                runtime.stop().await.unwrap();
                runtime.stop().await.unwrap(); // Successful recovery is idempotent.
                let recovered = runtime.snapshot().await;
                assert!(!recovered.local_output_recovery_pending);
                assert!(!recovered.running);
                assert!(!recovered.connected);
                assert_eq!(recovered.engine, "stopped");
                assert!(recovered.active_device_id.is_none());
                assert!(runtime.output_lease.lock().await.is_none());
                assert!(runtime.engine.lock().await.is_none());
                assert!(!runtime.platform.resources.exists());
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("retrying-guardian.calls")).unwrap(),
                    "restore\nrestore\nrestore\n"
                );
            }
        }
    }

    #[tokio::test]
    async fn target_readiness_failure_never_disables_the_original_output() {
        use std::sync::Mutex as SyncMutex;
        for fail in [
            None,
            Some("enable"),
            Some("keyboard"),
            Some("disable"),
            Some("confirm"),
        ] {
            let calls = SyncMutex::new(Vec::new());
            let result = prepare_outputs("target", "original", |device, operation| {
                calls.lock().unwrap().push((device, operation));
                async move {
                    ensure!(fail != Some(operation), "simulated failure");
                    Ok(())
                }
            })
            .await;
            let calls = calls.into_inner().unwrap();
            assert_eq!(calls[0], ("target".into(), "enable"));
            let expected = match fail {
                Some("enable") => 1,
                Some("keyboard") => 2,
                Some("disable") => 3,
                _ => 4,
            };
            assert_eq!(calls.len(), expected);
            if calls.len() >= 2 {
                assert_eq!(calls[1], ("target".into(), "keyboard"));
            }
            if calls.len() >= 3 {
                assert_eq!(calls[2], ("original".into(), "disable"));
            }
            if calls.len() == 4 {
                assert_eq!(calls[3], ("original".into(), "confirm"));
            }
            assert_eq!(result.is_ok(), fail.is_none());
        }
    }

    #[test]
    fn shared_output_cannot_be_the_retained_screen_on_either_platform() {
        let mut c = Config {
            output_id: "uuid".into(),
            retained_display_id: "123".into(),
            ..Config::default()
        };
        assert!(validate_retained_output(&c, &json!([{"id":"uuid","cursorId":"123"}])).is_err());
        c.output_id = "win-monitor-abc".into();
        c.retained_display_id = c.output_id.clone();
        assert!(validate_retained_output(&c, &json!([])).is_err());
        c.retained_display_id = "internal".into();
        assert!(validate_retained_output(&c, &json!([])).is_ok());
    }

    #[test]
    fn completion_requires_confirmed_keyboard_owner_and_allows_repeated_switches() {
        let mut state = super::super::tests::snapshot();
        state.engine = "ready".into();
        assert!(record_completed_switch(&mut state, "receiver").is_err());
        assert!(state.output_switch.is_none());
        assert!(state.display_device_id.is_none());
        state.active_device_id = Some("receiver".into());
        record_completed_switch(&mut state, "receiver").unwrap();
        let first = state.output_switch.as_ref().unwrap().token;
        assert_eq!(state.display_device_id.as_deref(), Some("receiver"));
        assert_eq!(state.last_display_device_id.as_deref(), Some("receiver"));
        assert_eq!(state.display_status, "commanded");
        let local = state.device_id.clone();
        state.active_device_id = Some(local.clone());
        record_completed_switch(&mut state, &local).unwrap();
        assert_ne!(state.output_switch.as_ref().unwrap().token, first);
        assert_eq!(state.last_display_device_id.as_ref(), Some(&local));
    }

    #[tokio::test]
    async fn recovery_rejects_stale_tokens_and_retains_controls_on_remote_failure() {
        for mode in ["restore", "stale", "remote-error", "client"] {
            let directory = tempfile::tempdir().unwrap();
            let runtime = super::super::pointer_bridge::tests::fixture(
                directory.path(),
                if mode == "client" { "client" } else { "host" },
            );
            let token = uuid::Uuid::new_v4();
            {
                let mut i = runtime.inner.lock().await;
                i.state.config.display_method = DisplayMethod::Signal;
                i.state.output_switch = Some(SwitchRecord {
                    token,
                    target: i.state.device_id.clone(),
                });
            }
            let (remote, mut requests) =
                super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
            let server = tokio::spawn(async move {
                let mut operations = vec![];
                while let Some(message) = requests.recv().await {
                    if let Incoming::Request {
                        id,
                        method,
                        payload,
                    } = message
                    {
                        if mode == "client" {
                            assert_eq!(method, "restore-output");
                            assert_eq!(payload, json!({"token":token}));
                        } else {
                            assert_eq!(method, "output-action");
                            assert_eq!(payload, "restore");
                        }
                        operations.push(method);
                        let reply = if mode == "remote-error" {
                            Err(anyhow::anyhow!("fake restore failure"))
                        } else {
                            Ok(json!(true))
                        };
                        if remote.reply(id, reply).await.is_err() {
                            break;
                        }
                    }
                }
                operations
            });
            let result = runtime
                .restore_output(RestoreRequest {
                    token: if mode == "stale" {
                        uuid::Uuid::new_v4()
                    } else {
                        token
                    },
                })
                .await;
            runtime.link.lock().await.as_ref().unwrap().session.close();
            let operations = server.await.unwrap();
            let state = runtime.snapshot().await;
            match mode {
                "restore" => {
                    result.unwrap();
                    assert_eq!(operations, ["output-action"]);
                    assert!(state.output_switch.is_none());
                    assert!(state.display_device_id.is_none());
                }
                "stale" => {
                    assert!(result.is_err());
                    assert!(operations.is_empty());
                    assert_eq!(state.output_switch.unwrap().token, token);
                }
                "client" => {
                    result.unwrap();
                    assert_eq!(operations, ["restore-output"]);
                }
                _ => {
                    assert!(result.is_err());
                    assert_eq!(operations, ["output-action"]);
                    assert_eq!(state.output_switch.unwrap().token, token);
                    assert!(state.display_device_id.is_none());
                    assert_eq!(state.display_status, "uncertain");
                }
            }
        }
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[tokio::test]
    async fn automatic_commit_requires_guard_acceptance_and_failure_restores_both_outputs() {
        use std::os::unix::fs::PermissionsExt;
        for mode in ["complete", "confirm-fails", "restore-fails"] {
            let fail = mode != "complete";
            let directory = tempfile::tempdir().unwrap();
            let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "host");
            let local = runtime.snapshot().await.device_id;
            {
                let mut i = runtime.inner.lock().await;
                i.state.config.display_method = DisplayMethod::Signal;
                i.state.config.output_id = uuid::Uuid::new_v4().to_string();
                i.state.engine = "ready".into();
                // Already-owned input avoids generating any real input events.
                i.state.active_device_id = Some(local.clone());
                i.state.peer = Some(Peer {
                    id: "receiver".into(),
                    name: "Receiver".into(),
                    platform: "win32".into(),
                    fingerprint: "a".repeat(64),
                    file_transfer: false,
                    lock_control: false,
                    input_configured: true,
                    retained_display: false,
                    modifier_mapping: ModifierMapping::Original,
                });
            }
            let helper = runtime.platform.resources.join("native/bin/mac-output");
            std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
            std::fs::write(
                &helper,
                "#!/bin/sh\ntest \"$1\" = capture || exit 9\nprintf '%s\\n' '{}'\n",
            )
            .unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
            let (remote, mut requests) =
                super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
            let server = tokio::spawn(async move {
                let mut operations = vec![];
                while let Some(message) = requests.recv().await {
                    if let Incoming::Request {
                        id,
                        method,
                        payload,
                    } = message
                    {
                        assert_eq!(method, "output-action");
                        operations.push(payload.as_str().unwrap().to_owned());
                        let result = if (fail && payload == "confirm")
                            || (mode == "restore-fails" && payload == "restore")
                        {
                            Err(anyhow::anyhow!("guardian already restored"))
                        } else {
                            Ok(json!(true))
                        };
                        if remote.reply(id, result).await.is_err() {
                            break;
                        }
                    }
                }
                operations
            });
            let link = runtime.link.lock().await.clone().unwrap();
            let result = runtime
                .command_output_switch(&link, Some(local.clone()))
                .await;
            let state = runtime.snapshot().await;
            if fail {
                assert!(result.is_err());
                assert_eq!(state.output_switch.is_some(), mode == "restore-fails");
                assert!(state.display_device_id.is_none());
                assert_eq!(state.display_status, "uncertain");
            } else {
                result.unwrap();
                let record = state.output_switch.as_ref().unwrap();
                assert_eq!(record.target, local);
                assert_eq!(state.display_device_id.as_ref(), Some(&local));
                assert_eq!(state.last_display_device_id.as_ref(), Some(&local));
                // A completed record must not block an immediate explicit retry.
                runtime
                    .command_output_switch(&link, Some(local.clone()))
                    .await
                    .unwrap();
                assert_ne!(
                    runtime.snapshot().await.output_switch.unwrap().token,
                    record.token
                );
            }
            assert!(!state.switching);
            assert!(!runtime.command_pending.load(Ordering::Acquire));
            link.session.close();
            assert_eq!(
                server.await.unwrap(),
                if fail {
                    vec!["disable", "confirm", "restore"]
                } else {
                    vec!["disable", "confirm", "disable", "confirm"]
                }
            );
        }
    }

    #[test]
    fn unknown_source_requires_explicit_target_instead_of_guessing() {
        let mut s = super::super::tests::snapshot();
        s.peer = Some(Peer {
            id: "receiver".into(),
            name: "peer".into(),
            platform: "win32".into(),
            fingerprint: "a".repeat(64),
            file_transfer: false,
            lock_control: false,
            input_configured: true,
            retained_display: false,
            modifier_mapping: ModifierMapping::Original,
        });
        s.display_device_id = None;
        assert!(resolve_target(&s, None).is_err());
        assert_eq!(
            resolve_target(&s, Some("receiver".into())).unwrap(),
            "receiver"
        );
        assert!(resolve_target(&s, Some("stranger".into())).is_err());
        s.display_device_id = Some("receiver".into());
        assert_eq!(resolve_target(&s, None).unwrap(), s.device_id);
    }
}
