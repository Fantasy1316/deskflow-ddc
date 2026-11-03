use anyhow::Result;
use windows_registry::{Key, CURRENT_USER};

const NAME: &str = "Deskflow DDC";
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

fn optional<T>(result: windows_registry::Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            let error = std::io::Error::from(error);
            if error.kind() == std::io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(error.into())
            }
        }
    }
}

pub(super) fn enabled() -> Result<bool> {
    let command = optional(CURRENT_USER.open(RUN).and_then(|key| key.get_string(NAME)))?;
    if command.is_none_or(|command| command.is_empty()) {
        return Ok(false);
    }
    let approved = optional(
        CURRENT_USER
            .open(APPROVED)
            .and_then(|key| key.get_value(NAME)),
    )?;
    Ok(!approved.is_some_and(|value| matches!(value.first(), Some(3 | 7))))
}

fn remove(key: &Key, path: &str) -> Result<()> {
    optional(
        key.options()
            .write()
            .open(path)
            .and_then(|key| key.remove_value(NAME)),
    )?;
    Ok(())
}

pub(super) fn set_enabled(wanted: bool) -> Result<()> {
    if wanted {
        let executable = std::env::current_exe()?;
        let command = crate::startup::windows_login_command(&executable.to_string_lossy())?;
        // Always HKCU, even when the UI was launched as administrator. Keep the
        // service executable separate from this user's login startup entry.
        CURRENT_USER.create(RUN)?.set_string(NAME, command)?;
    } else {
        remove(CURRENT_USER, RUN)?;
    }
    // A deliberate in-app change supersedes Task Manager's previous override.
    remove(CURRENT_USER, APPROVED)?;
    Ok(())
}
