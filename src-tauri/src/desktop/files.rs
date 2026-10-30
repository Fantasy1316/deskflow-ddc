use super::*;
use serde_json::Value;
use uuid::Uuid;
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum FileAction {
    SendFiles {},
    SendDropped {
        token: Uuid,
    },
    Accept {
        id: Uuid,
    },
    Resume {
        id: Uuid,
    },
    Pause {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
    },
    Remove {
        id: Uuid,
    },
    Open {
        id: Uuid,
    },
    ResolveConflict {
        id: Uuid,
        token: Uuid,
        decision: crate::transfer::Decision,
    },
}
#[tauri::command]
pub async fn file_snapshot(
    window: tauri::WebviewWindow,
    runtime: tauri::State<'_, Arc<Runtime>>,
) -> Result<Value, String> {
    sender(&window).map_err(|e| e.to_string())?;
    Ok(
        json!({"transfers":runtime.transfers.snapshot(),"warnings":runtime.transfers.warnings(),"network":runtime.transfers.network_snapshot()}),
    )
}
#[tauri::command]
pub async fn file_action(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    runtime: tauri::State<'_, Arc<Runtime>>,
    action: FileAction,
) -> Result<(), String> {
    sender(&window).map_err(|e| e.to_string())?;
    ensure_active(&app).map_err(|e| e.to_string())?;
    let result = dispatch_file(&app, &runtime, action).await;
    if let Err(e) = &result {
        runtime.log("error", &format!("文件传输：{e}")).await;
    }
    result.map_err(|e| e.to_string())
}
fn ensure_active(app: &tauri::AppHandle) -> Result<()> {
    ensure!(!app.state::<ExitState>().requested(), "应用正在退出");
    Ok(())
}
async fn folder(app: &tauri::AppHandle, title: &'static str) -> Result<Option<std::path::PathBuf>> {
    let app = app.clone();
    let file = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title(title).blocking_pick_folder()
    })
    .await?;
    file.map(|p| p.into_path().map_err(anyhow::Error::from))
        .transpose()
}
async fn dispatch_file(
    app: &tauri::AppHandle,
    runtime: &Arc<Runtime>,
    action: FileAction,
) -> Result<()> {
    if matches!(
        &action,
        FileAction::SendFiles {}
            | FileAction::SendDropped { .. }
            | FileAction::Accept { .. }
            | FileAction::Resume { .. }
    ) {
        ensure!(
            runtime.snapshot().await.config.mode == crate::model::OperationMode::Sharing,
            "仅切换显示器模式不支持文件传输，请先恢复完整共享"
        );
    }
    match action {
        FileAction::SendFiles {} => {
            ensure!(runtime.snapshot().await.connected, "请先连接已配对的电脑");
            let handle = app.clone();
            let paths = tauri::async_runtime::spawn_blocking(move || {
                handle
                    .dialog()
                    .file()
                    .set_title("选择要发送的文件")
                    .blocking_pick_files()
            })
            .await?;
            if let Some(paths) = paths {
                let paths = paths
                    .into_iter()
                    .map(|p| p.into_path())
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                runtime.send_files(paths).await?;
            }
        }
        FileAction::SendDropped { token } => {
            ensure!(runtime.snapshot().await.connected, "请先连接已配对的电脑");
            let paths = app.state::<super::file_drop::FileDrops>().take(token)?;
            runtime.send_files(paths).await?;
        }
        FileAction::Accept { id } => {
            let destination = if runtime.transfers.needs_destination(id)? {
                let Some(path) = folder(app, "选择接收文件的保存位置").await? else {
                    return Ok(());
                };
                Some(path)
            } else {
                None
            };
            runtime.transfers.accept(id, destination).await?;
        }
        FileAction::Resume { id } => runtime.resume_transfer(id).await?,
        FileAction::ResolveConflict {
            id,
            token,
            decision,
        } => runtime.transfers.resolve_conflict(id, token, decision)?,
        FileAction::Pause { id } => runtime.transfers.pause(id)?,
        FileAction::Cancel { id } => runtime.cancel_transfer(id).await?,
        FileAction::Remove { id } => {
            runtime.cancel_transfer(id).await?;
            runtime.transfers.remove(id)?;
        }
        FileAction::Open { id } => {
            let path = runtime.transfers.destination(id)?;
            app.opener()
                .open_path(path.to_string_lossy().to_string(), None::<&str>)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_actions_do_not_accept_raw_paths_from_the_webview() {
        for action in [
            json!({"type":"sendFiles","paths":["/etc/passwd"]}),
            json!({"type":"sendDropped","token":Uuid::new_v4(),"paths":["/etc/passwd"]}),
            json!({"type":"sendDropped","token":"not-a-native-token"}),
            json!({"type":"accept","id":Uuid::new_v4(),"destination":"/tmp"}),
            json!({"type":"open","id":"../../etc/passwd"}),
            json!({"type":"shell","command":"anything"}),
        ] {
            assert!(serde_json::from_value::<FileAction>(action).is_err());
        }
    }
}
