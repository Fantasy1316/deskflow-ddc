use super::*;
use std::time::{Instant, SystemTime};

#[derive(Debug)]
pub(super) struct ReadbackUnavailable(anyhow::Error);
impl std::fmt::Display for ReadbackUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "输入源指令已发送，但暂时无法确认是否生效；已保留键鼠转移结果，请确认实际画面后重试：{}", self.0)
    }
}
impl std::error::Error for ReadbackUnavailable {}

// Sleep can make a monitor auto-select another input without any DDC command.
// Keep the source invalidated until a fresh read, regardless of wake duration.
#[derive(Default)]
pub(super) struct DisplayRecovery {
    last_activity: Option<SystemTime>,
    pending: Option<Instant>,
    detected: Option<Instant>,
}

impl DisplayRecovery {
    pub(super) fn observe(&mut self, now: SystemTime) -> bool {
        // Wall time includes sleep on macOS, unlike some monotonic clocks.
        let resumed = self.last_activity.replace(now).is_some_and(|previous| {
            now.duration_since(previous)
                .is_ok_and(|gap| gap >= Duration::from_secs(5))
        });
        if resumed {
            self.mark();
        }
        resumed
    }

    pub(super) fn mark(&mut self) {
        self.pending = Some(Instant::now());
        self.detected = None;
    }

    pub(super) fn pending(&self) -> Option<Instant> {
        self.pending
    }

    pub(super) fn complete(&mut self, token: Instant) {
        // A second topology recovery during the writes must remain pending.
        if self.pending == Some(token) {
            self.pending = None;
            self.detected = None;
        }
    }
}

fn agree_source(local: Option<String>, remote: Option<String>) -> Result<String> {
    match (local, remote) {
        (Some(a), Some(b)) if a != b => bail!("两端读取的输入源不一致，暂时无法确认当前输入源，请稍后重试"),
        (Some(source), _) | (_, Some(source)) => Ok(source),
        _ => bail!("显示器连接可能尚未恢复，暂时无法读取当前输入源，请稍后重试；不会依据休眠前的记录猜测切换方向"),
    }
}

impl Runtime {
    pub(super) async fn query_display_source(&self, link: &Link) -> Result<String> {
        let s = self.snapshot().await;
        let peer = &s.peer.as_ref().context("对端未连接")?.id;
        let (local, remote) = tokio::join!(
            self.read_display_source(peer),
            link.session
                .request("get-input-source", Value::Null, Duration::from_secs(4))
        );
        ensure!(self.current(link), "共享会话已结束");
        let remote = remote.and_then(|value| {
            let source: Option<String> = serde_json::from_value(value)?;
            ensure!(
                source
                    .as_ref()
                    .is_none_or(|id| id == &s.device_id || id == peer),
                "无效的输入源设备"
            );
            Ok(source)
        });
        self.trace("display.sourceRead", json!({"local":local.as_ref().ok(),"remote":remote.as_ref().ok(),
            "localError":local.as_ref().err().map(|e|e.to_string()),"remoteError":remote.as_ref().err().map(|e|e.to_string()),"pictureVerified":false})).await;
        agree_source(local.ok().flatten(), remote.ok().flatten())
    }

    async fn detect_resumed_source(&self, link: &Link, token: Instant) -> Result<String> {
        let source = self.query_display_source(link).await?;
        let mut recovery = self.display_recovery.lock().await;
        ensure!(
            recovery.pending() == Some(token),
            "显示器连接再次变化，请稍后重试"
        );
        let mut i = self.inner.lock().await;
        ensure!(self.current(link), "共享会话已结束");
        i.state.display_device_id = Some(source.clone());
        i.state.display_source = Some(DisplaySource::Ddc {
            device: source.clone(),
        });
        // Keep the existing status understood by the UI and control protocol.
        // This is a routing observation, never proof of the physical picture.
        i.state.display_status = "commanded".into();
        recovery.detected = Some(token);
        Ok(source)
    }

