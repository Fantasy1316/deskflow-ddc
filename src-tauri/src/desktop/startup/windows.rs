use anyhow::Result;
use windows_registry::{Key, CURRENT_USER};

const NAME: &str = "Deskflow DDC";
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

fn optional<T>(result: windows_registry::Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        // windows-registry returns HRESULTs. Converting to io::Error retains
        // the HRESULT as a negative OS code, so ErrorKind::NotFound misses it.
        Err(error) if crate::startup::windows_registry_entry_missing(error.code().0) => Ok(None),
        Err(error) => Err(error.into()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_key_and_value_are_optional_and_removing_twice_is_safe() {
        // Exercise real HRESULTs without touching the user's startup entries.
        let path = format!(r"Software\Deskflow-DDC-Tests\{}", uuid::Uuid::new_v4());
        assert!(optional(CURRENT_USER.open(&path)).unwrap().is_none());
        let key = CURRENT_USER.create(&path).unwrap();
        let result = (|| -> Result<()> {
            assert!(optional(key.get_string(NAME))?.is_none());
            key.set_string(NAME, "test --autostart")?;
            assert_eq!(
                optional(key.get_string(NAME))?.as_deref(),
                Some("test --autostart")
            );
            drop(key);
            remove(CURRENT_USER, &path)?;
            remove(CURRENT_USER, &path)?;
            Ok(())
        })();
        let cleanup = CURRENT_USER.remove_tree(&path);
        result.unwrap();
        cleanup.unwrap();
    }
}
