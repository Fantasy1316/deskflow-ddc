use super::*;

/// Separate a DDC observation/user confirmation from a sent video command.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DisplaySource {
    Checking,
    Unknown { reason: String },
    Ddc { device: String },
    Manual { device: String },
}

impl DisplaySource {
    fn device(&self) -> Option<&str> {
        match self {
            Self::Ddc { device } | Self::Manual { device } => Some(device),
            _ => None,
        }
    }
}

impl Snapshot {
    pub(crate) fn needs_signal_source_confirmation(&self) -> bool {
        self.config.mode == OperationMode::Sharing
            && self.config.display_method == DisplayMethod::Signal
            && self.running
            && self.connected
            && self.peer.is_some()
            && self.display_source != Some(DisplaySource::Checking)
            && (matches!(self.display_source, Some(DisplaySource::Unknown { .. }))
                || self.display_device_id.is_none()
                || self.display_status != "commanded")
    }

    pub(crate) fn can_confirm_signal_source(&self) -> bool {
        self.needs_signal_source_confirmation()
            && self.engine == "ready"
            && !self.switching
            && !self.emergency_stopping
    }
}

/// An unanswered native dialog must never apply its choice to a replacement
/// connection, changed configuration, or a source already resolved elsewhere.
pub(crate) struct SourcePrompt {
    link: Arc<Link>,
    config: Config,
    device: String,
    peer: String,
    source: Option<DisplaySource>,
    created: std::time::Instant,
    pub local_name: String,
    pub peer_name: String,
}

impl SourcePrompt {
    pub fn key(&self) -> (u64, u64) {
        (self.link.epoch, self.link.id)
    }

    pub(crate) fn same_context(&self, other: &Self) -> bool {
        self.key() == other.key()
            && self.config == other.config
            && self.device == other.device
            && self.peer == other.peer
            && self.source == other.source
    }

    fn matches(&self, state: &Snapshot) -> bool {
        state.can_confirm_signal_source()
            && state.config == self.config
            && state.device_id == self.device
            && state.peer.as_ref().is_some_and(|peer| peer.id == self.peer)
            && state.display_source == self.source
    }
}

impl Runtime {
    /// A user switch attempt is also a new reminder opportunity. Keep this
    /// independent of log text, so repeated identical failures still notify.
    pub(super) fn remind_signal_source(&self, state: &Snapshot) {
        if state.needs_signal_source_confirmation() {
            self.source_reminders.send_modify(|request| *request += 1);
        }
    }

    pub(crate) async fn signal_source_prompt(&self) -> Option<SourcePrompt> {
        let link = self.link.lock().await.clone()?;
        let state = self.snapshot().await;
        if !self.current(&link) || !state.needs_signal_source_confirmation() {
            return None;
        }
        let peer = state.peer.as_ref()?;
        Some(SourcePrompt {
            link,
            config: state.config.clone(),
            device: state.device_id.clone(),
            peer: peer.id.clone(),
            source: state.display_source.clone(),
            created: std::time::Instant::now(),
            local_name: state.config.device_name.clone(),
            peer_name: peer.name.clone(),
        })
    }

    pub(crate) async fn confirm_signal_source(
        &self,
        prompt: &SourcePrompt,
        local: bool,
    ) -> Result<bool> {
        let state = self.snapshot().await;
        if !self.current(&prompt.link)
            || !prompt.matches(&state)
            || self.switch_protection.blocks(prompt.created)
        {
            return Ok(false);
        }
        let target = if local { &prompt.device } else { &prompt.peer };
        if state.config.role == "host" {
            self.confirm_display_source_checked(&prompt.link, target.clone(), Some(prompt))
                .await
        } else {
            // Use the captured authenticated session, never look up a new peer
            // after the user has spent time answering the dialog.
            prompt
                .link
                .session
                .request(
                    "confirm-display-source",
                    json!(target),
                    Duration::from_secs(10),
                )
                .await?;
            Ok(true)
        }
    }

