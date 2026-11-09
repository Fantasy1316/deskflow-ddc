use super::*;
use std::ffi::{c_char, c_void, CString};

unsafe extern "C" {
    fn ddc_source_popover_show(
        item: *mut c_void,
        ticket: u64,
        local: *const c_char,
        peer: *const c_char,
        busy: bool,
        error: *const c_char,
        action: extern "C" fn(u64, i32),
    ) -> bool;
    fn ddc_source_popover_hide(ticket: u64);
}
extern "C" fn action(ticket: u64, choice: i32) {
    event(ticket, choice);
}
fn text(value: &str) -> CString {
    CString::new(value.replace('\0', "�")).expect("NULs replaced")
}
pub(super) fn show(app: &tauri::AppHandle, view: View) -> Result<()> {
    let tray = app
        .tray_by_id("main-tray")
        .ok_or_else(|| anyhow::anyhow!("托盘图标不存在"))?;
    tray.with_inner_tray_icon(move |tray| -> Result<()> {
        let item = tray
            .ns_status_item()
            .ok_or_else(|| anyhow::anyhow!("系统托盘控件不可用"))?;
        let local = text(&view.local_name);
        let peer = text(&view.peer_name);
        let error = view.error.as_deref().map(text);
        let accepted = unsafe {
            ddc_source_popover_show(
                &*item as *const _ as *mut c_void,
                view.ticket,
                local.as_ptr(),
                peer.as_ptr(),
                view.busy,
                error.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                action,
            )
        };
        ensure!(accepted, "NSPopover 未能依附托盘图标显示");
        Ok(())
    })?
}
pub(super) fn hide(ticket: u64) {
    unsafe {
        ddc_source_popover_hide(ticket);
    }
}
