use super::*;

impl Runtime {
    pub(super) async fn read_display_source(&self, peer_id: &str) -> Result<Option<String>> {
        let s = self.snapshot().await;
        let value = self.platform.current_input(&s.config.display_id).await?;
        Ok(value.and_then(|value| {
            if Some(value) == s.config.local_input {
                Some(s.device_id)
            } else if Some(value) == s.config.peer_input {
                Some(peer_id.into())
            } else {
                None
            }
        }))
    }
    async fn write_display_with_retry(&self, link: &Link, target: &str) -> Result<()> {
        let initial = self.snapshot().await;
        // A read reply may be stale, cached or connection-specific. It is not
        // evidence of the physical picture and must not rewrite routing/target.
        ensure!(
            initial.display_status != "uncertain",
            "画面状态不确定，请手动恢复提供端画面并重新开始共享"
        );
        let source = initial
            .display_device_id
            .as_ref()
            .context("尚未确定外接屏的画面来源")?;
        let alternate = if source == &initial.device_id {
            initial.peer.as_ref().map(|peer| peer.id.clone())
        } else {
            Some(initial.device_id.clone())
        };
        let first = self.attempt_display_input(link, source, target).await;
        ensure!(self.current(link), "共享会话已结束");
        let result = if let Some(alternate) = alternate {
            self.trace(
                if first.is_ok() {
                    "display.unverifiedRetry"
                } else {
                    "display.failedRetry"
                },
                json!({"source":alternate,"target":target,
                    "firstCommandAccepted":first.is_ok(),"pictureVerified":false,
                    "firstError":first.as_ref().err().map(|e|e.to_string())}),
            )
            .await;
            // A successful API call proves bus acceptance, not a visible picture.
            // The inactive input may silently ignore it. Send the SAME target
            // once through the other endpoint, even after an accepted command.
            // Do not infer a second toggle from a VCP read or keyboard ownership.
            if first.is_ok() {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            ensure!(self.current(link), "共享会话已结束");
            let second = self.attempt_display_input(link, &alternate, target).await;
            match (first, second) {
                (Ok(()), _) | (_, Ok(())) => Ok(()),
                (Err(first), Err(second)) => Err(second)
                    .with_context(|| format!("首次输入源写入失败：{first}；另一端重试也失败")),
            }
        } else {
            first
        };
        // Commit only once: a failed supplementary write must not erase an
        // accepted first command or cause the keyboard to be restored incorrectly.
        self.finish_display_write(link, target, initial, result)
            .await
    }
    async fn send_display_input(&self, link: &Link, source: &str, target: &str) -> Result<Value> {
        let s = self.snapshot().await;
        ensure!(self.current(link), "共享会话已结束");
        if source == s.device_id {
            let value = if target == s.device_id {
                s.config.local_input.context("未设置本机输入源")?
            } else {
                s.config.peer_input.context("未设置对端输入源")?
            };
            self.trace(
                "display.localWriteRequested",
                json!({"target":target,"monitor":s.config.display_id,"value":value}),
            )
            .await;
            self.platform.input(&s.config.display_id, value).await?;
            Ok(json!({"monitor":s.config.display_id,"value":value,
                "commandAccepted":true,"pictureVerified":false}))
        } else {
            // Older peers return true; newer peers also include their configured
            // monitor/input value for diagnosis. Neither is picture verification.
            link.session
                .request("set-input", json!(target), Duration::from_secs(10))
                .await
        }
    }
    pub(super) async fn publish_control(&self, link: &Link, error: Option<String>) {
        if !self.current(link) {
            return;
        }
        let s = self.snapshot().await;
        let _=link.session.notify("display-state",json!({"active":s.active_device_id,"display":s.display_device_id,"lastDisplay":s.last_display_device_id,"displayStatus":s.display_status,"busy":s.switching||self.command_pending.load(Ordering::Acquire),"error":error})).await;
        self.publish();
    }
    pub(super) async fn write_display(&self, link: &Link, target: &str) -> Result<()> {
        let s = self.snapshot().await;
        ensure!(
            s.display_status != "uncertain",
            "画面状态不确定，请手动恢复提供端画面并重新开始共享"
        );
        let source = s
            .display_device_id
            .as_ref()
            .context("尚未确定外接屏的画面来源")?;
        self.write_display_from(link, source, target).await
    }
    async fn write_display_from(&self, link: &Link, source: &str, target: &str) -> Result<()> {
        let s = self.snapshot().await;
        let result = self.attempt_display_input(link, source, target).await;
        self.finish_display_write(link, target, s, result).await
    }
    async fn attempt_display_input(&self, link: &Link, source: &str, target: &str) -> Result<()> {
        let s = self.snapshot().await;
        self.trace(
            "display.writeRequested",
            json!({"source":source,"target":target}),
        )
        .await;
        let result = self.send_display_input(link, source, target).await;
        ensure!(self.current(link), "共享会话已结束");
        self.trace(
            "display.writeCompleted",
            json!({
                "source": source,
                "target": target,
                "transport": if source == s.device_id { "local" } else { "remote" },
                "commandAccepted": result.is_ok(),
                "pictureVerified": false,
                "details": result.as_ref().ok(),
                "error": result.as_ref().err().map(|e| e.to_string()),
            }),
        )
        .await;
        result.map(|_| ())
    }
    async fn finish_display_write(
        &self,
        link: &Link,
        target: &str,
        before: Snapshot,
        result: Result<()>,
    ) -> Result<()> {
        let mut i = self.inner.lock().await;
        ensure!(self.current(link), "共享会话已结束");
        match result {
            Ok(()) => {
                i.state.display_device_id = Some(target.into());
                i.state.last_display_device_id = Some(target.into());
                i.state.display_status = "commanded".into();
                Ok(())
            }
            Err(e) => {
                i.state.last_display_device_id =
                    before.display_device_id.or(before.last_display_device_id);
                i.state.display_device_id = None;
                i.state.display_status = "uncertain".into();
                Err(e)
            }
        }
    }
    pub(super) async fn observe_transition(&self, link: &Arc<Link>, change: Transition) {
        let _effects = self.effects.lock().await;
        if !self.current(link) {
            return;
        }
        let s = self.snapshot().await;
        self.trace("engine.transition", json!({"session":link.id,"from":change.from,"to":change.to,"active":s.active_device_id,"display":s.display_device_id,"commandPending":self.command_pending.load(Ordering::Acquire)})).await;
        let Some(peer) = &s.peer else { return };
        if ![s.device_id.as_str(), peer.id.as_str()].contains(&change.from.as_str())
            || ![s.device_id.as_str(), peer.id.as_str()].contains(&change.to.as_str())
            || change.from == change.to
        {
            return;
        }
        if s.active_device_id.as_ref() == Some(&change.to) {
            // A queued edge can complete the requested move before its injected
            // hotkey is processed. Still acknowledge the real engine transition.
            let _ = self.completed.send(Completed {
                session: link.id,
                to: change.to,
                error: None,
            });
            return;
        }
        self.inner.lock().await.state.switching = true;
        self.publish_control(link, None).await;
        let result: Result<()> = async {
            let retained = if change.to == s.device_id {
                pointer::selected(&s.cursor_displays, &s.config.retained_display_id).is_some()
            } else {
                peer.retained_display
            };
            let effect = crate::switching::decide(crate::switching::Policy {
                explicit: self.command_pending.load(Ordering::Acquire),
                manual: s
                    .session_layout
                    .as_ref()
                    .is_some_and(|l| l.video == "manual"),
                keep: keep_mode(&s),
                target_retained: retained,
                provider_retained: pointer::selected(
                    &s.cursor_displays,
                    &s.config.retained_display_id,
                )
                .is_some(),
                from_provider: change.from == s.device_id,
                requested: self.keyboard_pending.load(Ordering::Acquire) == link.id,
                video_source: s.display_device_id.as_deref(),
                target: &change.to,
                uncertain: s.display_status == "uncertain",
            })?;
            match effect {
                crate::switching::Effect::None => {}
                crate::switching::Effect::Write => self.write_display(link, &change.to).await?,
                crate::switching::Effect::Land => {
                    if change.to == s.device_id {
                        self.land(link.id).await?;
                    } else {
                        link.session
                            .request("land-pointer", Value::Null, Duration::from_secs(10))
                            .await?;
                    }
                }
            }
            Ok(())
        }
        .await;
        if !self.current(link) {
            return;
        }
        let error = result.err().map(|e| e.to_string());
        let active = if error.is_some() {
            match self.inject_wait(link, &change.from).await {
                Ok(()) => {
                    if change.from == s.device_id {
                        let _ = self.recover_pointer(link.id).await;
                    }
                    Some(change.from.clone())
                }
                Err(_) => None,
            }
        } else {
            Some(change.to.clone())
        };
        {
            let mut i = self.inner.lock().await;
            if !self.current(link) {
                return;
            }
            i.state.active_device_id = active;
            i.state.switching = false;
        }
        if let Some(error) = &error {
            self.log("error", error).await;
        } else {
            self.log(
                "success",
                if keep_mode(&s)
                    || s.session_layout
                        .as_ref()
                        .is_some_and(|l| l.video == "manual")
                {
                    "键鼠已切换"
                } else {
                    "显示器输入指令已发送，请以实际画面为准"
                },
            )
            .await;
        }
        self.publish_control(link, error.clone()).await;
        let _ = self.completed.send(Completed {
            session: link.id,
            to: change.to,
            error,
        });
    }
    pub(super) async fn inject_wait(&self, link: &Link, target: &str) -> Result<()> {
        ensure!(self.current(link), "共享会话已结束");
        let mut observed = self.observed.subscribe();
        let host = self.snapshot().await.device_id == target;
        ensure!(self.current(link), "共享会话已结束");
        self.platform.inject(host).await?;
        let wait = async {
            loop {
                let (session, change) = observed.recv().await?;
                if session == link.id && change.to == target {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        };
        tokio::select! {
            _=link.session.closed.cancelled()=>bail!("共享会话已结束"),
            result=tokio::time::timeout(Duration::from_millis(2500),wait)=>result.context("未收到 Deskflow 键鼠恢复确认")?,
        }
    }
    pub(super) async fn keyboard_switch(
        &self,
        link: &Arc<Link>,
        target: String,
        cross: Option<f64>,
    ) -> Result<()> {
        // Serialize the owner check and injection with transition effects. Drop
        // this before awaiting completion so the observer can acknowledge it.
        let effects = self.effects.lock().await;
        ensure!(self.current(link), "共享会话已结束");
        let s = self.snapshot().await;
        ensure!(
            s.engine == "ready" && !s.switching,
            "键鼠尚未就绪或正在切换"
        );
        ensure!(
            target == s.device_id || s.peer.as_ref().is_some_and(|p| p.id == target),
            "无效的切换目标"
        );
        if s.active_device_id.as_ref() == Some(&target) {
            return Ok(());
        }
        if !self.command_pending.load(Ordering::Acquire)
            && s.session_layout
                .as_ref()
                .is_some_and(|l| l.video == "manual")
        {
            let retained = if target == s.device_id {
                pointer::selected(&s.cursor_displays, &s.config.retained_display_id).is_some()
            } else {
                s.peer.as_ref().is_some_and(|p| p.retained_display)
            };
            ensure!(
                s.display_status != "uncertain"
                    && (s.display_device_id.as_ref() == Some(&target) || retained),
                "目标没有可见屏幕，请使用快捷键或按钮切换画面"
            );
        }
        ensure!(
            self.keyboard_pending
                .compare_exchange(0, link.id, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "正在切换，请稍后"
        );
        let result=async {
            let mut completed=self.completed.subscribe();
            ensure!(self.current(link),"共享会话已结束");
            if let Some(position)=cross {
                let side=s.session_layout.as_ref().context("未设置布局")?.host_side.as_str();
                let p=pointer::exit(&s.cursor_displays,side,position)?;
                self.platform.point(p.x,p.y).await?;
            } else { self.platform.inject(target==s.device_id).await?; }
            drop(effects);
            let wait=async {
                loop {
                    let change=completed.recv().await?;
                    if change.session==link.id && change.to==target {
                        return match change.error { Some(error)=>Err(anyhow::anyhow!(error)),None=>Ok(()) };
                    }
                }
            };
            tokio::select! {
                _=link.session.closed.cancelled()=>bail!("共享会话已结束"),
                result=tokio::time::timeout(Duration::from_secs(12),wait)=>result.context("未收到 Deskflow 切换完成确认，请检查引擎日志与辅助功能权限")?,
            }
        }.await;
        self.keyboard_pending
            .compare_exchange(link.id, 0, Ordering::AcqRel, Ordering::Acquire)
            .ok();
        result
    }
    pub(super) async fn command_switch(&self, link: &Arc<Link>, target: String) -> Result<()> {
        self.command_switch_target(link, Some(target)).await
    }
    pub(super) async fn command_switch_target(
        &self,
        link: &Arc<Link>,
        target: Option<String>,
    ) -> Result<()> {
        let _command = self
            .commands
            .try_lock()
            .map_err(|_| anyhow::anyhow!("正在切换，请稍后"))?;
        ensure!(self.current(link), "共享会话已结束");
        let s = self.snapshot().await;
        // Resolve toggles at the provider using the last picture command,
        // independently of keyboard ownership or unverified VCP read replies.
        let target = match target {
            Some(target) => target,
            None if s.display_device_id.as_ref() == Some(&s.device_id) => {
                s.peer.as_ref().context("对端未连接")?.id.clone()
            }
            None => s.device_id.clone(),
        };
        self.trace("switch.command", json!({"target":target,"active":s.active_device_id,"display":s.display_device_id,"session":link.id})).await;
        ensure!(
            s.display_status != "uncertain",
            "画面状态不确定，请手动恢复提供端画面后重新开始共享"
        );
        self.command_pending.store(true, Ordering::Release);
        let result = async {
            self.keyboard_switch(link, target.clone(), None).await?;
            let _effects = self.effects.lock().await;
            ensure!(self.current(link), "共享会话已结束");
            ensure!(
                self.snapshot().await.active_device_id.as_ref() == Some(&target),
                "键鼠已发生新的跨屏，请重试"
            );
            self.inner.lock().await.state.switching = true;
            self.publish_control(link, None).await;
            let result = self.write_display_with_retry(link, &target).await;
            if result.is_err() && self.current(link) {
                if let Some(from) = &s.active_device_id {
                    if from != &target {
                        let restored = self.inject_wait(link, from).await.is_ok();
                        self.inner.lock().await.state.active_device_id =
                            restored.then(|| from.clone());
                    }
                }
            }
            result
        }
        .await;
        self.command_pending.store(false, Ordering::Release);
        if self.current(link) {
            self.inner.lock().await.state.switching = false;
            self.publish_control(link, result.as_ref().err().map(|e| e.to_string()))
                .await;
        }
        result
    }
    pub async fn switch_to(&self, local: bool) -> Result<()> {
        if self.snapshot().await.config.mode == OperationMode::DisplayOnly {
            return self.switch_display_only(Some(local)).await;
        }
        let link = self
            .link
            .lock()
            .await
            .clone()
            .context("另一台电脑尚未连接")?;
        let s = self.snapshot().await;
        let target = if local {
            s.device_id
        } else {
            s.peer.context("另一台电脑尚未连接")?.id
        };
        if s.config.role == "host" {
            self.command_switch(&link, target).await
        } else {
            link.session
                .request("switch", json!(target), Duration::from_secs(60))
                .await
                .map(|_| ())
        }
    }
    pub async fn toggle(&self) -> Result<()> {
        if self.snapshot().await.config.mode == OperationMode::DisplayOnly {
            return self.switch_display_only(None).await;
        }
        let s = self.snapshot().await;
        let link = self
            .link
            .lock()
            .await
            .clone()
            .context("另一台电脑尚未连接")?;
        if s.config.role == "host" {
            self.command_switch_target(&link, None).await
        } else {
            link.session
                .request("toggle", Value::Null, Duration::from_secs(60))
                .await
                .map(|_| ())
        }
    }
    pub async fn test_input(&self, value: u16) -> Result<()> {
        let _lifecycle = self.start_stop.lock().await;
        let s = self.snapshot().await;
        ensure!(!s.running, "请先停止共享");
        s.config.ready()?;
        ensure!(
            Some(value) == s.config.local_input || Some(value) == s.config.peer_input,
            "只能测试已保存输入源"
        );
        let target = {
            let mut i = self.inner.lock().await;
            let target = if Some(value) == s.config.local_input {
                Some(s.device_id.clone())
            } else {
                i.store
                    .value
                    .trusted_peer
                    .as_ref()
                    .map(|p| p.id.to_string())
            };
            // A timed-out or cancelled call may already have changed the monitor.
            // Never retain the previous source as a usable DDC sender.
            i.state.display_device_id = None;
            i.state.display_target = None;
            i.state.display_status = "uncertain".into();
            target
        };
        self.publish();
        self.platform.input(&s.config.display_id, value).await?;
        if s.config.mode == OperationMode::DisplayOnly {
            let mut i = self.inner.lock().await;
            i.state.display_target = Some(
                if Some(value) == s.config.local_input {
                    "local"
                } else {
                    "peer"
                }
                .into(),
            );
            i.state.display_status = "commanded".into();
        } else if let Some(target) = target {
            let mut i = self.inner.lock().await;
            i.state.display_device_id = Some(target.clone());
            i.state.last_display_device_id = Some(target);
            i.state.display_status = "commanded".into();
        }
        self.log("info", "输入切换指令已发送，请以实际画面为准")
            .await;
        Ok(())
    }
}
