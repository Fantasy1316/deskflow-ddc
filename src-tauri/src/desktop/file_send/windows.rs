//! Bind OLE file drops after the lazy WebView has created its child HWNDs.
//! Wry only enumerates them once at creation and ignores registration failures.
//! Keep this override scoped to the file panel; the main window retains Wry's handler.
use super::*;
use ::windows::{
    core::{implement, Ref, BOOL},
    Win32::{
        Foundation::{DRAGDROP_E_NOTREGISTERED, HWND, LPARAM, POINT, POINTL},
        Graphics::Gdi::ScreenToClient,
        System::{
            Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL},
            Ole::{
                IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
                CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE,
            },
            SystemServices::MODIFIERKEYS_FLAGS,
        },
        UI::{
            Shell::{DragQueryFileW, HDROP},
            WindowsAndMessaging::EnumChildWindows,
        },
    },
};
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    os::windows::ffi::OsStringExt,
};

// OLE targets belong to the window's STA. Never store/release COM pointers from
// a Tokio worker; retain them here until replacement or native destruction.
thread_local! {
    static TARGETS: RefCell<Vec<(HWND, IDropTarget)>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn remove() {
    TARGETS.with_borrow_mut(|targets| {
        for (hwnd, _) in targets.drain(..) {
            let _ = unsafe { RevokeDragDrop(hwnd) };
        }
    });
}

pub(super) fn install(window: &WebviewWindow) {
    let app = window.app_handle().clone();
    // with_webview runs on the UI thread, after the controller is available.
    // Install again after page-load and on subsequent shows to cover late or
    // recreated WebView child windows, rather than relying on hidden creation.
    let handle = window.clone();
    let _ = window.with_webview(move |_| {
        let result = (|| -> Result<usize> {
            let root = HWND(handle.hwnd()?.0);
            let mut handles = vec![root];
            unsafe extern "system" fn collect(hwnd: HWND, param: LPARAM) -> BOOL {
                unsafe { &mut *(param.0 as *mut Vec<HWND>) }.push(hwnd);
                true.into()
            }
            unsafe {
                let _ = EnumChildWindows(
                    Some(root),
                    Some(collect),
                    LPARAM(&mut handles as *mut _ as isize),
                );
            }
            // Revoke only this panel's existing registrations. Releasing our
            // retained targets before replacing them also clears drag sessions.
            remove();
            TARGETS.with_borrow_mut(|targets| -> Result<usize> {
                for hwnd in handles {
                    let revoke = unsafe { RevokeDragDrop(hwnd) };
                    if let Err(error) = revoke {
                        ensure!(
                            error.code() == DRAGDROP_E_NOTREGISTERED,
                            "无法接管文件发送窗口拖放：{error}"
                        );
                    }
                    let target: IDropTarget = FileTarget {
                        root,
                        app: app.clone(),
                        entered: Cell::new(false),
                    }
                    .into();
                    unsafe { RegisterDragDrop(hwnd, &target) }
                        .context("无法注册文件发送窗口拖放")?;
                    targets.push((hwnd, target));
                }
                Ok(targets.len())
            })
        })();
        match result {
            Ok(count) => {
                let runtime = app.state::<Arc<Runtime>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    runtime
                        .trace("file.panelDropReady", json!({"targets":count}))
                        .await;
                });
            }
            Err(error) => outcome(&app, &Err(error)),
        }
    });
}

// IDataObject owns this medium until ReleaseStgMedium. DragFinish is only for
// WM_DROPFILES and must not be used here (including on Enter or rejected drops).
struct Medium(STGMEDIUM);
impl Drop for Medium {
    fn drop(&mut self) {
        unsafe { ReleaseStgMedium(&mut self.0) };
    }
}

fn paths(data: Ref<'_, IDataObject>) -> Option<Vec<PathBuf>> {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let medium = Medium(unsafe { data.as_ref()?.GetData(&format).ok()? });
    medium_paths(&medium)
}

