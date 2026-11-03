use super::*;
use std::time::{Instant, SystemTime};

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
    async fn query_display_source(&self, link: &Link) -> Result<String> {
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
        // Preserve the existing wire status for compatibility with 0.2.4 peers.
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

    pub(super) async fn ensure_target_connected(&self, link: &Link, target: &str) -> Result<()> {
        let s = self.snapshot().await;
        let result = if target == s.device_id {
            self.platform.display_connected(&s.config.display_id).await
        } else {
            link.session
                .request("display-connection", Value::Null, Duration::from_secs(10))
                .await
                .and_then(|value| Ok(serde_json::from_value::<bool>(value)?))
        };
        ensure!(self.current(link), "共享会话已结束");
        match result {
            Ok(true) => Ok(()),
            Ok(false) => bail!("目标电脑尚未识别到共用显示器，视频连接可能仍在恢复，请稍后重试；本次未切换画面与键鼠"),
            Err(error) => {
                // Older peers have no presence RPC. A successful live DDC read
                // still proves the configured monitor can be reached there.
                if target != s.device_id {
                    if let Ok(value) = link.session.request("get-input-source", Value::Null, Duration::from_secs(4)).await {
                        let source = serde_json::from_value::<Option<String>>(value).ok().flatten();
                        if self.current(link) && source.as_ref().is_some_and(|id| id == &s.device_id || s.peer.as_ref().is_some_and(|peer| id == &peer.id)) {
                            return Ok(());
                        }
                    }
                }
                Err(error).context("暂时无法确认目标显示器连接是否恢复，请稍后重试；本次未切换画面与键鼠")
            }
        }
    }

    pub(super) async fn confirm_resumed_switch(
        &self,
        link: &Link,
        token: Instant,
        target: &str,
    ) -> Result<()> {
        tokio::select! {
            _ = link.session.closed.cancelled() => bail!("共享会话已结束"),
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        let source = match self.detect_resumed_source(link, token).await {
            Ok(source) => source,
            Err(error) => {
                if self.current(link) {
                    let mut i = self.inner.lock().await;
                    i.state.display_device_id = None;
                    i.state.display_status = "uncertain".into();
                }
                return Err(error).context("输入源指令已发送，但暂时无法确认是否生效；目标视频连接可能仍在恢复，请稍后重试");
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
    async fn resumed_toggle_uses_live_source_and_reports_unready_or_unapplied_target() {
        use std::os::unix::fs::PermissionsExt;
        // Actual picture, whether writes take effect, target enumerated, readable.
        for (actual_local, apply, present, readable) in [
            (true, true, true, true),
            (false, true, true, true),
            (false, false, true, true),
            (false, true, false, true),
            (false, true, true, false),
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
                "#!/bin/sh\nif test \"$2\" = list; then echo '{list}'; exit 0; fi\nif test \"$3\" = get; then {} cat \"$0.source\"; exit 0; fi\nif test \"$3\" = set; then echo \"$5\" >> \"$0.called\"; {} exit 0; fi\nexit 9\n",
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
                        "display-connection" => json!(present),
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
            if apply && present && readable {
                result.unwrap();
                assert_eq!(state.display_device_id, Some(target));
                assert!(runtime.display_recovery.lock().await.pending().is_none());
                assert_eq!(writes.load(Ordering::Acquire), 1);
            } else {
                let error = result.unwrap_err().to_string();
                assert!(runtime.display_recovery.lock().await.pending().is_some());
                if !readable {
                    assert!(error.contains("无法读取当前输入源"), "{error}");
                } else if !present {
                    assert!(error.contains("尚未识别到共用显示器"), "{error}");
                } else {
                    assert!(error.contains("尚未切到目标电脑"), "{error}");
                    assert_eq!(state.display_device_id, Some("receiver".into()));
                }
                if !present || !readable {
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
