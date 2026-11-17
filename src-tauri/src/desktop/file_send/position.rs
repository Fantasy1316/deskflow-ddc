use super::*;
#[cfg(not(target_os = "macos"))]
use tauri::PhysicalPosition;

pub(super) fn place(
    app: &tauri::AppHandle,
    window: &WebviewWindow,
    panel_height: f64,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    return super::macos::place(app, window, panel_height);

    #[cfg(not(target_os = "macos"))]
    {
        let icon = app
            .tray_by_id("main-tray")
            .and_then(|t| t.rect().ok().flatten());
        let icon = icon.map(|r| tauri::PhysicalRect {
            position: r.position.to_physical::<i32>(1.0),
            size: r.size.to_physical::<u32>(1.0),
        });
        let monitor = icon
            .as_ref()
            .and_then(|r| {
                window
                    .monitor_from_point(r.position.x as f64, r.position.y as f64)
                    .ok()
                    .flatten()
            })
            .or_else(|| window.primary_monitor().ok().flatten())
            .ok_or_else(|| anyhow::anyhow!("无法读取托盘所在屏幕"))?;
        let scale = monitor.scale_factor();
        let width = (WIDTH * scale).round() as i32;
        let height = (panel_height * scale).round() as i32;
        let gap = (8.0 * scale).round() as i32;
        #[cfg(windows)]
        let point = display_source::file_panel_position(
            icon.map(|r| (r.position.x, r.position.y, r.size.width, r.size.height)),
            width,
            height,
            gap,
        )?;
        #[cfg(not(windows))]
        let point = {
            let work = monitor.work_area();
            (
                work.position.x + work.size.width as i32 - width - gap,
                work.position.y + work.size.height as i32 - height - gap,
            )
        };
        window.set_size(tauri::PhysicalSize::new(width as u32, height as u32))?;
        window.set_position(PhysicalPosition::new(point.0, point.1))?;
        Ok(())
    }
}
