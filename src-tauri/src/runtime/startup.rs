use super::*;

impl Runtime {
    pub async fn validate_startup(&self, settings: &crate::startup::Preferences) -> Result<()> {
        if settings.auto_share {
            let i = self.inner.lock().await;
            crate::startup::validate_auto_share(
                &i.state.config,
                i.store.value.trusted_peer.is_some(),
            )?;
        }
        Ok(())
    }

    pub async fn save_startup(
        &self,
        settings: crate::startup::Preferences,
        registered: bool,
    ) -> Result<()> {
        let _lifecycle = self.start_stop.lock().await;
        self.validate_startup(&settings).await?;
        let mut i = self.inner.lock().await;
        let previous = i.store.value.startup.clone();
        i.store.value.startup = settings.clone();
        if let Err(error) = i.store.save() {
            i.store.value.startup = previous;
            return Err(error);
        }
        i.state.startup = settings;
        i.state.startup_registered = Some(registered);
        drop(i);
        self.publish();
        self.log("success", "启动设置已保存，下次启动应用时生效")
            .await;
        Ok(())
    }

    pub async fn set_startup_registered(&self, registered: bool) {
        self.inner.lock().await.state.startup_registered = Some(registered);
        self.publish();
    }

    pub async fn start_automatically(self: &Arc<Self>) -> Result<bool> {
        let _action = self.actions.lock().await;
        let state = self.snapshot().await;
        if !crate::startup::should_start(
            &state.startup,
            state.running,
            self.epoch.load(Ordering::Acquire),
        ) {
            return Ok(false);
        }
        self.validate_startup(&state.startup).await?;
        ensure!(!self.shutdown.is_cancelled(), "应用正在退出");
        // The normal start path rechecks permissions, native dependencies,
        // credentials and Windows service readiness. Never bypass its guards.
        self.start(None).await?;
        if state.config.mode == OperationMode::Sharing {
            // A background login launch cannot assume which input the monitor
            // selected while the computers were starting. Read before toggling.
            self.display_recovery.lock().await.mark();
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn automatic_display_only_enables_once_without_input_engine_network_or_ddc_write() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "host");
        let helper = runtime.platform.resources.join(if cfg!(windows) {
            "native/monitor.ps1"
        } else {
            "native/bin/m1ddc"
        });
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(
            helper,
            "not an executable: automatic enable must not write DDC",
        )
        .unwrap();
        {
            let mut i = runtime.inner.lock().await;
            i.state.running = false;
            i.state.config.mode = OperationMode::DisplayOnly;
            i.state.config.display_id = "shared".into();
            i.state.config.local_input = Some(3);
            i.state.config.peer_input = Some(1);
            i.state.startup.auto_share = true;
        }
        runtime.epoch.store(0, Ordering::Release);
        assert!(runtime.start_automatically().await.unwrap());
        assert!(!runtime.start_automatically().await.unwrap());
        assert!(runtime.snapshot().await.running);
        assert!(runtime.network.lock().await.is_none());
        assert!(runtime.engine.lock().await.is_none());
        runtime.stop().await.unwrap();
        assert!(!runtime.start_automatically().await.unwrap());
        assert!(!runtime.snapshot().await.running);
    }

    #[tokio::test]
    async fn disabled_unpaired_and_manually_stopped_launches_never_start_sharing() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "host");
        {
            let mut i = runtime.inner.lock().await;
            i.state.running = false;
            i.state.config.display_id = "shared".into();
            i.state.config.local_input = Some(3);
            i.state.config.peer_input = Some(1);
        }
        runtime.epoch.store(0, Ordering::Release);
        assert!(!runtime.start_automatically().await.unwrap());
        runtime.inner.lock().await.state.startup.auto_share = true;
        let error = runtime.start_automatically().await.unwrap_err().to_string();
        assert!(error.contains("配对"));
        assert!(runtime.network.lock().await.is_none());
        assert!(runtime.engine.lock().await.is_none());
        runtime.stop().await.unwrap();
        assert!(!runtime.start_automatically().await.unwrap());
    }

    #[tokio::test]
    async fn startup_preferences_save_while_sharing_without_overwriting_config_and_roll_back_on_disk_failure(
    ) {
        let directory = tempfile::tempdir().unwrap();
        let runtime = super::super::pointer_bridge::tests::fixture(directory.path(), "host");
        let before = runtime.snapshot().await;
        let settings = crate::startup::Preferences {
            launch_at_login: true,
            auto_share: false,
        };
        runtime.save_startup(settings.clone(), true).await.unwrap();
        let saved = runtime.snapshot().await;
        assert!(saved.running);
        assert_eq!(saved.config, before.config);
        assert_eq!(saved.startup, settings);
        let value: Value =
            serde_json::from_slice(&std::fs::read(directory.path().join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(value["startup"]["launchAtLogin"], true);
        std::fs::remove_file(directory.path().join("settings.json")).unwrap();
        std::fs::create_dir(directory.path().join("settings.json")).unwrap();
        assert!(runtime
            .save_startup(crate::startup::Preferences::default(), false)
            .await
            .is_err());
        assert_eq!(runtime.snapshot().await.startup, settings);
        assert_eq!(runtime.inner.lock().await.store.value.startup, settings);
    }
}
