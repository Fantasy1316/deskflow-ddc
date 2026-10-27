#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    deskflow_ddc_tauri::desktop::run();
}
