//! Returning to a visible independent screen must not wait for a helper process
//! or overwrite movement made since Deskflow's native entry.
use super::*;
use std::time::Duration;

fn desktop(panels: &[Panel]) -> Result<Bounds> {
    ensure!(!panels.is_empty(), "没有可用屏幕");
    let mut bounds = panels[0].bounds.clone();
    for panel in &panels[1..] {
        let b = &panel.bounds;
        let right = (bounds.x + bounds.width).max(b.x + b.width);
        let bottom = (bounds.y + bounds.height).max(b.y + b.height);
        bounds.x = bounds.x.min(b.x);
        bounds.y = bounds.y.min(b.y);
        bounds.width = right - bounds.x;
        bounds.height = bottom - bounds.y;
    }
    ensure!(
        [bounds.x, bounds.y, bounds.width, bounds.height]
            .iter()
            .all(|v| v.is_finite())
            && bounds.width > 0.0
            && bounds.height > 0.0,
        "屏幕尺寸无效"
    );
    Ok(bounds)
}

pub(crate) fn target(
    panels: &[Panel],
    retained: &Bounds,
    side: &str,
    entry: Option<Point>,
    current: Point,
) -> Result<Option<Point>> {
    ensure!(
        matches!(side, "left" | "right" | "top" | "bottom")
            && [current.x, current.y].iter().all(|v| v.is_finite())
            && entry.is_none_or(|p| p.x.is_finite() && p.y.is_finite()),
        "跨屏位置无效"
    );
    // A user who has already reached the visible screen owns the position. Do
    // not pull them back to the edge, even if the transition log arrived late.
    if retained.contains(current) {
        return Ok(None);
    }
    let all = desktop(panels)?;
    let anchor = entry.unwrap_or(current);
    let horizontal = matches!(side, "left" | "right");
    let (cross, origin, span, start, size, delta) = if horizontal {
        (
            anchor.y,
            all.y,
            all.height,
            retained.y,
            retained.height,
            current.y - anchor.y,
        )
    } else {
        (
            anchor.x,
            all.x,
            all.width,
            retained.x,
            retained.width,
            current.x - anchor.x,
        )
    };
    // Deskflow maps the perpendicular axis across the aggregate desktop. Map
    // that fraction to the retained screen, then keep subsequent motion in
    // pixels (do not rescale the user's ongoing mouse speed).
    let cross = start
        + ((cross - origin) / (span - 1.0).max(1.0)).clamp(0.0, 1.0) * (size - 1.0).max(0.0)
        + delta;
    let inset = 2.0_f64.min(retained.width / 4.0).min(retained.height / 4.0);
    let point = match side {
        "left" => Point {
            x: retained.x + retained.width - inset - (all.x + all.width - 1.0 - current.x).max(0.0),
            y: cross,
        },
        "right" => Point {
            x: retained.x + inset + (current.x - all.x).max(0.0),
            y: cross,
        },
        "top" => Point {
            x: cross,
            y: retained.y + retained.height
                - inset
                - (all.y + all.height - 1.0 - current.y).max(0.0),
        },
        "bottom" => Point {
            x: cross,
            y: retained.y + inset + (current.y - all.y).max(0.0),
        },
        _ => unreachable!(),
    };
    Ok(Some(Point {
        x: point
            .x
            .clamp(retained.x + inset, retained.x + retained.width - inset),
        y: point
            .y
            .clamp(retained.y + inset, retained.y + retained.height - inset),
    }))
}

