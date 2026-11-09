#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Everything is in physical screen pixels. The taskbar fixes the popup's
/// edge; a visible/overflow icon only refines its position along that edge.
pub(super) fn position(
    work: Rect,
    taskbar: Rect,
    icon: Option<Rect>,
    width: i32,
    height: i32,
    gap: i32,
) -> (i32, i32) {
    let horizontal = taskbar.right - taskbar.left >= taskbar.bottom - taskbar.top;
    let (x, y) = if horizontal {
        let center = icon.map_or(taskbar.right - gap, |r| r.left + (r.right - r.left) / 2);
        let top = taskbar.top + taskbar.bottom < work.top + work.bottom;
        (
            center - width / 2,
            if top {
                taskbar.bottom + gap
            } else {
                taskbar.top - height - gap
            },
        )
    } else {
        let center = icon.map_or(taskbar.bottom - gap, |r| r.top + (r.bottom - r.top) / 2);
        let left = taskbar.left + taskbar.right < work.left + work.right;
        (
            if left {
                taskbar.right + gap
            } else {
                taskbar.left - width - gap
            },
            center - height / 2,
        )
    };
    (
        x.clamp(work.left, (work.right - width).max(work.left)),
        y.clamp(work.top, (work.bottom - height).max(work.top)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }
    #[test]
    fn missing_or_overflow_icon_stays_above_bottom_taskbar_instead_of_near_cursor() {
        let work = rect(0, 0, 1920, 1040);
        let taskbar = rect(0, 1040, 1920, 1080);
        assert_eq!(position(work, taskbar, None, 360, 308, 8), (1560, 724));
        // Hidden icons may report a rectangle in the overflow panel, not the bar.
        let overflow = Some(rect(1710, 760, 1730, 780));
        assert_eq!(position(work, taskbar, overflow, 360, 308, 8), (1540, 724));
    }
    #[test]
    fn top_and_bottom_trays_use_their_actual_monitor_and_physical_scale() {
        let work = rect(-2560, 48, 0, 1440);
        let icon = Some(rect(-120, 0, -72, 48));
        assert_eq!(
            position(work, rect(-2560, 0, 0, 48), icon, 720, 616, 16),
            (-720, 64)
        );
        let bottom = rect(-2560, 1440, 0, 1488);
        assert_eq!(position(work, bottom, None, 720, 616, 16), (-720, 808));
    }
    #[test]
    fn side_taskbars_and_tiny_work_areas_keep_actions_reachable() {
        let work = rect(48, 0, 1920, 1080);
        let left = rect(0, 0, 48, 1080);
        assert_eq!(
            position(work, left, Some(rect(0, 800, 48, 848)), 360, 308, 8),
            (56, 670)
        );
        let right = rect(1920, 0, 1968, 1080);
        assert_eq!(position(work, right, None, 360, 308, 8), (1552, 772));
        assert_eq!(
            position(rect(0, 0, 300, 240), left, None, 360, 308, 8),
            (0, 0)
        );
    }
}
