use super::*;
use std::{
    ffi::{c_char, c_void, CStr},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static DRAG_PEER: Mutex<Option<Uuid>> = Mutex::new(None);
static HOVER: AtomicU64 = AtomicU64::new(0);
unsafe extern "C" {
    fn ddc_file_drop_install(
        item: *mut c_void,
        callback: extern "C" fn(i32, *const c_char) -> bool,
    ) -> bool;
    fn ddc_file_drop_hide();
    fn ddc_file_drop_place(
        item: *mut c_void,
        window: *mut c_void,
        width: f64,
        height: f64,
        callback: extern "C" fn(i32, *const c_char) -> bool,
    ) -> bool;
}

extern "C" fn event(kind: i32, paths: *const c_char) -> bool {
    let Some(app) = APP.get() else {
        return false;
    };
    if app.state::<ExitState>().requested() {
        return false;
    }
    let ready = {
        let view = app.state::<Panel>().snapshot();
        view.disabled_reason.is_none() && !view.busy
    };
    match kind {
        3 => {
            dismiss_on_blur(app);
            return true;
        }
        1 => {
            *DRAG_PEER.lock().unwrap() = app.state::<Panel>().snapshot().peer_id;
            drag_changed(app, true, true);
            let ticket = HOVER.fetch_add(1, Ordering::AcqRel) + 1;
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(350)).await;
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || {
                    if HOVER.load(Ordering::Acquire) == ticket {
                        open(&handle, false);
                    }
                });
            });
        }
        4 => {
            let ticket = HOVER.fetch_add(1, Ordering::AcqRel) + 1;
            // Preserve the tray's captured recipient while crossing the gap
            // into the panel, even if pairing changes during that drag.
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(160)).await;
                if HOVER.load(Ordering::Acquire) == ticket {
                    *DRAG_PEER.lock().unwrap() = None;
                }
            });
            drag_changed(app, true, false);
            return true;
        }
        2 if ready && !paths.is_null() => {
            let Some(peer) = DRAG_PEER.lock().unwrap().take() else {
                return false;
            };
            HOVER.fetch_add(1, Ordering::AcqRel);
            app.state::<Panel>().interaction.lock().unwrap().dropped();
            open(app, false);
            // Cocoa owns the string only for this callback; copy before spawning.
            let paths = unsafe { CStr::from_ptr(paths) }.to_bytes();
            if paths.len() > 1024 * 1024 {
                return false;
            }
            let Ok(paths) = serde_json::from_slice::<Vec<PathBuf>>(paths) else {
                return false;
            };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let result = async {
                    let sending = begin(&app, peer).await?;
                    send(&app, paths, &sending).await
                }
                .await;
                outcome_for(&app, Some(peer), &result);
            });
        }
        _ => return false,
    }
    ready
}

pub(super) fn take_drag_peer() -> Option<Uuid> {
    DRAG_PEER.lock().unwrap().take()
}

pub(super) fn hide() {
    HOVER.fetch_add(1, Ordering::AcqRel);
    *DRAG_PEER.lock().unwrap() = None;
    // The owner groups monitor removal and window hiding on the UI thread.
    unsafe { ddc_file_drop_hide() };
}

pub(super) fn install(app: &tauri::AppHandle) -> Result<()> {
    let _ = APP.set(app.clone());
    let tray = app
        .tray_by_id("main-tray")
        .ok_or_else(|| anyhow::anyhow!("托盘不存在"))?;
    tray.with_inner_tray_icon(move |tray| -> Result<()> {
        let item = tray
            .ns_status_item()
            .ok_or_else(|| anyhow::anyhow!("系统托盘不可用"))?;
        ensure!(
            unsafe { ddc_file_drop_install(&*item as *const _ as *mut c_void, event) },
            "无法注册托盘文件拖放"
        );
        Ok(())
    })?
}

pub(super) fn place(app: &tauri::AppHandle, window: &WebviewWindow, height: f64) -> Result<()> {
    let panel = window.ns_window()? as usize;
    let tray = app
        .tray_by_id("main-tray")
        .ok_or_else(|| anyhow::anyhow!("托盘不存在"))?;
    tray.with_inner_tray_icon(move |tray| -> Result<()> {
        let item = tray
            .ns_status_item()
            .ok_or_else(|| anyhow::anyhow!("系统托盘不可用"))?;
        ensure!(
            unsafe {
                ddc_file_drop_place(
                    &*item as *const _ as *mut c_void,
                    panel as *mut c_void,
                    WIDTH,
                    height,
                    event,
                )
            },
            "无法定位文件发送面板"
        );
        Ok(())
    })?
}
