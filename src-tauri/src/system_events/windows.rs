use super::super::{Event, Events};
use anyhow::{ensure, Context, Result};
use std::{
    mem::size_of,
    ptr::{null, null_mut},
    sync::mpsc,
    thread::JoinHandle,
};
use windows_sys::{
    core::GUID,
    Win32::{Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*},
};

pub struct Registration {
    window: usize,
    thread: Option<JoinHandle<()>>,
}
const MONITOR: GUID = GUID {
    data1: 0xe6f07b5f,
    data2: 0xee97,
    data3: 0x4a90,
    data4: [0xb0, 0x76, 0x33, 0xf5, 0x7b, 0xf4, 0xea, 0xa7],
};

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(l as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
    }
    let sink = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Events;
    if !sink.is_null() {
        let event = match msg {
            WM_POWERBROADCAST => match w as u32 {
                PBT_APMSUSPEND => Some(Event::Sleep),
                PBT_APMRESUMEAUTOMATIC => Some(Event::Wake),
                PBT_APMRESUMESUSPEND => Some(Event::InteractiveWake),
                _ => None,
            },
            WM_DISPLAYCHANGE => Some(Event::Displays),
            WM_DEVICECHANGE
                if (w as u32 == DBT_DEVICEARRIVAL || w as u32 == DBT_DEVICEREMOVECOMPLETE)
                    && l != 0 =>
            {
                let header = &*(l as *const DEV_BROADCAST_HDR);
                if header.dbch_devicetype == DBT_DEVTYP_DEVICEINTERFACE
                    && header.dbch_size as usize >= size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>()
                {
                    let interface = &*(l as *const DEV_BROADCAST_DEVICEINTERFACE_W);
                    let guid = &interface.dbcc_classguid;
                    (guid.data1 == MONITOR.data1
                        && guid.data2 == MONITOR.data2
                        && guid.data3 == MONITOR.data3
                        && guid.data4 == MONITOR.data4)
                        .then_some(Event::Displays)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(event) = event {
            (*sink).emit(event);
        }
    }
    match msg {
        WM_POWERBROADCAST => 1,
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, msg, w, l)
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

impl Registration {
    pub fn install(events: Events) -> Result<Self> {
        let (send, receive) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("system-events".into())
            .spawn(move || unsafe {
                let class: Vec<u16> = "DeskflowDDCSystemEvents\0".encode_utf16().collect();
                let instance = GetModuleHandleW(null());
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(procedure),
                    hInstance: instance,
                    lpszClassName: class.as_ptr(),
                    ..std::mem::zeroed()
                };
                if RegisterClassW(&wc) == 0 {
                    let _ = send.send(Err(std::io::Error::last_os_error().to_string()));
                    return;
                }
                let mut sink = Box::new(events);
                // A hidden top-level window receives broadcast power/display events.
                // HWND_MESSAGE would miss those broadcasts. Never show or activate it.
                let hwnd = CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    class.as_ptr(),
                    class.as_ptr(),
                    WS_OVERLAPPED,
                    0,
                    0,
                    0,
                    0,
                    null_mut(),
                    null_mut(),
                    instance,
                    (&mut *sink as *mut Events).cast(),
                );
                if hwnd.is_null() {
                    let error = std::io::Error::last_os_error().to_string();
                    UnregisterClassW(class.as_ptr(), instance);
                    let _ = send.send(Err(error));
                    return;
                }
                let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
                    dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
                    dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE,
                    dbcc_reserved: 0,
                    dbcc_classguid: MONITOR,
                    dbcc_name: [0],
                };
                let notification = RegisterDeviceNotificationW(
                    hwnd,
                    (&filter as *const DEV_BROADCAST_DEVICEINTERFACE_W).cast(),
                    DEVICE_NOTIFY_WINDOW_HANDLE,
                );
                if notification.is_null() {
                    let error = std::io::Error::last_os_error().to_string();
                    DestroyWindow(hwnd);
                    UnregisterClassW(class.as_ptr(), instance);
                    let _ = send.send(Err(error));
                    return;
                }
                if send.send(Ok(hwnd as usize)).is_ok() {
                    let mut message: MSG = std::mem::zeroed();
                    while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                UnregisterDeviceNotification(notification);
                if IsWindow(hwnd) != 0 {
                    DestroyWindow(hwnd);
                }
                UnregisterClassW(class.as_ptr(), instance);
                drop(sink);
            })
            .context("无法创建 Windows 系统事件线程")?;
        let result = receive.recv().context("Windows 系统事件线程提前退出")?;
        match result {
            Ok(window) => {
                ensure!(window != 0, "Windows 系统事件窗口无效");
                Ok(Self {
                    window,
                    thread: Some(thread),
                })
            }
            Err(error) => {
                let _ = thread.join();
                anyhow::bail!("Windows 系统事件注册失败：{error}")
            }
        }
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        unsafe {
            PostMessageW(self.window as HWND, WM_CLOSE, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_window_handles_synthetic_messages_and_can_be_registered_again_after_drop() {
        // Messages go only to our hidden window. Never ask Windows to sleep,
        // change displays, or inject keyboard/mouse input.
        for _ in 0..3 {
            let events = Events::default();
            let registration = Registration::install(events.clone()).unwrap();
            let hwnd = registration.window as HWND;
            unsafe {
                SendMessageW(hwnd, WM_POWERBROADCAST, PBT_APMSUSPEND as usize, 0);
                assert!(events.state().sleeping);
                SendMessageW(hwnd, WM_DISPLAYCHANGE, 32, 0);
                assert!(events.state().sleeping);
                SendMessageW(hwnd, WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC as usize, 0);
                SendMessageW(hwnd, WM_POWERBROADCAST, PBT_APMRESUMESUSPEND as usize, 0);
                assert_eq!(events.state().wakes, 1);
                let mut device = DEV_BROADCAST_DEVICEINTERFACE_W {
                    dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
                    dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE,
                    dbcc_reserved: 0,
                    dbcc_classguid: MONITOR,
                    dbcc_name: [0],
                };
                SendMessageW(
                    hwnd,
                    WM_DEVICECHANGE,
                    DBT_DEVICEARRIVAL as usize,
                    (&device as *const _) as isize,
                );
                assert_eq!(events.state().displays, 2);
                device.dbcc_classguid.data1 = 0; // Ignore unrelated USB/device notifications.
                SendMessageW(
                    hwnd,
                    WM_DEVICECHANGE,
                    DBT_DEVICEARRIVAL as usize,
                    (&device as *const _) as isize,
                );
                assert_eq!(events.state().displays, 2);
                SendMessageW(hwnd, WM_POWERBROADCAST, 0x8013, 0); // Monitor power is not system sleep.
                assert_eq!((events.state().sleeps, events.state().wakes), (1, 1));
            }
            drop(registration);
        }
    }
}
