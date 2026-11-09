use super::*;
use std::sync::Mutex as SyncMutex;
use std::time::Instant;

#[derive(Clone, Copy)]
struct Timing {
    minimum: Duration,
    maximum: Duration,
    poll: Duration,
}

pub(super) struct Protection {
    state: Arc<SyncMutex<ProtectionState>>,
    released: Arc<tokio::sync::Notify>,
    timing: Timing,
}

#[derive(Default)]
struct ProtectionState {
    target: Option<String>,
    active: bool,
    settling: bool,
    started: Option<Instant>,
    discarded_through: Option<Instant>,
    cancel: CancellationToken,
}

/// A transaction owns admission through native execution AND settling. Dropping
/// it on every return/cancellation also invalidates already queued edge events.
pub(super) struct Permit(Arc<SyncMutex<ProtectionState>>, Arc<tokio::sync::Notify>);

impl Default for Protection {
    fn default() -> Self {
        Self {
            state: Arc::new(SyncMutex::new(ProtectionState::default())),
            released: Arc::new(tokio::sync::Notify::new()),
            timing: Timing {
                minimum: Duration::from_secs(2),
                maximum: Duration::from_secs(8),
                poll: Duration::from_millis(400),
            },
        }
    }
}

impl Protection {
    pub(super) fn enter(&self, target: Option<&str>) -> Result<Permit> {
        let mut state = self.state.lock().unwrap();
        ensure!(!state.active, "正在切换或等待显示器稳定，请稍后重试");
        state.active = true;
        state.settling = false;
        state.started = Some(Instant::now());
        state.target = target.map(str::to_owned);
        state.cancel = CancellationToken::new();
        Ok(Permit(self.state.clone(), self.released.clone()))
    }

    pub(super) fn busy(&self) -> bool {
        self.state.lock().unwrap().active
    }

    pub(super) fn settling(&self) -> bool {
        self.state.lock().unwrap().settling
    }

    pub(super) fn waits_for_display(&self) -> bool {
        !self.timing.maximum.is_zero()
    }

    pub(super) async fn wait_released(&self) {
        loop {
            let released = self.released.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            if !self.busy() {
                return;
            }
            released.await;
        }
    }

    pub(super) fn blocks(&self, received: Instant) -> bool {
        let state = self.state.lock().unwrap();
        state.active || state.discarded_through.is_some_and(|end| received <= end)
    }

    pub(super) fn expected_transfer(&self, target: &str, received: Instant) -> bool {
        let state = self.state.lock().unwrap();
        !state.active
            || (!state.settling
                && state.target.as_deref() == Some(target)
                && state.started.is_some_and(|start| received >= start))
    }

