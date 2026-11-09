use super::*;

impl Runtime {
    /// Called with effects locked. Open only the already visible peer edge,
    /// cross through Deskflow's native geometry, then close the temporary edge.
    /// No jump hotkey, receiver warp, DDC write or post-entry correction.
    pub(super) async fn cross_visible_native_edge(
        &self,
        link: &Link,
        target: &str,
        fraction: f64,
    ) -> Result<()> {
        let s = self.snapshot().await;
        ensure!(
            self.current(link)
                && s.config.role == "host"
                && s.engine == "ready"
                && s.active_device_id.as_deref() == Some(&s.device_id)
                && s.display_device_id.as_deref() == Some(target)
                && s.display_status != "uncertain",
            "目标画面不可见，取消边缘跨屏"
        );
        let peer = s.peer.as_ref().context("另一台电脑未连接")?;
        ensure!(
            peer.id == target && !peer.retained_display,
            "无效的跨屏目标"
        );
        let side = &s.session_layout.as_ref().context("未设置布局")?.host_side;
        let (inside, exit) = pointer::edge_approach(&s.cursor_displays, side, fraction)?;
        let config = engine::visible_edge_config(&s.config, &s.device_id, target, false)?;
        let file = self.directory.join("deskflow/server.conf");
        let original = std::fs::read(&file).context("无法读取引擎布局")?;
        let guard = self.engine.lock().await;
        let engine = guard.as_ref().context("键鼠引擎未运行")?;
        let mut observed = self.observed.subscribe();
        crate::store::atomic_write(&file, config.as_bytes())?;
        let crossed = async {
            engine.reload_config().await?;
            ensure!(self.current(link), "共享会话已结束");
            // This is still on the provider. Deskflow maps the border fraction
            // into the very first enter packet sent to the receiver.
            // macOS discards same-position motion. Allow the inward event to
            // reach Deskflow before posting the outward event at the border.
            self.platform.point(inside.x, inside.y).await?;
            tokio::time::sleep(Duration::from_millis(50)).await;
            ensure!(self.current(link), "共享会话已结束");
            self.platform.point(exit.x, exit.y).await?;
            let wait = async {
                loop {
                    let (session, change) = observed.recv().await?;
                    if session == link.id && change.from == s.device_id && change.to == target {
                        return Ok::<_, anyhow::Error>(change.position);
                    }
                }
            };
            tokio::select! {
                _ = link.session.closed.cancelled() => bail!("共享会话已结束"),
                result = tokio::time::timeout(Duration::from_secs(3), wait) =>
                    result.context("未收到原生边缘跨屏确认")?,
            }
        }
        .await;
        // Restore even if the native crossing failed or the session was closed.
        // A failed reload must close sharing, never leave an invisible edge open.
        let restored = async {
            crate::store::atomic_write(&file, &original)?;
            engine.reload_config().await
        }
        .await;
        if crossed.is_err() || restored.is_err() {
            link.session.close();
        }
        restored.context("无法恢复受控跨屏布局，已停止本次连接")?;
        let entered = crossed?;
        self.trace("input.nativeEdgeEntered", json!({"session":link.id,"target":target,"fraction":fraction,"exit":exit,"entry":entered,"postEntryCorrection":false})).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    async fn setup(
        directory: &std::path::Path,
        side: &str,
    ) -> (
        Arc<Runtime>,
        Session,
        mpsc::Receiver<Incoming>,
        String,
        Vec<u8>,
    ) {
        let runtime = super::super::pointer_bridge::tests::fixture(directory, "host");
        let peer = uuid::Uuid::new_v4().to_string();
        {
            let mut i = runtime.inner.lock().await;
            i.state.engine = "ready".into();
            i.state.config.layout_side = side.into();
            i.state.config.cross_screen_video = "manual".into();
            i.state.session_layout = Some(Layout {
                host_side: side.into(),
                video: "manual".into(),
            });
            i.state.active_device_id = Some(i.state.device_id.clone());
            i.state.display_device_id = Some(peer.clone());
            i.state.last_display_device_id = Some(peer.clone());
            i.state.peer = Some(Peer {
                id: peer.clone(),
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
        let state = runtime.snapshot().await;
        let original = engine::server_config(&state.config, &state.device_id, &peer, false)
            .unwrap()
            .into_bytes();
        let file = directory.join("deskflow/server.conf");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, &original).unwrap();
        let helper = runtime.platform.resources.join("native/bin/mac-hotkey");
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        // A jump hotkey or post-entry correction makes this helper fail.
        std::fs::write(&helper, "#!/bin/sh\n[ \"$1\" = pointer ] || exit 9\necho \"$*\" >> \"$0.calls\"\necho positioned\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (remote, requests) =
            super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
        (runtime, remote, requests, peer, original)
    }

    #[tokio::test]
    async fn parked_border_moves_inward_then_outward_without_receiver_correction() {
        for side in ["left", "right", "top", "bottom"] {
            for fraction in [0.1, 0.8] {
                let dir = tempfile::tempdir().unwrap();
                let (runtime, remote, mut requests, peer, original) = setup(dir.path(), side).await;
                let (engine, mut reloads, confirmed) = Engine::test_reloader();
                *runtime.engine.lock().await = Some(engine);
                let state = runtime.snapshot().await;
                let expected =
                    engine::visible_edge_config(&state.config, &state.device_id, &peer, false)
                        .unwrap()
                        .into_bytes();
                let path = dir.path().join("deskflow/server.conf");
                let baseline = original.clone();
                let reloader = tokio::spawn(async move {
                    for wanted in [expected, baseline] {
                        let reply = reloads.recv().await.unwrap();
                        assert_eq!(std::fs::read(&path).unwrap(), wanted);
                        reply.send(Ok(())).unwrap();
                        confirmed.send_modify(|n| *n += 1);
                    }
                });
                let link = runtime.link.lock().await.clone().unwrap();
                let observing = runtime.clone();
                let context = link.clone();
                let target = peer.clone();
                let marker = runtime
                    .platform
                    .resources
                    .join("native/bin/mac-hotkey.calls");
                let expected_exit = pointer::exit(&state.cursor_displays, side, fraction).unwrap();
                let (inside, _) =
                    pointer::edge_approach(&state.cursor_displays, side, fraction).unwrap();
                let observer = tokio::spawn(async move {
                    tokio::time::timeout(Duration::from_secs(3), async {
                        // The shell creates the file before writing the line.
                        // Observe a complete record, not just file existence.
                        while !std::fs::read_to_string(&marker).is_ok_and(|record| {
                            record.ends_with('\n') && record.lines().count() >= 2
                        }) {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    })
                    .await
                    .unwrap();
                    let calls = std::fs::read_to_string(&marker).unwrap();
                    assert_eq!(
                        calls.lines().count(),
                        2,
                        "Only the two pre-entry provider positions are allowed"
                    );
                    assert_eq!(
                        calls.lines().collect::<Vec<_>>(),
                        vec![
                            format!("pointer {} {}", inside.x, inside.y),
                            format!("pointer {} {}", expected_exit.x, expected_exit.y)
                        ]
                    );
                    // Model Deskflow's same-position filter with a cursor
                    // initially parked at exit. The first event must move in,
                    // and the second must move out at the unchanged fraction.
                    assert_ne!(inside, expected_exit);
                    assert!(match side {
                        "left" => inside.x < expected_exit.x && inside.y == expected_exit.y,
                        "right" => inside.x > expected_exit.x && inside.y == expected_exit.y,
                        "top" => inside.y < expected_exit.y && inside.x == expected_exit.x,
                        _ => inside.y > expected_exit.y && inside.x == expected_exit.x,
                    });
                    let change = Transition {
                        from: state.device_id,
                        to: target,
                        position: Some(Point { x: 1.0, y: 80.0 }),
                    };
                    let _ = observing.observed.send((context.id, change.clone()));
                    observing.observe_transition(&context, change).await;
                });
                runtime
                    .keyboard_switch(&link, peer.clone(), Some(fraction))
                    .await
                    .unwrap();
                observer.await.unwrap();
                reloader.await.unwrap();
                assert_eq!(
                    std::fs::read(dir.path().join("deskflow/server.conf")).unwrap(),
                    original
                );
                let after = runtime.snapshot().await;
                assert_eq!(after.active_device_id.as_deref(), Some(peer.as_str()));
                assert_eq!(after.display_device_id.as_deref(), Some(peer.as_str()));
                while let Ok(message) = requests.try_recv() {
                    assert!(
                        matches!(message, Incoming::Event { .. }),
                        "No receiver warp or display RPC is allowed"
                    );
                }
                runtime
                    .engine
                    .lock()
                    .await
                    .as_mut()
                    .unwrap()
                    .stop()
                    .await
                    .unwrap();
                remote.close();
            }
        }
    }

    #[tokio::test]
    async fn failed_open_restores_original_gate_and_closes_session_without_moving_input() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, remote, _, peer, original) = setup(dir.path(), "left").await;
        let (engine, mut reloads, confirmed) = Engine::test_reloader();
        *runtime.engine.lock().await = Some(engine);
        let path = dir.path().join("deskflow/server.conf");
        let baseline = original.clone();
        let reloader = tokio::spawn(async move {
            reloads
                .recv()
                .await
                .unwrap()
                .send(Err(anyhow::anyhow!("rejected")))
                .unwrap();
            let restore = reloads.recv().await.unwrap();
            assert_eq!(std::fs::read(path).unwrap(), baseline);
            restore.send(Ok(())).unwrap();
            confirmed.send_modify(|n| *n += 1);
        });
        let link = runtime.link.lock().await.clone().unwrap();
        assert!(runtime
            .keyboard_switch(&link, peer, Some(0.5))
            .await
            .is_err());
        reloader.await.unwrap();
        assert!(link.session.closed.is_cancelled());
        assert!(!runtime
            .platform
            .resources
            .join("native/bin/mac-hotkey.calls")
            .exists());
        assert_eq!(
            std::fs::read(dir.path().join("deskflow/server.conf")).unwrap(),
            original
        );
        runtime
            .engine
            .lock()
            .await
            .as_mut()
            .unwrap()
            .stop()
            .await
            .unwrap();
        remote.close();
    }

    #[tokio::test]
    async fn missing_entry_or_failed_gate_restore_stops_instead_of_falling_back_to_jump() {
        for fail_restore in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (runtime, remote, _, peer, original) = setup(dir.path(), "left").await;
            let (engine, mut reloads, confirmed) = Engine::test_reloader();
            *runtime.engine.lock().await = Some(engine);
            let link = runtime.link.lock().await.clone().unwrap();
            let observing = runtime.clone();
            let context = link.clone();
            let source = runtime.snapshot().await.device_id;
            let target = peer.clone();
            let marker = runtime
                .platform
                .resources
                .join("native/bin/mac-hotkey.calls");
            let calls = marker.clone();
            let reloader = tokio::spawn(async move {
                reloads.recv().await.unwrap().send(Ok(())).unwrap();
                confirmed.send_modify(|n| *n += 1);
                if fail_restore {
                    while !marker.exists() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    let _ = observing.observed.send((
                        context.id,
                        Transition {
                            from: source,
                            to: target,
                            position: Some(Point { x: 1.0, y: 500.0 }),
                        },
                    ));
                }
                let restore = reloads.recv().await.unwrap();
                if fail_restore {
                    restore
                        .send(Err(anyhow::anyhow!("restore rejected")))
                        .unwrap();
                } else {
                    restore.send(Ok(())).unwrap();
                    confirmed.send_modify(|n| *n += 1);
                }
            });
            let error = runtime
                .keyboard_switch(&link, peer, Some(0.5))
                .await
                .unwrap_err();
            reloader.await.unwrap();
            assert!(link.session.closed.is_cancelled());
            assert!(format!("{error:#}").contains(if fail_restore {
                "无法恢复受控跨屏布局"
            } else {
                "未收到原生边缘跨屏确认"
            }));
            assert_eq!(std::fs::read_to_string(calls).unwrap().lines().count(), 2);
            assert_eq!(
                std::fs::read(dir.path().join("deskflow/server.conf")).unwrap(),
                original
            );
            assert_eq!(runtime.keyboard_pending.load(Ordering::Acquire), 0);
            runtime
                .engine
                .lock()
                .await
                .as_mut()
                .unwrap()
                .stop()
                .await
                .unwrap();
            remote.close();
        }
    }
}