fn medium_paths(medium: &Medium) -> Option<Vec<PathBuf>> {
    if medium.0.tymed != TYMED_HGLOBAL.0 as u32 {
        return None;
    }
    let drop = HDROP(unsafe { medium.0.u.hGlobal.0 });
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    if !(1..=256).contains(&count) {
        return None;
    }
    let mut paths = Vec::with_capacity(count as usize);
    let mut total = 0;
    for i in 0..count {
        let len = unsafe { DragQueryFileW(drop, i, None) } as usize;
        total += len;
        if len == 0 || total > 1024 * 1024 {
            return None;
        }
        let mut value = vec![0u16; len + 1];
        if unsafe { DragQueryFileW(drop, i, Some(&mut value)) } as usize != len {
            return None;
        }
        let path = PathBuf::from(OsString::from_wide(&value[..len]));
        if !path.is_absolute() {
            return None;
        }
        paths.push(path);
    }
    Some(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::windows::Win32::{
        System::{
            Com::STGMEDIUM_0,
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT},
        },
        UI::Shell::DROPFILES,
    };

    fn medium(files: &[&str]) -> Medium {
        let wide: Vec<u16> = files
            .iter()
            .flat_map(|p| p.encode_utf16().chain([0]))
            .chain([0])
            .collect();
        let size = std::mem::size_of::<DROPFILES>();
        let memory =
            unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size + wide.len() * 2).unwrap() };
        let pointer = unsafe { GlobalLock(memory) };
        assert!(!pointer.is_null());
        unsafe {
            pointer.cast::<DROPFILES>().write(DROPFILES {
                pFiles: size as u32,
                fWide: true.into(),
                ..Default::default()
            });
            std::ptr::copy_nonoverlapping(
                wide.as_ptr(),
                pointer.byte_add(size).cast::<u16>(),
                wide.len(),
            );
            let _ = GlobalUnlock(memory);
        }
        Medium(STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: memory },
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        })
    }

    #[test]
    fn ole_drop_reads_unicode_files_folders_and_unc_without_max_path_truncation() {
        let long = format!("C:\\{}\\file.zip", "long-directory\\".repeat(30));
        let files = [
            "C:\\Users\\测试\\file with spaces.zip",
            "C:\\Users\\jay\\folder",
            "\\\\server\\share\\file",
            long.as_str(),
        ];
        let data = medium(&files);
        assert_eq!(medium_paths(&data).unwrap(), files.map(PathBuf::from));
    }

    #[test]
    fn ole_drop_rejects_empty_relative_and_oversized_lists() {
        assert!(medium_paths(&medium(&[])).is_none());
        assert!(medium_paths(&medium(&["relative.txt"])).is_none());
        assert!(medium_paths(&medium(&vec!["C:\\file"; 257])).is_none());
    }
}

#[implement(IDropTarget)]
struct FileTarget {
    root: HWND,
    app: tauri::AppHandle,
    entered: Cell<bool>,
}

impl FileTarget {
    fn ready(&self) -> bool {
        let panel = self.app.state::<Panel>();
        let view = panel.snapshot();
        !self.app.state::<ExitState>().requested()
            && view.disabled_reason.is_none()
            && !view.busy
            && view.peer_id.is_some()
    }
    fn position(&self, point: &POINTL) -> tauri::PhysicalPosition<f64> {
        let mut point = POINT {
            x: point.x,
            y: point.y,
        };
        let _ = unsafe { ScreenToClient(self.root, &mut point) };
        tauri::PhysicalPosition::new(point.x as f64, point.y as f64)
    }
    fn leave(&self) {
        if self.entered.replace(false) {
            native_drop(&self.app, &tauri::DragDropEvent::Leave);
        }
    }
    fn copy_allowed(&self, effect: *mut DROPEFFECT) -> bool {
        !effect.is_null() && unsafe { (*effect & DROPEFFECT_COPY) != DROPEFFECT_NONE }
    }
    fn effect(&self, effect: *mut DROPEFFECT, accepted: bool) {
        if !effect.is_null() {
            unsafe {
                *effect = if accepted {
                    DROPEFFECT_COPY
                } else {
                    DROPEFFECT_NONE
                };
            }
        }
    }
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for FileTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<'_, IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> ::windows::core::Result<()> {
        self.leave();
        let files = if self.ready() && self.copy_allowed(effect) {
            paths(data)
        } else {
            None
        };
        self.entered.set(files.is_some());
        if let Some(paths) = files {
            native_drop(
                &self.app,
                &tauri::DragDropEvent::Enter {
                    paths,
                    position: self.position(point),
                },
            );
        }
        self.effect(effect, self.entered.get());
        Ok(())
    }
    fn DragOver(
        &self,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> ::windows::core::Result<()> {
        let accepted = self.entered.get() && self.ready() && self.copy_allowed(effect);
        if accepted {
            native_drop(
                &self.app,
                &tauri::DragDropEvent::Over {
                    position: self.position(point),
                },
            );
        }
        self.effect(effect, accepted);
        Ok(())
    }
    fn DragLeave(&self) -> ::windows::core::Result<()> {
        self.leave();
        Ok(())
    }
    fn Drop(
        &self,
        data: Ref<'_, IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> ::windows::core::Result<()> {
        let files = if self.entered.replace(false) && self.ready() && self.copy_allowed(effect) {
            paths(data)
        } else {
            None
        };
        self.effect(effect, files.is_some());
        if let Some(paths) = files {
            native_drop(
                &self.app,
                &tauri::DragDropEvent::Drop {
                    paths,
                    position: self.position(point),
                },
            );
        } else {
            native_drop(&self.app, &tauri::DragDropEvent::Leave);
        }
        Ok(())
    }
}