    /// Only the provider coordinates reads and publishes their result. This
    /// runs after reciprocal identification, before starting the input engine.
    pub(super) async fn initialize_display_source(&self, link: &Arc<Link>) -> Result<()> {
        self.detect_display_source_on(link).await
    }

    async fn detect_display_source_on(&self, link: &Arc<Link>) -> Result<()> {
        let received = std::time::Instant::now();
        let _command = self
            .commands
            .try_lock()
            .map_err(|_| anyhow::anyhow!("正在切换，请稍后再检测输入源"))?;
        self.ensure_switch_available_at(received)?;
        let _effects = self.effects.lock().await;
        // An automatic edge may have acquired protection while we awaited its
        // native effects. Do not reset that transaction's source during settle.
        self.ensure_switch_available_at(received)?;
        let s = self.snapshot().await;
        ensure!(
            self.current(link) && s.connected && s.config.role == "host",
            "设备尚未连接"
        );
        if s.config.display_method == DisplayMethod::Signal {
            let mut i = self.inner.lock().await;
            i.state.display_device_id = None;
            i.state.display_status = "uninitialized".into();
            i.state.display_source = Some(DisplaySource::Unknown {
                reason: "软断开模式无法读取显示器当前输入，请选择共用显示器现在显示的电脑".into(),
            });
            drop(i);
            self.publish_control(link, None).await;
            return Ok(());
        }
        {
            let mut i = self.inner.lock().await;
            i.state.display_device_id = None;
            i.state.display_status = "uncertain".into();
            i.state.display_source = Some(DisplaySource::Checking);
        }
        self.publish_control(link, None).await;
        let result = tokio::select! {
            biased;
            _ = link.session.closed.cancelled() => bail!("共享会话已结束"),
            result = tokio::time::timeout(Duration::from_secs(5), self.query_display_source(link)) =>
                result.unwrap_or_else(|_| Err(anyhow::anyhow!("读取当前输入源超时"))),
        };
        {
            let mut i = self.inner.lock().await;
            ensure!(self.current(link), "共享会话已结束");
            match &result {
                Ok(device) => {
                    i.state.display_device_id = Some(device.clone());
                    // Retain the existing status on the wire for older peers.
                    i.state.display_status = "commanded".into();
                    i.state.display_source = Some(DisplaySource::Ddc {
                        device: device.clone(),
                    });
                }
                Err(error) => {
                    i.state.display_source = Some(DisplaySource::Unknown {
                        reason: format!("无法确定当前输入源，请核对两端输入源配置，或按实际画面选择电脑。{error}"),
                    });
                }
            }
        }
        self.trace("display.initialSource", json!({"source":result.as_ref().ok(),"error":result.as_ref().err().map(|e|e.to_string()),"pictureVerified":false})).await;
        self.publish_control(link, None).await;
        // Failure to read a monitor must not fail an authenticated connection.
        Ok(())
    }

    pub async fn detect_display_source(&self) -> Result<()> {
        let s = self.snapshot().await;
        ensure!(
            s.config.display_method == DisplayMethod::Ddc,
            "软断开模式请按实际画面确认来源"
        );
        let link = self
            .link
            .lock()
            .await
            .clone()
            .context("另一台电脑尚未连接")?;
        if s.config.role == "host" {
            self.detect_display_source_on(&link).await
        } else {
            link.session
                .request(
                    "detect-display-source",
                    Value::Null,
                    Duration::from_secs(10),
                )
                .await
                .map(|_| ())
        }
    }

    pub async fn confirm_display_source(&self, local: bool) -> Result<()> {
        let s = self.snapshot().await;
        let target = if local {
            s.device_id
        } else {
            s.peer.context("另一台电脑尚未连接")?.id
        };
        let link = self
            .link
            .lock()
            .await
            .clone()
            .context("另一台电脑尚未连接")?;
        if s.config.role == "host" {
            self.confirm_display_source_on(&link, target).await
        } else {
            link.session
                .request(
                    "confirm-display-source",
                    json!(target),
                    Duration::from_secs(10),
                )
                .await
                .map(|_| ())
        }
    }

