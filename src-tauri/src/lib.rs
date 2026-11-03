pub mod diagnostics;
pub mod discovery;
pub mod engine;
#[cfg(target_os = "macos")]
pub mod engine_guard;
pub mod model;
pub mod platform;
// Pure parsers also compile on other hosts for fixture tests.
pub mod linux;
pub mod pointer;
pub mod protocol;
pub mod startup;
pub mod store;

pub mod runtime;
pub mod session;

#[cfg(feature = "desktop")]
pub mod desktop;

pub mod switching;

pub mod bundle;
pub mod transfer;
pub mod windows_service;