    pub(super) async fn refresh_display_after_resume(&self) {
        let Some(token) = ({
            let recovery = self.display_recovery.lock().await;
            recovery
                .pending()
                .filter(|token| recovery.detected != Some(*token))
        }) else {
            return;
        };
        let Ok(_command) = self.commands.try_lock() else {
            return;
        };
        let _effects = self.effects.lock().await;
        let s = self.snapshot().await;
        if s.config.display_method == DisplayMethod::Signal {
            // Sleep invalidates user-confirmed routing. Release the old lease
            // and session on BOTH roles so the peer restores its own output too.
            if let Err(error) = self.restore_local_output().await {
                self.log("error", &format!("唤醒后恢复显示输出失败：{error:#}"))
                    .await;
            }
            if let Some(link) = self.link.lock().await.as_ref() {
                link.session.close();
            }
            self.inner.lock().await.state.reset_display();
            self.display_recovery.lock().await.complete(token);
            self.publish();
            return;
        }
        if !s.running || s.config.role != "host" {
            return;
        }
        let Some(link) = self
            .link
            .lock()
            .await
            .clone()
            .filter(|link| self.current(link))
        else {
            return;
        };
        match self.detect_resumed_source(&link, token).await {
            Ok(_) => self.publish_control(&link, None).await,
            Err(error) => {
                self.trace(
                    "display.resumeReadPending",
                    json!({"error":error.to_string()}),
                )
                .await;
            }
        }
    }

    pub(super) async fn prepare_resumed_switch(&self, link: &Link, token: Instant) -> Result<()> {
        // Re-read at command time; the monitor could have auto-selected another
        // input since the background wake scan. No keyboard or DDC writes yet.
        self.detect_resumed_source(link, token).await?;
        Ok(())
    }

