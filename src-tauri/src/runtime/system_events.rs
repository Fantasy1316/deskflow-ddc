use super::*;
use crate::system_events::{Event, Scans, State, WallClock};
use std::time::{Instant, SystemTime};

impl Runtime {
    pub(super) fn ensure_system_awake(&self) -> Result<()> {
        ensure!(
            self.system_events.ready(),
            "系统正在休眠或恢复显示连接，请稍后重试"
        );
        Ok(())
    }

    async fn apply_power_event(&self, state: &State) {
        // Admission is already closed synchronously by the native callback.
        // Keep the listener/configuration; closing the session releases input
        // through the existing engine cleanup and automatic reconnect path.
        self.interrupt_switch_wait();
        let _lifecycle = self.start_stop.lock().await;
        self.topology_ready.send_replace(false);
        if let Some(link) = self.link.lock().await.as_ref() {
            link.session.close();
        }
        self.display_recovery.lock().await.mark();
        *self.display_only_history.lock().await = display_only::ToggleHistory::default();
        {
            let mut i = self.inner.lock().await;
            // A monitor may auto-select a different input while the host sleeps.
            // Signal mode requires user confirmation; DDC uses fresh readback.
            i.state.reset_display();
            i.state.switching = false;
            i.state.switch_progress = None;
        }
        self.system_events.acknowledge(state.power);
        self.publish();
        self.trace(
            "system.power",
            json!({"sleeping":state.sleeping,"sleeps":state.sleeps,
            "wakes":state.wakes,"pollGaps":state.gaps,"pictureVerified":false}),
        )
        .await;
    }

