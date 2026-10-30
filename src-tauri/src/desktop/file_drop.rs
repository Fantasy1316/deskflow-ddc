//! Only OS-provided drag paths are eligible for sending. The renderer receives
//! a short-lived, single-use token instead of authority to submit raw paths.
use super::*;
use serde_json::Value;
use std::{path::PathBuf, sync::Mutex, time::Instant};
use uuid::Uuid;
const TTL: Duration = Duration::from_secs(30);
struct Pending {
    token: Uuid,
    paths: Vec<PathBuf>,
    created: Instant,
}
#[derive(Default)]
pub(super) struct FileDrops(Mutex<Option<Pending>>);
impl FileDrops {
    fn register(&self, paths: Vec<PathBuf>) -> Result<Uuid> {
        let mut pending = self.0.lock().unwrap();
        *pending = None;
        ensure!(
            !paths.is_empty() && paths.len() <= 256,
            "一次最多拖入 256 个文件或目录"
        );
        ensure!(
            paths.iter().all(|p| p.is_absolute())
                && paths.iter().map(|p| p.as_os_str().len()).sum::<usize>() <= 1024 * 1024,
            "拖入路径无效或过长"
        );
        let token = Uuid::new_v4();
        *pending = Some(Pending {
            token,
            paths,
            created: Instant::now(),
        });
        Ok(token)
    }
    pub(super) fn take(&self, token: Uuid) -> Result<Vec<PathBuf>> {
        let mut pending = self.0.lock().unwrap();
        ensure!(
            pending.as_ref().is_some_and(|p| p.token == token),
            "拖入内容已失效，请重新拖入"
        );
        let drop = pending.take().unwrap();
        ensure!(drop.created.elapsed() < TTL, "拖入内容已过期，请重新拖入");
        Ok(drop.paths)
    }
}
fn logical_position(position: &tauri::PhysicalPosition<f64>, scale: f64, macos: bool) -> Value {
    // Wry's AppKit bridge supplies points, even though Tauri wraps them in
    // PhysicalPosition. Windows supplies pixels. Normalize before DOM hit tests.
    let divisor = if macos || !scale.is_finite() || scale <= 0.0 {
        1.0
    } else {
        scale
    };
    json!({"x":position.x / divisor,"y":position.y / divisor})
}
pub(super) fn handle(app: &tauri::AppHandle, event: &tauri::DragDropEvent) {
    let Some(drops) = app.try_state::<FileDrops>() else {
        return;
    };
    let scale = app
        .get_webview_window("main")
        .and_then(|w| w.scale_factor().ok())
        .unwrap_or(1.0);
    let position =
        |p: &tauri::PhysicalPosition<f64>| logical_position(p, scale, cfg!(target_os = "macos"));
    let payload = match event {
        tauri::DragDropEvent::Enter { position: p, .. } => {
            *drops.0.lock().unwrap() = None;
            json!({"kind":"enter","position":position(p)})
        }
        tauri::DragDropEvent::Over { position: p } => {
            json!({"kind":"over","position":position(p)})
        }
        tauri::DragDropEvent::Leave => json!({"kind":"leave"}),
        tauri::DragDropEvent::Drop { paths, position: p } => match drops.register(paths.clone()) {
            Ok(token) => json!({"kind":"drop","position":position(p),"token":token}),
            Err(error) => json!({"kind":"drop","position":position(p),"error":error.to_string()}),
        },
        _ => return,
    };
    let result = app.emit_to("main", "file-drag", &payload);
    if let tauri::DragDropEvent::Drop { paths, .. } = event {
        if let Some(runtime) = app.try_state::<Arc<Runtime>>() {
            let runtime = runtime.inner().clone();
            let details = json!({"count":paths.len(),"position":payload["position"],
                "registered":payload.get("token").is_some(),"validationError":payload["error"],
                "emitError":result.err().map(|e|e.to_string())});
            tauri::async_runtime::spawn(async move {
                runtime.trace("file.nativeDrop", details).await;
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drop_coordinates_match_dom_on_windows_scaling_and_appkit_retina() {
        let p = tauri::PhysicalPosition::new(300.0, 450.0);
        assert_eq!(
            logical_position(&p, 1.5, false),
            json!({"x":200.0,"y":300.0})
        );
        assert_eq!(
            logical_position(&p, 2.0, false),
            json!({"x":150.0,"y":225.0})
        );
        assert_eq!(
            logical_position(&p, 2.0, true),
            json!({"x":300.0,"y":450.0})
        );
    }
    #[test]
    fn native_drop_tokens_are_single_use_and_new_drops_invalidate_old_tokens() {
        let state = FileDrops::default();
        let path = std::env::temp_dir().join("folder");
        let first = state.register(vec![path.clone()]).unwrap();
        assert!(state.take(Uuid::new_v4()).is_err());
        assert_eq!(state.take(first).unwrap(), vec![path.clone()]);
        assert!(state.take(first).is_err());
        let old = state.register(vec![path.clone()]).unwrap();
        let current = state.register(vec![path.clone()]).unwrap();
        assert!(state.take(old).is_err());
        assert_eq!(state.take(current).unwrap(), vec![path]);
    }
    #[test]
    fn expired_invalid_or_oversized_drops_never_grant_access() {
        let state = FileDrops::default();
        let path = std::env::temp_dir().join("file.txt");
        let token = state.register(vec![path.clone()]).unwrap();
        state.0.lock().unwrap().as_mut().unwrap().created = Instant::now() - TTL;
        assert!(state.take(token).is_err());
        assert!(state.register(vec![]).is_err());
        assert!(state.register(vec![PathBuf::from("relative")]).is_err());
        assert!(state.register(vec![path; 257]).is_err());
    }
}