    pub(super) async fn confirm_resumed_switch(
        &self,
        link: &Link,
        token: Instant,
        target: &str,
    ) -> Result<()> {
        // A source change can temporarily make both DDC replies unavailable.
        // Re-read the same target without reissuing/toggling the command. Bound
        // the entire confirmation, including slow native/RPC reads.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut observation = Err(anyhow::anyhow!("输入源读取超时"));
        for delay in [500, 750, 1000] {
            tokio::select! {
                _ = link.session.closed.cancelled() => bail!("共享会话已结束"),
                _ = tokio::time::sleep_until(deadline) => break,
                _ = tokio::time::sleep(Duration::from_millis(delay)) => {}
            }
            observation =
                tokio::time::timeout_at(deadline, self.detect_resumed_source(link, token))
                    .await
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("输入源读取超时")));
            if observation.as_ref().is_ok_and(|source| source == target) {
                self.display_recovery.lock().await.complete(token);
                return Ok(());
            }
            ensure!(self.current(link), "共享会话已结束");
            if tokio::time::Instant::now() >= deadline {
                break;
            }
        }
        let source = match observation {
            Ok(source) => source,
            Err(error) => {
                if self.current(link) {
                    let mut i = self.inner.lock().await;
                    i.state.display_device_id = None;
                    i.state.display_status = "uncertain".into();
                    i.state.display_source = None;
                }
                return Err(ReadbackUnavailable(error).into());
            }
        };
        ensure!(
            source == target,
            "输入源尚未切到目标电脑，视频连接可能仍在恢复，请稍后重试"
        );
        self.display_recovery.lock().await.complete(token);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn signal_wake_clears_routing_and_closes_old_session_without_ddc_or_input_calls() {
        for role in ["host", "client"] {
            let directory = tempfile::tempdir().unwrap();
            let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), role);
            runtime.inner.lock().await.state.config.display_method = DisplayMethod::Signal;
            let (_remote, _) =
                super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
            let link = runtime.link.lock().await.as_ref().unwrap().clone();
            runtime.display_recovery.lock().await.mark();
            runtime.refresh_display_after_resume().await;
            assert!(link.session.closed.is_cancelled());
            assert!(runtime.display_recovery.lock().await.pending().is_none());
            assert!(runtime.snapshot().await.display_device_id.is_none());
        }
    }

    #[test]
    fn reads_require_a_known_source_and_conflicts_never_choose_a_default() {
        assert_eq!(agree_source(Some("mac".into()), None).unwrap(), "mac");
        assert_eq!(agree_source(None, Some("win".into())).unwrap(), "win");
        assert_eq!(
            agree_source(Some("win".into()), Some("win".into())).unwrap(),
            "win"
        );
        assert!(agree_source(None, None).is_err());
        assert!(agree_source(Some("mac".into()), Some("win".into())).is_err());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn accepted_switch_retries_transient_readback_without_undoing_input_or_reissuing_writes()
    {
        use std::os::unix::fs::PermissionsExt;
        // The final case stays unreadable: preserve the confirmed input move,
        // report unconfirmed video, and require a fresh source next time.
        for unavailable in [1, 2, 3] {
            let directory = tempfile::tempdir().unwrap();
            let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "host");
            let local = runtime.snapshot().await.device_id;
            {
                let mut i = runtime.inner.lock().await;
                i.state.engine = "ready".into();
                i.state.active_device_id = Some(local.clone());
                i.state.display_device_id = Some(local.clone());
                i.state.config.display_id = "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE".into();
                i.state.config.local_input = Some(3);
                i.state.config.peer_input = Some(1);
                i.state.peer = Some(Peer {
                    id: "receiver".into(),
                    name: "Receiver".into(),
                    platform: "win32".into(),
                    fingerprint: "a".repeat(64),
                    file_transfer: false,
                    lock_control: false,
                    input_configured: true,
                    retained_display: false,
                    modifier_mapping: ModifierMapping::Common,
                });
            }
            let native = runtime.platform.resources.join("native/bin");
            std::fs::create_dir_all(&native).unwrap();
            let hotkey = native.join("mac-hotkey");
            let ddc = native.join("m1ddc");
            for (file, script) in [
                (&hotkey, "#!/bin/sh\necho \"$1\" >> \"$0.calls\"\necho sent\n"),
                (&ddc, "#!/bin/sh\nif test \"$3\" = get; then test ! -f \"$0.written\" || exit 1; echo 3; exit 0; fi\nif test \"$3\" = set; then echo \"$5\" >> \"$0.written\"; exit 0; fi\nexit 9\n"),
            ] {
                std::fs::write(file, script).unwrap();
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            runtime.display_recovery.lock().await.mark();
            let (remote, mut requests) =
                super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
            let link = runtime.link.lock().await.clone().unwrap();
            let reads = Arc::new(AtomicU64::new(0));
            let count = reads.clone();
            let writes = Arc::new(AtomicU64::new(0));
            let written = writes.clone();
            let replying = remote.clone();
            let source = local.clone();
            let receiver = tokio::spawn(async move {
                while let Some(message) = requests.recv().await {
                    if let Incoming::Request {
                        id,
                        method,
                        payload,
                    } = message
                    {
                        let response = match method.as_str() {
                            "get-input-source" => match count.fetch_add(1, Ordering::AcqRel) {
                                0 => json!(source),
                                read if read <= unavailable => Value::Null,
                                _ => json!("receiver"),
                            },
                            "set-input" => {
                                assert_eq!(payload, json!("receiver"));
                                written.fetch_add(1, Ordering::AcqRel);
                                json!(true)
                            }
                            _ => panic!("Unexpected RPC: {method}"),
                        };
                        replying.reply(id, Ok(response)).await.unwrap();
                    }
                }
            });
            let rt = runtime.clone();
            let context = link.clone();
            let calls = hotkey.with_extension("calls");
            let observation = tokio::spawn(async move {
                tokio::time::timeout(Duration::from_secs(3), async {
                    while !calls.exists() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .unwrap();
                rt.observe_transition(
                    &context,
                    Transition {
                        from: local,
                        to: "receiver".into(),
                        position: None,
                    },
                )
                .await;
            });
            let result = runtime.command_switch_target(&link, None).await;
            observation.await.unwrap();
            if unavailable < 3 {
                result.unwrap();
                assert!(runtime.display_recovery.lock().await.pending().is_none());
                assert_eq!(
                    runtime.snapshot().await.display_device_id.as_deref(),
                    Some("receiver")
                );
            } else {
                assert!(result.unwrap_err().is::<ReadbackUnavailable>());
                assert!(runtime.display_recovery.lock().await.pending().is_some());
                assert_eq!(runtime.snapshot().await.display_status, "uncertain");
            }
            assert_eq!(
                runtime.snapshot().await.active_device_id.as_deref(),
                Some("receiver")
            );
            assert_eq!(
                std::fs::read_to_string(hotkey.with_extension("calls"))
                    .unwrap()
                    .trim(),
                "peer"
            );
            assert_eq!(
                std::fs::read_to_string(ddc.with_extension("written"))
                    .unwrap()
                    .trim(),
                "1"
            );
            assert_eq!(
                writes.load(Ordering::Acquire),
                1,
                "Read retries must not send another DDC command"
            );
            assert_eq!(
                reads.load(Ordering::Acquire),
                if unavailable < 3 { unavailable + 2 } else { 4 }
            );
            assert!(!runtime.command_pending.load(Ordering::Acquire));
            remote.close();
            receiver.await.unwrap();
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn resumed_toggle_uses_live_source_even_when_target_is_not_enumerated() {
        use std::os::unix::fs::PermissionsExt;
        // Actual picture, whether writes take effect, target enumerated, readable.
        for (actual_local, apply, present, readable) in [
            (true, true, true, true),
            (false, true, true, true),
            (false, false, true, true),
            (false, true, false, true),
            (true, true, false, true),
            (false, false, false, true),
            (true, false, false, true),
            (false, true, true, false),
            (false, true, false, false),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "host");
            let local = runtime.snapshot().await.device_id;
            let target = if actual_local {
                "receiver".into()
            } else {
                local.clone()
            };
            let display_id = "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE";
            {
                let mut i = runtime.inner.lock().await;
                i.state.engine = "ready".into();
                // Avoid real key injection, and deliberately seed the WRONG
                // video history so a cached toggle chooses the wrong direction.
                i.state.active_device_id = Some(target.clone());
                i.state.display_device_id = Some(target.clone());
                i.state.config.display_id = display_id.into();
                i.state.config.local_input = Some(3);
                i.state.config.peer_input = Some(1);
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
            let helper = runtime.platform.resources.join("native/bin/m1ddc");
            std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
            let source_file = helper.with_extension("source");
            std::fs::write(&source_file, if actual_local { "3" } else { "1" }).unwrap();
            let list = if present {
                format!("[1] Shared ({display_id})")
            } else {
                String::new()
            };
            std::fs::write(&helper, format!(
                "#!/bin/sh\nif test \"$2\" = list; then touch \"$0.listed\"; echo '{list}'; exit 0; fi\nif test \"$3\" = get; then {} cat \"$0.source\"; exit 0; fi\nif test \"$3\" = set; then echo \"$5\" >> \"$0.called\"; {} exit 0; fi\nexit 9\n",
                if readable { "" } else { "exit 1;" },
                if apply { "echo \"$5\" > \"$0.source\";" } else { "" }
            )).unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
            runtime.display_recovery.lock().await.mark();
            let (remote, mut requests) =
                super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
            let (done, mut finished) = tokio::sync::oneshot::channel::<()>();
            let expected = target.clone();
            let local_id = local.clone();
            let writes = Arc::new(AtomicU64::new(0));
            let remote_writes = writes.clone();
            let receiver = tokio::spawn(async move {
                loop {
                    let next = tokio::select! {
                        _ = &mut finished => break,
                        next = requests.recv() => next,
                    };
                    let (id, method, payload) = match next {
                        Some(Incoming::Request {
                            id,
                            method,
                            payload,
                        }) => (id, method, payload),
                        Some(Incoming::Event { .. }) => continue,
                        None => break,
                    };
                    let reply = match method.as_str() {
                        "get-input-source" => {
                            if !readable {
                                Value::Null
                            } else if std::fs::read_to_string(&source_file).unwrap().trim() == "3" {
                                json!(local_id)
                            } else {
                                json!("receiver")
                            }
                        }
                        "set-input" => {
                            assert_eq!(payload, json!(expected));
                            remote_writes.fetch_add(1, Ordering::AcqRel);
                            if apply {
                                std::fs::write(
                                    &source_file,
                                    if expected == local_id { "3" } else { "1" },
                                )
                                .unwrap();
                            }
                            json!(true)
                        }
                        _ => panic!("Unexpected RPC: {method}"),
                    };
                    remote.reply(id, Ok(reply)).await.unwrap();
                }
                remote
            });
            let link = runtime.link.lock().await.clone().unwrap();
            let result = runtime.command_switch_target(&link, None).await;
            done.send(()).unwrap();
            receiver.await.unwrap().close();
            let state = runtime.snapshot().await;
            // Inactive inputs may disappear from enumeration. A fresh source
            // read, actual writes and readback decide the result in both directions.
            assert!(!helper.with_extension("listed").exists());
            if readable {
                assert_eq!(writes.load(Ordering::Acquire), 1);
                assert_eq!(
                    std::fs::read_to_string(helper.with_extension("called"))
                        .unwrap()
                        .trim(),
                    if actual_local { "1" } else { "3" }
                );
            }
            if apply && readable {
                result.unwrap();
                assert_eq!(state.display_device_id, Some(target));
                assert!(runtime.display_recovery.lock().await.pending().is_none());
            } else {
                let error = result.unwrap_err().to_string();
                assert!(runtime.display_recovery.lock().await.pending().is_some());
                if !readable {
                    assert!(error.contains("无法读取当前输入源"), "{error}");
                } else {
                    assert!(error.contains("尚未切到目标电脑"), "{error}");
                    assert_eq!(
                        state.display_device_id,
                        Some(if actual_local {
                            local
                        } else {
                            "receiver".into()
                        })
                    );
                }
                if !readable {
                    assert_eq!(writes.load(Ordering::Acquire), 0);
                    assert!(!helper.with_extension("called").exists());
                }
            }
        }
    }

    #[test]
    fn sleep_gap_arms_recovery_but_normal_ticks_and_clock_rollback_do_not() {
        let mut state = DisplayRecovery::default();
        let now = SystemTime::now();
        assert!(!state.observe(now));
        assert!(!state.observe(now + Duration::from_millis(16)));
        assert!(state.pending().is_none());
        assert!(state.observe(now + Duration::from_secs(60)));
        let token = state.pending().unwrap();
        assert!(!state.observe(now));
        assert_eq!(state.pending(), Some(token));
        state.complete(token);
        assert!(state.pending().is_none());
    }

    #[test]
    fn completing_old_recovery_does_not_erase_new_topology_recovery() {
        let mut state = DisplayRecovery::default();
        let old = Instant::now() - Duration::from_secs(1);
        state.pending = Some(old);
        state.mark();
        let current = state.pending().unwrap();
        state.complete(old);
        assert_eq!(state.pending(), Some(current));
        state.complete(current);
        assert!(state.pending().is_none());
        state.pending = Some(Instant::now() - Duration::from_secs(31));
        assert!(
            state.pending().is_some(),
            "An unread source must not expire into cached routing"
        );
    }
}