    #[cfg(test)]
    pub(super) fn short_readback_wait() -> Self {
        Self {
            timing: Timing {
                minimum: Duration::from_millis(2),
                maximum: Duration::from_millis(150),
                poll: Duration::from_millis(2),
            },
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn immediate() -> Self {
        Self {
            timing: Timing {
                minimum: Duration::ZERO,
                maximum: Duration::ZERO,
                poll: Duration::ZERO,
            },
            ..Self::default()
        }
    }
}

impl Permit {
    pub(super) fn target(&self, target: &str) {
        self.0.lock().unwrap().target = Some(target.to_owned());
    }

    pub(super) fn settling(&self) {
        self.0.lock().unwrap().settling = true;
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = self.0.lock().unwrap();
        state.active = false;
        state.settling = false;
        state.target = None;
        state.started = None;
        state.discarded_through = Some(Instant::now());
        drop(state);
        self.1.notify_waiters();
    }
}

impl Runtime {
    /// Admission check for UI/tray/RPC ingress, before any action queue or task.
    pub fn ensure_switch_available(&self) -> Result<()> {
        self.ensure_switch_available_at(Instant::now())
    }

    pub fn ensure_switch_available_at(&self, received: Instant) -> Result<()> {
        ensure!(
            !self.switch_protection.blocks(received),
            "正在切换或等待显示器稳定，请稍后重试"
        );
        Ok(())
    }

    pub fn interrupt_switch_wait(&self) {
        self.switch_protection.state.lock().unwrap().cancel.cancel();
    }

    pub(super) async fn settle_display(
        &self,
        permit: &Permit,
        link: Option<&Link>,
        target: &str,
        accepted: bool,
    ) -> bool {
        permit.settling();
        let epoch = self.epoch.load(Ordering::Acquire);
        let timing = self.switch_protection.timing;
        if timing.maximum.is_zero() {
            return false;
        }
        let cancelled = link
            .map(|link| link.session.closed.clone())
            .unwrap_or_else(|| self.shutdown.clone());
        let interrupted = self.switch_protection.state.lock().unwrap().cancel.clone();
        {
            let mut i = self.inner.lock().await;
            if i.state.emergency_stopping
                || interrupted.is_cancelled()
                || cancelled.is_cancelled()
                || self.shutdown.is_cancelled()
                || self.epoch.load(Ordering::Acquire) != epoch
                || link.is_some_and(|link| !self.current(link))
            {
                return false;
            }
            i.state.switching = true;
            i.state.switch_progress = Some(SwitchProgress {
                stage: SwitchStage::WaitingDisplay,
            });
        }
        if let Some(link) = link {
            self.publish_control(link, None).await;
        } else {
            self.publish();
        }
        let started = tokio::time::Instant::now();
        let deadline = started + timing.maximum;
        let signal = self.snapshot().await.config.display_method == DisplayMethod::Signal;
        // All waits and probes are bounded and cancellable. No effects lock is
        // held: input acknowledgements and independent-screen movement continue.
        let observed = async {
            tokio::time::sleep(timing.minimum).await;
            if signal || !accepted {
                return false;
            }
            let mut matches = 0;
            loop {
                let matches_target = if let Some(link) = link {
                    self.query_display_source(link)
                        .await
                        .is_ok_and(|source| source == target)
                } else {
                    let s = self.snapshot().await;
                    let value = if target == "local" {
                        s.config.local_input
                    } else {
                        s.config.peer_input
                    };
                    self.platform
                        .current_input(&s.config.display_id)
                        .await
                        .ok()
                        .flatten()
                        .is_some_and(|input| Some(input) == value)
                };
                matches = if matches_target { matches + 1 } else { 0 };
                if matches >= 2 {
                    return true;
                }
                tokio::time::sleep(timing.poll).await;
            }
        };
        let observed = tokio::select! {
            biased;
            _ = cancelled.cancelled() => return false,
            _ = self.shutdown.cancelled() => return false,
            _ = interrupted.cancelled() => return false,
            result = tokio::time::timeout_at(deadline, observed) => result.unwrap_or(false),
        };
        if self.epoch.load(Ordering::Acquire) != epoch
            || link.is_some_and(|link| !self.current(link))
        {
            return false;
        }
        self.trace(
            "display.settleCompleted",
            json!({
                "target":target,"elapsedMs":started.elapsed().as_millis(),
                "readbackConsistent":observed,"pictureVerified":false,
                "commandAccepted":accepted,"signal":signal,
            }),
        )
        .await;
        if accepted && !signal && !observed {
            self.log(
                "info",
                "输入源指令已发送，稳定等待已超时；暂未确认实际画面，可稍后重试",
            )
            .await;
        }
        observed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_covers_execution_settling_and_already_queued_events() {
        let protection = Protection::default();
        let before = Instant::now();
        let permit = protection.enter(Some("peer")).unwrap();
        let during = Instant::now();
        assert!(protection.enter(Some("local")).is_err());
        assert!(protection.expected_transfer("peer", during));
        assert!(!protection.expected_transfer("peer", before));
        assert!(!protection.expected_transfer("local", during));
        permit.settling();
        assert!(!protection.expected_transfer("peer", during));
        drop(permit);
        assert!(
            protection.blocks(during),
            "A queued edge must never become a fresh video command"
        );
        assert!(!protection.blocks(Instant::now()));
        assert!(protection.enter(Some("local")).is_ok());
    }

    #[cfg(target_os = "macos")]
    async fn setup(
        timing: Timing,
    ) -> (
        tempfile::TempDir,
        Arc<Runtime>,
        Session,
        tokio::task::JoinHandle<()>,
        Arc<AtomicU64>,
    ) {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let mut runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "host");
        Arc::get_mut(&mut runtime).unwrap().switch_protection = Protection {
            timing,
            ..Protection::default()
        };
        let local = runtime.snapshot().await.device_id;
        {
            let mut i = runtime.inner.lock().await;
            i.state.engine = "ready".into();
            i.state.config.display_id = "external".into();
            i.state.config.local_input = Some(3);
            i.state.config.peer_input = Some(1);
            i.state.config.retained_display_id.clear();
            i.state.active_device_id = Some(local.clone());
            i.state.display_device_id = Some(local.clone());
            i.state.last_display_device_id = Some(local.clone());
            i.state.session_layout = Some(Layout {
                host_side: "right".into(),
                video: "follow".into(),
            });
            i.state.peer = Some(Peer {
                id: "receiver".into(),
                name: "Windows".into(),
                platform: "win32".into(),
                fingerprint: "a".repeat(64),
                file_transfer: false,
                lock_control: false,
                input_configured: true,
                retained_display: false,
                modifier_mapping: ModifierMapping::Original,
            });
        }
        let native = runtime.platform.resources.join("native/bin");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(native.join("current"), "3").unwrap();
        for (name, script) in [
            (
                "m1ddc",
                r#"#!/bin/sh
base="$(dirname "$0")"
case "$3" in
set) echo "$5" >> "$base/writes"; echo "$5" > "$base/current" ;;
get) test ! -f "$base/unreadable" || exit 1; cat "$base/current" ;;
*) exit 9 ;;
esac
"#,
            ),
            (
                "mac-hotkey",
                "#!/bin/sh\necho \"$*\" >> \"$0.calls\"\necho sent\n",
            ),
        ] {
            let file = native.join(name);
            std::fs::write(&file, script).unwrap();
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let (remote, mut requests) =
            super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
        let replying = remote.clone();
        let count = Arc::new(AtomicU64::new(0));
        let writes = count.clone();
        let service = tokio::spawn(async move {
            while let Some(message) = requests.recv().await {
                if let Incoming::Request {
                    id,
                    method,
                    payload,
                } = message
                {
                    let result = match method.as_str() {
                        "set-input" => {
                            writes.fetch_add(1, Ordering::AcqRel);
                            let value = if payload == local { "3" } else { "1" };
                            std::fs::write(native.join("current"), value).unwrap();
                            Ok(json!(true))
                        }
                        "get-input-source" => {
                            if native.join("unreadable").exists() {
                                Err(anyhow::anyhow!("readback unavailable"))
                            } else {
                                let value =
                                    std::fs::read_to_string(native.join("current")).unwrap();
                                Ok(json!(if value.trim() == "3" {
                                    &local
                                } else {
                                    "receiver"
                                }))
                            }
                        }
                        "land-pointer" => Ok(json!(true)),
                        _ => panic!("Unexpected request: {method}"),
                    };
                    if replying.reply(id, result).await.is_err() {
                        break;
                    }
                }
            }
        });
        (directory, runtime, remote, service, count)
    }