    pub(super) fn start_system_events(self: &Arc<Self>) {
        let this = self.clone();
        let mut changes = this.system_events.subscribe();
        tokio::spawn(async move {
            let mut previous = State::default();
            let mut scans = Scans::default();
            let mut clock = WallClock::default();
            let mut periodic = Instant::now();
            let mut tick = tokio::time::interval(Duration::from_millis(100));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // One scan in flight, regardless of how many callbacks arrive.
            // It must not block processing a Sleep notification during enumeration.
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = this.shutdown.cancelled() => break,
                    result = changes.changed() => { if result.is_err() { break; } },
                    _ = tick.tick() => {},
                    _ = tasks.join_next(), if !tasks.is_empty() => {},
                }
                let mut state = this.system_events.state();
                let now = Instant::now();
                let gap = clock.observe(SystemTime::now());
                if gap && (state.power == previous.power || state.sleeping) {
                    this.system_events.emit(Event::PollGap);
                    state = this.system_events.state();
                }
                if state.power != previous.power {
                    this.apply_power_event(&state).await;
                    if state.sleeping {
                        scans.cancel();
                    } else {
                        scans.resume(now);
                        if state.gaps != previous.gaps {
                            scans.cause = "pollGap";
                        }
                    }
                }
                if state.displays != previous.displays && !state.sleeping {
                    scans.displays(now);
                }
                if state.screen_wakes != previous.screen_wakes && !state.sleeping {
                    // Signal fallback may wake an output itself. Check without
                    // invalidating the source, restoring outputs or closing RPC.
                    scans.screen_wake(now);
                }
                if state.generation != previous.generation {
                    this.trace(
                        "system.event",
                        json!({"power":state.power,"sleeping":state.sleeping,
                            "displays":state.displays,"screenWakes":state.screen_wakes,
                            "pollGaps":state.gaps}),
                    )
                    .await;
                }
                previous = state.clone();
                if state.sleeping || !this.system_events.ready() {
                    continue;
                }
                if periodic.elapsed() >= Duration::from_secs(5) {
                    periodic = now;
                    let s = this.inner.lock().await;
                    if s.state.running && s.state.config.mode == OperationMode::Sharing {
                        scans.periodic(now);
                    }
                }
                if tasks.is_empty() && scans.due.is_some_and(|due| due <= now) {
                    let cause = scans.cause;
                    scans.completed(now);
                    let rt = this.clone();
                    tasks.spawn(async move {
                        let epoch = rt.epoch.load(Ordering::Acquire);
                        let power = rt.system_events.state().power;
                        let started = Instant::now();
                        rt.trace("display.topologyScanStarted", json!({"cause":cause}))
                            .await;
                        if let Err(error) = rt.refresh_topology().await {
                            rt.trace(
                                "display.topologyScanFailed",
                                json!({"error":error.to_string()}),
                            )
                            .await;
                        }
                        if rt.epoch.load(Ordering::Acquire) == epoch
                            && rt.system_events.state().power == power
                            && rt.system_events.ready()
                        {
                            rt.refresh_display_after_resume().await;
                        }
                        rt.trace(
                            "display.topologyScanFinished",
                            json!({"cause":cause,
                            "elapsedMs":started.elapsed().as_millis(),
                            "ready":*rt.topology_ready.borrow()}),
                        )
                        .await;
                    });
                }
            }
            // No helper/refresh task outlives the runtime's shutdown worker.
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn screen_wake_worker_scans_without_waiting_for_poll_or_resetting_live_routing() {
        use std::os::unix::fs::PermissionsExt;
        for role in ["host", "client"] {
            for method in [DisplayMethod::Ddc, DisplayMethod::Signal] {
                let dir = tempfile::tempdir().unwrap();
                let rt = super::super::pointer_bridge::tests::fixture(dir.path(), role);
                rt.inner.lock().await.state.config.display_method = method;
                let before = rt.snapshot().await;
                let helper = rt.platform.resources.join("native/bin/mac-hotkey");
                std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
                let mut expected = before.cursor_displays.clone();
                expected[0].bounds.x += 10.0;
                let panels = serde_json::to_string(&expected).unwrap();
                std::fs::write(&helper, format!("#!/bin/sh\n[ \"$1\" = screens ] || exit 9\necho scan >> \"$0.calls\"\necho '{panels}'\n")).unwrap();
                std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
                let (remote, _) =
                    super::super::pointer_bridge::tests::attach_with_requests(&rt).await;
                let link = rt.link.lock().await.clone().unwrap();
                rt.start_system_events();
                rt.system_events.emit(Event::ScreenWake);
                let mut scans = rt.topology_ready.subscribe();
                tokio::time::timeout(Duration::from_secs(1), async {
                    while rt.snapshot().await.cursor_displays != expected {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .unwrap();
                let after = rt.snapshot().await;
                assert_eq!(after.display_device_id, before.display_device_id);
                assert_eq!(after.active_device_id, before.active_device_id);
                assert!(rt.current(&link) && !link.session.closed.is_cancelled());
                assert!(rt.display_recovery.lock().await.pending().is_none());
                assert!(
                    !scans.has_changed().unwrap(),
                    "Unchanged topology must not restart client retries"
                );
                scans.borrow_and_update();
                rt.stop().await.unwrap();
                rt.system_events.emit(Event::ScreenWake);
                tokio::time::sleep(Duration::from_millis(50)).await;
                assert!(!rt.snapshot().await.running);
                rt.shutdown.cancel();
                remote.close();
            }
        }
    }
    #[tokio::test]
    async fn native_power_invalidates_both_roles_and_modes_but_display_events_do_not() {
        for role in ["host", "client"] {
            for method in [DisplayMethod::Ddc, DisplayMethod::Signal] {
                let dir = tempfile::tempdir().unwrap();
                let rt = super::super::pointer_bridge::tests::fixture(dir.path(), role);
                {
                    let mut i = rt.inner.lock().await;
                    i.state.config.display_method = method;
                    i.state.config.retained_display_id.clear();
                }
                let (_remote, _) =
                    super::super::pointer_bridge::tests::attach_with_requests(&rt).await;
                let link = rt.link.lock().await.clone().unwrap();
                rt.system_events.emit(Event::Displays);
                assert!(rt.current(&link));
                assert!(rt.display_recovery.lock().await.pending().is_none());
                rt.system_events.emit(Event::Sleep);
                assert!(!rt.current(&link));
                assert!(rt.ensure_switch_available().is_err());
                rt.apply_power_event(&rt.system_events.state()).await;
                assert!(link.session.closed.is_cancelled());
                assert!(!*rt.topology_ready.borrow());
                assert!(rt.snapshot().await.display_device_id.is_none());
                assert!(rt.display_recovery.lock().await.pending().is_some());
                rt.apply_topology(1, vec![]).await;
                assert!(
                    !*rt.topology_ready.borrow(),
                    "A scan cannot resume a sleeping system"
                );
                rt.system_events.emit(Event::Wake);
                assert!(rt.ensure_switch_available().is_err());
                rt.apply_power_event(&rt.system_events.state()).await;
                assert!(rt.system_events.ready());
                rt.apply_topology(1, vec![]).await;
                assert!(*rt.topology_ready.borrow());
            }
        }
    }

    #[tokio::test]
    async fn worker_handles_coalesced_power_events_even_when_native_enumeration_fails() {
        let dir = tempfile::tempdir().unwrap();
        let rt = super::super::pointer_bridge::tests::fixture(dir.path(), "host");
        rt.system_events.emit(Event::Sleep);
        rt.system_events.emit(Event::Wake);
        rt.start_system_events();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if rt.system_events.ready() && rt.snapshot().await.display_device_id.is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(rt.display_recovery.lock().await.pending().is_some());
        assert!(!*rt.topology_ready.borrow()); // Failed native reads cannot resume input.
        rt.shutdown.cancel();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn sleep_during_standalone_ddc_write_discards_its_result_and_clears_switch_progress() {
        let (_dir, rt) = super::super::display_only::tests::standalone_fixture().await;
        rt.start(None).await.unwrap();
        let helper = rt.platform.mac_ddc_helper();
        std::fs::write(&helper, "#!/bin/sh\nif [ \"$3\" = get ]; then echo 15; else touch \"$0.started\"; sleep 0.2; fi\n").unwrap();
        let switching = rt.clone();
        let task = tokio::spawn(async move { switching.switch_to(false).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !helper.with_extension("started").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        rt.system_events.emit(Event::Sleep);
        assert!(task.await.unwrap().is_err());
        rt.apply_power_event(&rt.system_events.state()).await;
        let s = rt.snapshot().await;
        assert!(s.display_target.is_none() && !s.switching && s.switch_progress.is_none());
        rt.stop().await.unwrap(); // A later wake must not undo manual Stop.
        rt.system_events.emit(Event::Wake);
        rt.apply_power_event(&rt.system_events.state()).await;
        rt.apply_topology(rt.epoch.load(Ordering::Acquire), vec![])
            .await;
        assert!(!rt.snapshot().await.running);
    }
}
