use super::*;

impl Runtime {
    pub(super) async fn start_engine(
        self: &Arc<Self>,
        link: &Arc<Link>,
        peer: &Identity,
        address: std::net::IpAddr,
    ) -> Result<()> {
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
        *guard = Some(Engine::start(binary, &c, &settings, &dir, tx).await?);
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
            while let Some(change) = changes.recv().await {
                if !this.current(&context) {
                    break;
                }
                this.observe_transition(&context, change).await;
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
                                if transitions.try_send(change).is_err() {
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
