use super::*;

impl Runtime {
    pub(super) async fn start_engine(
        self: &Arc<Self>,
        link: &Arc<Link>,
        peer: &Identity,
        address: std::net::IpAddr,
    ) -> Result<()> {
        let _lifecycle = self.start_stop.lock().await;
        let mut guard = self.engine.lock().await;
        ensure!(self.current(link), "共享会话已结束");
        ensure!(guard.is_none(), "上一份引擎尚未退出");
        let (mut c, dir, id) = {
            let i = self.inner.lock().await;
            (
                i.state.config.clone(),
                i.store.directory.clone(),
                i.state.device_id.clone(),
            )
        };
        if c.role == "client" {
            c.shortcut = peer.shortcut.clone();
            c.modifier_mapping = peer.modifier_mapping;
            c.clipboard = peer.clipboard;
            c.clipboard_megabytes = peer.clipboard_megabytes;
            c.deskflow_port = peer.deskflow_port;
            c.layout_side = peer.layout_side.clone();
            c.cross_screen_video = peer.cross_screen_video.clone();
        }
        let settings = engine::prepare(&c, &id, peer, &dir, &address.to_string())?;
        let binary = self.binary(&c)?;
        let (tx, mut rx) = mpsc::channel(64);
        let (transitions, mut changes) = mpsc::channel(32);
        let starting_at = std::time::Instant::now();
        let launch_mode = if cfg!(windows) && c.windows_secure_input && c.role == "client" {
            "windowsService"
        } else {
            "userProcess"
        };
        self.trace(
            "engine.startRequested",
            json!({"launchMode":launch_mode,"session":link.id}),
        )
        .await;
        #[cfg(windows)]
        let engine = if c.windows_secure_input && c.role == "client" {
            ensure!(c.deskflow_path.is_empty(), "安全桌面模式仅支持内置引擎");
            engine::assert_no_running_engine(&dir).await?;
            crate::windows_service::start(
                crate::windows_service::Launch {
                    protocol: crate::windows_service::PROTOCOL,
                    version: env!("CARGO_PKG_VERSION").into(),
                    local: uuid::Uuid::parse_str(&id)?,
                    host: peer.id,
                    address,
                    port: c.deskflow_port,
                    certificate: std::fs::read_to_string(dir.join("identity.pem"))?,
                    fingerprint: peer.fingerprint.clone(),
                    swap_modifiers: c.modifier_mapping.swaps_between(&peer.platform, "win32"),
                },
                tx,
            )
            .await?
        } else {
            Engine::start(binary.clone(), &c, &settings, &dir, tx).await?
        };
        #[cfg(not(windows))]
        let engine = Engine::start(binary.clone(), &c, &settings, &dir, tx).await?;
        self.trace("engine.started", json!({"pid":engine.pid(),"guardianPid":engine.guardian_pid(),"parentPid":std::process::id(),"binary":binary,"launchMode":launch_mode,"startupElapsedMs":starting_at.elapsed().as_millis(),"session":link.id,"version":env!("CARGO_PKG_VERSION")})).await;
        *guard = Some(engine);
        {
            let mut i = self.inner.lock().await;
            if self.current(link) {
                i.state.engine = "starting".into();
            }
        }
        self.publish();
        drop(guard);
        let this = self.clone();
        let context = link.clone();
        tokio::spawn(async move {
            while let Some((received, change)) = changes.recv().await {
                if !this.current(&context) {
                    break;
                }
                this.observe_transition_at(&context, change, received).await;
            }
        });
        let this = self.clone();
        let context = link.clone();
        let expected = peer.fingerprint.clone();
        let host = c.role == "host";
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if !this.current(&context) {
                    break;
                }
                match event {
                    engine::Event::ShortcutPressed(provider) => {
                        if this.switch_protection.busy() {
                            this.trace("switch.suppressed", json!({"entry":"nativeShortcut"}))
                                .await;
                            continue;
                        }
                        let runtime = this.clone();
                        let session = context.clone();
                        let received = std::time::Instant::now();
                        tokio::spawn(async move {
                            if let Err(error) = runtime
                                .native_switch_shortcut_at(&session, &provider, received)
                                .await
                            {
                                runtime.log("error", &format!("{error:#}")).await;
                            }
                        });
                    }
                    engine::Event::Ipc(line) => {
                        if line == "observerDisconnected" {
                            this.log("error", "引擎状态通道已断开，已停止本次共享")
                                .await;
                            context.session.close();
                            break;
                        }
                        if let Some(actual) = line.strip_prefix("peerFingerprint=") {
                            if actual.replace(':', "").to_lowercase() != expected {
                                this.log("error", "引擎连接证书与配对身份不一致，已停止共享")
                                    .await;
                                context.session.close();
                                break;
                            }
                        }
                        if line == "connectionState=Connected" {
                            {
                                let mut i = this.inner.lock().await;
                                if this.current(&context) {
                                    i.state.engine = "ready".into();
                                }
                            }
                            this.publish();
                        } else if line.starts_with("connectionState=") {
                            {
                                let mut i = this.inner.lock().await;
                                if this.current(&context) {
                                    i.state.engine = "starting".into();
                                }
                            }
                            this.publish();
                        }
                        this.trace("engine.ipc", json!({"line":line})).await;
                    }
                    engine::Event::Output(line) => {
                        let lower = line.to_lowercase();
                        // Keep input-capture lifecycle evidence without logging
                        // ordinary keystrokes or publishing every engine line.
                        if lower.contains("quartz event tap") || lower.contains("hotkey") {
                            this.trace("engine.inputDiagnostic", json!({"line":line}))
                                .await;
                        }
                        if lower.contains("fingerprint does not match")
                            || lower.contains("failed to verify") && lower.contains("fingerprint")
                        {
                            this.log("error", "引擎证书校验失败，已停止共享").await;
                            context.session.close();
                            break;
                        }
                        if host {
                            if let Some(change) = parse_transition(&line) {
                                let _ = this.observed.send((context.id, change.clone()));
                                if transitions
                                    .try_send((std::time::Instant::now(), change))
                                    .is_err()
                                {
                                    context.session.close();
                                    break;
                                }
                            }
                        }
                        if (lower.contains("failed")
                            || lower.contains("cannot")
                            || lower.contains("error"))
                            && !lower.contains("screenbridge-probe")
                        {
                            this.diagnostic_line(&line).await;
                        }
                    }
                    engine::Event::Exited(reason) => {
                        {
                            let mut i = this.inner.lock().await;
                            if this.current(&context) {
                                i.state.engine = "error".into();
                            }
                        }
                        this.log("error", &format!("键鼠引擎已退出：{reason}"))
                            .await;
                        context.session.close();
                        break;
                    }
                }
            }
        });
        let this = self.clone();
        let context = link.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(25)).await;
            if this.current(&context) && this.snapshot().await.engine != "ready" {
                this.log("error", "引擎连接超时，请检查两端权限、端口和诊断日志")
                    .await;
                context.session.close();
            }
        });
        Ok(())
    }
    #[cfg(test)]
    pub(super) async fn native_switch_shortcut(
        &self,
        link: &Arc<Link>,
        provider: &str,
    ) -> Result<()> {
        self.native_switch_shortcut_at(link, provider, std::time::Instant::now())
            .await
    }

    async fn native_switch_shortcut_at(
        &self,
        link: &Arc<Link>,
        provider: &str,
        received: std::time::Instant,
    ) -> Result<()> {
        let s = self.snapshot().await;
        let eligible = self.current(link)
            && s.connected
            && s.engine == "ready"
            && s.config.role == "host"
            && s.device_id == provider
            && (s
                .session_layout
                .as_ref()
                .is_some_and(|l| l.video == "manual")
                || keep_mode(&s));
        self.trace("shortcut.pressed", json!({"backend":"deskflowNative","active":s.active_device_id,"role":s.config.role,"eligible":eligible,"session":link.id})).await;
        if eligible && !self.switch_protection.blocks(received) {
            self.command_switch_target(link, None).await?;
        }
        Ok(())
    }
    pub(super) async fn diagnostic_line(&self, line: &str) {
        let mut i = self.inner.lock().await;
        let now = std::time::Instant::now();
        if let Some((last, count, when)) = &mut i.diagnostic_line {
            if last == line && now.duration_since(*when) < Duration::from_secs(5) {
                *count += 1;
                return;
            }
            let summary = (*count > 1).then(|| json!({"line":last,"count":count}));
            if let Some(summary) = summary {
                let _ = i.diagnostics.record("engine.repeated", summary);
            }
        }
        i.diagnostic_line = Some((line.into(), 1, now));
        drop(i);
        self.log("error", line).await;
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn provider_native_shortcut_round_trips_without_receiver_callbacks_and_rejects_stale_events(
    ) {
        for mode in ["manual", "keep"] {
            let directory = tempfile::tempdir().unwrap();
            let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "host");
            let local = runtime.snapshot().await.device_id;
            {
                let mut i = runtime.inner.lock().await;
                i.state.engine = "ready".into();
                i.state.session_layout = Some(Layout {
                    host_side: "left".into(),
                    video: mode.into(),
                });
                i.state.config.cross_screen_video = mode.into();
                i.state.config.display_id = "external".into();
                i.state.config.local_input = Some(3);
                i.state.config.peer_input = Some(1);
                i.state.active_device_id = Some(local.clone());
                i.state.display_device_id = Some(local.clone());
                i.state.peer = Some(Peer {
                    id: "receiver".into(),
                    name: "Receiver".into(),
                    platform: "darwin".into(),
                    fingerprint: "a".repeat(64),
                    file_transfer: false,
                    lock_control: false,
                    input_configured: true,
                    retained_display: true,
                    modifier_mapping: ModifierMapping::Common,
                });
            }
            let native = runtime.platform.resources.join("native/bin");
            std::fs::create_dir_all(&native).unwrap();
            let input = native.join("mac-hotkey");
            let ddc = native.join("m1ddc");
            for (file, script) in [
                (&input, "#!/bin/sh\ncase \"$1\" in host|peer) echo \"$1\" >> \"$0.calls\"; echo sent ;; *) exit 9 ;; esac\n"),
                (&ddc, "#!/bin/sh\ntest \"$3\" = set || exit 9\necho \"$5\" >> \"$0.calls\"\n"),
            ] {
                std::fs::write(file, script).unwrap();
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let (remote, mut requests) =
                super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
            let link = runtime.link.lock().await.clone().unwrap();
            let replying = remote.clone();
            let writes = Arc::new(AtomicU64::new(0));
            let count = writes.clone();
            let provider = local.clone();
            let receiver = tokio::spawn(async move {
                while let Some(event) = requests.recv().await {
                    if let Incoming::Request {
                        id,
                        method,
                        payload,
                    } = event
                    {
                        assert_eq!(
                            method, "set-input",
                            "The receiver must not be asked to detect or relay the shortcut"
                        );
                        let next = count.fetch_add(1, Ordering::AcqRel);
                        assert_eq!(
                            payload,
                            json!(if next % 2 == 0 { "receiver" } else { &provider })
                        );
                        replying.reply(id, Ok(json!(true))).await.unwrap();
                    }
                }
            });
            runtime
                .native_switch_shortcut(&link, "wrong-provider")
                .await
                .unwrap();
            assert!(!input.with_extension("calls").exists());
            for step in 0..8 {
                let target = if step % 2 == 0 {
                    "receiver".to_owned()
                } else {
                    local.clone()
                };
                let rt = runtime.clone();
                let context = link.clone();
                let calls = input.with_extension("calls");
                let to = target.clone();
                let observation = tokio::spawn(async move {
                    tokio::time::timeout(Duration::from_secs(3), async {
                        while std::fs::read_to_string(&calls)
                            .unwrap_or_default()
                            .lines()
                            .count()
                            <= step
                        {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    })
                    .await
                    .unwrap();
                    let from = rt.snapshot().await.active_device_id.unwrap();
                    rt.observe_transition(
                        &context,
                        Transition {
                            from,
                            to,
                            position: None,
                        },
                    )
                    .await;
                });
                runtime.native_switch_shortcut(&link, &local).await.unwrap();
                observation.await.unwrap();
                let s = runtime.snapshot().await;
                assert_eq!(s.active_device_id.as_deref(), Some(target.as_str()));
                assert_eq!(s.display_device_id.as_deref(), Some(target.as_str()));
                assert!(!runtime.command_pending.load(Ordering::Acquire));
            }
            link.session.close();
            runtime.native_switch_shortcut(&link, &local).await.unwrap();
            assert_eq!(writes.load(Ordering::Acquire), 8);
            assert_eq!(
                std::fs::read_to_string(input.with_extension("calls"))
                    .unwrap()
                    .lines()
                    .count(),
                8
            );
            assert_eq!(
                std::fs::read_to_string(ddc.with_extension("calls"))
                    .unwrap()
                    .lines()
                    .count(),
                8
            );
            remote.close();
            receiver.await.unwrap();
        }
    }
}
