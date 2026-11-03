use super::*;

fn can_lock(s: &Snapshot) -> bool {
    s.config.mode == OperationMode::Sharing
        && s.running
        && s.connected
        && s.engine == "ready"
        && s.config.role == "host"
        && s.peer
            .as_ref()
            .is_some_and(|p| p.platform == "win32" && p.lock_control)
}
fn host_lock_shortcut(s: &Snapshot) -> bool {
    can_lock(s)
        && s.peer
            .as_ref()
            .is_some_and(|p| s.active_device_id.as_deref() == Some(p.id.as_str()))
}
fn local_lock_shortcut(s: &Snapshot) -> Option<String> {
    if !(s.running
        && s.connected
        && s.engine == "ready"
        && s.config.role == "client"
        && s.active_device_id.as_deref() == Some(s.device_id.as_str()))
    {
        return None;
    }
    let peer = s.peer.as_ref()?;
    Some(
        peer.modifier_mapping
            .receiver_shortcut("control+alt+shift+L", &peer.platform, "win32"),
    )
}
impl Runtime {
    pub async fn lock_peer(&self) -> Result<()> {
        let s = self.snapshot().await;
        ensure!(can_lock(&s), "当前没有可锁定的 Windows 接收会话");
        let link = self.link.lock().await.clone().context("共享连接已断开")?;
        ensure!(self.current(&link), "共享会话已结束");
        let result = link
            .session
            .request("lock-session", json!({}), Duration::from_secs(5))
            .await;
        self.trace("system.lockRequested", json!({"peer":s.peer.as_ref().map(|p|&p.id), "accepted":result.is_ok(),"error":result.as_ref().err().map(|e|e.to_string())})).await;
        result?;
        self.log(
            "info",
            "已向 Windows 发送锁屏指令；密码界面控制需要在接收端启用安全桌面服务",
        )
        .await;
        Ok(())
    }
    pub async fn lock_shortcut(&self) -> Option<String> {
        let s = self.snapshot().await;
        if host_lock_shortcut(&s) {
            return Some("control+alt+shift+L".into());
        }
        // While Deskflow forwards keys, the provider's OS may not receive its
        // global shortcut. Register the translated chord on the receiver too.
        if platform() == "win32" {
            local_lock_shortcut(&s)
        } else {
            None
        }
    }
    pub async fn lock_from_shortcut(&self) -> Result<()> {
        let s = self.snapshot().await;
        if host_lock_shortcut(&s) {
            return self.lock_peer().await;
        }
        ensure!(
            platform() == "win32" && local_lock_shortcut(&s).is_some(),
            "当前没有可锁定的 Windows 接收会话"
        );
        let link = self.link.lock().await.clone().context("共享连接已断开")?;
        ensure!(self.current(&link), "共享会话已结束");
        self.platform.lock_session().await?;
        self.trace("system.lockShortcutAccepted", json!({"session":link.id}))
            .await;
        Ok(())
    }
    pub async fn refresh_windows_service(&self) -> Result<()> {
        #[cfg(windows)]
        {
            let status = tokio::task::spawn_blocking(crate::windows_service::status).await??;
            self.inner.lock().await.state.windows_service = status;
            self.publish();
        }
        Ok(())
    }
    pub async fn manage_windows_service(&self, remove: bool) -> Result<()> {
        ensure!(
            !self.snapshot().await.running,
            "请先停止共享再安装、更新或移除后台服务"
        );
        #[cfg(windows)]
        {
            crate::windows_service::manage(self.platform.resources.clone(), remove).await?;
            self.refresh_windows_service().await?;
            if remove {
                let mut c = self.snapshot().await.config;
                c.windows_secure_input = false;
                self.save(c).await?;
            }
            return Ok(());
        }
        #[cfg(not(windows))]
        {
            let _ = remove;
            bail!("后台服务仅适用于 Windows 接收端")
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lock_requires_connected_windows_receiver_and_chord_requires_remote_input() {
        let mut s = super::super::tests::snapshot();
        // Begin from a disconnected snapshot: every missing precondition denies.
        assert!(!can_lock(&s));
        s.running = true;
        s.connected = true;
        s.engine = "ready".into();
        s.config.role = "host".into();
        s.peer = Some(Peer {
            file_transfer: false,
            lock_control: true,
            id: "peer".into(),
            name: "Windows".into(),
            platform: "win32".into(),
            fingerprint: "a".repeat(64),
            input_configured: true,
            retained_display: false,
            modifier_mapping: ModifierMapping::Original,
        });
        s.active_device_id = Some("peer".into());
        assert!(can_lock(&s));
        assert!(host_lock_shortcut(&s));
        s.active_device_id = Some(s.device_id.clone());
        assert!(can_lock(&s)); // Explicit button/tray actions remain usable on Mac.
        assert!(!host_lock_shortcut(&s));
        s.active_device_id = Some("peer".into());
        s.peer.as_mut().unwrap().platform = "darwin".into();
        assert!(!can_lock(&s));
        s.peer.as_mut().unwrap().platform = "win32".into();
        s.config.role = "client".into();
        assert!(!can_lock(&s));
    }
    #[test]
    fn forwarded_lock_chord_follows_modifier_mapping_and_input_ownership() {
        let mut s = super::super::tests::snapshot();
        s.running = true;
        s.connected = true;
        s.engine = "ready".into();
        s.config.role = "client".into();
        s.active_device_id = Some(s.device_id.clone());
        assert!(local_lock_shortcut(&s).is_none());
        s.peer = Some(Peer {
            file_transfer: false,
            lock_control: false,
            id: "host".into(),
            name: "Mac".into(),
            platform: "darwin".into(),
            fingerprint: "a".repeat(64),
            input_configured: true,
            retained_display: false,
            modifier_mapping: ModifierMapping::Common,
        });
        assert_eq!(
            local_lock_shortcut(&s).as_deref(),
            Some("super+alt+shift+L")
        );
        s.peer.as_mut().unwrap().modifier_mapping = ModifierMapping::Original;
        assert_eq!(
            local_lock_shortcut(&s).as_deref(),
            Some("control+alt+shift+L")
        );
        s.active_device_id = Some("host".into());
        assert!(local_lock_shortcut(&s).is_none());
        s.active_device_id = Some(s.device_id.clone());
        s.connected = false;
        assert!(local_lock_shortcut(&s).is_none());
    }
}
