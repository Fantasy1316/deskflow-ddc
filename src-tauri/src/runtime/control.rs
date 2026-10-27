use super::*;

impl Runtime {
    pub(super) async fn host_loop(
        self: Arc<Self>,
        listener: TcpListener,
        epoch: u64,
        cancel: CancellationToken,
    ) {
        loop {
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
        while !cancel.is_cancelled() && self.epoch.load(Ordering::Acquire) == epoch {
            if let Err(e) = self.discover().await {
                self.trace("discovery.unavailable", json!({"error":e.to_string()}))
                    .await;
            }
            let (c, pin, cache) = {
                let i = self.inner.lock().await;
                (
                    i.state.config.clone(),
                    i.store.value.trusted_peer.clone(),
                    i.store.value.last_verified_address,
                )
            };
            let discovered = self.snapshot().await.discovered_devices;
            match discovery::candidates(&c, pin.as_ref(), cache, &discovered).await {
                Ok(candidates) => {
                    for candidate in candidates {
                        if cancel.is_cancelled() || self.epoch.load(Ordering::Acquire) != epoch {
                            return;
                        }
                        let result = tokio::select! {
                            _ = cancel.cancelled() => return,
                            result = discovery::connect(&candidate) => result,
                        };
                        match result {
                            Ok(socket) => {
                                let id = self.session_sequence.fetch_add(1, Ordering::AcqRel) + 1;
                                self.active_session.store(id, Ordering::Release);
                                let result = self
                                    .run_socket(
                                        socket,
                                        epoch,
                                        id,
                                        false,
                                        candidate.source,
                                        cancel.clone(),
                                    )
                                    .await;
                                let verified = self.snapshot().await.connected;
                                if let Err(e) = result {
                                    self.log(
                                        "error",
                                        &format!(
                                            "连接 {}（{}）结束：{e}",
                                            candidate.address, candidate.source,
                                        ),
                                    )
                                    .await;
                                }
                                self.cleanup_session(epoch, id).await;
                                if verified {
                                    break;
                                }
                            }
                            Err(e) => {
                                self.trace(
                                    "address.failed",
                                    json!({
                                        "address": candidate.address.to_string(),
                                        "source": candidate.source,
                                        "error": e.to_string(),
                                    }),
                                )
                                .await
                            }
                        }
                    }
                }
                Err(e) => self.log("error", &e.to_string()).await,
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            }
        }
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
        let address = socket.peer_addr()?.ip();
        let key = {
            let i = self.inner.lock().await;
            protocol::pairing_key(&i.store.pairing)?
        };
        let wire = tokio::select! {
            _ = cancel.cancelled() => bail!("共享已停止"),
            result = tokio::time::timeout(
                Duration::from_secs(8), Wire::handshake(socket, key, host),
            ) => result.context("配对超时")??,
        };
        ensure!(self.epoch.load(Ordering::Acquire) == epoch, "共享已停止");
        let (session, mut incoming) = Session::spawn(wire);
        let link = Arc::new(Link {
            id,
            epoch,
            session: session.clone(),
            peer: Mutex::new(None),
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
                        let Ok(permit) = limit.clone().try_acquire_owned() else {
                            context.session.close();
                            break;
                        };
                        let this = this.clone();
                        let link = context.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            let result = this.request(&link, &method, payload).await;
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
        let response = link
            .session
            .request(
                "identify",
                serde_json::to_value(self.identity().await?)?,
                Duration::from_secs(10),
            )
            .await?;
        let response: Identity = serde_json::from_value(response)?;
        response.validate()?;
        let peer = link
            .peer
            .lock()
            .await
            .clone()
            .context("双向设备识别未完成")?;
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
        self.start_engine(link, &peer, address).await?;
        let mut heartbeat = tokio::time::interval(Duration::from_secs(4));
        loop {
            tokio::select! {
                _=link.session.closed.cancelled()=>bail!("对端连接已断开"),
                _ = heartbeat.tick() => {
                    link.session.request("ping", Value::Null, Duration::from_secs(5)).await?;
                }
            }
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
        {
            let mut guard = self.engine.lock().await;
            if !self.session_matches(epoch, id) {
                return;
            }
            if let Some(engine) = guard.as_mut() {
                if let Err(e) = engine.stop().await {
                    self.log("error", &e.to_string()).await;
                    return;
                }
            }
            *guard = None;
        }
        let mut i = self.inner.lock().await;
        if !self.session_matches(epoch, id) {
            return;
        }
        i.state.connected = false;
        i.state.engine = "stopped".into();
        i.state.peer = None;
        i.state.session_layout = None;
        i.state.active_device_id = None;
        i.state.display_device_id = None;
        i.state.last_display_device_id = None;
        i.state.display_status = "uninitialized".into();
        i.state.switching = false;
        i.state.connection = None;
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
            let mut identified = link.peer.lock().await;
            ensure!(identified.is_none(), "已完成设备识别");
            let peer: Identity = serde_json::from_value(payload)?;
            peer.validate()?;
            let mut i = self.inner.lock().await;
            ensure!(self.current(link), "共享会话已结束");
            let c = &i.state.config;
            validate_peer(
                c,
                i.store.value.id,
                i.store.value.trusted_peer.as_ref(),
                &peer,
            )?;
            i.store.value.trusted_peer = Some(Pin {
                id: peer.id,
                fingerprint: peer.fingerprint.clone(),
            });
            i.store.save()?;
            i.state.peer = Some(Peer {
                id: peer.id.to_string(),
                name: peer.name.clone(),
                platform: peer.platform.clone(),
                fingerprint: peer.fingerprint.clone(),
                input_configured: true,
                retained_display: peer.retained_display,
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
            i.state.connected = true;
            let host = if c_role(&i.state.config) {
                i.state.device_id.clone()
            } else {
                peer.id.to_string()
            };
            i.state.active_device_id = Some(host.clone());
            i.state.display_device_id = Some(host.clone());
            i.state.last_display_device_id = Some(host);
            *identified = Some(peer);
            drop(i);
            drop(identified);
            self.publish();
            return Ok(serde_json::to_value(self.identity().await?)?);
        }
        let peer = link.peer.lock().await.clone().context("尚未识别对端设备")?;
        match method {
            "ping" => Ok(json!("pong")),
            "set-input" => {
                let target = target_id(&payload)?;
                let s = self.snapshot().await;
                ensure!(
                    s.config.role == "client",
                    "只有接收端执行提供端的远程显示器指令"
                );
                let _effects = self.effects.lock().await;
                ensure!(self.current(link), "共享会话已结束");
                ensure!(
                    target == s.device_id || target == peer.id.to_string(),
                    "无效的切换目标"
                );
                self.platform
                    .input(
                        &s.config.display_id,
                        if target == s.device_id {
                            s.config.local_input.unwrap()
                        } else {
                            s.config.peer_input.unwrap()
                        },
                    )
                    .await?;
                ensure!(self.current(link), "共享会话已结束");
                Ok(json!(true))
            }
            "land-pointer" => {
                let _effects = self.effects.lock().await;
                let s = self.snapshot().await;
                ensure!(
                    s.config.role == "client" && keep_mode(&s),
                    "当前没有启用独立屏幕跨屏"
                );
                self.land(link.id).await?;
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
                self.keyboard_switch(link, s.device_id, None).await?;
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
            self.current(link) && link.peer.lock().await.is_some(),
            "尚未识别对端设备"
        );
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
            i.state.active_device_id = v.active;
            i.state.display_device_id = v.display;
            i.state.display_status = status;
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
