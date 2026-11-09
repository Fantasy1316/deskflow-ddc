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
        let mut payload = json!({"active":s.active_device_id,"display":s.display_device_id,"lastDisplay":s.last_display_device_id,"displayStatus":s.display_status,"busy":s.switching||self.command_pending.load(Ordering::Acquire),"error":error});
        // Legacy DDC peers reject unknown fields. Signal peers negotiate the
        // matching output protocol before exchanging this additional state.
        if s.config.display_method == DisplayMethod::Signal {
            payload["outputSwitch"] = json!(s.output_switch);
        }
        if link
            .peer
            .borrow()
            .as_ref()
            .is_some_and(|p| p.output_protocol == crate::output_guard::PROTOCOL)
        {
            // The existing output protocol predates WaitingDisplay. Keep its
            // wire enum compatible so an older receiver does not disconnect.
            payload["switchProgress"] = json!(s.switch_progress.map(|mut progress| {
                if progress.stage == SwitchStage::WaitingDisplay {
                    progress.stage = if s.config.display_method == DisplayMethod::Signal {
                        SwitchStage::Committing
                    } else {
                        SwitchStage::WritingInput
                    };
                }
                progress
            }));
        }
        let _ = link.session.notify("display-state", payload).await;
        // Additional named events are ignored by legacy peers. Do not add
        // fields/enums to their strictly validated display-state payload.
        if let Some(source) = s.display_source {
            let _ = link.session.notify("display-source", json!(source)).await;
        }
        self.publish();
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
        i.state.display_source = None;
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
    #[cfg(test)]
    pub(super) async fn observe_transition(self: &Arc<Self>, link: &Arc<Link>, change: Transition) {
        self.observe_transition_at(link, change, std::time::Instant::now())
            .await;
    }
    pub(super) async fn observe_transition_at(
        self: &Arc<Self>,
        link: &Arc<Link>,
        change: Transition,
        received: std::time::Instant,
    ) {
        let effects = self.effects.lock().await;
        if !self.current(link) {
            return;
        }
        let s = self.snapshot().await;
        self.trace("engine.transition", json!({"session":link.id,"from":change.from,"to":change.to,"position":change.position,"active":s.active_device_id,"display":s.display_device_id,"commandPending":self.command_pending.load(Ordering::Acquire)})).await;
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
                position: change.position,
            });
            return;
        }
        self.inner.lock().await.state.switching = true;
        self.publish_control(link, None).await;
        let mut protection = None;
        let mut suppressed = false;
        let result: Result<crate::switching::Effect> = async {
            let retained = if change.to == s.device_id {
                pointer::selected(&s.cursor_displays, &s.config.retained_display_id).is_some()
            } else {
                peer.retained_display
            };
            let explicit = self.command_pending.load(Ordering::Acquire)
                && self
                    .switch_protection
                    .expected_transfer(&change.to, received);
            let protected = self.switch_protection.blocks(received);
            if protected
                && !explicit
                && !retained
                && (s.display_status == "uncertain"
                    || s.display_device_id.as_ref() != Some(&change.to))
            {
                suppressed = true;
                bail!("正在等待显示器稳定，已忽略本次边缘切源");
            }
            let effect = if protected && !explicit && retained {
                // Returning to an independent visible screen never changes the
                // monitor input, even when follow mode normally would do so.
                crate::switching::Effect::Land
            } else {
                crate::switching::decide(crate::switching::Policy {
                    explicit,
                    manual: s
                        .session_layout
                        .as_ref()
                        .is_some_and(|l| l.video == "manual"),
                    keep: keep_mode(&s),
                    target_retained: retained,
                    video_source: s.display_device_id.as_deref(),
                    target: &change.to,
                    uncertain: s.display_status == "uncertain",
                })?
            };
            match effect {
                crate::switching::Effect::None => {}
                crate::switching::Effect::Write => {
                    ensure!(!protected, "正在等待显示器稳定，已忽略本次边缘切源");
                    protection = Some(self.switch_protection.enter(Some(&change.to))?);
                    // Edge and explicit switching share the same endpoint retry
                    // contract. Retry one fixed target, never a second toggle.
                    self.write_display_with_retry(link, &change.to).await?;
                }
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
            Ok(effect)
        }
        .await;
        if !self.current(link) {
            return;
        }
        let wrote_display = matches!(result.as_ref(), Ok(crate::switching::Effect::Write));
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
            i.state.switching = protection.is_some() || self.switch_protection.settling();
        }
        if let Some(error) = &error {
            if suppressed {
                self.trace("switch.suppressed", json!({"entry":"edge","to":change.to,"receivedBeforeRelease":!self.switch_protection.busy()})).await;
            } else {
                self.log("error", error).await;
            }
        } else {
            self.log(
                "success",
                if wrote_display {
                    "显示器输入指令已发送，请以实际画面为准"
                } else {
                    "键鼠已切换"
                },
            )
            .await;
        }
        self.publish_control(link, error.clone().filter(|_| !suppressed))
            .await;
        let _ = self.completed.send(Completed {
            session: link.id,
            to: change.to.clone(),
            error: error.clone(),
            position: change.position,
        });
        drop(effects);
        if let Some(permit) = protection {
            permit.settling();
            let this = self.clone();
            let context = link.clone();
            let settling = async move {
                this.settle_display(&permit, Some(&context), &change.to, wrote_display)
                    .await;
                let mut i = this.inner.lock().await;
                if this.current(&context) {
                    i.state.switching = false;
                    i.state.switch_progress = None;
                }
                // Admission opens only after the old progress is cleared.
                drop(permit);
                drop(i);
                this.publish_control(&context, error).await;
            };
            if self.switch_protection.waits_for_display() {
                // The observer must continue consuming and reconciling input
                // transitions instead of filling its queue during settling.
                tokio::spawn(settling);
            } else {
                settling.await;
            }
        }
    }
    pub(super) async fn inject_wait(&self, link: &Link, target: &str) -> Result<()> {
        ensure!(self.current(link), "共享会话已结束");
        let mut observed = self.observed.subscribe();
        let host = self.snapshot().await.device_id == target;
        ensure!(self.current(link), "共享会话已结束");
        self.post_input_switch(link, target, host).await?;
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
    async fn post_input_switch(&self, link: &Link, target: &str, host: bool) -> Result<()> {
        self.trace(
            "input.injectRequested",
            json!({"session":link.id,"target":target}),
        )
        .await;
        ensure!(self.current(link), "共享会话已结束");
        let result = self.platform.inject(host).await;
        self.trace(
            "input.injectReturned",
            json!({
                "session":link.id,"target":target,"nativeAccepted":result.is_ok(),
                "engineConfirmed":false,"nativeReply":result.as_ref().ok(),
                "error":result.as_ref().err().map(|e|format!("{e:#}"))
            }),
        )
        .await;
        result.map(|_| ())
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
            s.engine == "ready" && (!s.switching || self.switch_protection.settling()),
            "键鼠尚未就绪或正在切换"
        );
        ensure!(
            target == s.device_id || s.peer.as_ref().is_some_and(|p| p.id == target),
            "无效的切换目标"
        );
        ensure!(
            cross.is_none_or(|p| p.is_finite() && (0.0..=1.0).contains(&p)),
            "跨屏位置无效"
        );
        if s.active_device_id.as_ref() == Some(&target) {
            self.trace(
                "input.transferSkipped",
                json!({"session":link.id,"target":target,"reason":"alreadyActive"}),
            )
            .await;
            return Ok(());
        }
        let manual = s
            .session_layout
            .as_ref()
            .is_some_and(|l| l.video == "manual");
        let retained = if target == s.device_id {
            pointer::selected(&s.cursor_displays, &s.config.retained_display_id).is_some()
        } else {
            s.peer.as_ref().is_some_and(|p| p.retained_display)
        };
        let gated_edge = manual && !retained;
        let native_edge =
            cfg!(target_os = "macos") && gated_edge && cross.is_some() && target != s.device_id;
        if !self.command_pending.load(Ordering::Acquire) && manual {
            ensure!(
                retained
                    || (s.display_status != "uncertain"
                        && s.display_device_id.as_ref() == Some(&target)),
                "目标没有可见屏幕，请使用快捷键或按钮切换画面"
            );
        }
        ensure!(
            self.keyboard_pending
                .compare_exchange(0, link.id, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "正在切换，请稍后"
        );
        let started = std::time::Instant::now();
        let result=async {
            let mut completed=self.completed.subscribe();
            ensure!(self.current(link),"共享会话已结束");
            if native_edge {
                #[cfg(target_os = "macos")]
                self.cross_visible_native_edge(link, &target, cross.unwrap()).await?;
            } else if let Some(position)=cross.filter(|_| !gated_edge && target != s.device_id) {
                let side=s.session_layout.as_ref().context("未设置布局")?.host_side.as_str();
                let p=pointer::exit(&s.cursor_displays,side,position)?;
                self.platform.point(p.x,p.y).await?;
            } else { self.post_input_switch(link,&target,target==s.device_id).await?; }
            drop(effects);
            let wait=async {
                loop {
                    let change=completed.recv().await?;
                    if change.session==link.id && change.to==target {
                        return match change.error { Some(error)=>Err(anyhow::anyhow!(error)),None=>Ok(change.position) };
                    }
                }
            };
            tokio::select! {
                _=link.session.closed.cancelled()=>bail!("共享会话已结束"),
                result=tokio::time::timeout(Duration::from_secs(12),wait)=>result.context("未收到 Deskflow 键鼠转移确认；请停止共享后重新连接，若再次发生请导出诊断日志")?,
            }?;
            if let Some(position) = cross.filter(|_| gated_edge && !native_edge) {
                // This destination has no unconditional native edge. Transfer
                // only after the visibility check above, then restore its entry
                // fraction instead of using Deskflow's previous jump position.
                ensure!(self.current(link) && self.snapshot().await.active_device_id.as_ref() == Some(&target), "键鼠归属已变化，取消鼠标落点");
                if target == s.device_id {
                    let _effects = self.effects.lock().await;
                    self.place_shared_pointer(link, position).await?;
                } else {
                    link.session.request("place-shared-pointer", json!(position), Duration::from_secs(5)).await?;
                }
            }
            Ok(())
        }.await;
        self.keyboard_pending
            .compare_exchange(link.id, 0, Ordering::AcqRel, Ordering::Acquire)
            .ok();
        let diagnostics = if result.is_err() && self.current(link) && platform() == "darwin" {
            match tokio::time::timeout(Duration::from_secs(1), self.platform.input_diagnostics())
                .await
            {
                Ok(Ok(status)) => status,
                Ok(Err(error)) => json!({"error":format!("{error:#}")}),
                Err(_) => json!({"error":"键鼠状态诊断超时"}),
            }
        } else {
            Value::Null
        };
        let after = self.snapshot().await;
        self.trace(
            "input.transferCompleted",
            json!({
                "session":link.id,"target":target,"from":s.active_device_id,
                "active":after.active_device_id,"engine":after.engine,
                "engineConfirmed":result.is_ok(),"elapsedMs":started.elapsed().as_millis(),
                "error":result.as_ref().err().map(|e|format!("{e:#}")),"diagnostics":diagnostics,
            }),
        )
        .await;
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
        if self.inner.lock().await.state.config.display_method == DisplayMethod::Signal {
            self.remind_signal_source(&self.snapshot().await);
            return self.command_output_switch(link, target).await;
        }
        let _command = self
            .commands
            .try_lock()
            .map_err(|_| anyhow::anyhow!("正在切换，请稍后"))?;
        let protection = self.switch_protection.enter(target.as_deref())?;
        ensure!(self.current(link), "共享会话已结束");
        let recovery = self.display_recovery.lock().await.pending();
        if let Some(token) = recovery {
            let _effects = self.effects.lock().await;
            self.prepare_resumed_switch(link, token).await?;
        }
        let mut s = self.snapshot().await;
        if s.display_status == "uncertain" {
            // Recover routing before resolving a toggle, without changing its
            // direction based on keyboard ownership or a pre-failure cache.
            let source = self
                .query_display_source(link)
                .await
                .context("画面来源尚未恢复，请稍后重试或使用显示器实体按键恢复")?;
            let mut i = self.inner.lock().await;
            ensure!(self.current(link), "共享会话已结束");
            i.state.display_source = Some(DisplaySource::Ddc {
                device: source.clone(),
            });
            i.state.display_device_id = Some(source);
            i.state.display_status = "commanded".into();
            s = i.state.clone();
        }
        // Resolve toggles at the provider using the last picture command,
        // except after resume, when a fresh DDC observation is required above.
        let target = match target {
            Some(target) => target,
            None if s.display_device_id.as_ref() == Some(&s.device_id) => {
                s.peer.as_ref().context("对端未连接")?.id.clone()
            }
            None => s.device_id.clone(),
        };
        protection.target(&target);
        self.trace("switch.command", json!({"target":target,"active":s.active_device_id,"display":s.display_device_id,"session":link.id})).await;
        ensure!(
            s.display_status != "uncertain",
            "画面状态不确定，请手动恢复提供端画面后重新开始共享"
        );
        // An inactive input may not appear in the target's display enumeration.
        // Send the chosen target through the current source instead of vetoing
        // it on enumeration; recovery verifies the result with a fresh readback.
        self.command_pending.store(true, Ordering::Release);
        let result = async {
            self.switch_stage(link, SwitchStage::TransferringInput)
                .await?;
            self.keyboard_switch(link, target.clone(), None)
                .await
                .context("转移键鼠失败")?;
            let _effects = self.effects.lock().await;
            ensure!(self.current(link), "共享会话已结束");
            ensure!(
                self.snapshot().await.active_device_id.as_ref() == Some(&target),
                "键鼠已发生新的跨屏，请重试"
            );
            self.inner.lock().await.state.switching = true;
            self.switch_stage(link, SwitchStage::WritingInput).await?;
            let result = match self
                .write_display_with_retry(link, &target)
                .await
                .context("发送输入源指令失败")
            {
                Ok(()) => match recovery {
                    Some(token) => self.confirm_resumed_switch(link, token, &target).await,
                    None => Ok(()),
                },
                Err(error) => Err(error),
            };
            // An accepted DDC write with temporarily unavailable readback is
            // not a failed input transfer. Rolling it back causes the first
            // press to move input twice while the monitor is changing source.
            let rollback = result
                .as_ref()
                .err()
                .is_some_and(|error| !error.is::<super::display_recovery::ReadbackUnavailable>());
            if rollback && self.current(link) {
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
            self.settle_display(&protection, Some(link), &target, result.is_ok())
                .await;
        }
        if self.current(link) {
            {
                let mut i = self.inner.lock().await;
                i.state.switching = false;
                i.state.switch_progress = None;
                drop(protection);
            }
            self.publish_control(link, result.as_ref().err().map(|e| format!("{e:#}")))
                .await;
        } else {
            drop(protection);
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
        if s.config.role != "host" {
            self.remind_signal_source(&s);
        }
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
        if s.config.role != "host" {
            self.remind_signal_source(&s);
        }
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
        let _lifecycle = self
            .start_stop
            .try_lock()
            .map_err(|_| anyhow::anyhow!("正在处理其他操作，请稍候再切换"))?;
        let s = self.snapshot().await;
        let epoch = self.epoch.load(Ordering::Acquire);
        ensure!(!s.running, "请先停止共享");
        s.config.ready()?;
        ensure!(
            s.config.display_method == DisplayMethod::Ddc,
            "自动选源模式请通过切换按钮测试，不发送 DDC 指令"
        );
        ensure!(
            Some(value) == s.config.local_input || Some(value) == s.config.peer_input,
            "只能测试已保存输入源"
        );
        let label = if Some(value) == s.config.local_input {
            "local"
        } else {
            "peer"
        };
        let protection = self.switch_protection.enter(Some(label))?;
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
            i.state.display_source = None;
            target
        };
        self.publish();
        let result = self.platform.input(&s.config.display_id, value).await;
        ensure!(
            self.epoch.load(Ordering::Acquire) == epoch,
            "切换已取消，正在恢复输出"
        );
        if result.is_ok() && s.config.mode == OperationMode::DisplayOnly {
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
        } else if let Some(target) = target.filter(|_| result.is_ok()) {
            let mut i = self.inner.lock().await;
            i.state.display_device_id = Some(target.clone());
            i.state.last_display_device_id = Some(target);
            i.state.display_status = "commanded".into();
        }
        self.settle_display(&protection, None, label, result.is_ok())
            .await;
        {
            let mut i = self.inner.lock().await;
            i.state.switching = false;
            i.state.switch_progress = None;
            drop(protection);
        }
        self.publish();
        result?;
        self.log("info", "输入切换指令已发送，请以实际画面为准")
            .await;
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn automatic_edges_preserve_ddc_policy_and_never_use_the_manual_gate() {
        for mode in ["keep", "follow"] {
            for target_local in [false, true] {
                for retained in [false, true] {
                    for side in ["left", "right", "top", "bottom"] {
                        let directory = tempfile::tempdir().unwrap();
                        let runtime =
                            super::super::pointer_bridge::tests::fixture(directory.path(), "host");
                        let local = runtime.snapshot().await.device_id;
                        let peer = uuid::Uuid::new_v4().to_string();
                        let (source, target) = if target_local {
                            (peer.clone(), local.clone())
                        } else {
                            (local.clone(), peer.clone())
                        };
                        let writes = mode == "follow" || !retained;
                        {
                            let mut i = runtime.inner.lock().await;
                            i.state.engine = "ready".into();
                            i.state.session_layout = Some(Layout {
                                host_side: side.into(),
                                video: mode.into(),
                            });
                            i.state.config.layout_side = side.into();
                            i.state.config.cross_screen_video = mode.into();
                            i.state.config.retained_display_id = if !target_local || retained {
                                "internal".into()
                            } else {
                                String::new()
                            };
                            i.state.config.display_id = "external".into();
                            i.state.config.local_input = Some(3);
                            i.state.config.peer_input = Some(1);
                            i.state.active_device_id = Some(source.clone());
                            i.state.display_device_id = Some(source.clone());
                            i.state.last_display_device_id = Some(source.clone());
                            i.state.peer = Some(Peer {
                                id: peer,
                                name: "Receiver".into(),
                                platform: "win32".into(),
                                fingerprint: "a".repeat(64),
                                file_transfer: false,
                                lock_control: false,
                                input_configured: true,
                                retained_display: !target_local && retained,
                                modifier_mapping: ModifierMapping::Original,
                            });
                        }
                        // No engine/reloader is installed. Automatic modes must
                        // use their ordinary native links, never the manual gate.
                        let config = directory.path().join("deskflow/server.conf");
                        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
                        std::fs::write(&config, "unchanged automatic layout").unwrap();
                        let native = runtime.platform.resources.join("native/bin");
                        std::fs::create_dir_all(&native).unwrap();
                        let input = native.join("mac-hotkey");
                        let ddc = native.join("m1ddc");
                        for (path, script) in [
                            (&input, "#!/bin/sh\ncase \"$1\" in\nhost|pointer) echo \"$*\" >> \"$0.calls\"; echo sent ;;\n*) exit 9 ;;\nesac\n"),
                            (&ddc, "#!/bin/sh\ntest \"$1\" = display && test \"$2\" = external && test \"$3\" = set && test \"$4\" = input || exit 9\ncase \"$5\" in 1|3) ;; *) exit 9 ;; esac\necho \"$*\" >> \"$0.calls\"\n"),
                        ] {
                            std::fs::write(path, script).unwrap();
                            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
                        }
                        let (remote, mut requests) =
                            super::super::pointer_bridge::tests::attach_with_requests(&runtime)
                                .await;
                        let link = runtime.link.lock().await.clone().unwrap();
                        let replying = remote.clone();
                        let received = Arc::new(AtomicU64::new(0));
                        let count = received.clone();
                        let destination = target.clone();
                        let receiver = tokio::spawn(async move {
                            while let Some(next) = requests.recv().await {
                                if let Incoming::Request {
                                    id,
                                    method,
                                    payload,
                                } = next
                                {
                                    if writes {
                                        // Both endpoints receive one fixed
                                        // target, whichever endpoint is first.
                                        assert_eq!(method, "set-input");
                                        assert_eq!(payload, json!(destination));
                                    } else {
                                        assert!(!target_local, "Local independent-screen landing uses the local helper");
                                        assert_eq!(method, "land-pointer");
                                        assert!(payload.is_null());
                                    }
                                    count.fetch_add(1, Ordering::AcqRel);
                                    replying.reply(id, Ok(json!(true))).await.unwrap();
                                }
                            }
                        });
                        let observing = runtime.clone();
                        let context = link.clone();
                        let marker = input.with_extension("calls");
                        let from = source.clone();
                        let to = target.clone();
                        let observer = tokio::spawn(async move {
                            tokio::time::timeout(Duration::from_secs(3), async {
                                while !marker.exists() {
                                    tokio::time::sleep(Duration::from_millis(5)).await;
                                }
                            })
                            .await
                            .unwrap();
                            observing
                                .observe_transition(
                                    &context,
                                    Transition {
                                        from,
                                        to,
                                        position: Some(Point { x: 1.0, y: 300.0 }),
                                    },
                                )
                                .await;
                        });
                        runtime
                            .keyboard_switch(&link, target.clone(), Some(0.4))
                            .await
                            .unwrap();
                        observer.await.unwrap();
                        let state = runtime.snapshot().await;
                        assert_eq!(
                            state.active_device_id,
                            Some(target.clone()),
                            "{mode}/{side}/{target_local}/{retained}"
                        );
                        assert_eq!(
                            state.display_device_id,
                            Some(if writes { target } else { source })
                        );
                        assert!(!state.switching);
                        assert_eq!(state.display_status, "commanded");
                        assert_eq!(ddc.with_extension("calls").exists(), writes);
                        if writes {
                            assert_eq!(
                                std::fs::read_to_string(ddc.with_extension("calls"))
                                    .unwrap()
                                    .trim(),
                                format!(
                                    "display external set input {}",
                                    if target_local { 3 } else { 1 }
                                )
                            );
                        }
                        assert_eq!(
                            received.load(Ordering::Acquire),
                            u64::from(writes || !target_local)
                        );
                        let calls = std::fs::read_to_string(input.with_extension("calls")).unwrap();
                        let calls: Vec<_> = calls.lines().collect();
                        assert_eq!(calls.len(), if !writes && target_local { 2 } else { 1 });
                        assert_eq!(
                            calls[0].split_whitespace().next().unwrap(),
                            if target_local { "host" } else { "pointer" }
                        );
                        assert_eq!(
                            std::fs::read_to_string(config).unwrap(),
                            "unchanged automatic layout"
                        );
                        assert!(runtime.engine.lock().await.is_none());
                        assert!(!runtime.command_pending.load(Ordering::Acquire));
                        assert_eq!(runtime.keyboard_pending.load(Ordering::Acquire), 0);
                        remote.close();
                        receiver.abort();
                        let _ = receiver.await;
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn native_acceptance_needs_engine_confirmation_before_ddc() {
        for confirm in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "host");
            let local = runtime.snapshot().await.device_id;
            {
                let mut i = runtime.inner.lock().await;
                i.state.engine = "ready".into();
                i.state.active_device_id = Some(local.clone());
                i.state.display_device_id = Some(local.clone());
                i.state.last_display_device_id = Some(local.clone());
                i.state.config.display_id = "external".into();
                i.state.config.local_input = Some(3);
                i.state.config.peer_input = Some(1);
                i.state.peer = Some(Peer {
                    id: "receiver".into(),
                    name: "Receiver".into(),
                    platform: "win32".into(),
                    fingerprint: "a".repeat(64),
                    file_transfer: false,
                    lock_control: false,
                    input_configured: true,
                    retained_display: false,
                    modifier_mapping: ModifierMapping::Original,
                });
            }
            let native = runtime.platform.resources.join("native/bin");
            std::fs::create_dir_all(&native).unwrap();
            let hotkey = native.join("mac-hotkey");
            let ddc = native.join("m1ddc");
            for (path, script) in [
                (&hotkey, "#!/bin/sh\ncase \"$1\" in\ndiagnose) echo '{\"accessibility\":true,\"postEventAccess\":true,\"secureEventInput\":false}' ;;\npeer) echo sent >> \"$0.calls\"; echo sent ;;\n*) exit 9 ;;\nesac\n"),
                (&ddc, "#!/bin/sh\necho ddc >> \"$0.calls\"\n"),
            ] {
                std::fs::write(path, script).unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let (remote, mut requests) =
                super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
            let link = runtime.link.lock().await.clone().unwrap();
            let remote_count = Arc::new(AtomicU64::new(0));
            let count = remote_count.clone();
            let replying = remote.clone();
            let receiver = tokio::spawn(async move {
                while let Some(next) = requests.recv().await {
                    if let Incoming::Request {
                        id,
                        method,
                        payload,
                    } = next
                    {
                        assert_eq!(
                            method, "set-input",
                            "DDC must never run signal output actions"
                        );
                        assert_eq!(payload, json!("receiver"));
                        count.fetch_add(1, Ordering::AcqRel);
                        replying.reply(id, Ok(json!(true))).await.unwrap();
                    }
                }
            });
            let observer = if confirm {
                let rt = runtime.clone();
                let context = link.clone();
                let marker = hotkey.with_extension("calls");
                let ddc_marker = ddc.with_extension("calls");
                Some(tokio::spawn(async move {
                    tokio::time::timeout(Duration::from_secs(3), async {
                        while !marker.exists() {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    })
                    .await
                    .unwrap();
                    assert!(
                        !ddc_marker.exists(),
                        "Native acceptance alone cannot start DDC"
                    );
                    rt.observe_transition(
                        &context,
                        Transition {
                            from: local,
                            to: "receiver".into(),
                            position: None,
                        },
                    )
                    .await;
                }))
            } else {
                None
            };
            let result = runtime.command_switch(&link, "receiver".into()).await;
            assert_eq!(result.is_ok(), confirm);
            if let Some(observer) = observer {
                observer.await.unwrap();
            }
            assert_eq!(ddc.with_extension("calls").exists(), confirm);
            assert_eq!(remote_count.load(Ordering::Acquire), u64::from(confirm));
            assert!(!runtime.command_pending.load(Ordering::Acquire));
            assert_eq!(runtime.keyboard_pending.load(Ordering::Acquire), 0);
            let state = runtime.snapshot().await;
            let expected = if confirm {
                "receiver"
            } else {
                &state.device_id
            };
            assert_eq!(state.active_device_id.as_deref(), Some(expected));
            assert_eq!(state.display_device_id.as_deref(), Some(expected));
            let records: Vec<Value> =
                std::fs::read_to_string(directory.path().join("logs/deskflow-ddc.jsonl"))
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect();
            let accepted = records
                .iter()
                .find(|r| r["event"] == "input.injectReturned")
                .unwrap();
            assert_eq!(accepted["details"]["nativeAccepted"], true);
            assert_eq!(accepted["details"]["engineConfirmed"], false);
            let completed = records
                .iter()
                .find(|r| r["event"] == "input.transferCompleted")
                .unwrap();
            assert_eq!(completed["details"]["engineConfirmed"], confirm);
            if !confirm {
                assert_eq!(completed["details"]["diagnostics"]["postEventAccess"], true);
                let message = format!("{:#}", result.unwrap_err());
                assert!(message.contains("停止共享后重新连接"));
                assert!(!message.contains("辅助功能权限"));
            }
            remote.close();
            receiver.await.unwrap();
        }
    }
}
