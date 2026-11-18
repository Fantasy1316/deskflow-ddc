use super::*;

async fn wait_identified(
    mut peer: watch::Receiver<Option<Identity>>,
    closed: &CancellationToken,
    timeout: Duration,
) -> Result<Identity> {
    tokio::select! {
        biased;
        _ = closed.cancelled() => bail!("共享会话已结束"),
        result = tokio::time::timeout(timeout, peer.wait_for(|p| p.is_some())) => {
            let peer = result.context("双向设备识别超时")?.context("设备识别通道已关闭")?;
            peer.clone().context("双向设备识别未完成")
        }
    }
}

async fn wait_topology(mut ready: watch::Receiver<bool>, cancelled: &CancellationToken) -> bool {
    tokio::select! {
        biased;
        _ = cancelled.cancelled() => false,
        result = ready.wait_for(|ready| *ready) => result.is_ok(),
    }
}

async fn wait_retry(
    checks: &mut watch::Receiver<u64>,
    topology: &mut watch::Receiver<bool>,
    cancelled: &CancellationToken,
    delay: Duration,
) -> Option<&'static str> {
    tokio::select! {
        biased;
        _ = cancelled.cancelled() => None,
        result = checks.changed() => result.ok().map(|_| "recovery"),
        result = topology.changed() => result.ok().map(|_| "topology"),
        _ = tokio::time::sleep(delay) => Some("timer"),
    }
}