    fn timing() -> Timing {
        Timing {
            minimum: Duration::from_millis(40),
            maximum: Duration::from_millis(500),
            poll: Duration::from_millis(10),
        }
    }

    async fn waiting(runtime: &Runtime) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if runtime
                    .snapshot()
                    .await
                    .switch_progress
                    .is_some_and(|p| p.stage == SwitchStage::WaitingDisplay)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn explicit_requests_are_discarded_during_settling_and_admitted_afterwards() {
        let (_directory, runtime, remote, service, count) = setup(timing()).await;
        let link = runtime.link.lock().await.clone().unwrap();
        let local = runtime.snapshot().await.device_id;
        let rt = runtime.clone();
        let context = link.clone();
        let target = local.clone();
        let command = tokio::spawn(async move { rt.command_switch(&context, target).await });
        waiting(&runtime).await;
        let before = count.load(Ordering::Acquire);
        for _ in 0..4 {
            assert!(runtime.switch_to(false).await.is_err());
            assert!(runtime.toggle().await.is_err());
        }
        assert_eq!(count.load(Ordering::Acquire), before);
        assert!(runtime.snapshot().await.switching);
        command.await.unwrap().unwrap();
        assert!(!runtime.switch_protection.busy());
        let before_release = Instant::now();
        let admission = runtime.switch_protection.enter(Some(&local)).unwrap();
        drop(admission);
        assert!(runtime.ensure_switch_available_at(before_release).is_err());
        runtime.command_switch(&link, local).await.unwrap();
        assert_eq!(
            count.load(Ordering::Acquire),
            before + 1,
            "No rejected command is replayed"
        );
        assert!(!runtime.snapshot().await.switching);
        remote.close();
        service.abort();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn queued_reverse_edge_cannot_rewrite_video_but_fresh_edge_still_works() {
        let (_directory, runtime, remote, service, count) = setup(timing()).await;
        let link = runtime.link.lock().await.clone().unwrap();
        let local = runtime.snapshot().await.device_id;
        let rt = runtime.clone();
        let context = link.clone();
        let from = local.clone();
        let first = tokio::spawn(async move {
            rt.observe_transition(
                &context,
                Transition {
                    from,
                    to: "receiver".into(),
                    position: None,
                },
            )
            .await;
        });
        waiting(&runtime).await;
        let queued_at = Instant::now();
        first.await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(2),
            runtime.switch_protection.wait_released(),
        )
        .await
        .unwrap();
        assert_eq!(count.load(Ordering::Acquire), 1);
        let rt = runtime.clone();
        let context = link.clone();
        let queued = tokio::spawn(async move {
            rt.observe_transition_at(
                &context,
                Transition {
                    from: "receiver".into(),
                    to: local,
                    position: None,
                },
                queued_at,
            )
            .await;
        });
        let marker = runtime
            .platform
            .resources
            .join("native/bin/mac-hotkey.calls");
        tokio::time::timeout(Duration::from_secs(1), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        let _ = runtime.observed.send((
            link.id,
            Transition {
                from: runtime.snapshot().await.device_id,
                to: "receiver".into(),
                position: None,
            },
        ));
        queued.await.unwrap();
        assert_eq!(count.load(Ordering::Acquire), 1);
        assert_eq!(
            runtime.snapshot().await.display_device_id.as_deref(),
            Some("receiver")
        );
        let local = runtime.snapshot().await.device_id;
        runtime
            .observe_transition(
                &link,
                Transition {
                    from: "receiver".into(),
                    to: local.clone(),
                    position: None,
                },
            )
            .await;
        tokio::time::timeout(
            Duration::from_secs(2),
            runtime.switch_protection.wait_released(),
        )
        .await
        .unwrap();
        assert_eq!(count.load(Ordering::Acquire), 2);
        let state = runtime.snapshot().await;
        assert_eq!(state.display_device_id, Some(local.clone()));
        assert_eq!(state.active_device_id, Some(local));
        assert!(!state.switching);
        remote.close();
        service.abort();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn independent_screen_movement_during_protection_does_not_issue_ddc() {
        for video in ["follow", "manual", "keep"] {
            let (_directory, runtime, remote, service, count) = setup(timing()).await;
            let link = runtime.link.lock().await.clone().unwrap();
            let local = runtime.snapshot().await.device_id;
            {
                let mut i = runtime.inner.lock().await;
                i.state.peer.as_mut().unwrap().retained_display = true;
                i.state.session_layout.as_mut().unwrap().video = video.into();
                i.state.display_status = "uncertain".into();
                i.state.display_device_id = None;
            }
            let permit = runtime.switch_protection.enter(Some(&local)).unwrap();
            permit.settling();
            runtime
                .observe_transition(
                    &link,
                    Transition {
                        from: local,
                        to: "receiver".into(),
                        position: None,
                    },
                )
                .await;
            assert_eq!(count.load(Ordering::Acquire), 0);
            assert_eq!(
                runtime.snapshot().await.active_device_id.as_deref(),
                Some("receiver")
            );
            assert!(runtime.switch_protection.busy());
            drop(permit);
            remote.close();
            service.abort();
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn unreadable_ddc_times_out_without_rolling_back_or_permanently_locking() {
        let (_directory, runtime, remote, service, _) = setup(timing()).await;
        std::fs::write(runtime.platform.resources.join("native/bin/unreadable"), "").unwrap();
        let link = runtime.link.lock().await.clone().unwrap();
        let local = runtime.snapshot().await.device_id;
        let began = Instant::now();
        runtime.command_switch(&link, local.clone()).await.unwrap();
        assert!(began.elapsed() >= timing().maximum);
        assert!(!runtime.switch_protection.busy());
        let state = runtime.snapshot().await;
        assert_eq!(state.active_device_id, Some(local));
        assert_eq!(state.display_status, "commanded");
        assert!(state
            .logs
            .iter()
            .any(|log| log.message.contains("稳定等待已超时")));
        assert!(runtime.switch_protection.enter(Some("receiver")).is_ok());
        remote.close();
        service.abort();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn stop_and_emergency_cancel_settling_before_the_deadline() {
        for (emergency, edge) in [(false, false), (true, false), (false, true), (true, true)] {
            let (_directory, runtime, remote, service, _) = setup(Timing {
                minimum: Duration::from_secs(10),
                maximum: Duration::from_secs(20),
                poll: Duration::from_secs(1),
            })
            .await;
            let rt = runtime.clone();
            let command = tokio::spawn(async move {
                let link = rt.link.lock().await.clone().unwrap();
                let local = rt.snapshot().await.device_id;
                if edge {
                    rt.observe_transition(
                        &link,
                        Transition {
                            from: local,
                            to: "receiver".into(),
                            position: None,
                        },
                    )
                    .await;
                    Ok(())
                } else {
                    rt.command_switch(&link, local).await
                }
            });
            waiting(&runtime).await;
            tokio::time::timeout(Duration::from_secs(2), async {
                if emergency {
                    runtime.emergency_stop().await
                } else {
                    runtime.stop().await
                }
            })
            .await
            .unwrap()
            .unwrap();
            let _ = command.await.unwrap();
            assert!(!runtime.switch_protection.busy());
            let state = runtime.snapshot().await;
            assert!(!state.running && !state.switching && state.switch_progress.is_none());
            remote.close();
            service.abort();
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_delayed_edge_wait_cannot_republish_progress_after_session_cleanup() {
        let (_directory, runtime, remote, service, count) = setup(timing()).await;
        let link = runtime.link.lock().await.clone().unwrap();
        let local = runtime.snapshot().await.device_id;
        let permit = runtime.switch_protection.enter(Some(&local)).unwrap();
        link.session.close();
        runtime.active_session.store(0, Ordering::Release);
        runtime
            .settle_display(&permit, Some(&link), &local, true)
            .await;
        let state = runtime.snapshot().await;
        assert!(!state.switching && state.switch_progress.is_none());
        assert_eq!(count.load(Ordering::Acquire), 0);
        drop(permit);
        remote.close();
        service.abort();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn standalone_mode_rejects_repeated_toggles_while_waiting_and_stops_promptly() {
        let (_directory, mut runtime) =
            super::super::display_only::tests::standalone_fixture().await;
        Arc::get_mut(&mut runtime).unwrap().switch_protection = Protection {
            timing: Timing {
                minimum: Duration::from_secs(10),
                maximum: Duration::from_secs(20),
                poll: Duration::from_secs(1),
            },
            ..Protection::default()
        };
        runtime.start(None).await.unwrap();
        let rt = runtime.clone();
        let command = tokio::spawn(async move { rt.switch_to(false).await });
        waiting(&runtime).await;
        assert!(runtime.switch_to(true).await.is_err());
        assert!(runtime.toggle().await.is_err());
        tokio::time::timeout(Duration::from_secs(2), runtime.stop())
            .await
            .unwrap()
            .unwrap();
        let _ = command.await.unwrap();
        assert!(!runtime.switch_protection.busy());
        assert!(!runtime.snapshot().await.switching);
        let writes =
            std::fs::read_to_string(runtime.platform.resources.join("native/bin/writes")).unwrap();
        assert_eq!(writes.lines().count(), 1);
        assert!(runtime.engine.lock().await.is_none());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn failed_video_state_can_be_recovered_by_a_fresh_explicit_request() {
        let (_directory, runtime, remote, service, count) = setup(timing()).await;
        let link = runtime.link.lock().await.clone().unwrap();
        let local = runtime.snapshot().await.device_id;
        {
            let mut i = runtime.inner.lock().await;
            i.state.display_status = "uncertain".into();
            i.state.display_device_id = None;
        }
        runtime.command_switch(&link, local.clone()).await.unwrap();
        assert_eq!(count.load(Ordering::Acquire), 1);
        let state = runtime.snapshot().await;
        assert_eq!(state.display_device_id, Some(local));
        assert_eq!(state.display_status, "commanded");
        assert!(!state.switching && !runtime.switch_protection.busy());
        remote.close();
        service.abort();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn signal_settling_never_queries_ddc_and_has_a_bounded_wait() {
        let (_directory, runtime, remote, service, count) = setup(timing()).await;
        runtime.inner.lock().await.state.config.display_method = DisplayMethod::Signal;
        let link = runtime.link.lock().await.clone().unwrap();
        let local = runtime.snapshot().await.device_id;
        let permit = runtime.switch_protection.enter(Some(&local)).unwrap();
        let began = Instant::now();
        runtime
            .settle_display(&permit, Some(&link), &local, true)
            .await;
        assert!(began.elapsed() >= timing().minimum);
        assert!(began.elapsed() < timing().maximum);
        assert_eq!(count.load(Ordering::Acquire), 0);
        drop(permit);
        remote.close();
        service.abort();
    }
}
