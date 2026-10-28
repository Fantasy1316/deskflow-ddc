#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    #[cfg(target_os = "macos")]
    if let Some(code) = deskflow_ddc_tauri::engine_guard::run_if_requested() {
        std::process::exit(code);
    }
    deskflow_ddc_tauri::desktop::run();
}
