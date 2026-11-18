use super::*;

impl Runtime {
    fn fail_input_waiters(&self, link: &Link, state: &Snapshot) {
        for target in [
            Some(state.device_id.clone()),
            state.peer.as_ref().map(|p| p.id.clone()),
        ]
        .into_iter()
        .flatten()
        {
            let _ = self.completed.send(Completed {
                session: link.id,
                to: target,
                error: Some("键鼠连接已中断，请等待重新连接".into()),
                position: None,
            });
        }
    }
    pub(super) async fn engine_connection_changed(&self, link: &Link, line: &str) {
        if !self.current(link) {
            return;
        }
        let connected = line == "connectionState=Connected";
        let generation = if connected {
            link.input_generation.load(Ordering::Acquire)
        } else {
            link.input_generation.fetch_add(1, Ordering::AcqRel) + 1
        };
        let mut i = self.inner.lock().await;
        if !self.current(link) {
            return;
        }
        let s = &mut i.state;
        let host = s.config.role == "host";
        let was_ready = s.engine == "ready";
        s.engine = if connected { "ready" } else { "starting" }.into();
        if host && (!connected || !was_ready) {
            // Deskflow force-leaves a disconnected secondary. Reconnecting a
            // socket does not re-enter that secondary. Video is independent.
            s.active_device_id = Some(s.device_id.clone());
        } else if !host && !connected {
            s.active_device_id = None;
        }
        if !connected && !self.command_pending.load(Ordering::Acquire) {
            s.switching = false;
        }
        let state = s.clone();
        drop(i);
        if !connected {
            self.fail_input_waiters(link, &state);
        }
        self.trace("input.connection", json!({"session":link.id,"generation":generation,"connected":connected,"owner":state.active_device_id})).await;
        if host {
            self.publish_control(link, None).await;
        }
        self.publish();
    }

    pub(super) async fn engine_forced_return(&self, link: &Link, change: Transition) {
        let mut i = self.inner.lock().await;
        if !self.current(link)
            || i.state.config.role != "host"
            || change.to != i.state.device_id
            || !i.state.peer.as_ref().is_some_and(|p| p.id == change.from)
        {
            return;
        }
        // A forced return is recovery evidence, never a natural edge or an
        // acknowledgement of an outstanding video/input command.
        link.input_generation.fetch_add(1, Ordering::AcqRel);
        i.state.active_device_id = Some(change.to.clone());
        if !self.command_pending.load(Ordering::Acquire) {
            i.state.switching = false;
        }
        let state = i.state.clone();
        drop(i);
        self.fail_input_waiters(link, &state);
        self.trace(
            "input.forcedReturn",
            json!({"session":link.id,"from":change.from,"to":change.to,"position":change.position}),
        )
        .await;
        self.publish_control(link, None).await;
        self.publish();
    }

