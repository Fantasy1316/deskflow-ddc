use super::*;
use crate::ddc::{Detection, InputProtocol, Profile, Reading, TestReceipt};

fn monitor_identity(monitors: &Value, id: &str) -> Option<String> {
    monitors.as_array()?.iter().find(|m| m["id"] == id)?["identity"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

impl Runtime {
    /// Explicit, read-only setup operation. Never probes/writes a different
    /// protocol in the middle of a live switch or on wake/reconnect.
    pub async fn detect_ddc(&self, display_id: String) -> Result<()> {
        ensure!(
            !display_id.is_empty()
                && display_id.len() <= 512
                && !display_id.contains(['\n', '\r', '\0']),
            "请先选择显示器"
        );
        let _lifecycle = self.start_stop.lock().await;
        let state = self.snapshot().await;
        ensure!(
            !state.running && !state.emergency_stopping,
            "请先停止共享或停用显示器切换，再检测控制方式"
        );
        let monitors = self.platform.list().await?;
        ensure!(
            monitors
                .as_array()
                .is_some_and(|items| items.iter().any(|m| m["id"] == display_id)),
            "选中的显示器当前未连接，请重新扫描"
        );
        let identity = monitor_identity(&monitors, &display_id);
        let saved = state
            .ddc_profiles
            .iter()
            .find(|p| {
                Some(&p.identity) == identity.as_ref()
                    && p.platform == platform()
                    && p.protocol.supported(platform())
            })
            .cloned();
        let mut readings = vec![];
        for protocol in [InputProtocol::Standard, InputProtocol::InputAlt] {
            if !protocol.supported(platform()) {
                continue;
            }
            let result = self
                .platform
                .current_input_with_protocol(&display_id, protocol)
                .await;
            let (current_input, error) = match result {
                Ok(Some(value)) => (Some(value), None),
                Ok(None) => (None, Some("暂时无法读取；写入能力需单独测试".into())),
                Err(error) => (None, Some(format!("{error:#}").chars().take(700).collect())),
            };
            readings.push(Reading {
                protocol,
                current_input,
                error,
            });
        }
        let detection = Detection::new(display_id, identity, readings, saved);
        let mut inner = self.inner.lock().await;
        inner.state.monitors = monitors;
        inner.state.ddc_detection = Some(detection);
        drop(inner);
        self.publish();
        Ok(())
    }

    pub(super) async fn record_ddc_test(&self, config: &Config, value: u16, success: bool) {
        let mut i = self.inner.lock().await;
        if i.state.config != *config {
            return;
        }
        let identity = monitor_identity(&i.state.monitors, &config.display_id);
        if !i
            .state
            .ddc_test_receipt
            .as_ref()
            .is_some_and(|r| r.matches(config) && r.identity == identity)
        {
            let mut receipt = TestReceipt::for_config(config);
            receipt.identity = identity;
            i.state.ddc_test_receipt = Some(receipt);
        }
        let receipt = i.state.ddc_test_receipt.as_mut().unwrap();
        if Some(value) == config.local_input {
            receipt.local = success;
            receipt.returned_local = success && receipt.peer;
        } else if Some(value) == config.peer_input {
            receipt.peer = success;
            receipt.returned_local = false;
        }
    }

    /// Called only after separate user confirmations of BOTH successful tests.
    /// Profiles remain local; peers need not use the same addressing or values.
    pub async fn remember_ddc_profile(&self) -> Result<()> {
        let _lifecycle = self.start_stop.lock().await;
        let state = self.snapshot().await;
        ensure!(
            !state.running && state.config.display_method == DisplayMethod::Ddc,
            "请先停止共享，并使用 DDC 模式测试"
        );
        ensure!(
            state
                .ddc_test_receipt
                .as_ref()
                .is_some_and(|r| r.matches(&state.config) && r.local && r.peer && r.returned_local),
            "请先测试另一台输入，再测试切回本机，并确认两个方向的实际画面"
        );
        let identity = monitor_identity(&state.monitors, &state.config.display_id)
            .context("未读到稳定的显示器标识，请重新扫描后测试；当前映射仍可正常使用")?;
        ensure!(
            state
                .ddc_test_receipt
                .as_ref()
                .and_then(|r| r.identity.as_ref())
                == Some(&identity),
            "显示器标识已变化，请重新测试两个方向"
        );
        // Revalidate identity before associating a portable Windows DISPLAY
        // selector with persistent history; never match profiles by name alone.
        let monitors = self.platform.list_for(&state.config).await?;
        ensure!(
            monitor_identity(&monitors, &state.config.display_id).as_ref() == Some(&identity),
            "显示器或连接路径已变化，请重新扫描并测试"
        );
        let name = monitors
            .as_array()
            .and_then(|ms| ms.iter().find(|m| m["id"] == state.config.display_id))
            .and_then(|m| m["name"].as_str())
            .unwrap_or("显示器")
            .chars()
            .take(128)
            .collect();
        let profile = Profile {
            identity: identity.clone(),
            name,
            platform: platform().into(),
            protocol: state.config.input_protocol(),
            local_input: state.config.local_input.context("本机输入未设置")?,
            peer_input: state.config.peer_input.context("对端输入未设置")?,
        };
        let mut i = self.inner.lock().await;
        let previous = i.store.value.ddc_profiles.clone();
        i.store
            .value
            .ddc_profiles
            .retain(|p| p.identity != identity || p.platform != platform());
        i.store.value.ddc_profiles.insert(0, profile);
        i.store.value.ddc_profiles.truncate(24);
        if let Err(error) = i.store.save() {
            i.store.value.ddc_profiles = previous;
            return Err(error);
        }
        i.state.ddc_profiles = i.store.value.ddc_profiles.clone();
        drop(i);
        self.log(
            "success",
            "已记住本机显示器的双向确认配置；不代表其他线缆、电脑或接口也兼容",
        )
        .await;
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::ddc::ProtocolChoice;
    use std::os::unix::fs::PermissionsExt;
    const DISPLAY: &str = "12345678-1234-1234-1234-123456789abc";

    async fn fixture(directory: &std::path::Path) -> Arc<Runtime> {
        let runtime = super::super::pointer_bridge::tests::fixture(directory, "host");
        let native = runtime.platform.resources.join("native/bin");
        std::fs::create_dir_all(&native).unwrap();
        let helper = runtime.platform.mac_ddc_helper();
        std::fs::write(
            &helper,
            format!(
                r#"#!/bin/sh
echo "$*" >> "$0.calls"
if [ "$1 $2" = 'display list' ]; then echo '[1] LG Monitor ({DISPLAY})'; exit 0; fi
if [ "$2" != '{DISPLAY}' ]; then exit 9; fi
if [ "$3 $4" = 'get input-alt' ]; then echo 208; exit 0; fi
if [ "$3" = get ]; then echo unsupported; exit 1; fi
if [ -e "$0.fail" ]; then exit 1; fi
[ "$3 $4" = 'set input-alt' ] || exit 9
case "$5" in 208|144) exit 0;; *) exit 9;; esac
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut i = runtime.inner.lock().await;
        i.state.running = false;
        i.state.config.display_id = DISPLAY.into();
        i.state.config.local_input = Some(208);
        i.state.config.peer_input = Some(144);
        i.state.config.ddc_protocol = ProtocolChoice::InputAlt;
        i.store.value.config = i.state.config.clone();
        drop(i);
        runtime
    }
    #[tokio::test]
    async fn detection_is_read_only_and_history_requires_two_successful_current_mapping_tests() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = fixture(dir.path()).await;
        runtime.detect_ddc(DISPLAY.into()).await.unwrap();
        let before = runtime.snapshot().await;
        assert_eq!(
            before.ddc_detection.unwrap().recommended,
            Some(InputProtocol::InputAlt)
        );
        let calls = runtime.platform.mac_ddc_helper().with_extension("calls");
        assert!(!std::fs::read_to_string(&calls).unwrap().contains(" set "));
        assert!(runtime.remember_ddc_profile().await.is_err());
        runtime.test_input(208).await.unwrap();
        assert!(runtime.remember_ddc_profile().await.is_err());
        runtime.test_input(144).await.unwrap();
        assert!(runtime.remember_ddc_profile().await.is_err());
        runtime.test_input(208).await.unwrap();
        runtime.remember_ddc_profile().await.unwrap();
        let bytes = std::fs::read(dir.path().join("settings.json")).unwrap();
        let settings: crate::store::Stored = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(settings.ddc_profiles.len(), 1);
        assert_eq!(settings.ddc_profiles[0].protocol, InputProtocol::InputAlt);
        assert_eq!(settings.ddc_profiles[0].identity, DISPLAY);
        assert_eq!(settings.config, before.config);
        runtime.detect_ddc(DISPLAY.into()).await.unwrap();
        assert!(runtime
            .snapshot()
            .await
            .ddc_detection
            .unwrap()
            .saved_profile
            .is_some());
        // A failed repeat cannot reuse the previous successful receipt.
        std::fs::write(runtime.platform.mac_ddc_helper().with_extension("fail"), "").unwrap();
        assert!(runtime.test_input(208).await.is_err());
        assert!(runtime.remember_ddc_profile().await.is_err());
        assert!(std::fs::read_to_string(calls)
            .unwrap()
            .contains("set input-alt 144"));
    }
    #[tokio::test]
    async fn changed_protocol_or_display_identity_cannot_reuse_old_tests() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = fixture(dir.path()).await;
        runtime.detect_ddc(DISPLAY.into()).await.unwrap();
        runtime.test_input(208).await.unwrap();
        runtime.test_input(144).await.unwrap();
        runtime.test_input(208).await.unwrap();
        runtime.inner.lock().await.state.config.ddc_protocol = ProtocolChoice::Standard;
        assert!(runtime.remember_ddc_profile().await.is_err());
        let mut i = runtime.inner.lock().await;
        i.state.config.ddc_protocol = ProtocolChoice::InputAlt;
        i.state.monitors[0]["identity"] = json!("replacement");
        drop(i);
        assert!(runtime.remember_ddc_profile().await.is_err());
    }
}
