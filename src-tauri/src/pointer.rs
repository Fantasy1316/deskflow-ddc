use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Panel {
    pub id: String,
    pub name: String,
    pub internal: bool,
    pub bounds: Bounds,
}
impl Bounds {
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.width && p.y >= self.y && p.y < self.y + self.height
    }
}
pub fn selected<'a>(panels: &'a [Panel], id: &str) -> Option<&'a Panel> {
    panels.iter().find(|p| {
        if id == "internal" {
            p.internal
        } else {
            p.id == id
        }
    })
}
pub fn seam(previous: Point, current: Point, b: &Bounds, side: &str) -> Option<f64> {
    if !b.contains(previous) {
        return None;
    }
    let horizontal = side == "left" || side == "right";
    let forward = side == "left" || side == "top";
    let (start, end, origin, size, cross_start, cross_end, cross_origin, cross_size) = if horizontal
    {
        (
            previous.x, current.x, b.x, b.width, previous.y, current.y, b.y, b.height,
        )
    } else {
        (
            previous.y, current.y, b.y, b.height, previous.x, current.x, b.x, b.width,
        )
    };
    let boundary = if forward {
        origin + size - 2.0
    } else {
        origin + 1.0
    };
    if if forward {
        end <= start || end < boundary
    } else {
        end >= start || end > boundary
    } {
        return None;
    }
    let t = ((boundary - start) / (end - start)).clamp(0.0, 1.0);
    let cross = cross_start + t * (cross_end - cross_start);
    if cross < cross_origin || cross >= cross_origin + cross_size {
        return None;
    }
    Some(((cross - cross_origin) / (cross_size - 1.0).max(1.0)).clamp(0.0, 1.0))
}
pub fn exit(panels: &[Panel], side: &str, position: f64) -> Result<Point> {
    ensure!(
        !panels.is_empty() && position.is_finite() && (0.0..=1.0).contains(&position),
        "跨屏位置无效"
    );
    let horizontal = side == "left" || side == "right";
    let forward = side == "left" || side == "top";
    let start = panels
        .iter()
        .map(|p| if horizontal { p.bounds.x } else { p.bounds.y })
        .fold(f64::INFINITY, f64::min);
    let end = panels
        .iter()
        .map(|p| {
            if horizontal {
                p.bounds.x + p.bounds.width
            } else {
                p.bounds.y + p.bounds.height
            }
        })
        .fold(f64::NEG_INFINITY, f64::max);
    let lo = panels
        .iter()
        .map(|p| if horizontal { p.bounds.y } else { p.bounds.x })
        .fold(f64::INFINITY, f64::min);
    let hi = panels
        .iter()
        .map(|p| {
            if horizontal {
                p.bounds.y + p.bounds.height
            } else {
                p.bounds.x + p.bounds.width
            }
        })
        .fold(f64::NEG_INFINITY, f64::max);
    let edge = if forward { end - 1.0 } else { start };
    let desired = lo + position * (hi - lo - 1.0).max(0.0);
    let cross = panels
        .iter()
        .filter_map(|p| {
            let b = &p.bounds;
            let (origin, size, cross, size_cross) = if horizontal {
                (b.x, b.width, b.y, b.height)
            } else {
                (b.y, b.height, b.x, b.width)
            };
            if edge < origin || edge >= origin + size {
                None
            } else {
                Some(desired.clamp(cross, cross + size_cross - 1.0))
            }
        })
        .min_by(|a, b| (a - desired).abs().total_cmp(&(b - desired).abs()))
        .ok_or_else(|| anyhow::anyhow!("找不到跨屏边缘"))?;
    Ok(if horizontal {
        Point { x: edge, y: cross }
    } else {
        Point { x: cross, y: edge }
    })
}
pub fn landing(b: &Bounds, current: Point, side: &str) -> Point {
    let margin = 24.0_f64.min(b.width / 4.0).min(b.height / 4.0);
    let mut p = Point {
        x: current.x.clamp(b.x + margin, b.x + b.width - margin),
        y: current.y.clamp(b.y + margin, b.y + b.height - margin),
    };
    match side {
        "left" => p.x = b.x + b.width - margin,
        "right" => p.x = b.x + margin,
        "top" => p.y = b.y + b.height - margin,
        "bottom" => p.y = b.y + margin,
        _ => {}
    }
    p
}
pub fn opposite(side: &str) -> &str {
    match side {
        "left" => "right",
        "right" => "left",
        "top" => "bottom",
        "bottom" => "top",
        _ => "off",
    }
}
#[cfg(target_os = "macos")]
pub fn cursor() -> Result<Point> {
    #[repr(C)]
    struct NativePoint {
        x: f64,
        y: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const std::ffi::c_void) -> *mut std::ffi::c_void;
        fn CGEventGetLocation(event: *mut std::ffi::c_void) -> NativePoint;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: *const std::ffi::c_void);
    }
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        ensure!(!event.is_null(), "无法获取鼠标坐标");
        let p = CGEventGetLocation(event);
        CFRelease(event);
        Ok(Point { x: p.x, y: p.y })
    }
}
#[cfg(target_os = "windows")]
pub fn cursor() -> Result<Point> {
    #[repr(C)]
    struct NativePoint {
        x: i32,
        y: i32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(p: *mut NativePoint) -> i32;
        fn SetThreadDpiAwarenessContext(context: isize) -> isize;
    }
    unsafe {
        let old = SetThreadDpiAwarenessContext(-4);
        let mut p = NativePoint { x: 0, y: 0 };
        let result = GetCursorPos(&mut p);
        if old != 0 {
            SetThreadDpiAwarenessContext(old);
        }
        ensure!(result != 0, "无法获取鼠标坐标");
        Ok(Point {
            x: p.x as f64,
            y: p.y as f64,
        })
    }
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn cursor() -> Result<Point> {
    anyhow::bail!("不支持的系统")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seam_preserves_fraction() {
        let b = Bounds {
            x: 0.0,
            y: 0.0,
            width: 1500.0,
            height: 1000.0,
        };
        let f = seam(
            Point {
                x: 1490.0,
                y: 300.0,
            },
            Point {
                x: 1550.0,
                y: 300.0,
            },
            &b,
            "left",
        )
        .unwrap();
        assert!((f - 300.0 / 999.0).abs() < 1e-9);
        assert!(seam(
            Point {
                x: 1500.0,
                y: 300.0
            },
            Point {
                x: 1550.0,
                y: 300.0
            },
            &b,
            "left"
        )
        .is_none());
    }
    #[test]
    fn escape_uses_real_aggregate_edge() {
        let panels = vec![
            Panel {
                id: "a".into(),
                name: "".into(),
                internal: true,
                bounds: Bounds {
                    x: 0.0,
                    y: 0.0,
                    width: 1500.0,
                    height: 1000.0,
                },
            },
            Panel {
                id: "b".into(),
                name: "".into(),
                internal: false,
                bounds: Bounds {
                    x: 1500.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1080.0,
                },
            },
        ];
        assert_eq!(
            exit(&panels, "left", 0.5).unwrap(),
            Point {
                x: 3419.0,
                y: 539.5
            }
        );
    }
}