    pub(super) async fn confirm_display_source_on(
        &self,
        link: &Arc<Link>,
        target: String,
    ) -> Result<()> {
        self.confirm_display_source_checked(link, target, None)
            .await
            .map(|_| ())
    }

    async fn confirm_display_source_checked(
        &self,
        link: &Arc<Link>,
        target: String,
        prompt: Option<&SourcePrompt>,
    ) -> Result<bool> {
        let received = std::time::Instant::now();
        let _command = self
            .commands
            .try_lock()
            .map_err(|_| anyhow::anyhow!("正在切换或检测输入源，请稍后再确认"))?;
        if prompt.is_some_and(|prompt| {
            !self.current(link) || self.switch_protection.blocks(prompt.created)
        }) {
            return Ok(false);
        }
        self.ensure_switch_available_at(received)?;
        let _effects = self.effects.lock().await;
        if prompt.is_some_and(|prompt| {
            !self.current(link) || self.switch_protection.blocks(prompt.created)
        }) {
            return Ok(false);
        }
        self.ensure_switch_available_at(received)?;
        {
            let mut i = self.inner.lock().await;
            let s = &mut i.state;
            if let Some(prompt) = prompt {
                if !self.current(link)
                    || !prompt.matches(s)
                    || self.switch_protection.blocks(prompt.created)
                {
                    return Ok(false);
                }
            }
            ensure!(
                self.current(link)
                    && s.connected
                    && s.config.role == "host"
                    && !s.emergency_stopping,
                "当前无法确认画面来源"
            );
            ensure!(
                target == s.device_id || s.peer.as_ref().is_some_and(|p| p.id == target),
                "无效的画面来源设备"
            );
            s.display_device_id = Some(target.clone());
            s.display_status = "commanded".into();
            s.display_source = Some(DisplaySource::Manual {
                device: target.clone(),
            });
        }
        // A current user observation can resolve a pending wake read failure.
        let mut recovery = self.display_recovery.lock().await;
        if let Some(token) = recovery.pending() {
            recovery.complete(token);
        }
        drop(recovery);
        self.publish_control(link, None).await;
        self.trace(
            "display.sourceConfirmed",
            json!({"source":target,"displayCommandSent":false,"inputTransferred":false}),
        )
        .await;
        Ok(true)
    }

