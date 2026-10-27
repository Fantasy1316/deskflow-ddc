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
        let panels: Vec<Panel> = serde_json::from_value(self.platform.screens().await?)?;
        let missing = {
            let i = self.inner.lock().await;
            i.state.running
                && !i.state.config.retained_display_id.is_empty()
                && pointer::selected(&panels, &i.state.config.retained_display_id).is_none()
        };
        if missing {
            self.log("error", "所选独立屏幕已断开，已停止共享").await;
            self.stop().await?;
        }
        let mut i = self.inner.lock().await;
        if i.state.cursor_displays != panels {
            i.state.cursor_displays = panels;
            self.publish();
        }
        Ok(())
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
