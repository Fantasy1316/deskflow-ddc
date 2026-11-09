use super::geometry::{position, Rect as Bounds};
use super::*;
use std::{
    cell::Cell,
    mem::size_of,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED, WM_MOUSELEAVE},
        HiDpi::*,
        Input::KeyboardAndMouse::*,
        Shell::{
            DefSubclassProc, SHAppBarMessage, SetWindowSubclass, ABM_GETTASKBARPOS, APPBARDATA,
        },
        WindowsAndMessaging::*,
    },
};
#[path = "windows_paint.rs"]
mod paint;

const CLASS: &str = "DeskflowDDCSourcePopover";
const LOCAL: usize = 101;
const PEER: usize = 102;
const LATER: usize = 103;
const CLOSE: usize = 104;
const REPOSITION: u32 = WM_APP + 43;
const STYLE: u32 = WS_POPUP | WS_CLIPCHILDREN;
const EX_STYLE: u32 = WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
thread_local! { static CURRENT: Cell<HWND> = const { Cell::new(null_mut()) }; }

#[derive(Clone, Copy)]
enum FontKind {
    Body,
    Heading,
    Small,
}
struct Control {
    hwnd: HWND,
    rect: (i32, i32, i32, i32),
    font: FontKind,
    footer: bool,
}
struct Popup {
    owned_by_window: bool,
    ticket: u64,
    controls: Vec<Control>,
    fonts: [HFONT; 3],
    local: HWND,
    peer: HWND,
    status: HWND,
    hover: HWND,
    local_name: String,
    peer_name: String,
    error: bool,
    icon: Option<Bounds>,
    dpi: u32,
}
fn wide(s: &str) -> Vec<u16> {
    s.replace('\0', "�").encode_utf16().chain(Some(0)).collect()
}
fn px(value: i32, dpi: u32) -> i32 {
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}
unsafe fn data(hwnd: HWND) -> *mut Popup {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Popup
}
fn bounds(r: RECT) -> Bounds {
    Bounds {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}
fn native_rect(r: Bounds) -> RECT {
    RECT {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}
unsafe extern "system" fn child_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    parent: usize,
) -> LRESULT {
    let parent = parent as HWND;
    let state = data(parent);
    if msg == WM_SETCURSOR && IsWindowEnabled(hwnd) != 0 {
        SetCursor(LoadCursorW(null_mut(), IDC_HAND));
        return 1;
    }
    if msg == WM_MOUSEMOVE && !state.is_null() && (*state).hover != hwnd {
        let old = (*state).hover;
        (*state).hover = hwnd;
        if !old.is_null() {
            InvalidateRect(old, null(), 0);
        }
        InvalidateRect(hwnd, null(), 0);
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        TrackMouseEvent(&mut track);
    }
    if msg == WM_MOUSELEAVE && !state.is_null() && (*state).hover == hwnd {
        (*state).hover = null_mut();
        InvalidateRect(hwnd, null(), 0);
    }
    if msg == WM_KEYDOWN {
        if w == 0x1b {
            PostMessageW(parent, WM_CLOSE, 0, 0);
            return 0;
        }
        if w == 9 {
            let next = GetNextDlgTabItem(parent, hwnd, (GetKeyState(0x10) < 0) as i32);
            if !next.is_null() {
                SetFocus(next);
            }
            return 0;
        }
        // This is a regular top-level window, not a dialog message loop.
        if w == 13 && IsWindowEnabled(hwnd) != 0 {
            SendMessageW(hwnd, BM_CLICK, 0, 0);
            return 0;
        }
    }
    DefSubclassProc(hwnd, msg, w, l)
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let state = data(hwnd);
    match msg {
        WM_COMMAND if !state.is_null() && (w >> 16) == BN_CLICKED as usize => {
            let choice = match w & 0xffff {
                LOCAL => 1,
                PEER => 2,
                LATER | CLOSE => 0,
                _ => return 0,
            };
            event((*state).ticket, choice);
            return 0;
        }
        WM_CLOSE if !state.is_null() => {
            event((*state).ticket, 0);
            return 0;
        }
        WM_KEYDOWN if w == 0x1b && !state.is_null() => {
            event((*state).ticket, 0);
            return 0;
        }
        WM_DRAWITEM if !state.is_null() => {
            paint::button(&*state, &*(l as *const DRAWITEMSTRUCT));
            return 1;
        }
        WM_CTLCOLORSTATIC if !state.is_null() => {
            let dc = w as HDC;
            SetBkMode(dc, TRANSPARENT as i32);
            let child = l as HWND;
            let heading = (*state)
                .controls
                .iter()
                .any(|c| c.hwnd == child && matches!(c.font, FontKind::Heading));
            SetTextColor(
                dc,
                if child == (*state).status && (*state).error {
                    paint::error_color()
                } else if heading {
                    GetSysColor(COLOR_WINDOWTEXT)
                } else {
                    paint::muted()
                },
            );
            return GetSysColorBrush(COLOR_WINDOW) as LRESULT;
        }
        WM_PAINT if !state.is_null() => {
            paint::background(hwnd, &*state);
            return 0;
        }
        WM_DPICHANGED if !state.is_null() => {
            layout(hwnd, (w & 0xffff) as u32);
            PostMessageW(hwnd, REPOSITION, 0, 0);
            return 0;
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE if !state.is_null() => {
            PostMessageW(hwnd, REPOSITION, 0, 0);
        }
        REPOSITION if !state.is_null() => {
            let _ = place(hwnd, false);
            return 0;
        }
        WM_NCDESTROY if !state.is_null() => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !(*state).owned_by_window {
                return DefWindowProcW(hwnd, msg, w, l);
            }
            let state = Box::from_raw(state);
            for f in state.fonts {
                if !f.is_null() {
                    DeleteObject(f);
                }
            }
            CURRENT.with(|c| {
                if c.get() == hwnd {
                    c.set(null_mut());
                }
            });
        }
        // No dismissal on activation/focus loss. All close paths are explicit.
        _ => {}
    }
    DefWindowProcW(hwnd, msg, w, l)
}
unsafe fn font(dpi: u32, size: i32, bold: bool) -> HFONT {
    CreateFontW(
        -px(size, dpi),
        0,
        0,
        0,
        if bold {
            FW_SEMIBOLD as i32
        } else {
            FW_NORMAL as i32
        },
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        DEFAULT_PITCH as u32,
        wide("Segoe UI").as_ptr(),
    )
}
unsafe fn layout(hwnd: HWND, dpi: u32) {
    let state = data(hwnd);
    if state.is_null() {
        return;
    }
    let dpi = dpi.max(96);
    if (*state).dpi != dpi || (*state).fonts[0].is_null() {
        let old = (*state).fonts;
        (*state).fonts = [
            font(dpi, 13, false),
            font(dpi, 17, true),
            font(dpi, 11, false),
        ];
        (*state).dpi = dpi;
        for c in &(*state).controls {
            let i = match c.font {
                FontKind::Body => 0,
                FontKind::Heading => 1,
                FontKind::Small => 2,
            };
            SendMessageW(c.hwnd, WM_SETFONT, (*state).fonts[i] as usize, 1);
        }
        for f in old {
            if !f.is_null() {
                DeleteObject(f);
            }
        }
    }
    let extra = if (*state).error { 26 } else { 0 };
    for c in &(*state).controls {
        let (x, y, width, height) = c.rect;
        MoveWindow(
            c.hwnd,
            px(x, dpi),
            px(y + if c.footer { extra } else { 0 }, dpi),
            px(width, dpi),
            px(
                height + if c.hwnd == (*state).status { extra } else { 0 },
                dpi,
            ),
            1,
        );
    }
    InvalidateRect(hwnd, null(), 1);
}
unsafe fn control(
    parent: HWND,
    class: &str,
    text: &str,
    id: usize,
    rect: (i32, i32, i32, i32),
    font: FontKind,
    footer: bool,
) -> Result<HWND> {
    let button = class == "BUTTON";
    let hwnd = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(text).as_ptr(),
        WS_CHILD
            | WS_VISIBLE
            | if button {
                WS_TABSTOP | BS_OWNERDRAW as u32
            } else {
                0x80 /* SS_NOPREFIX */
            },
        0,
        0,
        0,
        0,
        parent,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    ensure!(
        !hwnd.is_null(),
        "无法创建原生确认控件：{}",
        std::io::Error::last_os_error()
    );
    (*data(parent)).controls.push(Control {
        hwnd,
        rect,
        font,
        footer,
    });
    if button {
        ensure!(
            SetWindowSubclass(hwnd, Some(child_proc), 1, parent as usize) != 0,
            "无法初始化原生键盘导航"
        );
    }
    Ok(hwnd)
}
unsafe fn create(owner: HWND, x: i32, y: i32, ticket: u64) -> Result<HWND> {
    let class = wide(CLASS);
    let wc = WNDCLASSW {
        style: CS_DROPSHADOW,
        lpfnWndProc: Some(procedure),
        hInstance: GetModuleHandleW(null()),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: GetSysColorBrush(COLOR_WINDOW),
        lpszClassName: class.as_ptr(),
        ..std::mem::zeroed()
    };
    ensure!(
        RegisterClassW(&wc) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS,
        "无法注册原生浮窗类"
    );
    let mut allocation = Box::new(Popup {
        owned_by_window: false,
        ticket,
        controls: vec![],
        fonts: [null_mut(); 3],
        local: null_mut(),
        peer: null_mut(),
        status: null_mut(),
        hover: null_mut(),
        local_name: String::new(),
        peer_name: String::new(),
        error: false,
        icon: None,
        dpi: 96,
    });
    let state: *mut Popup = &mut *allocation;
    let hwnd = CreateWindowExW(
        EX_STYLE,
        class.as_ptr(),
        wide("确认当前画面 · Deskflow DDC").as_ptr(),
        STYLE,
        x,
        y,
        360,
        308,
        owner,
        null_mut(),
        wc.hInstance,
        state.cast(),
    );
    ensure!(
        !hwnd.is_null(),
        "无法创建原生托盘浮窗：{}",
        std::io::Error::last_os_error()
    );
    (*state).owned_by_window = true;
    let _ = Box::into_raw(allocation);
    CURRENT.with(|c| c.set(hwnd));
    let built = (|| -> Result<()> {
        control(
            hwnd,
            "STATIC",
            "Deskflow DDC",
            0,
            (20, 16, 270, 18),
            FontKind::Small,
            false,
        )?;
        control(
            hwnd,
            "STATIC",
            "共用显示器正显示哪台？",
            0,
            (20, 42, 320, 24),
            FontKind::Heading,
            false,
        )?;
        control(
            hwnd,
            "STATIC",
            "请按实际画面选择，让下一次切换方向正确。",
            0,
            (20, 72, 320, 26),
            FontKind::Body,
            false,
        )?;
        (*state).local = control(
            hwnd,
            "BUTTON",
            "本机",
            LOCAL,
            (20, 104, 320, 50),
            FontKind::Body,
            false,
        )?;
        (*state).peer = control(
            hwnd,
            "BUTTON",
            "另一台",
            PEER,
            (20, 164, 320, 50),
            FontKind::Body,
            false,
        )?;
        (*state).status = control(
            hwnd,
            "STATIC",
            "",
            0,
            (20, 226, 320, 18),
            FontKind::Small,
            false,
        )?;
        control(
            hwnd,
            "STATIC",
            "未确认，下次切换仍会提醒",
            0,
            (20, 269, 216, 16),
            FontKind::Small,
            true,
        )?;
        control(
            hwnd,
            "BUTTON",
            "稍后确认",
            LATER,
            (244, 260, 96, 32),
            FontKind::Body,
            true,
        )?;
        control(
            hwnd,
            "BUTTON",
            "关闭提醒",
            CLOSE,
            (318, 9, 28, 28),
            FontKind::Body,
            false,
        )?;
        layout(hwnd, GetDpiForWindow(hwnd));
        Ok(())
    })();
    if let Err(error) = built {
        DestroyWindow(hwnd);
        return Err(error);
    }
    Ok(hwnd)
}

struct TaskbarSearch {
    monitor: HMONITOR,
    found: Option<RECT>,
}
unsafe extern "system" fn find_taskbar(hwnd: HWND, raw: LPARAM) -> i32 {
    let search = &mut *(raw as *mut TaskbarSearch);
    let mut class = [0u16; 64];
    let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
    let class = String::from_utf16_lossy(&class[..len.max(0) as usize]);
    if (class == "Shell_TrayWnd" || class == "Shell_SecondaryTrayWnd")
        && MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) == search.monitor
    {
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r) != 0 && r.right > r.left && r.bottom > r.top {
            search.found = Some(r);
            return 0;
        }
    }
    1
}
unsafe fn taskbar(icon: Option<Bounds>) -> Result<(MONITORINFO, RECT)> {
    let primary = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
    let monitor = icon.map_or_else(
        || MonitorFromWindow(primary, MONITOR_DEFAULTTOPRIMARY),
        |r| MonitorFromRect(&native_rect(r), MONITOR_DEFAULTTONEAREST),
    );
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..std::mem::zeroed()
    };
    ensure!(
        GetMonitorInfoW(monitor, &mut info) != 0,
        "无法读取托盘所在屏幕"
    );
    let mut search = TaskbarSearch {
        monitor,
        found: None,
    };
    EnumWindows(Some(find_taskbar), &mut search as *mut _ as LPARAM);
    if let Some(bar) = search.found {
        return Ok((info, bar));
    }
    let mut appbar = APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        ..std::mem::zeroed()
    };
    if SHAppBarMessage(ABM_GETTASKBARPOS, &mut appbar) != 0
        && MonitorFromRect(&appbar.rc, MONITOR_DEFAULTTONEAREST) == monitor
    {
        return Ok((info, appbar.rc));
    }
    // Explorer may be restarting. Use this monitor's reserved work-area edge,
    // never the mouse position (which can be in the middle of any display).
    let (m, w) = (info.rcMonitor, info.rcWork);
    let bar = if w.left > m.left {
        RECT { right: w.left, ..m }
    } else if w.top > m.top {
        RECT { bottom: w.top, ..m }
    } else if w.right < m.right {
        RECT { left: w.right, ..m }
    } else {
        RECT {
            top: w.bottom,
            bottom: m.bottom.max(w.bottom + 1),
            ..m
        }
    };
    Ok((info, bar))
}
unsafe fn place(hwnd: HWND, activate: bool) -> Result<()> {
    let state = data(hwnd);
    if state.is_null() {
        return Ok(());
    }
    let (info, bar) = taskbar((*state).icon)?;
    let dpi = GetDpiForWindow(hwnd).max(96);
    layout(hwnd, dpi);
    let (width, height) = (
        px(360, dpi),
        px(308 + if (*state).error { 26 } else { 0 }, dpi),
    );
    let (x, y) = position(
        bounds(info.rcWork),
        bounds(bar),
        (*state).icon,
        width,
        height,
        px(10, dpi),
    );
    ensure!(
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_SHOWWINDOW | SWP_NOACTIVATE
        ) != 0,
        "无法定位原生托盘浮窗：{}",
        std::io::Error::last_os_error()
    );
    let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, px(16, dpi), px(16, dpi));
    // On success the system owns the region; on failure this scope owns it.
    if !region.is_null() && SetWindowRgn(hwnd, region, 1) == 0 {
        DeleteObject(region);
    }
    if activate {
        SetForegroundWindow(hwnd);
        SetFocus((*state).local);
    }
    Ok(())
}
pub(super) fn show(app: &tauri::AppHandle, view: View) -> Result<()> {
    let tray = app
        .tray_by_id("main-tray")
        .ok_or_else(|| anyhow::anyhow!("托盘图标不存在"))?;
    tray.with_inner_tray_icon(move |tray| -> Result<()> {
        unsafe {
            let icon = tray
                .rect()
                .filter(|r| r.size.width > 0 && r.size.height > 0)
                .map(|r| Bounds {
                    left: r.position.x as i32,
                    top: r.position.y as i32,
                    right: (r.position.x + r.size.width as f64) as i32,
                    bottom: (r.position.y + r.size.height as f64) as i32,
                });
            let (_, bar) = taskbar(icon)?;
            let old = CURRENT.with(Cell::get);
            let existing =
                !old.is_null() && !data(old).is_null() && (*data(old)).ticket == view.ticket;
            if !old.is_null() && !existing {
                DestroyWindow(old);
            }
            let hwnd = if existing {
                old
            } else {
                create(tray.window_handle() as HWND, bar.left, bar.top, view.ticket)?
            };
            let state = data(hwnd);
            (*state).local_name = view.local_name;
            (*state).peer_name = view.peer_name;
            (*state).error = view.error.is_some();
            (*state).icon = icon;
            SetWindowTextW(
                (*state).local,
                wide(&format!("本机：{}", (*state).local_name)).as_ptr(),
            );
            SetWindowTextW(
                (*state).peer,
                wide(&format!("另一台：{}", (*state).peer_name)).as_ptr(),
            );
            EnableWindow((*state).local, !view.busy as i32);
            EnableWindow((*state).peer, !view.busy as i32);
            let message = view.error.as_deref().unwrap_or(if view.busy {
                "正在同步画面来源…"
            } else {
                "仅确认来源，不切屏或转移键鼠。"
            });
            SetWindowTextW((*state).status, wide(message).as_ptr());
            place(hwnd, !existing)
        }
    })?
}
pub(super) fn hide(ticket: u64) {
    unsafe {
        let hwnd = CURRENT.with(Cell::get);
        if !hwnd.is_null() && !data(hwnd).is_null() && (*data(hwnd)).ticket == ticket {
            DestroyWindow(hwnd);
        }
    }
}
