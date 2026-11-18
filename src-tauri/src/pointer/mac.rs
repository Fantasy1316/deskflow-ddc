use super::*;
use std::ffi::c_void;

#[repr(C)]
struct NativePoint {
    x: f64,
    y: f64,
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> NativePoint;
    // A legacy hint used only while awaiting native entry. If unavailable in
    // practice, the serialized local-ownership poll still repairs the cursor.
    #[cfg(not(test))]
    fn CGCursorIsVisible() -> u32; // boolean_t, not C _Bool
    #[cfg(not(test))]
    fn CGWarpMouseCursorPosition(point: NativePoint) -> i32;
    #[cfg(not(test))]
    fn CGEventSourceCreate(state: i32) -> *mut c_void;
    #[cfg(not(test))]
    fn CGEventSourceSetLocalEventsSuppressionInterval(source: *mut c_void, seconds: f64);
    #[cfg(not(test))]
    fn CGEventSourceSetLocalEventsFilterDuringSuppressionState(
        source: *mut c_void,
        filter: u32,
        state: u32,
    );
    #[cfg(not(test))]
    fn CGEventCreateMouseEvent(
        source: *const c_void,
        kind: u32,
        point: NativePoint,
        button: u32,
    ) -> *mut c_void;
    #[cfg(not(test))]
    fn CGEventPost(tap: u32, event: *mut c_void);
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
}
#[cfg(not(test))]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8; // Carbon Boolean
}

pub fn cursor() -> Result<Point> {
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        ensure!(!event.is_null(), "无法获取鼠标坐标");
        let p = CGEventGetLocation(event);
        CFRelease(event);
        Ok(Point { x: p.x, y: p.y })
    }
}

#[cfg(not(test))]
pub(crate) fn visible_cursor() -> Result<Option<Point>> {
    if unsafe { CGCursorIsVisible() } != 0 {
        cursor().map(Some)
    } else {
        Ok(None)
    }
}

#[cfg(not(test))]
pub(crate) fn point(point: Point) -> Result<()> {
    ensure!(point.x.is_finite() && point.y.is_finite(), "坐标无效");
    unsafe {
        ensure!(AXIsProcessTrusted() != 0, "需要辅助功能权限才能定位鼠标");
        let native = || NativePoint {
            x: point.x,
            y: point.y,
        };
        let source = CGEventSourceCreate(1); // HID state; preserve held modifiers.
        ensure!(!source.is_null(), "无法创建鼠标事件源");
        // This single movement must never suppress subsequent physical input.
        // Unlike a nil/default source, explicitly permit all local events in
        // both suppression states, with no post-event suppression interval.
        CGEventSourceSetLocalEventsSuppressionInterval(source, 0.0);
        CGEventSourceSetLocalEventsFilterDuringSuppressionState(source, 7, 0);
        CGEventSourceSetLocalEventsFilterDuringSuppressionState(source, 7, 1);
        // Create the movement notification first so allocation failure cannot
        // leave Deskflow's internal coordinates out of sync after a warp.
        let event = CGEventCreateMouseEvent(source, 5, native(), 0);
        CFRelease(source);
        ensure!(!event.is_null(), "无法创建鼠标位置事件");
        let result = CGWarpMouseCursorPosition(native());
        if result == 0 {
            // Same movement-only notification as mac-hotkey's pointer action.
            // No button, key or modifier state is changed.
            CGEventPost(0, event);
        }
        CFRelease(event);
        ensure!(result == 0, "无法定位鼠标：{result}");
    }
    Ok(())
}