async fn wait_connection_unavailable(
    mut events: watch::Receiver<crate::system_events::State>,
    mut topology: watch::Receiver<bool>,
    power: u64,
) {
    loop {
        let state = events.borrow_and_update().clone();
        if state.sleeping || state.power != power || !*topology.borrow_and_update() {
            return;
        }
        tokio::select! {
            result = events.changed() => { if result.is_err() { return; } },
            result = topology.changed() => { if result.is_err() { return; } },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn retry_wait_is_interrupted_by_recovery_or_topology_without_losing_early_events() {
        let events = crate::system_events::Events::default();
        let mut checks = events.connection_checks();
        let (ready, mut topology) = watch::channel(true);
        let cancelled = CancellationToken::new();
        let delay = Duration::from_secs(60);
        {
            let waiting = wait_retry(&mut checks, &mut topology, &cancelled, delay);
            tokio::pin!(waiting);
            assert!(futures_util::poll!(&mut waiting).is_pending());
            events.emit(crate::system_events::Event::ScreenWake);
            assert_eq!(waiting.await, Some("recovery"));
        }
        events.request_connection_check(); // Arrived during a previous connect.
        assert_eq!(
            wait_retry(&mut checks, &mut topology, &cancelled, delay).await,
            Some("recovery")
        );
        ready.send_replace(false);
        assert_eq!(
            wait_retry(&mut checks, &mut topology, &cancelled, delay).await,
            Some("topology")
        );
        ready.send_replace(true);
        assert_eq!(
            wait_retry(&mut checks, &mut topology, &cancelled, delay).await,
            Some("topology")
        );
        events.request_connection_check();
        cancelled.cancel();
        assert_eq!(
            wait_retry(&mut checks, &mut topology, &cancelled, delay).await,
            None
        );
    }

    #[tokio::test]
    async fn screen_wake_checks_live_session_without_tearing_it_down_in_both_roles_and_modes() {
        for role in ["host", "client"] {
            for method in [DisplayMethod::Ddc, DisplayMethod::Signal] {
                let dir = tempfile::tempdir().unwrap();
                let rt = super::super::pointer_bridge::tests::fixture(dir.path(), role);
                rt.inner.lock().await.state.config.display_method = method;
                let before = rt.snapshot().await;
                let (remote, mut incoming) =
                    super::super::pointer_bridge::tests::attach_with_requests(&rt).await;
                let link = rt.link.lock().await.clone().unwrap();
                let cloned = rt.clone();
                let current = link.clone();
                let task = tokio::spawn(async move { cloned.maintain_connection(&current).await });
                // Answer the initial heartbeat; the next timer tick is 1s away.
                let Some(Incoming::Request { id, method, .. }) = incoming.recv().await else {
                    panic!("missing ping")
                };
                assert_eq!(method, "ping");
                remote.reply(id, Ok(json!(true))).await.unwrap();
                rt.system_events
                    .emit(crate::system_events::Event::ScreenWake);
                let next = tokio::time::timeout(Duration::from_millis(500), incoming.recv())
                    .await
                    .unwrap();
                let Some(Incoming::Request { id, method, .. }) = next else {
                    panic!("missing immediate ping")
                };
                assert_eq!(method, "ping");
                remote.reply(id, Ok(json!(true))).await.unwrap();
                assert!(rt.current(&link));
                assert!(!link.session.closed.is_cancelled());
                let after = rt.snapshot().await;
                assert_eq!(after.display_device_id, before.display_device_id);
                assert_eq!(after.active_device_id, before.active_device_id);
                assert!(rt.display_recovery.lock().await.pending().is_none());
                remote.close();
                assert!(tokio::time::timeout(Duration::from_secs(1), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err());
            }
        }
    }

    #[tokio::test]
    async fn interrupted_discovery_releases_busy_state_and_preserves_previous_hints() {
        let dir = tempfile::tempdir().unwrap();
        let rt = super::super::pointer_bridge::tests::fixture(dir.path(), "client");
        let device = discovery::Device {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Provider".into(),
            hostname: "provider.local".into(),
            addresses: vec![],
            port: 45990,
        };
        rt.inner
            .lock()
            .await
            .state
            .discovered_devices
            .push(device.clone());
        // Hold the lock so this test needs no actual mDNS network access.
        let _lock = rt.discovery.lock().await;
        assert!(rt.discover_until(async {}).await.unwrap());
        let after = rt.snapshot().await;
        assert!(!after.discovering);
        assert_eq!(after.discovered_devices[0].id, device.id);
    }

    #[tokio::test]
    async fn client_connects_to_verified_address_before_starting_discovery() {
        let dir = tempfile::tempdir().unwrap();
        let rt = super::super::pointer_bridge::tests::fixture(dir.path(), "client");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        {
            let mut i = rt.inner.lock().await;
            i.state.config.peer_address = "not-ready.local".into();
            i.state.config.control_port = address.port();
            i.store.value.trusted_peer = Some(crate::model::Pin {
                id: uuid::Uuid::new_v4(),
                fingerprint: "a".repeat(64),
            });
            i.store.value.last_verified_address = Some(address.ip());
        }
        let cancelled = CancellationToken::new();
        let running = rt.clone();
        let token = cancelled.clone();
        let task = tokio::spawn(async move { running.client_loop(1, token).await });
        let (socket, _) = tokio::time::timeout(Duration::from_millis(500), listener.accept())
            .await
            .unwrap()
            .unwrap();
        assert!(!rt.snapshot().await.discovering);
        cancelled.cancel();
        drop(socket);
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn sleep_interrupts_pending_handshake_but_screen_wake_does_not() {
        for host in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let rt = super::super::pointer_bridge::tests::fixture(
                dir.path(),
                if host { "host" } else { "client" },
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let socket = TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (_remote, _) = listener.accept().await.unwrap();
            let cancel = CancellationToken::new();
            let attempt = rt.run_socket(socket, 1, 1, host, "test", cancel);
            tokio::pin!(attempt);
            assert!(futures_util::poll!(&mut attempt).is_pending());
            rt.system_events
                .emit(crate::system_events::Event::ScreenWake);
            assert!(futures_util::poll!(&mut attempt).is_pending());
            rt.system_events.emit(crate::system_events::Event::Sleep);
            assert!(tokio::time::timeout(Duration::from_millis(500), attempt)
                .await
                .unwrap()
                .is_err());
            assert!(rt.link.lock().await.is_none());
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn real_receiver_accepts_retained_landing_in_all_video_modes_but_rejects_invalid_targets()
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let runtime = super::super::pointer_bridge::tests::fixture(dir.path(), "client");
        let remote = super::super::pointer_bridge::tests::attach_with_requests(&runtime)
            .await
            .0;
        let link = runtime.link.lock().await.clone().unwrap();
        let mut host = super::super::tests::peer();
        host.role = "host".into();
        link.peer.send_replace(Some(host.clone()));
        let helper = runtime.platform.resources.join("native/bin/mac-hotkey");
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(&helper, "#!/bin/sh\n[ \"$1\" = pointer ] || exit 9\necho \"$*\" >> \"$0.calls\"\necho positioned\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        for video in ["follow", "manual", "keep"] {
            runtime.inner.lock().await.state.session_layout = Some(Layout {
                host_side: "left".into(),
                video: video.into(),
            });
            assert_eq!(
                runtime
                    .request(&link, "land-pointer", Value::Null)
                    .await
                    .unwrap(),
                json!(true)
            );
        }
        runtime.inner.lock().await.state.config.retained_display_id = "missing".into();
        assert!(runtime
            .request(&link, "land-pointer", Value::Null)
            .await
            .is_err());
        runtime.inner.lock().await.state.config.retained_display_id = "internal".into();
        runtime
            .inner
            .lock()
            .await
            .state
            .session_layout
            .as_mut()
            .unwrap()
            .host_side = "off".into();
        assert!(runtime
            .request(&link, "land-pointer", Value::Null)
            .await
            .is_err());
        runtime
            .inner
            .lock()
            .await
            .state
            .session_layout
            .as_mut()
            .unwrap()
            .host_side = "left".into();
        host.role = "client".into();
        link.peer.send_replace(Some(host));
        assert!(runtime
            .request(&link, "land-pointer", Value::Null)
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(helper.with_extension("calls"))
                .unwrap()
                .lines()
                .count(),
            3
        );
        remote.close();
    }

    #[tokio::test]
    async fn waits_for_delayed_reciprocal_identification_without_missing_completion() {
        let (identified, peer) = watch::channel(None);
        let closed = CancellationToken::new();
        let waiting = wait_identified(peer, &closed, Duration::from_secs(10));
        tokio::pin!(waiting);
        // Our own identify response has arrived, but its reciprocal handler has
        // not run yet. This must remain pending rather than failing the session.
        assert!(futures_util::poll!(&mut waiting).is_pending());
        let expected = super::super::tests::peer();
        identified.send_replace(Some(expected.clone()));
        assert_eq!(waiting.await.unwrap().id, expected.id);
        // Completion before subscribing must work as well (no lost wakeup).
        assert_eq!(
            wait_identified(identified.subscribe(), &closed, Duration::from_secs(10))
                .await
                .unwrap()
                .id,
            expected.id
        );
    }
    #[tokio::test]
    async fn identification_wait_ends_on_disconnect_or_timeout() {
        let (_identified, peer) = watch::channel(None);
        let closed = CancellationToken::new();
        let waiting = wait_identified(peer.clone(), &closed, Duration::from_secs(10));
        tokio::pin!(waiting);
        assert!(futures_util::poll!(&mut waiting).is_pending());
        closed.cancel();
        assert!(waiting
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("会话已结束"));
        assert!(
            wait_identified(peer, &CancellationToken::new(), Duration::ZERO)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("识别超时")
        );
    }

    #[tokio::test]
    async fn topology_wait_resumes_after_restore_but_stop_takes_precedence() {
        let (ready, receiver) = watch::channel(false);
        let cancelled = CancellationToken::new();
        let waiting = wait_topology(receiver, &cancelled);
        tokio::pin!(waiting);
        assert!(futures_util::poll!(&mut waiting).is_pending());
        ready.send_replace(true);
        assert!(waiting.await);
        // Restoring before a loop subscribes must not lose the notification.
        assert!(wait_topology(ready.subscribe(), &cancelled).await);
        cancelled.cancel();
        assert!(!wait_topology(ready.subscribe(), &cancelled).await);
        ready.send_replace(false);
        assert!(!wait_topology(ready.subscribe(), &cancelled).await);
    }
}

impl Runtime {
    pub(super) async fn host_loop(
        self: Arc<Self>,
        listener: TcpListener,
        epoch: u64,
        cancel: CancellationToken,
    ) {
        loop {
            if !wait_topology(self.topology_ready.subscribe(), &cancel).await {
                break;
            }
            let result = tokio::select! {
                _ = cancel.cancelled() => break,
                result = listener.accept() => result,
            };
            match result {
                Ok((socket, address)) => {
                    let id = self.session_sequence.fetch_add(1, Ordering::AcqRel) + 1;
                    if self
                        .active_session
                        .compare_exchange(0, id, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                    {
                        continue;
                    }
                    let this = self.clone();
                    let cancel = cancel.clone();
                    tokio::spawn(async move {
                        let result = this
                            .run_socket(socket, epoch, id, true, "incoming", cancel)
                            .await;
                        if let Err(e) = result {
                            this.log("error", &format!("来自 {address} 的连接结束：{e}"))
                                .await;
                        }
                        this.cleanup_session(epoch, id).await;
                    });
                }
                Err(e) => {
                    self.log("error", &format!("控制监听失败：{e}")).await;
                    break;
                }
            }
        }
    }
    pub(super) async fn client_loop(self: Arc<Self>, epoch: u64, cancel: CancellationToken) {
        // Subscribe before attempting anything: a wake/topology recovery during
        // connect/handshake must not be lost when we later enter backoff.
        let mut checks = self.system_events.connection_checks();
        let mut topology = self.topology_ready.subscribe();
        let mut next_discovery = tokio::time::Instant::now();
        while !cancel.is_cancelled() && self.epoch.load(Ordering::Acquire) == epoch {
            if !wait_topology(topology.clone(), &cancel).await {
                break;
            }
            if checks.has_changed().unwrap_or(false) {
                next_discovery = tokio::time::Instant::now();
            }
            checks.borrow_and_update();
            topology.borrow_and_update();
            let (c, pin, cache, discovered) = {
                let i = self.inner.lock().await;
                (
                    i.state.config.clone(),
                    i.store.value.trusted_peer.clone(),
                    i.store.value.last_verified_address,
                    i.state.discovered_devices.clone(),
                )
            };
            let candidates = discovery::immediate_candidates(&c, pin.as_ref(), cache, &discovered);
            let quick_retry = !candidates.is_empty();
            let mut attempted = vec![];
            let ended = self
                .try_candidates(candidates, &mut attempted, epoch, &cancel)
                .await;
            // A verified address normally reconnects before any mDNS/DNS wait.
            // Refresh discovery after failures, but not on every fast retry.
            if !ended
                && !cancel.is_cancelled()
                && self.epoch.load(Ordering::Acquire) == epoch
                && self.system_events.ready()
                && *self.topology_ready.borrow()
                && tokio::time::Instant::now() >= next_discovery
            {
                let interrupt = async {
                    tokio::select! {
                        _ = cancel.cancelled() => {},
                        _ = checks.changed() => {},
                        _ = topology.changed() => {},
                    }
                };
                match self.discover_until(interrupt).await {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(e) => {
                        self.trace("discovery.unavailable", json!({"error":e.to_string()}))
                            .await
                    }
                }
                next_discovery = tokio::time::Instant::now() + Duration::from_secs(5);
                let discovered = self.snapshot().await.discovered_devices;
                let candidates = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    _ = checks.changed() => continue,
                    _ = topology.changed() => continue,
                    candidates = discovery::candidates(&c, pin.as_ref(), cache, &discovered) => candidates,
                };
                match candidates {
                    Ok(candidates) => {
                        self.try_candidates(candidates, &mut attempted, epoch, &cancel)
                            .await;
                    }
                    Err(e) => self.log("error", &e.to_string()).await,
                }
            }
            let known = {
                let i = self.inner.lock().await;
                quick_retry
                    || !discovery::immediate_candidates(
                        &i.state.config,
                        i.store.value.trusted_peer.as_ref(),
                        i.store.value.last_verified_address,
                        &i.state.discovered_devices,
                    )
                    .is_empty()
            };
            let delay = Duration::from_secs(if known { 1 } else { 5 });
            let waiting = std::time::Instant::now();
            let Some(cause) = wait_retry(&mut checks, &mut topology, &cancel, delay).await else {
                break;
            };
            self.trace(
                "connection.retry",
                json!({"cause":cause,
                "waitedMs":waiting.elapsed().as_millis()}),
            )
            .await;
        }
    }

    async fn try_candidates(
        self: &Arc<Self>,
        candidates: Vec<discovery::Candidate>,
        attempted: &mut Vec<std::net::SocketAddr>,
        epoch: u64,
        cancel: &CancellationToken,
    ) -> bool {
        for candidate in candidates {
            if cancel.is_cancelled()
                || self.epoch.load(Ordering::Acquire) != epoch
                || !self.system_events.ready()
                || !*self.topology_ready.borrow()
            {
                return false;
            }
            if attempted.contains(&candidate.address) {
                continue;
            }
            attempted.push(candidate.address);
            self.trace(
                "connection.attempt",
                json!({"address":candidate.address.to_string(),
                "source":candidate.source}),
            )
            .await;
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => return false,
                _ = wait_connection_unavailable(self.system_events.subscribe(), self.topology_ready.subscribe(), self.system_events.state().power) => return false,
                result = discovery::connect(&candidate) => result,
            };
            match result {
                Ok(socket) => {
                    let id = self.session_sequence.fetch_add(1, Ordering::AcqRel) + 1;
                    self.active_session.store(id, Ordering::Release);
                    let result = self
                        .run_socket(socket, epoch, id, false, candidate.source, cancel.clone())
                        .await;
                    let verified = self.snapshot().await.connected;
                    if let Err(e) = result {
                        self.log(
                            "error",
                            &format!(
                                "连接 {}（{}）结束：{e}",
                                candidate.address, candidate.source
                            ),
                        )
                        .await;
                    }
                    self.cleanup_session(epoch, id).await;
                    if verified {
                        return true;
                    }
                }
                Err(e) => {
                    self.trace(
                        "address.failed",
                        json!({
                            "address":candidate.address.to_string(), "source":candidate.source,
                            "error":e.to_string(),
                        }),
                    )
                    .await
                }
            }
        }
        false
    }
    pub(super) async fn run_socket(
        self: &Arc<Self>,
        socket: TcpStream,
        epoch: u64,
        id: u64,
        host: bool,
        source: &str,
        cancel: CancellationToken,
    ) -> Result<()> {
        ensure!(
            *self.topology_ready.borrow() && self.system_events.ready(),
            "独立屏幕暂不可用，等待恢复后自动重连"
        );
        let address = socket.peer_addr()?.ip();
        let key = {
            let i = self.inner.lock().await;
            protocol::pairing_key(&i.store.pairing)?
        };
        let wire = tokio::select! {
            biased;
            _ = cancel.cancelled() => bail!("共享已停止"),
            _ = wait_connection_unavailable(self.system_events.subscribe(), self.topology_ready.subscribe(), self.system_events.state().power) => bail!("显示连接正在恢复，重新尝试配对"),
            result = tokio::time::timeout(
                Duration::from_secs(8), Wire::handshake(socket, key, host),
            ) => result.context("配对超时")??,
        };
        ensure!(self.epoch.load(Ordering::Acquire) == epoch, "共享已停止");
        ensure!(
            self.system_events.ready() && *self.topology_ready.borrow(),
            "显示连接正在恢复，重新尝试配对"
        );
        let (session, mut incoming) = Session::spawn(wire);
        let link = Arc::new(Link {
            address,
            id,
            epoch,
            session: session.clone(),
            peer: watch::channel(None).0,
            input_generation: AtomicU64::new(0),
            input_entries: AtomicU64::new(0),
        });
        {
            let mut slot = self.link.lock().await;
            ensure!(
                self.session_matches(epoch, id) && !cancel.is_cancelled(),
                "共享已停止"
            );
            *slot = Some(link.clone());
        }
        let this = self.clone();
        let context = link.clone();
        let handler = tokio::spawn(async move {
            let limit = Arc::new(tokio::sync::Semaphore::new(8));
            while let Some(next) = incoming.recv().await {
                if !this.current(&context) {
                    break;
                }
                match next {
                    Incoming::Request {
                        id,
                        method,
                        payload,
                    } => {
                        if matches!(
                            method.as_str(),
                            "switch"
                                | "toggle"
                                | "confirm-display-source"
                                | "detect-display-source"
                        ) && this.switch_protection.busy()
                        {
                            let _ = context
                                .session
                                .reply(
                                    id,
                                    Err(anyhow::anyhow!("正在切换或等待显示器稳定，请稍后重试")),
                                )
                                .await;
                            continue;
                        }
                        let Ok(permit) = limit.clone().try_acquire_owned() else {
                            context.session.close();
                            break;
                        };
                        let this = this.clone();
                        let link = context.clone();
                        let received = std::time::Instant::now();
                        tokio::spawn(async move {
                            let _permit = permit;
                            let result = if matches!(
                                method.as_str(),
                                "switch"
                                    | "toggle"
                                    | "confirm-display-source"
                                    | "detect-display-source"
                            ) && this.switch_protection.blocks(received)
                            {
                                Err(anyhow::anyhow!("正在切换或等待显示器稳定，请稍后重试"))
                            } else {
                                this.request(&link, &method, payload).await
                            };
                            let _ = link.session.reply(id, result).await;
                        });
                    }
                    Incoming::Event { name, payload } => {
                        if let Err(e) = this.event(&context, &name, payload).await {
                            this.log("error", &e.to_string()).await;
                            context.session.close();
                        }
                    }
                }
            }
        });
        let work = self.identify_and_run(&link, address, source);
        let result = tokio::select! {
            _=cancel.cancelled()=>Err(anyhow::anyhow!("共享已停止")),
            result=work=>result,
        };
        session.close();
        handler.abort();
        result
    }
    pub(super) async fn identify_and_run(
        self: &Arc<Self>,
        link: &Arc<Link>,
        address: std::net::IpAddr,
        source: &str,
    ) -> Result<()> {
        self.transfers.reset_network();
        let identified_at = std::time::Instant::now();
        let response = link
            .session
            .request(
                "identify",
                serde_json::to_value(self.identity().await?)?,
                Duration::from_secs(10),
            )
            .await?;
        self.transfers
            .observe_control_latency(identified_at.elapsed());
        let response: Identity = serde_json::from_value(response)?;
        response.validate()?;
        let peer = wait_identified(
            link.peer.subscribe(),
            &link.session.closed,
            Duration::from_secs(10),
        )
        .await?;
        ensure!(
            response.id == peer.id && response.fingerprint == peer.fingerprint,
            "双向设备识别不一致"
        );
        {
            let mut i = self.inner.lock().await;
            ensure!(self.current(link), "共享会话已结束");
            i.store.value.last_verified_address = Some(address);
            i.store.save()?;
            i.state.connection =
                Some(json!({"address":address,"source":source,"authenticated":true}));
        }
        self.log("success", &format!("已与 {} 建立加密连接", peer.name))
            .await;
        if self.snapshot().await.config.role == "host" {
            // Both identities/configurations are available before querying.
            // Reading must not require starting or transferring keyboard input.
            if let Err(error) = self.initialize_display_source(link).await {
                self.trace(
                    "display.initialSourceSkipped",
                    json!({"error":error.to_string()}),
                )
                .await;
            }
        }
        self.start_engine(link, &peer, address).await?;
        self.maintain_connection(link).await
    }

    async fn maintain_connection(&self, link: &Link) -> Result<()> {
        let mut checks = self.system_events.connection_checks();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _=link.session.closed.cancelled()=>bail!("对端连接已断开"),
                result = checks.changed() => {
                    if result.is_err() { bail!("连接检查通道已关闭"); }
                    self.trace("connection.check", json!({"cause":"recovery"})).await;
                }
                _ = heartbeat.tick() => {}
            }
            // A screen wake checks the existing encrypted session. Only a real
            // RPC failure ends it; waking our own signal output is harmless.
            let ping = std::time::Instant::now();
            let result = link
                .session
                .request("ping", Value::Null, Duration::from_secs(5))
                .await;
            self.transfers.observe_control_latency(ping.elapsed());
            result?;
        }
    }
    pub(super) fn session_matches(&self, epoch: u64, id: u64) -> bool {
        self.epoch.load(Ordering::Acquire) == epoch
            && self.active_session.load(Ordering::Acquire) == id
    }
    pub(super) async fn cleanup_session(&self, epoch: u64, id: u64) {
        {
            let mut guard = self.link.lock().await;
            if !self.session_matches(epoch, id) {
                return;
            }
            if let Some(link) = guard.take() {
                link.session.close();
            }
        }
        // Drain old effects before admitting a new session; an in-flight native
        // command must not change the picture after the new session starts.
        let _command = self.commands.lock().await;
        let _effects = self.effects.lock().await;
        self.switch_protection.wait_released().await;
        if !self.session_matches(epoch, id) {
            return;
        }
        if let Err(error) = self.restore_local_output().await {
            self.log("error", &format!("共享结束后恢复显示输出失败：{error:#}"))
                .await;
        }
        {
            let mut guard = self.engine.lock().await;
            if !self.session_matches(epoch, id) {
                return;
            }
            if let Some(engine) = guard.as_mut() {
                let pid = engine.pid();
                if let Err(e) = engine.stop().await {
                    self.log("error", &e.to_string()).await;
                    return;
                }
                self.trace("engine.stopped", json!({"pid":pid,"reason":"sessionEnded"}))
                    .await;
            }
            *guard = None;
        }
        let mut i = self.inner.lock().await;
        if !self.session_matches(epoch, id) {
            return;
        }
        if i.state.config.display_method == DisplayMethod::Signal {
            i.state.reset_display();
        }
        i.state.control_disconnected();
        self.active_session
            .compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire)
            .ok();
        self.publish();
    }
    pub(super) async fn request(
        self: &Arc<Self>,
        link: &Arc<Link>,
        method: &str,
        payload: Value,
    ) -> Result<Value> {
        ensure!(self.current(link), "共享会话已结束");
        if method == "identify" {
            let peer: Identity = serde_json::from_value(payload)?;
            peer.validate()?;
            let mut i = self.inner.lock().await;
            ensure!(self.current(link), "共享会话已结束");
            // The inner lock serializes duplicate identify requests through save.
            ensure!(link.peer.borrow().is_none(), "已完成设备识别");
            let c = &i.state.config;
            validate_peer(
                c,
                i.store.value.id,
                i.store.value.trusted_peer.as_ref(),
                &peer,
            )?;
            if let Err(error) = i.diagnostics.record(
                "peer.identified",
                json!({"version":peer.app_version,"platform":peer.platform,"id":peer.id}),
            ) {
                i.state.diagnostics["error"] = error.to_string().into();
            }
            i.store.value.trusted_peer = Some(Pin {
                id: peer.id,
                fingerprint: peer.fingerprint.clone(),
            });
            i.store.save()?;
            i.state.has_saved_pairing = true;
            i.state.peer = Some(Peer {
                file_transfer: peer.file_protocol == 1,
                lock_control: peer.lock_control,
                id: peer.id.to_string(),
                name: peer.name.clone(),
                platform: peer.platform.clone(),
                fingerprint: peer.fingerprint.clone(),
                input_configured: true,
                retained_display: peer.retained_display,
                modifier_mapping: peer.modifier_mapping,
            });
            i.state.session_layout = Some(if c_role(&i.state.config) {
                Layout {
                    host_side: i.state.config.layout_side.clone(),
                    video: i.state.config.cross_screen_video.clone(),
                }
            } else {
                Layout {
                    host_side: peer.layout_side.clone(),
                    video: peer.cross_screen_video.clone(),
                }
            });
            let host = if c_role(&i.state.config) {
                i.state.device_id.clone()
            } else {
                peer.id.to_string()
            };
            i.state.control_connected(host);
            link.peer.send_replace(Some(peer));
            drop(i);
            self.publish();
            return Ok(serde_json::to_value(self.identity().await?)?);
        }
        let peer = link.peer.borrow().clone().context("尚未识别对端设备")?;
        match method {
            "file-cancel" => {
                ensure!(peer.file_protocol == 1, "对端未声明文件传输协议");
                let id: uuid::Uuid = serde_json::from_value(payload)?;
                self.transfers.cancel_from_peer(id, peer.id).await?;
                Ok(json!(true))
            }
            "file-offer" => {
                ensure!(peer.file_protocol == 1, "对端未声明文件传输协议");
                let invitation: crate::transfer::Invitation = serde_json::from_value(payload)?;
                self.transfers
                    .incoming(invitation, link.address, peer.id, peer.name)?;
                Ok(json!(true))
            }
            "ping" => Ok(json!("pong")),
            "detect-display-source" => {
                ensure!(
                    self.snapshot().await.config.role == "host",
                    "只有提供端可以协调来源检测"
                );
                self.detect_display_source().await?;
                Ok(json!(true))
            }
            "confirm-display-source" => {
                self.confirm_display_source_on(link, target_id(&payload)?)
                    .await?;
                Ok(json!(true))
            }
            "lock-session" => {
                let s = self.snapshot().await;
                ensure!(
                    s.config.role == "client" && platform() == "win32" && peer.role == "host",
                    "只允许提供端锁定 Windows 接收端"
                );
                ensure!(
                    s.running && s.engine == "ready" && self.current(link),
                    "共享引擎尚未就绪"
                );
                ensure!(
                    payload.as_object().is_some_and(|v| v.is_empty()),
                    "锁屏参数无效"
                );
                self.platform.lock_session().await?;
                self.trace("system.lockAccepted", json!({"session":link.id}))
                    .await;
                Ok(json!(true))
            }
            "output-action" => {
                ensure!(
                    peer.output_protocol == crate::output_guard::PROTOCOL
                        && peer.display_method == DisplayMethod::Signal
                        && peer.role == "host",
                    "不允许的显示输出请求"
                );
                let operation = payload.as_str().context("显示输出指令无效")?;
                let _effects = self.effects.lock().await;
                ensure!(self.current(link), "共享会话已结束");
                self.local_output_action(operation).await?;
                Ok(json!(true))
            }
            "restore-output" => {
                ensure!(
                    peer.role == "client"
                        && peer.output_protocol == crate::output_guard::PROTOCOL
                        && peer.display_method == DisplayMethod::Signal
                        && self.snapshot().await.config.role == "host",
                    "不允许的显示输出恢复请求"
                );
                let request: output_switch::RestoreRequest = serde_json::from_value(payload)?;
                self.restore_output(request).await?;
                Ok(json!(true))
            }
            "get-input-source" => {
                ensure!(
                    self.snapshot().await.config.display_method == DisplayMethod::Ddc,
                    "自动选源模式不读取 DDC 输入"
                );
                let source = self.read_display_source(&peer.id.to_string()).await?;
                ensure!(self.current(link), "共享会话已结束");
                Ok(json!(source))
            }
            "display-connection" => {
                let s = self.snapshot().await;
                let connected = self
                    .platform
                    .display_connected(&s.config.display_id)
                    .await?;
                ensure!(self.current(link), "共享会话已结束");
                Ok(json!(connected))
            }
            "toggle" => {
                ensure!(
                    self.snapshot().await.config.role == "host",
                    "只有提供端可以协调切换"
                );
                self.command_switch_target(link, None).await?;
                Ok(json!(true))
            }
            "set-input" => {
                let target = target_id(&payload)?;
                let s = self.snapshot().await;
                ensure!(
                    s.config.role == "client" && s.config.display_method == DisplayMethod::Ddc,
                    "只有 DDC 接收端执行提供端的远程显示器指令"
                );
                let _effects = self.effects.lock().await;
                ensure!(self.current(link), "共享会话已结束");
                ensure!(
                    target == s.device_id || target == peer.id.to_string(),
                    "无效的切换目标"
                );
                let value = if target == s.device_id {
                    s.config.local_input.context("未设置本机输入源")?
                } else {
                    s.config.peer_input.context("未设置对端输入源")?
                };
                self.trace(
                    "display.remoteWriteRequested",
                    json!({"target":target,"monitor":s.config.display_id,"value":value,"inputProtocol":s.config.input_protocol()}),
                )
                .await;
                let result = self
                    .platform
                    .input_with_protocol(&s.config.display_id, value, s.config.input_protocol())
                    .await;
                self.trace(
                    "display.remoteWriteCompleted",
                    json!({"target":target,"monitor":s.config.display_id,"value":value,"inputProtocol":s.config.input_protocol(),
                        "commandAccepted":result.is_ok(),"pictureVerified":false,
                        "error":result.as_ref().err().map(|e|e.to_string())}),
                )
                .await;
                result?;
                ensure!(self.current(link), "共享会话已结束");
                Ok(
                    json!({"monitor":s.config.display_id,"value":value,"inputProtocol":s.config.input_protocol(),
                    "commandAccepted":true,"pictureVerified":false}),
                )
            }
            "land-pointer" => {
                let _effects = self.effects.lock().await;
                let s = self.snapshot().await;
                ensure!(
                    s.config.role == "client"
                        && peer.role == "host"
                        && s.connected
                        && s.session_layout
                            .as_ref()
                            .is_some_and(|l| l.host_side != "off"),
                    "当前没有启用独立屏幕跨屏"
                );
                // The provider also lands on a retained screen while follow
                // mode is settling. Video policy is decided there; land() still
                // requires an available, explicitly selected retained screen.
                self.land(link.id).await?;
                Ok(json!(true))
            }
            "place-shared-pointer" => {
                let _effects = self.effects.lock().await;
                ensure!(
                    self.snapshot().await.config.role == "client" && peer.role == "host",
                    "只有接收端接受提供端的鼠标落点请求"
                );
                let position: f64 = serde_json::from_value(payload)?;
                self.place_shared_pointer(link, position).await?;
                Ok(json!(true))
            }
            "cross-screen" => {
                let s = self.snapshot().await;
                ensure!(
                    s.config.role == "host"
                        && keep_mode(&s)
                        && s.active_device_id.as_deref() == Some(&peer.id.to_string()),
                    "当前不能跨屏"
                );
                // Null remains accepted for older peers; new peers preserve
                // the crossing fraction when the native edge is gated.
                let position: Option<f64> = serde_json::from_value(payload)?;
                self.keyboard_switch(link, s.device_id, position).await?;
                Ok(json!(true))
            }
            "switch" => {
                ensure!(
                    self.snapshot().await.config.role == "host",
                    "只有提供端可以协调切换"
                );
                self.command_switch(link, target_id(&payload)?).await?;
                Ok(json!(true))
            }
            _ => bail!("不支持的控制请求"),
        }
    }
    pub(super) async fn event(&self, link: &Link, name: &str, payload: Value) -> Result<()> {
        ensure!(
            self.current(link) && link.peer.borrow().is_some(),
            "尚未识别对端设备"
        );
        if name == "display-source" {
            return self.receive_display_source(link, payload).await;
        }
        if name == "display-state" && self.snapshot().await.config.role == "client" {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct RemoteState {
                active: Option<String>,
                display: Option<String>,
                display_status: Option<String>,
                last_display: Option<String>,
                busy: bool,
                error: Option<String>,
                #[serde(default)]
                output_switch: Option<output_switch::SwitchRecord>,
                #[serde(default)]
                switch_progress: Option<SwitchProgress>,
            }
            let v: RemoteState = serde_json::from_value(payload)?;
            let mut i = self.inner.lock().await;
            ensure!(self.current(link), "共享会话已结束");
            for id in [&v.active, &v.display, &v.last_display]
                .into_iter()
                .flatten()
            {
                ensure!(
                    id == &i.state.device_id || i.state.peer.as_ref().is_some_and(|p| &p.id == id),
                    "无效的状态设备"
                );
            }
            let status = v.display_status.unwrap_or_else(|| {
                if v.display.is_some() {
                    "commanded"
                } else {
                    "uninitialized"
                }
                .into()
            });
            ensure!(
                ["commanded", "uninitialized", "uncertain"].contains(&status.as_str()),
                "无效的画面状态"
            );
            if let Some(completed) = &v.output_switch {
                ensure!(
                    i.state.config.display_method == DisplayMethod::Signal
                        && (completed.target == i.state.device_id
                            || i.state
                                .peer
                                .as_ref()
                                .is_some_and(|p| p.id == completed.target)),
                    "无效的输出切换状态"
                );
            }
            i.state.output_switch = v.output_switch;
            if !i.state.emergency_stopping {
                i.state.switch_progress = v.switch_progress;
            }
            i.state.active_device_id = v.active;
            i.state.display_device_id = v.display;
            i.state.display_status = status;
            i.state.display_source = None;
            i.state.last_display_device_id = v.last_display;
            i.state.switching = v.busy;
            drop(i);
            if let Some(e) = v.error {
                self.log("error", &e.chars().take(400).collect::<String>())
                    .await;
            }
            self.publish();
        }
        Ok(())
    }
}