    pub(super) async fn receive_display_source(&self, link: &Link, payload: Value) -> Result<()> {
        let source: DisplaySource = serde_json::from_value(payload)?;
        let mut i = self.inner.lock().await;
        ensure!(
            self.current(link) && i.state.config.role == "client",
            "只有接收端接收画面来源状态"
        );
        if let Some(device) = source.device() {
            ensure!(
                device == i.state.device_id
                    || i.state.peer.as_ref().is_some_and(|p| p.id == device),
                "无效的画面来源设备"
            );
            // A stale observation cannot replace a newer switch target.
            if i.state.display_device_id.as_deref() != Some(device)
                || i.state.display_status != "commanded"
            {
                return Ok(());
            }
        } else if i.state.display_device_id.is_some() && i.state.display_status == "commanded" {
            return Ok(());
        }
        i.state.display_source = Some(source);
        drop(i);
        self.publish();
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex as SyncMutex;

    struct Fixture {
        _directory: tempfile::TempDir,
        runtime: Arc<Runtime>,
        remote: Session,
        service: tokio::task::JoinHandle<()>,
        events: Arc<SyncMutex<Vec<(String, Value)>>>,
        peer: String,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.remote.close();
            self.service.abort();
        }
    }
    async fn setup(local_input: Option<u16>, remote_input: Option<bool>) -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "host");
        let peer = uuid::Uuid::new_v4().to_string();
        let local = runtime.snapshot().await.device_id;
        {
            let mut i = runtime.inner.lock().await;
            i.state.reset_display();
            i.state.control_connected(local.clone());
            i.state.config.display_id = "external".into();
            i.state.config.local_input = Some(3);
            i.state.config.peer_input = Some(1);
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
        let helper = runtime.platform.resources.join("native/bin/m1ddc");
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(
            &helper,
            r#"#!/bin/sh
base="$(dirname "$0")"
if [ "$3" != get ]; then echo unexpected-write > "$base/violation"; exit 9; fi
echo read >> "$base/reads"
test -f "$base/value" || exit 1
cat "$base/value"
"#,
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        if let Some(value) = local_input {
            std::fs::write(helper.with_file_name("value"), value.to_string()).unwrap();
        }
        let (remote, mut incoming) =
            super::super::pointer_bridge::tests::attach_with_requests(&runtime).await;
        let link = runtime.link.lock().await.clone().unwrap();
        let mut identity = super::super::tests::peer();
        identity.id = uuid::Uuid::parse_str(&peer).unwrap();
        link.peer.send_replace(Some(identity));
        let events = Arc::new(SyncMutex::new(vec![]));
        let recorded = events.clone();
        let replying = remote.clone();
        let peer_id = peer.clone();
        let service = tokio::spawn(async move {
            while let Some(message) = incoming.recv().await {
                match message {
                    Incoming::Event { name, payload } => {
                        recorded.lock().unwrap().push((name, payload))
                    }
                    Incoming::Request { id, method, .. } => {
                        assert_eq!(
                            method, "get-input-source",
                            "Detection/confirmation must not switch output or input"
                        );
                        let source = remote_input.map(|is_local| {
                            if is_local {
                                local.clone()
                            } else {
                                peer_id.clone()
                            }
                        });
                        if replying.reply(id, Ok(json!(source))).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Fixture {
            _directory: directory,
            runtime,
            remote,
            service,
            events,
            peer,
        }
    }

    #[tokio::test]
    async fn connection_reads_map_either_endpoint_and_keep_input_and_command_history_separate() {
        for (local_input, remote_input, expected_local) in [
            (Some(3), Some(true), true),
            (Some(1), Some(false), false),
            (None, Some(false), false),
            (Some(3), None, true),
        ] {
            let f = setup(local_input, remote_input).await;
            let link = f.runtime.link.lock().await.clone().unwrap();
            f.runtime.initialize_display_source(&link).await.unwrap();
            let s = f.runtime.snapshot().await;
            let expected = if expected_local {
                &s.device_id
            } else {
                &f.peer
            };
            assert_eq!(s.display_device_id.as_ref(), Some(expected));
            assert_eq!(
                s.display_source,
                Some(DisplaySource::Ddc {
                    device: expected.clone()
                })
            );
            assert_eq!(s.active_device_id.as_ref(), Some(&s.device_id));
            assert!(
                s.last_display_device_id.is_none(),
                "A read must not invent a sent command"
            );
            assert_eq!(
                s.engine, "stopped",
                "Reading does not require input-engine startup"
            );
            assert!(!s.switching && s.connected);
            assert!(!f
                .runtime
                .platform
                .resources
                .join("native/bin/violation")
                .exists());
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if f.events
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|(n, p)| n == "display-source" && p["kind"] == "ddc")
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            for (name, payload) in f.events.lock().unwrap().iter() {
                if name == "display-state" {
                    assert!(
                        payload.get("displaySource").is_none(),
                        "Legacy strict frames remain unchanged"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn conflicting_unreadable_and_unconfigured_values_request_confirmation_without_disconnect(
    ) {
        for (local, remote) in [(Some(3), Some(false)), (None, None), (Some(17), None)] {
            let f = setup(local, remote).await;
            let link = f.runtime.link.lock().await.clone().unwrap();
            {
                let mut i = f.runtime.inner.lock().await;
                // A reconnect must not retain stale routing after a failed read.
                i.state.display_device_id = Some(i.state.device_id.clone());
                i.state.display_status = "commanded".into();
            }
            f.runtime.initialize_display_source(&link).await.unwrap();
            let s = f.runtime.snapshot().await;
            assert!(s.connected && s.display_device_id.is_none());
            assert_eq!(s.display_status, "uncertain");
            assert!(matches!(
                s.display_source,
                Some(DisplaySource::Unknown { .. })
            ));
            assert!(!link.session.closed.is_cancelled());
            f.runtime.confirm_display_source(false).await.unwrap();
            let confirmed = f.runtime.snapshot().await;
            assert_eq!(confirmed.display_device_id.as_ref(), Some(&f.peer));
            assert_eq!(confirmed.active_device_id, s.active_device_id);
        }
    }

    #[tokio::test]
    async fn signal_selection_never_queries_ddc_or_moves_input_and_respects_switch_protection() {
        let f = setup(None, None).await;
        f.runtime.inner.lock().await.state.config.display_method = DisplayMethod::Signal;
        let link = f.runtime.link.lock().await.clone().unwrap();
        f.runtime.initialize_display_source(&link).await.unwrap();
        let before = f.runtime.snapshot().await;
        assert!(matches!(
            before.display_source,
            Some(DisplaySource::Unknown { .. })
        ));
        let permit = f.runtime.switch_protection.enter(Some(&f.peer)).unwrap();
        assert!(f.runtime.confirm_display_source(false).await.is_err());
        drop(permit);
        let command = f.runtime.commands.lock().await;
        assert!(f.runtime.confirm_display_source(false).await.is_err());
        drop(command);
        f.runtime.display_recovery.lock().await.mark();
        for local in [false, true] {
            f.runtime.confirm_display_source(local).await.unwrap();
            let s = f.runtime.snapshot().await;
            let target = if local { &s.device_id } else { &f.peer };
            assert_eq!(
                s.display_source,
                Some(DisplaySource::Manual {
                    device: target.clone()
                })
            );
            assert_eq!(s.display_device_id.as_ref(), Some(target));
            assert_eq!(s.active_device_id, before.active_device_id);
            assert!(s.last_display_device_id.is_none() && s.output_switch.is_none());
        }
        assert!(f.runtime.display_recovery.lock().await.pending().is_none());
        assert!(!f
            .runtime
            .platform
            .resources
            .join("native/bin/reads")
            .exists());
        assert!(!f
            .runtime
            .platform
            .resources
            .join("native/bin/violation")
            .exists());
        assert!(f.runtime.detect_display_source().await.is_err());
    }

    #[tokio::test]
    async fn observations_are_synced_to_receivers_without_overwriting_newer_targets() {
        let f = setup(Some(3), Some(true)).await;
        let link = f.runtime.link.lock().await.clone().unwrap();
        f.runtime.inner.lock().await.state.config.role = "client".into();
        let local = f.runtime.snapshot().await.device_id;
        f.runtime.event(&link, "display-state", json!({"active":local,"display":f.peer,"lastDisplay":null,"displayStatus":"commanded","busy":false,"error":null})).await.unwrap();
        f.runtime
            .event(
                &link,
                "display-source",
                json!({"kind":"manual","device":f.peer}),
            )
            .await
            .unwrap();
        assert_eq!(
            f.runtime.snapshot().await.display_source,
            Some(DisplaySource::Manual {
                device: f.peer.clone()
            })
        );
        f.runtime
            .event(
                &link,
                "display-source",
                json!({"kind":"ddc","device":local}),
            )
            .await
            .unwrap();
        assert_eq!(
            f.runtime.snapshot().await.display_device_id.as_ref(),
            Some(&f.peer)
        );
        assert!(f
            .runtime
            .event(
                &link,
                "display-source",
                json!({"kind":"manual","device":"unrelated"})
            )
            .await
            .is_err());
        f.runtime.event(&link, "display-state", json!({"active":local,"display":local,"lastDisplay":local,"displayStatus":"commanded","busy":false,"error":null})).await.unwrap();
        assert!(
            f.runtime.snapshot().await.display_source.is_none(),
            "A new command clears the previous observation label"
        );
    }

    #[tokio::test]
    async fn receiver_confirmation_maps_local_and_peer_to_authenticated_device_ids() {
        let f = setup(None, None).await;
        f.runtime.inner.lock().await.state.config.role = "client".into();
        let local = f.runtime.snapshot().await.device_id;
        let (remote, mut incoming) =
            super::super::pointer_bridge::tests::attach_with_requests(&f.runtime).await;
        let replying = remote.clone();
        let local_id = local.clone();
        let peer_id = f.peer.clone();
        let server = tokio::spawn(async move {
            for expected in [local_id, peer_id] {
                let Incoming::Request {
                    id,
                    method,
                    payload,
                } = incoming.recv().await.unwrap()
                else {
                    panic!("Expected confirmation RPC");
                };
                assert_eq!(method, "confirm-display-source");
                assert_eq!(payload, json!(expected));
                replying.reply(id, Ok(json!(true))).await.unwrap();
            }
        });
        let before = f.runtime.snapshot().await;
        f.runtime.confirm_display_source(true).await.unwrap();
        f.runtime.confirm_display_source(false).await.unwrap();
        server.await.unwrap();
        let after = f.runtime.snapshot().await;
        assert_eq!(after.active_device_id, before.active_device_id);
        assert_eq!(
            after.display_device_id, before.display_device_id,
            "Only the provider's acknowledged state updates receiver routing"
        );
        remote.close();
    }

    #[tokio::test]
    async fn detection_is_serialized_and_disconnect_cancels_without_committing_an_old_read() {
        let f = setup(Some(3), None).await;
        // Pause the native test script before returning its observation.
        let helper = f.runtime.platform.resources.join("native/bin/m1ddc");
        std::fs::write(&helper, "#!/bin/sh\nsleep 1\necho 3\n").unwrap();
        let link = f.runtime.link.lock().await.clone().unwrap();
        let runtime = f.runtime.clone();
        let context = link.clone();
        let reading =
            tokio::spawn(async move { runtime.initialize_display_source(&context).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while f.runtime.snapshot().await.display_source != Some(DisplaySource::Checking) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(f.runtime.confirm_display_source(false).await.is_err());
        assert!(f.runtime.switch_to(false).await.is_err());
        tokio::time::timeout(Duration::from_secs(1), f.runtime.stop())
            .await
            .unwrap()
            .unwrap();
        assert!(reading.await.unwrap().is_err());
        let s = f.runtime.snapshot().await;
        assert!(!s.connected && !s.running);
        assert!(s.display_device_id.is_none());
        assert!(!matches!(
            s.display_source,
            Some(DisplaySource::Checking | DisplaySource::Ddc { .. })
        ));
    }

    #[tokio::test]
    async fn read_or_confirmation_waiting_behind_an_edge_cannot_overwrite_its_source() {
        for confirm in [false, true] {
            let f = setup(Some(3), Some(true)).await;
            let link = f.runtime.link.lock().await.clone().unwrap();
            let effects = f.runtime.effects.lock().await;
            let runtime = f.runtime.clone();
            let operation = tokio::spawn(async move {
                if confirm {
                    runtime.confirm_display_source(false).await
                } else {
                    runtime.initialize_display_source(&link).await
                }
            });
            tokio::time::timeout(Duration::from_secs(1), async {
                while f.runtime.commands.try_lock().is_ok() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let permit = f.runtime.switch_protection.enter(Some(&f.peer)).unwrap();
            {
                let mut i = f.runtime.inner.lock().await;
                i.state.display_device_id = Some(f.peer.clone());
                i.state.display_status = "commanded".into();
            }
            // Simulate native effects releasing after the edge transaction has
            // already ended. The old request must still be discarded.
            drop(permit);
            drop(effects);
            assert!(operation.await.unwrap().is_err());
            let s = f.runtime.snapshot().await;
            assert_eq!(s.display_device_id.as_ref(), Some(&f.peer));
            assert!(s.display_source.is_none());
            assert!(!f
                .runtime
                .platform
                .resources
                .join("native/bin/reads")
                .exists());
        }
    }

    async fn pending_signal_fixture() -> Fixture {
        let f = setup(None, None).await;
        {
            let mut i = f.runtime.inner.lock().await;
            i.state.config.display_method = DisplayMethod::Signal;
            i.state.engine = "ready".into();
        }
        let link = f.runtime.link.lock().await.clone().unwrap();
        f.runtime.initialize_display_source(&link).await.unwrap();
        f
    }

    #[tokio::test]
    async fn tray_reminders_are_limited_to_unresolved_signal_sharing() {
        let f = pending_signal_fixture().await;
        let mut s = f.runtime.snapshot().await;
        assert!(s.can_confirm_signal_source());
        for engine in ["stopped", "starting", "error"] {
            s.engine = engine.into();
            assert!(s.needs_signal_source_confirmation());
            assert!(!s.can_confirm_signal_source());
        }
        s.engine = "ready".into();
        s.switching = true;
        assert!(s.needs_signal_source_confirmation());
        assert!(!s.can_confirm_signal_source());
        s.switching = false;
        s.emergency_stopping = true;
        assert!(!s.can_confirm_signal_source());
        s.emergency_stopping = false;
        s.config.display_method = DisplayMethod::Ddc;
        assert!(!s.needs_signal_source_confirmation());
        s.config.display_method = DisplayMethod::Signal;
        s.config.mode = OperationMode::DisplayOnly;
        assert!(!s.needs_signal_source_confirmation());
        s.config.mode = OperationMode::Sharing;
        s.display_source = Some(DisplaySource::Checking);
        assert!(!s.needs_signal_source_confirmation());
        s.display_source = None;
        assert!(s.needs_signal_source_confirmation());
        s.connected = false;
        assert!(!s.needs_signal_source_confirmation());
        s.connected = true;
        s.running = false;
        assert!(!s.needs_signal_source_confirmation());
        s.running = true;
        s.peer = None;
        assert!(!s.needs_signal_source_confirmation());
    }

    #[tokio::test]
    async fn every_unresolved_switch_attempt_notifies_but_ddc_and_resolved_sources_do_not() {
        let f = pending_signal_fixture().await;
        let state = f.runtime.snapshot().await;
        let mut requests = f.runtime.source_reminders.subscribe();
        for expected in 1..=3 {
            f.runtime.remind_signal_source(&state);
            assert!(requests.has_changed().unwrap());
            assert_eq!(*requests.borrow_and_update(), expected);
        }
        let mut normal_ddc = state.clone();
        normal_ddc.config.display_method = DisplayMethod::Ddc;
        f.runtime.remind_signal_source(&normal_ddc);
        assert!(!requests.has_changed().unwrap());
        let mut confirmed = state.clone();
        confirmed.display_device_id = Some(state.device_id.clone());
        confirmed.display_source = Some(DisplaySource::Manual {
            device: state.device_id,
        });
        confirmed.display_status = "commanded".into();
        f.runtime.remind_signal_source(&confirmed);
        assert!(!requests.has_changed().unwrap());
    }

    #[tokio::test]
    async fn tray_choice_records_source_only_and_does_not_override_another_confirmation() {
        for local in [true, false] {
            let f = pending_signal_fixture().await;
            let prompt = f.runtime.signal_source_prompt().await.unwrap();
            let before = f.runtime.snapshot().await;
            assert!(f
                .runtime
                .confirm_signal_source(&prompt, local)
                .await
                .unwrap());
            let after = f.runtime.snapshot().await;
            let expected = if local { &after.device_id } else { &f.peer };
            assert_eq!(after.display_device_id.as_ref(), Some(expected));
            assert_eq!(
                after.display_source,
                Some(DisplaySource::Manual {
                    device: expected.clone()
                })
            );
            assert_eq!(after.active_device_id, before.active_device_id);
            assert_eq!(after.last_display_device_id, before.last_display_device_id);
            assert!(after.output_switch.is_none());
            assert!(f.runtime.signal_source_prompt().await.is_none());
            assert!(!f
                .runtime
                .confirm_signal_source(&prompt, !local)
                .await
                .unwrap());
            assert_eq!(
                f.runtime.snapshot().await.display_device_id,
                after.display_device_id
            );
            assert!(!f
                .runtime
                .platform
                .resources
                .join("native/bin/reads")
                .exists());
            assert!(!f
                .runtime
                .platform
                .resources
                .join("native/bin/violation")
                .exists());
        }
    }

    #[tokio::test]
    async fn old_tray_choices_are_ignored_after_reconnect_switch_stop_or_config_change() {
        for changed in ["reconnect", "switch", "stop", "config"] {
            let f = pending_signal_fixture().await;
            let prompt = f.runtime.signal_source_prompt().await.unwrap();
            let mut new_remote = None;
            match changed {
                "reconnect" => {
                    f.runtime.active_session.fetch_add(1, Ordering::AcqRel);
                    new_remote = Some(
                        super::super::pointer_bridge::tests::attach_with_requests(&f.runtime)
                            .await
                            .0,
                    );
                }
                "switch" => {
                    let permit = f.runtime.switch_protection.enter(Some(&f.peer)).unwrap();
                    drop(permit);
                }
                "stop" => f.runtime.stop().await.unwrap(),
                "config" => {
                    f.runtime.inner.lock().await.state.config.output_id = "replacement".into()
                }
                _ => unreachable!(),
            }
            let before = f.runtime.snapshot().await;
            assert!(!f
                .runtime
                .confirm_signal_source(&prompt, false)
                .await
                .unwrap());
            assert_eq!(
                f.runtime.snapshot().await.display_device_id,
                before.display_device_id
            );
            if let Some(remote) = new_remote {
                remote.close();
            }
        }
    }

    #[tokio::test]
    async fn tray_choice_revalidates_after_waiting_for_effects() {
        let f = pending_signal_fixture().await;
        let prompt = f.runtime.signal_source_prompt().await.unwrap();
        let effects = f.runtime.effects.lock().await;
        let runtime = f.runtime.clone();
        let confirming =
            tokio::spawn(async move { runtime.confirm_signal_source(&prompt, false).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while f.runtime.commands.try_lock().is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let local = f.runtime.snapshot().await.device_id;
        {
            let mut i = f.runtime.inner.lock().await;
            i.state.display_device_id = Some(local.clone());
            i.state.display_status = "commanded".into();
            i.state.display_source = Some(DisplaySource::Manual {
                device: local.clone(),
            });
        }
        drop(effects);
        assert!(!confirming.await.unwrap().unwrap());
        assert_eq!(f.runtime.snapshot().await.display_device_id, Some(local));
    }

    #[tokio::test]
    async fn receiver_tray_choice_uses_captured_local_and_peer_ids_without_native_effects() {
        for local in [true, false] {
            let f = pending_signal_fixture().await;
            f.runtime.inner.lock().await.state.config.role = "client".into();
            let prompt = f.runtime.signal_source_prompt().await.unwrap();
            let expected = if local {
                prompt.device.clone()
            } else {
                prompt.peer.clone()
            };
            f.service.abort();
            // setup's reply loop only permits read RPCs; use a separate captured
            // session to assert the exact confirmation request and target.
            let (remote, mut incoming) =
                super::super::pointer_bridge::tests::attach_with_requests(&f.runtime).await;
            let prompt = f.runtime.signal_source_prompt().await.unwrap();
            let replying = remote.clone();
            let server = tokio::spawn(async move {
                let Incoming::Request {
                    id,
                    method,
                    payload,
                } = incoming.recv().await.unwrap()
                else {
                    panic!("Expected confirmation request");
                };
                assert_eq!(method, "confirm-display-source");
                assert_eq!(payload, json!(expected));
                replying.reply(id, Ok(json!(true))).await.unwrap();
            });
            let before = f.runtime.snapshot().await;
            assert!(f
                .runtime
                .confirm_signal_source(&prompt, local)
                .await
                .unwrap());
            server.await.unwrap();
            let after = f.runtime.snapshot().await;
            assert_eq!(after.active_device_id, before.active_device_id);
            assert_eq!(after.display_device_id, before.display_device_id);
            remote.close();
        }
    }
}