/// The engine prints its transition before its event thread performs the warp.
/// Wait for that placement instead of sleeping a fixed 80ms or racing the warp.
/// Defer hidden or not-yet-placed cursors. Abandon a pending correction after a
/// short deadline; the serialized local-ownership poll can recover a late
/// native entry, without a delayed warp stealing subsequent input.
pub(crate) async fn arrival(
    panels: &[Panel],
    retained: &Bounds,
    side: &str,
    entry: Option<Point>,
    mut sample: impl FnMut() -> Result<Option<Point>>,
    valid: impl Fn() -> bool,
) -> Result<Option<Point>> {
    if entry.is_some_and(|p| retained.contains(p)) {
        return Ok(None);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_millis(40);
    let mut previous = None;
    let mut initial = None;
    loop {
        ensure!(valid(), "共享会话已结束");
        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        if let Some(current) = sample()? {
            let Some(entry) = entry else {
                return target(panels, retained, side, None, current);
            };
            let first = *initial.get_or_insert(current);
            let near = (current.x - entry.x).abs() < 96.0 && (current.y - entry.y).abs() < 96.0;
            // Require a second visible sample: showCursor() may happen before
            // the event thread's queued warp. Movement after the first sample
            // also proves the cursor is no longer parked on the provider.
            if previous.is_some() && (near || current != first) {
                return target(panels, retained, side, Some(entry), current);
            }
            previous = Some(current);
        } else {
            previous = None;
            if entry.is_none() {
                return Ok(None);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(id: &str, internal: bool, x: f64, y: f64, width: f64, height: f64) -> Panel {
        Panel {
            id: id.into(),
            name: id.into(),
            internal,
            bounds: Bounds {
                x,
                y,
                width,
                height,
            },
        }
    }
    fn topology() -> Vec<Panel> {
        vec![
            panel("1", true, 0.0, 0.0, 1512.0, 982.0),
            panel("3", false, 1512.0, 80.0, 1920.0, 1080.0),
        ]
    }

    #[test]
    fn return_maps_aggregate_fraction_and_preserves_motion_after_native_entry() {
        let panels = topology();
        let b = &panels[0].bounds;
        let entry = Point {
            x: 3426.0,
            y: 600.0,
        };
        let current = Point {
            x: 3406.0,
            y: 610.0,
        };
        let point = target(&panels, b, "left", Some(entry), current)
            .unwrap()
            .unwrap();
        // Preserve both native inward overshoot and twenty additional pixels.
        assert_eq!(point.x, 1510.0 - 25.0);
        assert!((point.y - (600.0 / 1159.0 * 981.0 + 10.0)).abs() < 1e-9);
        let fallback = target(&panels, b, "left", None, entry).unwrap().unwrap();
        assert_eq!(fallback.x, 1505.0);
        assert!((fallback.y - 600.0 / 1159.0 * 981.0).abs() < 1e-9);
    }

    #[test]
    fn all_four_directions_support_offsets_unequal_sizes_and_corners() {
        let panels = vec![
            panel("retained", true, -800.0, -400.0, 800.0, 600.0),
            panel("shared", false, 0.0, 200.0, 1920.0, 1080.0),
        ];
        let b = &panels[0].bounds;
        for (side, entry, x, y) in [
            (
                "left",
                Point {
                    x: 1919.0,
                    y: 439.5,
                },
                -2.0,
                -100.5,
            ),
            (
                "right",
                Point {
                    x: -800.0,
                    y: 439.5,
                },
                -798.0,
                -100.5,
            ),
            (
                "top",
                Point {
                    x: 559.5,
                    y: 1279.0,
                },
                -400.5,
                198.0,
            ),
            (
                "bottom",
                Point {
                    x: 559.5,
                    y: -400.0,
                },
                -400.5,
                -398.0,
            ),
        ] {
            assert_eq!(
                target(&panels, b, side, Some(entry), entry).unwrap(),
                Some(Point { x, y }),
                "{side}"
            );
        }
        for side in ["left", "right", "top", "bottom"] {
            for p in [
                Point {
                    x: 1919.0,
                    y: 1279.0,
                },
                Point {
                    x: -800.0,
                    y: -400.0,
                },
            ] {
                if let Some(point) = target(&panels, b, side, Some(p), p).unwrap() {
                    assert!(b.contains(point), "{side} {point:?}");
                }
            }
        }
        assert!(target(&[], b, "left", None, Point { x: 10.0, y: 10.0 }).is_err());
        assert!(target(&panels, b, "off", None, Point { x: 10.0, y: 10.0 }).is_err());
        assert!(target(
            &panels,
            b,
            "left",
            Some(Point {
                x: f64::NAN,
                y: 0.0
            }),
            Point { x: 10.0, y: 10.0 }
        )
        .is_err());
    }

    #[tokio::test]
    async fn waits_for_native_entry_instead_of_warping_a_hidden_or_parked_cursor() {
        let panels = topology();
        let entry = Point {
            x: 3426.0,
            y: 600.0,
        };
        let parked = Point {
            x: 3431.0,
            y: 1000.0,
        };
        let mut samples = [
            None,
            Some(parked),
            Some(parked),
            Some(entry),
            Some(Point {
                x: 3420.0,
                y: 603.0,
            }),
        ]
        .into_iter();
        let point = arrival(
            &panels,
            &panels[0].bounds,
            "left",
            Some(entry),
            || Ok(samples.next().unwrap()),
            || true,
        )
        .await
        .unwrap()
        .unwrap();
        // Correct as soon as native placement is observed, without waiting
        // for another user movement. Runtime re-samples just before the warp.
        assert_eq!(point.x, 1505.0);
        assert!((point.y - 600.0 / 1159.0 * 981.0).abs() < 1e-9);
        assert_eq!(
            samples.next().unwrap(),
            Some(Point {
                x: 3420.0,
                y: 603.0
            })
        );
    }

    #[tokio::test]
    async fn visible_user_motion_is_not_pulled_back_and_stale_session_never_moves() {
        let panels = topology();
        let entry = Point {
            x: 3426.0,
            y: 600.0,
        };
        let visible = Point {
            x: 1400.0,
            y: 510.0,
        };
        let mut samples = [Some(entry), Some(visible)].into_iter();
        assert_eq!(
            arrival(
                &panels,
                &panels[0].bounds,
                "left",
                Some(entry),
                || Ok(samples.next().unwrap()),
                || true
            )
            .await
            .unwrap(),
            None
        );
        assert!(arrival(
            &panels,
            &panels[0].bounds,
            "left",
            Some(entry),
            || panic!("stale session must not read input"),
            || false
        )
        .await
        .is_err());
        assert_eq!(
            arrival(
                &panels,
                &panels[0].bounds,
                "left",
                None,
                || Ok(None),
                || true
            )
            .await
            .unwrap(),
            None
        );
        assert_eq!(
            arrival(
                &panels,
                &panels[0].bounds,
                "left",
                Some(entry),
                || Ok(None),
                || true
            )
            .await
            .unwrap(),
            None
        );
        assert_eq!(
            arrival(
                &panels,
                &panels[0].bounds,
                "left",
                Some(visible),
                || panic!("native entry already on retained screen"),
                || true
            )
            .await
            .unwrap(),
            None
        );
    }
}
