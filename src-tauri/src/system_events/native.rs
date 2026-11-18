use super::Events;

#[cfg(target_os = "macos")]
#[path = "mac.rs"]
mod implementation;
#[cfg(windows)]
#[path = "windows.rs"]
mod implementation;
#[cfg(not(any(target_os = "macos", windows)))]
mod implementation {
    pub struct Registration;
    impl Registration {
        pub fn install(_: super::Events) -> anyhow::Result<Self> {
            Ok(Self)
        }
    }
}

// Tauri owns this registration; explicit quit unregisters on its GUI thread.
// Drop is also a safety net for startup failure and normal state destruction.
pub(crate) struct Monitor(std::sync::Mutex<Option<implementation::Registration>>);
impl Monitor {
    pub fn install(events: Events) -> anyhow::Result<Self> {
        Ok(Self(std::sync::Mutex::new(Some(
            implementation::Registration::install(events)?,
        ))))
    }
    pub fn stop(&self) {
        self.0.lock().unwrap().take();
    }
}
