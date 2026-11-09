// Owner-drawn Win32 BUTTONs retain native accessibility, focus and keyboard
// activation. Only their appearance is customized; no webview or input hooks.
use super::*;

fn mix(a: COLORREF, b: COLORREF, percent_b: u32) -> COLORREF {
    let channel = |shift: u32| {
        (((a >> shift) & 255u32) * (100 - percent_b) + ((b >> shift) & 255u32) * percent_b) / 100
    };
    channel(0) | channel(8) << 8 | channel(16) << 16
}
pub(super) unsafe fn muted() -> COLORREF {
    mix(GetSysColor(COLOR_WINDOW), GetSysColor(COLOR_WINDOWTEXT), 64)
}
pub(super) fn error_color() -> COLORREF {
    0x002d2dc4
}
unsafe fn rounded(dc: HDC, rect: RECT, background: COLORREF, border: COLORREF, radius: i32) {
    let brush = CreateSolidBrush(background);
    let pen = CreatePen(PS_SOLID, 1, border);
    let old_brush = SelectObject(dc, brush);
    let old_pen = SelectObject(dc, pen);
    RoundRect(
        dc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        radius,
        radius,
    );
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(brush);
    DeleteObject(pen);
}
unsafe fn text(dc: HDC, label: &str, mut rect: RECT, font: HFONT, color: COLORREF, align: u32) {
    SelectObject(dc, font);
    SetTextColor(dc, color);
    SetBkMode(dc, TRANSPARENT as i32);
    let label = wide(label);
    DrawTextW(
        dc,
        label.as_ptr(),
        (label.len() - 1) as i32,
        &mut rect,
        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX | align,
    );
}
pub(super) unsafe fn background(hwnd: HWND, state: &Popup) {
    let mut ps = PAINTSTRUCT::default();
    let dc = BeginPaint(hwnd, &mut ps);
    let saved = SaveDC(dc);
    let mut rect = RECT::default();
    GetClientRect(hwnd, &mut rect);
    let bg = GetSysColor(COLOR_WINDOW);
    FillRect(dc, &rect, GetSysColorBrush(COLOR_WINDOW));
    rounded(
        dc,
        RECT {
            right: rect.right - 1,
            bottom: rect.bottom - 1,
            ..rect
        },
        bg,
        mix(bg, GetSysColor(COLOR_WINDOWTEXT), 16),
        px(16, state.dpi),
    );
    let y = px(250 + if state.error { 26 } else { 0 }, state.dpi);
    let pen = CreatePen(PS_SOLID, 1, mix(bg, GetSysColor(COLOR_WINDOWTEXT), 12));
    SelectObject(dc, pen);
    MoveToEx(dc, px(20, state.dpi), y, null_mut());
    LineTo(dc, rect.right - px(20, state.dpi), y);
    RestoreDC(dc, saved);
    DeleteObject(pen);
    EndPaint(hwnd, &ps);
}
pub(super) unsafe fn button(state: &Popup, draw: &DRAWITEMSTRUCT) {
    let dc = draw.hDC;
    let saved = SaveDC(dc);
    let dpi = state.dpi;
    let r = draw.rcItem;
    let bg = GetSysColor(COLOR_WINDOW);
    let fg = GetSysColor(COLOR_WINDOWTEXT);
    let disabled = draw.itemState & ODS_DISABLED != 0;
    let focused = draw.itemState & ODS_FOCUS != 0;
    let hover = state.hover == draw.hwndItem && !disabled;
    let pressed = draw.itemState & ODS_SELECTED != 0;
    FillRect(dc, &r, GetSysColorBrush(COLOR_WINDOW));
    let fill = mix(
        bg,
        fg,
        if pressed {
            12
        } else if hover {
            8
        } else {
            4
        },
    );
    let border = if focused {
        GetSysColor(COLOR_HIGHLIGHT)
    } else {
        mix(bg, fg, if hover { 22 } else { 12 })
    };
    let color = if disabled { muted() } else { fg };
    let inner = RECT {
        left: r.left + 1,
        top: r.top + 1,
        right: r.right - 1,
        bottom: r.bottom - 1,
    };
    if draw.CtlID as usize == CLOSE {
        if hover || focused {
            rounded(dc, inner, fill, border, px(8, dpi));
        }
        let pen = CreatePen(PS_SOLID, px(1, dpi).max(1), muted());
        SelectObject(dc, pen);
        let (x, y) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
        let d = px(4, dpi);
        MoveToEx(dc, x - d, y - d, null_mut());
        LineTo(dc, x + d + 1, y + d + 1);
        MoveToEx(dc, x + d, y - d, null_mut());
        LineTo(dc, x - d - 1, y + d + 1);
        RestoreDC(dc, saved);
        DeleteObject(pen);
        return;
    }
    rounded(dc, inner, fill, border, px(12, dpi));
    if draw.CtlID as usize == LOCAL || draw.CtlID as usize == PEER {
        let local = draw.CtlID as usize == LOCAL;
        let name = if local {
            &state.local_name
        } else {
            &state.peer_name
        };
        text(
            dc,
            if local { "本机" } else { "另一台" },
            RECT {
                left: px(48, dpi),
                top: px(5, dpi),
                right: r.right - px(14, dpi),
                bottom: px(22, dpi),
            },
            state.fonts[2],
            muted(),
            DT_LEFT,
        );
        text(
            dc,
            name,
            RECT {
                left: px(48, dpi),
                top: px(22, dpi),
                right: r.right - px(14, dpi),
                bottom: r.bottom - px(6, dpi),
            },
            state.fonts[0],
            color,
            DT_LEFT,
        );
        let pen = CreatePen(PS_SOLID, px(1, dpi).max(1), muted());
        SelectObject(dc, pen);
        SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
        let (x, y) = (px(14, dpi), px(16, dpi));
        Rectangle(dc, x, y, x + px(21, dpi), y + px(14, dpi));
        if local {
            MoveToEx(dc, x - px(3, dpi), y + px(17, dpi), null_mut());
            LineTo(dc, x + px(24, dpi), y + px(17, dpi));
        } else {
            MoveToEx(dc, x + px(10, dpi), y + px(14, dpi), null_mut());
            LineTo(dc, x + px(10, dpi), y + px(19, dpi));
            MoveToEx(dc, x + px(4, dpi), y + px(19, dpi), null_mut());
            LineTo(dc, x + px(17, dpi), y + px(19, dpi));
        }
        RestoreDC(dc, saved);
        DeleteObject(pen);
        return;
    }
    text(dc, "稍后确认", r, state.fonts[0], color, DT_CENTER);
    RestoreDC(dc, saved);
}
