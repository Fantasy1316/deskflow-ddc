use super::*;

impl Runtime {
    pub(super) async fn land(&self, id: u64) -> Result<()> {
        tokio::time::sleep(Duration::from_millis(80)).await;
        ensure!(
            self.active_session.load(Ordering::Acquire) == id,
            "共享会话已结束"
        );
        let s = self.snapshot().await;
        let b = &pointer::selected(&s.cursor_displays, &s.config.retained_display_id)
            .context("所选独立屏幕不可用")?
            .bounds;
        let side = local_side(&s);
        let p = pointer::landing(b, pointer::cursor()?, side);
        self.platform.point(p.x, p.y).await
    }
    pub(super) async fn recover_pointer(&self, id: u64) -> Result<()> {
        tokio::time::sleep(Duration::from_millis(80)).await;
        ensure!(
            self.active_session.load(Ordering::Acquire) == id,
            "共享会话已结束"
        );
        let s = self.snapshot().await;
        if s.display_status == "uncertain" || s.display_device_id.as_deref() != Some(&s.device_id) {
            return Ok(());
        }
        let current = pointer::cursor()?;
        if let Some(point) = s
            .cursor_displays
            .iter()
            .map(|p| pointer::landing(&p.bounds, current, "off"))
            .min_by(|a, b| distance(*a, current).total_cmp(&distance(*b, current)))
        {
            self.platform.point(point.x, point.y).await?
        }
        Ok(())
    }
    pub(super) fn start_pointer_poll(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(16));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut previous: Option<Point> = None;
            let mut cooldown = tokio::time::Instant::now();
            let mut topology = tokio::time::Instant::now();
            loop {
                tokio::select! {
                    _ = this.shutdown.cancelled() => break,
                    _ = interval.tick() => {}
                }
                // Sampling never clones the log history or starts a subprocess.
                let (running, selected) = {
                    let i = this.inner.lock().await;
                    let s = &i.state;
                    let enabled = keep_mode(s)
                        && s.engine == "ready"
                        && !s.switching
                        && s.active_device_id.as_ref() == Some(&s.device_id)
                        && s.display_device_id.as_deref() == s.peer.as_ref().map(|p| p.id.as_str())
                        && !this.command_pending.load(Ordering::Acquire);
                    (
                        s.running,
                        if enabled {
                            pointer::selected(&s.cursor_displays, &s.config.retained_display_id)
                                .map(|panel| (panel.clone(), local_side(s).to_owned()))
                        } else {
                            None
                        },
                    )
                };
                if running
                    && topology.elapsed() > Duration::from_secs(5)
                    && !this.topology_busy.swap(true, Ordering::AcqRel)
                {
                    topology = tokio::time::Instant::now();
                    let rt = this.clone();
                    tokio::spawn(async move {
                        let _ = rt.refresh_topology().await;
                        rt.topology_busy.store(false, Ordering::Release);
                    });
                }
                if this.pointer_busy.load(Ordering::Acquire)
                    || tokio::time::Instant::now() < cooldown
                {
                    previous = None;
                    continue;
                }
                let Some((panel, side)) = selected else {
                    previous = None;
                    continue;
                };
                let Ok(point) = pointer::cursor() else {
                    previous = None;
                    continue;
                };
                let position = previous.and_then(|p| pointer::seam(p, point, &panel.bounds, &side));
                previous = Some(point);
                if position.is_some() || !panel.bounds.contains(point) {
                    previous = None;
                    cooldown = tokio::time::Instant::now() + Duration::from_millis(650);
                    this.pointer_busy.store(true, Ordering::Release);
                    let rt = this.clone();
                    tokio::spawn(async move {
                        if let Err(e) = rt.poll_effect(position).await {
                            rt.log("error", &e.to_string()).await;
                        }
                        rt.pointer_busy.store(false, Ordering::Release);
                    });
                }
            }
        });
    }
    pub(super) async fn refresh_topology(&self) -> Result<()> {
        let epoch = self.epoch.load(Ordering::Acquire);
        let panels: Vec<Panel> = serde_json::from_value(self.platform.screens().await?)?;
        self.apply_topology(epoch, panels).await;
        Ok(())
    }
    pub(super) async fn apply_topology(&self, epoch: u64, panels: Vec<Panel>) {
        // Serialize against explicit stop/start so an old scan cannot suspend a
        // newly started session or re-enable sharing after the user stopped it.
        let _lifecycle = self.start_stop.lock().await;
        if self.epoch.load(Ordering::Acquire) != epoch {
            return;
        }
        let mut i = self.inner.lock().await;
        let missing = i.state.running
            && !i.state.config.retained_display_id.is_empty()
            && pointer::selected(&panels, &i.state.config.retained_display_id).is_none();
        if i.state.cursor_displays != panels {
            i.state.cursor_displays = panels;
            self.publish();
        }
        let was_ready = self.topology_ready.send_replace(!missing);
        drop(i);
        if missing {
            // Sleep can temporarily remove every active display. Release the
            // current input session, but keep the listener/retry loop and the
            // authoritative display source. Never start an engine while missing.
            if let Some(link) = self.link.lock().await.as_ref() {
                link.session.close();
            }
            if was_ready {
                self.log(
                    "info",
                    "所选独立屏幕暂不可用，已暂停键鼠共享；屏幕恢复后将自动重连",
                )
                .await;
            }
        } else if !was_ready {
            self.log("info", "独立屏幕已恢复，正在自动重连").await;
        }
    }
    pub(super) async fn poll_effect(&self, position: Option<f64>) -> Result<()> {
        let link = self.link.lock().await.clone().context("共享会话已结束")?;
        let s = self.snapshot().await;
        if let Some(position) = position {
            if s.config.role == "host" {
                self.keyboard_switch(&link, s.peer.context("对端未连接")?.id, Some(position))
                    .await
            } else {
                link.session
                    .request("cross-screen", Value::Null, Duration::from_secs(15))
                    .await
                    .map(|_| ())
            }
        } else {
            self.land(link.id).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(directory: &std::path::Path, role: &str) -> Arc<Runtime> {
        let store = Store {
            directory: directory.into(),
            value: crate::store::Stored {
                version: 3,
                id: uuid::Uuid::new_v4(),
                config: Config {
                    role: role.into(),
                    retained_display_id: "internal".into(),
                    ..Config::default()
                },
                trusted_peer: None,
                last_verified_address: None,
            },
            pairing: protocol::new_pairing(),
        };
        let diagnostics = Diagnostics::new(directory.join("logs"), store.pairing.clone()).unwrap();
        let mut state = Snapshot::new(&store, &diagnostics);
        state.running = true;
        state.listening = role == "host";
        state.control_connected("provider".into());
        state.display_device_id = Some("receiver".into());
        state.last_display_device_id = Some("receiver".into());
        state.display_status = "commanded".into();
        state.cursor_displays = panels();
        Arc::new(Runtime {
            updates: watch::channel(state.clone()).0,
            inner: Mutex::new(Inner {
                store,
                state,
                diagnostics,
                sequence: 0,
                diagnostic_line: None,
            }),
            platform: Platform::new(directory.join("no-native-binaries")),
            discovery: Mutex::new(None),
            fingerprint: "a".repeat(64),
            directory: directory.into(),
            dirty: AtomicBool::new(false),
            epoch: AtomicU64::new(1),
            session_sequence: AtomicU64::new(1),
            active_session: AtomicU64::new(1),
            network: Mutex::new(None),
            pointer_busy: AtomicBool::new(false),
            topology_busy: AtomicBool::new(false),
            topology_ready: watch::channel(true).0,
            link: Mutex::new(None),
            engine: Mutex::new(None),
            actions: Mutex::new(()),
            commands: Mutex::new(()),
            effects: Mutex::new(()),
            command_pending: AtomicBool::new(false),
            keyboard_pending: AtomicU64::new(0),
            start_stop: Mutex::new(()),
            observed: broadcast::channel(32).0,
            completed: broadcast::channel(32).0,
            shutdown: CancellationToken::new(),
        })
    }

    fn panels() -> Vec<Panel> {
        vec![Panel {
            id: "1".into(),
            name: "Built-in".into(),
            internal: true,
            bounds: pointer::Bounds {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
            },
        }]
    }

    async fn attach_with_requests(runtime: &Runtime) -> (Session, mpsc::Receiver<Incoming>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            Wire::handshake(listener.accept().await.unwrap().0, [4; 32], true)
                .await
                .unwrap()
        });
        let client = Wire::handshake(TcpStream::connect(address).await.unwrap(), [4; 32], false)
            .await
            .unwrap();
        let (local, _incoming) = Session::spawn(server.await.unwrap());
        let (remote, incoming) = Session::spawn(client);
        *runtime.link.lock().await = Some(Arc::new(Link {
            epoch: runtime.epoch.load(Ordering::Acquire),
            id: runtime.active_session.load(Ordering::Acquire),
            session: local,
            peer: watch::channel(None).0,
        }));
        (remote, incoming)
    }

    async fn attach(runtime: &Runtime) -> Session {
        attach_with_requests(runtime).await.0
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn standalone_input_tests_replace_retained_routing_in_both_roles() {
        use std::os::unix::fs::PermissionsExt;
        for role in ["host", "client"] {
            let directory = tempfile::tempdir().unwrap();
            let runtime = fixture(directory.path(), role);
            runtime.stop().await.unwrap();
            let peer = uuid::Uuid::new_v4();
            {
                let mut i = runtime.inner.lock().await;
                i.state.config.display_id = "test-display".into();
                i.state.config.local_input = Some(17);
                i.state.config.peer_input = Some(1);
                i.store.value.trusted_peer = Some(Pin {
                    id: peer,
                    fingerprint: "a".repeat(64),
                });
            }
            let helper = runtime.platform.resources.join("native/bin/m1ddc");
            std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
            std::fs::write(&helper, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();

            // The stopped session's peer field is empty; use the saved peer pin.
            assert!(runtime.snapshot().await.peer.is_none());
            runtime.test_input(1).await.unwrap();
            let s = runtime.snapshot().await;
            assert_eq!(s.display_device_id, Some(peer.to_string()));
            assert_eq!(s.last_display_device_id, s.display_device_id);
            assert_eq!(s.display_status, "commanded");
            runtime.test_input(17).await.unwrap();
            let s = runtime.snapshot().await;
            assert_eq!(s.display_device_id.as_ref(), Some(&s.device_id));
            assert_eq!(s.last_display_device_id, s.display_device_id);

            // Even an error may follow a physical switch. Do not route via the
            // retained local source on the next session.
            std::fs::write(&helper, "#!/bin/sh\nexit 1\n").unwrap();
            assert!(runtime.test_input(1).await.is_err());
            let s = runtime.snapshot().await;
            assert!(s.display_device_id.is_none());
            assert_eq!(s.display_status, "uncertain");
            assert_eq!(s.last_display_device_id.as_ref(), Some(&s.device_id));

            // Successful test before pairing cannot invent a receiver UUID.
            std::fs::write(&helper, "#!/bin/sh\nexit 0\n").unwrap();
            runtime.inner.lock().await.store.value.trusted_peer = None;
            runtime.test_input(1).await.unwrap();
            let s = runtime.snapshot().await;
            assert!(s.display_device_id.is_none());
            assert_eq!(s.display_status, "uncertain");
        }
    }

    #[tokio::test]
    async fn failed_pairing_replacement_preserves_retained_source_and_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = fixture(directory.path(), "host");
        let mut i = runtime.inner.lock().await;
        let code = i.store.pairing.clone();
        assert!(i.replace_pairing("invalid-code".into()).is_err());
        assert_eq!(i.store.pairing, code);
        assert_eq!(i.state.pairing_code, code);
        assert_eq!(i.state.display_device_id.as_deref(), Some("receiver"));
    }

    #[tokio::test]
    async fn stop_restart_keeps_receiver_picture_and_routes_return_command_through_receiver() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = fixture(directory.path(), "host");
        runtime.stop().await.unwrap();
        let stopped = runtime.snapshot().await;
        assert!(!stopped.running && !stopped.connected);
        assert!(stopped.active_device_id.is_none());
        assert_eq!(stopped.display_device_id.as_deref(), Some("receiver"));
        assert_eq!(stopped.display_status, "commanded");

        // Recreate the control/input session while the physical screen still
        // displays Windows. The input engine starts locally, independently of video.
        let local = stopped.device_id.clone();
        {
            let mut i = runtime.inner.lock().await;
            i.state.running = true;
            i.state.control_connected(local.clone());
            i.state.engine = "ready".into();
        }
        runtime.active_session.store(2, Ordering::Release);
        let (remote, mut requests) = attach_with_requests(&runtime).await;
        let link = runtime.link.lock().await.clone().unwrap();
        let restarted = runtime.snapshot().await;
        assert_eq!(restarted.active_device_id.as_deref(), Some(local.as_str()));
        assert_eq!(restarted.display_device_id.as_deref(), Some("receiver"));
        // Returning to the still-visible receiver must not be rejected as a
        // hidden edge; this was the cause of the repeated cursor bounce.
        assert_eq!(
            crate::switching::decide(crate::switching::Policy {
                explicit: false,
                manual: true,
                keep: true,
                target_retained: false,
                provider_retained: true,
                from_provider: true,
                requested: false,
                video_source: restarted.display_device_id.as_deref(),
                target: "receiver",
                uncertain: false,
            })
            .unwrap(),
            crate::switching::Effect::None
        );
        let target = local.clone();
        let receiver = tokio::spawn(async move {
            while let Some(incoming) = requests.recv().await {
                if let Incoming::Request {
                    id,
                    method,
                    payload,
                } = incoming
                {
                    assert_eq!(method, "set-input");
                    assert_eq!(payload, json!(target));
                    remote.reply(id, Ok(json!(true))).await.unwrap();
                    return remote;
                }
            }
            panic!("No input-source command reached the visible receiver");
        });
        // There is deliberately no local native binary. Choosing the wrong
        // DDC sender therefore fails rather than silently satisfying this test.
        runtime.command_switch(&link, local.clone()).await.unwrap();
        let remote = receiver.await.unwrap();
        let switched = runtime.snapshot().await;
        assert_eq!(switched.display_device_id.as_deref(), Some(local.as_str()));
        assert_eq!(switched.display_status, "commanded");
        remote.close();
    }

    #[tokio::test]
    async fn stop_keeps_picture_in_both_roles_and_marks_an_interrupted_switch_uncertain() {
        for role in ["host", "client"] {
            let directory = tempfile::tempdir().unwrap();
            let runtime = fixture(directory.path(), role);
            runtime.stop().await.unwrap();
            runtime.stop().await.unwrap();
            let state = runtime.snapshot().await;
            assert_eq!(state.display_device_id.as_deref(), Some("receiver"));
            assert_eq!(state.last_display_device_id.as_deref(), Some("receiver"));
            assert_eq!(state.display_status, "commanded");
            runtime.inner.lock().await.state.switching = true;
            runtime.stop().await.unwrap();
            let interrupted = runtime.snapshot().await;
            assert!(interrupted.display_device_id.is_none());
            assert_eq!(interrupted.display_status, "uncertain");
            assert_eq!(
                interrupted.last_display_device_id.as_deref(),
                Some("receiver")
            );
        }
    }

    #[tokio::test]
    async fn missing_panel_closes_input_session_but_preserves_reconnect_and_picture() {
        for role in ["host", "client"] {
            let directory = tempfile::tempdir().unwrap();
            let runtime = fixture(directory.path(), role);
            let remote = attach(&runtime).await;
            let link = runtime.link.lock().await.clone().unwrap();
            runtime.apply_topology(1, vec![]).await;
            assert!(!*runtime.topology_ready.borrow());
            assert!(!runtime.current(&link));
            assert!(link.session.closed.is_cancelled());
            // Even if a connection is accepted during a topology transition,
            // it cannot start an engine while the selected panel is absent.
            assert!(runtime
                .start_engine(
                    &link,
                    &super::super::tests::peer(),
                    "127.0.0.1".parse().unwrap()
                )
                .await
                .is_err());
            assert!(runtime.engine.lock().await.is_none());
            runtime.cleanup_session(1, 1).await;
            let paused = runtime.snapshot().await;
            assert!(paused.running);
            assert_eq!(paused.listening, role == "host");
            assert!(!paused.connected);
            assert_eq!(paused.display_device_id.as_deref(), Some("receiver"));
            assert_eq!(paused.display_status, "commanded");
            runtime.apply_topology(1, panels()).await;
            assert!(*runtime.topology_ready.borrow());
            let restored = runtime.snapshot().await;
            assert!(restored.running);
            assert_eq!(restored.display_device_id, paused.display_device_id);
            assert_eq!(restored.config.retained_display_id, "internal");
            assert_eq!(runtime.epoch.load(Ordering::Acquire), 1);
            remote.close();
        }
    }

    #[tokio::test]
    async fn windows_panel_with_new_handle_resumes_using_stable_device_identity() {
        fn scanned(handle: &str, name: &str, x: f64) -> Vec<Panel> {
            serde_json::from_value(json!([{
                "id":format!("win-monitor-{}", "a".repeat(64)),
                "handle":handle,
                "name":name,
                "internal":false,
                "bounds":{"x":x,"y":0.0,"width":1920.0,"height":1080.0}
            }]))
            .unwrap()
        }
        for role in ["host", "client"] {
            let directory = tempfile::tempdir().unwrap();
            let runtime = fixture(directory.path(), role);
            let before = scanned("100", "DISPLAY1", 0.0);
            let stable = before[0].id.clone();
            {
                let mut inner = runtime.inner.lock().await;
                inner.state.platform = "win32".into();
                inner.state.config.retained_display_id = stable.clone();
                inner.store.value.config.retained_display_id = stable.clone();
            }
            runtime.apply_topology(1, before).await;
            assert!(*runtime.topology_ready.borrow());
            runtime.apply_topology(1, vec![]).await;
            assert!(!*runtime.topology_ready.borrow());
            // An unrelated display, including one reusing the previous handle,
            // must not resume the session or replace the user's selection.
            let mut unrelated = scanned("100", "DISPLAY1", 0.0);
            unrelated[0].id = format!("win-monitor-{}", "b".repeat(64));
            runtime.apply_topology(1, unrelated).await;
            assert!(!*runtime.topology_ready.borrow());
            runtime
                .apply_topology(1, scanned("900", "DISPLAY3", 2560.0))
                .await;
            assert!(*runtime.topology_ready.borrow());
            let state = runtime.snapshot().await;
            assert!(state.running);
            assert_eq!(state.listening, role == "host");
            assert_eq!(state.config.retained_display_id, stable);
            let panel = pointer::selected(&state.cursor_displays, &stable).unwrap();
            assert_eq!(panel.name, "DISPLAY3");
            assert_eq!(panel.bounds.x, 2560.0);
            assert_eq!(state.display_device_id.as_deref(), Some("receiver"));
            assert_eq!(runtime.epoch.load(Ordering::Acquire), 1);
        }
    }

    #[tokio::test]
    async fn stopping_while_panel_missing_cannot_be_undone_by_a_late_scan() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = fixture(directory.path(), "host");
        runtime.apply_topology(1, vec![]).await;
        runtime.stop().await.unwrap();
        runtime.apply_topology(1, panels()).await;
        let stopped = runtime.snapshot().await;
        assert!(!stopped.running && !stopped.listening && !stopped.connected);
        assert!(runtime.link.lock().await.is_none());
        assert!(runtime.engine.lock().await.is_none());
        assert!(stopped.cursor_displays.is_empty());
        // A late missing-panel result cannot suspend a subsequent explicit start.
        runtime.inner.lock().await.state.cursor_displays = panels();
        runtime.apply_topology(1, vec![]).await;
        assert!(*runtime.topology_ready.borrow());
        assert!(!runtime.snapshot().await.cursor_displays.is_empty());
    }
}