    pub(super) fn disable_primary_route(&self) -> Result<()> {
        self.write_primary_route(&json!({"schema":1,"enabled":false}))
    }
    pub(super) fn update_primary_route(&self, s: &Snapshot) -> Result<()> {
        let route = if cfg!(target_os = "macos")
            && s.config.role == "host"
            && s.config.deskflow_path.is_empty()
            && s.connected
            && s.engine == "ready"
            && !self.command_pending.load(Ordering::Acquire)
            && keep_mode(s)
            && s.display_status != "uncertain"
            && s.peer
                .as_ref()
                .is_some_and(|p| s.display_device_id.as_ref() == Some(&p.id))
        {
            pointer::selected(&s.cursor_displays, &s.config.retained_display_id).map(|panel| {
                json!({"schema":1,"enabled":true,"displayId":panel.id,"bounds":panel.bounds,
                    "side":local_side(s),"peer":format!("sb-{}",s.peer.as_ref().unwrap().id)})
            })
        } else {
            None
        };
        self.write_primary_route(&route.unwrap_or_else(|| json!({"schema":1,"enabled":false})))
    }
    fn write_primary_route(&self, value: &Value) -> Result<()> {
        if !cfg!(target_os = "macos") {
            return Ok(());
        }
        let file = self.directory.join("deskflow/primary-route.json");
        let bytes = serde_json::to_vec(value)?;
        if std::fs::read(&file).is_ok_and(|current| current == bytes) {
            return Ok(());
        }
        std::fs::create_dir_all(file.parent().unwrap())?;
        crate::store::atomic_write(&file, &bytes)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    async fn setup(
        path: &std::path::Path,
        role: &str,
        method: DisplayMethod,
        video: &str,
    ) -> (Arc<Runtime>, Session) {
        let rt = super::super::pointer_bridge::tests::fixture(path, role);
        let (remote, mut requests) =
            super::super::pointer_bridge::tests::attach_with_requests(&rt).await;
        tokio::spawn(async move {
            while let Some(message) = requests.recv().await {
                assert!(
                    matches!(message, Incoming::Event { .. }),
                    "input lifecycle must not write display inputs"
                );
            }
        });
        let mut peer = super::super::tests::peer();
        peer.role = if role == "host" { "client" } else { "host" }.into();
        let link = rt.link.lock().await.clone().unwrap();
        link.peer.send_replace(Some(peer.clone()));
        let mut i = rt.inner.lock().await;
        i.state.connected = true;
        i.state.engine = "ready".into();
        i.state.config.display_method = method;
        i.state.config.retained_display_id = "internal".into();
        i.state.session_layout = Some(Layout {
            host_side: "left".into(),
            video: video.into(),
        });
        i.state.peer = Some(Peer {
            id: peer.id.to_string(),
            name: peer.name,
            platform: peer.platform,
            fingerprint: peer.fingerprint,
            file_transfer: false,
            lock_control: false,
            input_configured: true,
            retained_display: false,
            modifier_mapping: ModifierMapping::Original,
        });
        i.state.active_device_id = Some(peer.id.to_string());
        i.state.display_device_id = Some(peer.id.to_string());
        i.state.last_display_device_id = Some(peer.id.to_string());
        i.state.display_status = "commanded".into();
        drop(i);
        (rt, remote)
    }

    #[tokio::test]
    async fn transport_reconnect_restores_primary_owner_without_changing_picture_in_both_roles_and_modes(
    ) {
        for role in ["host", "client"] {
            for method in [DisplayMethod::Ddc, DisplayMethod::Signal] {
                for video in ["manual", "keep", "follow"] {
                    let dir = tempfile::tempdir().unwrap();
                    let (rt, remote) = setup(dir.path(), role, method, video).await;
                    let link = rt.link.lock().await.clone().unwrap();
                    let before = rt.snapshot().await;
                    let mut completion = rt.completed.subscribe();
                    rt.engine_connection_changed(&link, "retryIn=1").await;
                    assert!(completion.recv().await.unwrap().error.is_some());
                    let down = rt.snapshot().await;
                    assert_eq!(down.engine, "starting");
                    assert_eq!(
                        down.active_device_id,
                        (role == "host").then(|| before.device_id.clone())
                    );
                    rt.engine_connection_changed(&link, "connectionState=Connecting")
                        .await;
                    rt.engine_connection_changed(&link, "connectionState=Connected")
                        .await;
                    let after = rt.snapshot().await;
                    assert_eq!(after.engine, "ready");
                    assert_eq!(after.active_device_id, down.active_device_id);
                    assert_eq!(after.display_device_id, before.display_device_id);
                    assert_eq!(after.last_display_device_id, before.last_display_device_id);
                    assert_eq!(after.display_status, before.display_status);
                    assert_eq!(link.input_generation.load(Ordering::Acquire), 2);
                    remote.close();
                }
            }
        }
    }

    #[tokio::test]
    async fn forced_return_is_recovery_not_a_successful_transfer_and_stale_transition_cannot_override_it(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (rt, remote) = setup(dir.path(), "host", DisplayMethod::Ddc, "manual").await;
        let link = rt.link.lock().await.clone().unwrap();
        let before = rt.snapshot().await;
        let peer = before.peer.as_ref().unwrap().id.clone();
        let mut completed = rt.completed.subscribe();
        rt.engine_forced_return(
            &link,
            Transition {
                from: peer.clone(),
                to: before.device_id.clone(),
                position: None,
            },
        )
        .await;
        for _ in 0..2 {
            assert!(completed.recv().await.unwrap().error.is_some());
        }
        rt.observe_engine_transition_at(
            &link,
            Transition {
                from: before.device_id.clone(),
                to: peer,
                position: None,
            },
            std::time::Instant::now(),
            0,
        )
        .await;
        let after = rt.snapshot().await;
        assert_eq!(after.active_device_id, Some(before.device_id));
        assert_eq!(after.display_device_id, before.display_device_id);
        assert!(completed.try_recv().is_err());
        assert!(after.logs.iter().all(|line| line.message != "键鼠已切换"));
        remote.close();
    }

    #[tokio::test]
    async fn disconnect_cancels_pending_input_then_reconnect_allows_the_same_peer_again() {
        use std::os::unix::fs::PermissionsExt;
        for method in [DisplayMethod::Ddc, DisplayMethod::Signal] {
            for video in ["manual", "keep"] {
                let dir = tempfile::tempdir().unwrap();
                let (rt, remote) = setup(dir.path(), "host", method, video).await;
                let link = rt.link.lock().await.clone().unwrap();
                let before = rt.snapshot().await;
                let helper = rt.platform.resources.join("native/bin/mac-hotkey");
                std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
                std::fs::write(
                    &helper,
                    "#!/bin/sh\necho \"$*\" >> \"$0.calls\"\necho accepted\n",
                )
                .unwrap();
                std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
                let calls = helper.with_extension("calls");
                let waiter = {
                    let rt = rt.clone();
                    let link = link.clone();
                    let local = before.device_id.clone();
                    tokio::spawn(async move { rt.keyboard_switch(&link, local, None).await })
                };
                tokio::time::timeout(Duration::from_secs(2), async {
                    while !std::fs::read_to_string(&calls).is_ok_and(|s| s.ends_with('\n')) {
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                })
                .await
                .unwrap();
                rt.engine_connection_changed(&link, "connectionState=Listening")
                    .await;
                assert!(tokio::time::timeout(Duration::from_secs(1), waiter)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err());
                assert_eq!(rt.keyboard_pending.load(Ordering::Acquire), 0);
                rt.engine_connection_changed(&link, "connectionState=Connected")
                    .await;
                let target = before.peer.as_ref().unwrap().id.clone();
                let waiter = {
                    let rt = rt.clone();
                    let link = link.clone();
                    let target = target.clone();
                    tokio::spawn(async move { rt.keyboard_switch(&link, target, None).await })
                };
                tokio::time::timeout(Duration::from_secs(2), async {
                    while !std::fs::read_to_string(&calls)
                        .is_ok_and(|s| s.ends_with('\n') && s.lines().count() == 2)
                    {
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                })
                .await
                .unwrap();
                rt.observe_transition(
                    &link,
                    Transition {
                        from: before.device_id,
                        to: target.clone(),
                        position: None,
                    },
                )
                .await;
                waiter.await.unwrap().unwrap();
                let after = rt.snapshot().await;
                assert_eq!(after.active_device_id, Some(target));
                assert_eq!(after.display_device_id, before.display_device_id);
                assert_eq!(after.display_status, before.display_status);
                assert!(!after.switching);
                remote.close();
            }
        }
    }

    #[tokio::test]
    async fn primary_route_only_targets_visible_retained_screen_and_is_disabled_for_explicit_switches(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (rt, remote) = setup(dir.path(), "host", DisplayMethod::Ddc, "manual").await;
        let s = rt.snapshot().await;
        rt.update_primary_route(&s).unwrap();
        let read = || {
            serde_json::from_slice::<Value>(
                &std::fs::read(dir.path().join("deskflow/primary-route.json")).unwrap(),
            )
            .unwrap()
        };
        assert_eq!(read()["enabled"], true);
        assert_eq!(read()["bounds"], json!(s.cursor_displays[0].bounds));
        for entry in [
            "command",
            "picture",
            "uncertain",
            "receiver",
            "externalEngine",
            "follow",
        ] {
            let mut state = s.clone();
            match entry {
                "command" => rt.command_pending.store(true, Ordering::Release),
                "picture" => state.display_device_id = Some(state.device_id.clone()),
                "uncertain" => state.display_status = "uncertain".into(),
                "receiver" => state.config.role = "client".into(),
                "externalEngine" => state.config.deskflow_path = "user-core".into(),
                _ => state.session_layout.as_mut().unwrap().video = "follow".into(),
            }
            rt.update_primary_route(&state).unwrap();
            assert_eq!(read()["enabled"], false, "{entry}");
            rt.command_pending.store(false, Ordering::Release);
        }
        remote.close();
    }

    #[tokio::test]
    async fn live_topology_changes_update_entry_route_and_missing_panel_disables_it() {
        let dir = tempfile::tempdir().unwrap();
        let (rt, remote) = setup(dir.path(), "host", DisplayMethod::Ddc, "manual").await;
        let mut panels = rt.snapshot().await.cursor_displays;
        panels[0].bounds.x = -100.0;
        let epoch = rt.epoch.load(Ordering::Acquire);
        rt.apply_topology(epoch, panels.clone()).await;
        let file = dir.path().join("deskflow/primary-route.json");
        let route: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(route["enabled"], true);
        assert_eq!(route["bounds"]["x"], -100.0);
        rt.apply_topology(epoch, vec![]).await;
        let route: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(route["enabled"], false);
        remote.close();
    }
}
