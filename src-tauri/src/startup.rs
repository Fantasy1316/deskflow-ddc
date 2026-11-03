use crate::model::{Config, OperationMode};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub launch_at_login: bool,
    pub auto_share: bool,
}

pub fn validate_auto_share(config: &Config, paired: bool) -> Result<()> {
    config.ready()?;
    ensure!(
        config.mode == OperationMode::DisplayOnly || paired,
        "请先完成一次配对并成功建立共享，再开启启动后自动共享"
    );
    Ok(())
}

pub fn should_start(preferences: &Preferences, running: bool, epoch: u64) -> bool {
    // Any explicit start/stop already handled by this process wins over the
    // one-shot launch task, including a stop before that task is scheduled.
    preferences.auto_share && !running && epoch == 0
}

pub fn windows_login_command(executable: &str) -> Result<String> {
    ensure!(
        !executable.is_empty() && !executable.contains(['"', '\r', '\n', '\0']),
        "启动程序路径无效"
    );
    Ok(format!("\"{executable}\" --autostart"))
}

#[cfg(any(windows, test))]
pub(crate) fn windows_registry_entry_missing(hresult: i32) -> bool {
    // HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND).
    // Access denied, bad value types and other failures must remain errors.
    matches!(hresult as u32, 0x8007_0002 | 0x8007_0003)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_registry_entries_are_optional_but_other_failures_are_not() {
        assert!(windows_registry_entry_missing(-2147024894));
        assert!(windows_registry_entry_missing(0x8007_0003u32 as i32));
        for code in [0, 2, 3, 0x8007_0005u32 as i32, 0x8007_000du32 as i32] {
            assert!(!windows_registry_entry_missing(code));
        }
    }

    #[test]
    fn legacy_settings_default_off_and_options_are_independent() {
        let empty: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Preferences::default());
        for launch in [false, true] {
            let p = Preferences {
                launch_at_login: launch,
                auto_share: true,
            };
            assert!(should_start(&p, false, 0));
            assert!(!should_start(&p, true, 0));
            assert!(!should_start(&p, false, 1));
        }
        assert!(!should_start(&empty, false, 0));
        assert!(serde_json::from_str::<Preferences>(r#"{"autoShare":"yes"}"#).is_err());
        assert!(serde_json::from_str::<Preferences>(r#"{"unknown":true}"#).is_err());
    }

    #[test]
    fn windows_login_quotes_paths_with_spaces_and_unicode() {
        assert_eq!(
            windows_login_command(r"C:\Program Files\Deskflow DDC\屏桥.exe").unwrap(),
            "\"C:\\Program Files\\Deskflow DDC\\屏桥.exe\" --autostart"
        );
        assert!(windows_login_command("bad\"path.exe").is_err());
    }

    #[test]
    fn automatic_sharing_requires_configuration_and_trust_but_display_only_needs_no_peer() {
        let mut config = Config::default();
        assert!(validate_auto_share(&config, true).is_err());
        config.display_id = "shared".into();
        config.local_input = Some(3);
        config.peer_input = Some(1);
        assert!(validate_auto_share(&config, false).is_err());
        assert!(validate_auto_share(&config, true).is_ok());
        config.mode = OperationMode::DisplayOnly;
        assert!(validate_auto_share(&config, false).is_ok());
    }
}
