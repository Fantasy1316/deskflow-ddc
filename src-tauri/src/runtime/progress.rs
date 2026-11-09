use super::*;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SwitchStage {
    RestoringTarget,
    TransferringInput,
    DisablingSource,
    Committing,
    WritingInput,
    WaitingDisplay,
    RestoringOutputs,
}
impl SwitchStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::RestoringTarget => "恢复目标输出",
            Self::TransferringInput => "转移键鼠",
            Self::DisablingSource => "停用原输出",
            Self::Committing => "保留输出状态",
            Self::WritingInput => "发送输入源指令",
            Self::WaitingDisplay => "等待显示器稳定",
            Self::RestoringOutputs => "恢复显示输出",
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SwitchProgress {
    pub stage: SwitchStage,
}
impl Runtime {
    pub(super) async fn switch_stage(&self, link: &Link, stage: SwitchStage) -> Result<()> {
        ensure!(self.current(link), "共享会话已结束");
        {
            let mut i = self.inner.lock().await;
            ensure!(
                self.current(link) && !i.state.emergency_stopping,
                "切换已取消，正在恢复输出"
            );
            i.state.switch_progress = Some(SwitchProgress { stage });
        }
        self.publish_control(link, None).await;
        Ok(())
    }

    /// Interrupt network/input waits before acquiring the ordinary action lock.
    /// Native output operations finish under their existing deadlines and locks;
    /// never drop a half-applied display operation and race it with restoration.
    pub async fn emergency_stop(&self) -> Result<()> {
        self.interrupt_switch_wait();
        {
            let mut i = self.inner.lock().await;
            ensure!(!i.state.emergency_stopping, "正在停止共享并恢复输出");
            i.state.emergency_stopping = true;
            i.state.switch_progress = Some(SwitchProgress {
                stage: SwitchStage::RestoringOutputs,
            });
        }
        self.publish();
        self.epoch.fetch_add(1, Ordering::AcqRel);
        if let Some((cancel, _)) = self.network.lock().await.as_ref() {
            cancel.cancel();
        }
        if let Some(link) = self.link.lock().await.as_ref() {
            link.session.close();
        }
        self.topology_ready.send_replace(true);
        let _action = self.actions.lock().await;
        let result = self.stop().await;
        {
            let mut i = self.inner.lock().await;
            i.state.emergency_stopping = false;
            i.state.switch_progress = None;
            i.state.last_error = result.as_ref().err().map(|e| format!("{e:#}"));
        }
        self.publish();
        self.trace(
            "sharing.emergencyStop",
            json!({"restored":result.is_ok(),"pictureVerified":false}),
        )
        .await;
        if result.is_ok() {
            self.log(
                "info",
                "共享已停止，已请求恢复显示输出；请以显示器实际画面为准",
            )
            .await;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn progress_is_synced_to_new_peers_without_changing_legacy_ddc_frames() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = super::super::pointer_bridge::tests::fixture(dir.path(), "host");
        let (remote, mut incoming) =
            super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
        let link = runtime.link.lock().await.clone().unwrap();
        runtime
            .switch_stage(&link, SwitchStage::TransferringInput)
            .await
            .unwrap();
        let Some(Incoming::Event { payload, .. }) = incoming.recv().await else {
            panic!("missing display state")
        };
        assert!(payload.get("switchProgress").is_none());
        link.peer.send_replace(Some(super::super::tests::peer()));
        runtime
            .switch_stage(&link, SwitchStage::WritingInput)
            .await
            .unwrap();
        let Some(Incoming::Event { payload, .. }) = incoming.recv().await else {
            panic!("missing display state")
        };
        assert_eq!(payload["switchProgress"]["stage"], "writing-input");
        for (method, wire_stage) in [
            (DisplayMethod::Ddc, "writing-input"),
            (DisplayMethod::Signal, "committing"),
        ] {
            runtime.inner.lock().await.state.config.display_method = method;
            runtime
                .switch_stage(&link, SwitchStage::WaitingDisplay)
                .await
                .unwrap();
            let Some(Incoming::Event { payload, .. }) = incoming.recv().await else {
                panic!("missing settling state")
            };
            assert_eq!(payload["switchProgress"]["stage"], wire_stage);
            assert_eq!(
                runtime.snapshot().await.switch_progress.unwrap().stage,
                SwitchStage::WaitingDisplay
            );
        }
        runtime.inner.lock().await.state.emergency_stopping = true;
        assert!(runtime
            .switch_stage(&link, SwitchStage::Committing)
            .await
            .is_err());
        remote.close();
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[tokio::test]
    async fn scanning_a_draft_method_never_saves_it_or_touches_video_output() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let runtime = super::super::pointer_bridge::tests::fixture(dir.path(), "host");
        runtime.inner.lock().await.state.running = false;
        let config = runtime.snapshot().await.config;
        assert_eq!(config.display_method, DisplayMethod::Ddc);
        let helper = runtime.platform.resources.join("native/bin/mac-output");
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(&helper, "#!/bin/sh\ntest \"$1\" = list || exit 9\nprintf '%s\\n' '[{\"id\":\"shared-output\",\"internal\":false}]'\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        runtime.scan_displays(DisplayMethod::Signal).await.unwrap();
        let state = runtime.snapshot().await;
        assert_eq!(state.config, config);
        assert_eq!(runtime.inner.lock().await.store.value.config, config);
        assert_eq!(state.output_displays[0]["id"], "shared-output");
        assert!(runtime.output_lease.lock().await.is_none());
    }

    #[tokio::test]
    async fn emergency_interrupts_rpc_before_waiting_for_the_ordinary_action_lock() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = super::super::pointer_bridge::tests::fixture(dir.path(), "host");
        let (remote, mut incoming) =
            super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
        let link = runtime.link.lock().await.clone().unwrap();
        let request = tokio::spawn(async move {
            link.session
                .request("test", Value::Null, Duration::from_secs(60))
                .await
        });
        assert!(matches!(
            incoming.recv().await,
            Some(Incoming::Request { .. })
        ));
        let action = runtime.actions.lock().await;
        let rt = runtime.clone();
        let mut stop = tokio::spawn(async move { rt.emergency_stop().await });
        let interrupted = tokio::time::timeout(Duration::from_secs(1), request)
            .await
            .unwrap()
            .unwrap();
        assert!(interrupted.is_err());
        assert!(runtime.snapshot().await.emergency_stopping);
        assert!(runtime.emergency_stop().await.is_err());
        assert!(tokio::time::timeout(Duration::from_millis(30), &mut stop)
            .await
            .is_err());
        drop(action);
        tokio::time::timeout(Duration::from_secs(2), stop)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let state = runtime.snapshot().await;
        assert!(!state.running && !state.connected && !state.emergency_stopping);
        assert!(state.switch_progress.is_none());
        remote.close();
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[tokio::test]
    async fn interrupted_output_switch_cannot_commit_after_emergency_recovery() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "host");
        let local = runtime.snapshot().await.device_id;
        {
            let mut i = runtime.inner.lock().await;
            i.state.config.display_method = DisplayMethod::Signal;
            i.state.config.output_id = uuid::Uuid::new_v4().to_string();
            i.state.engine = "ready".into();
            // No keyboard injection: input already belongs to the target.
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
        let (remote, mut incoming) =
            super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
        let rt = runtime.clone();
        let switch = tokio::spawn(async move {
            let _action = rt.actions.lock().await;
            let link = rt.link.lock().await.clone().unwrap();
            rt.command_output_switch(&link, Some(local)).await
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(Incoming::Request {
                    method, payload, ..
                }) = incoming.recv().await
                {
                    assert_eq!(method, "output-action");
                    assert_eq!(payload, "disable");
                    break;
                }
            }
        })
        .await
        .unwrap();
        // Leave the remote disable unanswered, then interrupt its RPC wait.
        tokio::time::timeout(Duration::from_secs(3), runtime.emergency_stop())
            .await
            .unwrap()
            .unwrap();
        assert!(switch.await.unwrap().is_err());
        while let Some(message) = incoming.recv().await {
            assert!(!matches!(message, Incoming::Request { .. }));
        }
        let state = runtime.snapshot().await;
        assert!(!state.running && !state.switching && !state.emergency_stopping);
        assert!(state.output_switch.is_none() && state.switch_progress.is_none());
        assert!(state.display_device_id.is_none());
        assert!(!runtime.command_pending.load(Ordering::Acquire));
        remote.close();
    }
}
