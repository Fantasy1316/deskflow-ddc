use crate::{
    model::Config,
    runtime::{Runtime, Snapshot},
};
use anyhow::{ensure, Result};
use serde::Deserialize;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Action {
    Save {
        config: Config,
    },
    Start {
        #[serde(rename = "pairingCode")]
        pairing_code: Option<String>,
    },
    Switch {
        target: String,
    },
    TestInput {
        value: u16,
    },
    Scan {},
    Stop {},
    ChooseDeskflow {},
    UseBundledEngine {},
    RotatePairing {},
    CopyPairing {},
    OpenAccessibility {},
    CheckAccessibility {},
    OpenDataFolder {},
    ExportDiagnostics {},
    OpenLogFolder {},
    Discover {},
    ImportLegacyConfig {},
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererError {
    source: String,
    message: String,
    stack: Option<String>,
}
fn sender(window: &tauri::WebviewWindow) -> Result<()> {
    ensure!(window.label() == "main", "不允许的窗口");
    let url = window.url()?;
    let local = url.scheme() == "tauri"
        || url.host_str() == Some("tauri.localhost")
        || cfg!(debug_assertions)
            && url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port() == Some(5174);
    ensure!(local, "不允许的页面来源");
    Ok(())
}
#[tauri::command]
async fn snapshot(
    window: tauri::WebviewWindow,
    runtime: tauri::State<'_, Arc<Runtime>>,
) -> Result<Snapshot, String> {
    sender(&window).map_err(|e| e.to_string())?;
    Ok(runtime.snapshot().await)
}
#[tauri::command]
async fn act(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    runtime: tauri::State<'_, Arc<Runtime>>,
    action: Action,
) -> Result<(), String> {
    sender(&window).map_err(|e| e.to_string())?;
    let _action = runtime.actions.lock().await;
    let result = dispatch(&app, &runtime, action).await;
    if let Err(e) = &result {
        runtime.log("error", &e.to_string()).await;
    }
    result.map_err(|e| e.to_string())
}
async fn pick(app: &tauri::AppHandle, save: bool) -> Result<Option<std::path::PathBuf>> {
    let app = app.clone();
    let file = tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file().add_filter("JSON", &["json"]);
        if save {
            dialog
                .set_file_name(format!(
                    "deskflow-ddc-{}-{}.json",
                    crate::model::platform(),
                    chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ")
                ))
                .blocking_save_file()
        } else {
            dialog.blocking_pick_file()
        }
    })
    .await?;
    file.map(|p| p.into_path().map_err(anyhow::Error::from))
        .transpose()
}
async fn dispatch(app: &tauri::AppHandle, runtime: &Arc<Runtime>, action: Action) -> Result<()> {
    match action {
        Action::Save { config } => runtime.save(config).await,
        Action::Start { pairing_code } => runtime.start(pairing_code).await,
        Action::Switch { target } => {
            ensure!(["local", "peer"].contains(&target.as_str()), "切换目标无效");
            runtime.switch_to(target == "local").await
        }
        Action::TestInput { value } => runtime.test_input(value).await,
        Action::Scan {} => runtime.scan().await,
        Action::Stop {} => runtime.stop().await,
        Action::Discover {} => runtime.discover().await,
        Action::CheckAccessibility {} => runtime.check_accessibility().await,
        Action::RotatePairing {} => runtime.rotate_pairing().await,
        Action::CopyPairing {} => {
            app.clipboard()
                .write_text(runtime.snapshot().await.pairing_code)?;
            Ok(())
        }
        Action::UseBundledEngine {} => {
            let mut c = runtime.snapshot().await.config;
            c.deskflow_path.clear();
            runtime.save(c).await
        }
        Action::ChooseDeskflow {} => {
            ensure!(!runtime.snapshot().await.running, "请先停止共享");
            let app = app.clone();
            let file = tauri::async_runtime::spawn_blocking(move || {
                app.dialog()
                    .file()
                    .set_title("选择 Deskflow 1.27.0 deskflow-core（高级选项）")
                    .blocking_pick_file()
            })
            .await?;
            if let Some(file) = file {
                let mut c = runtime.snapshot().await.config;
                c.deskflow_path = file.into_path()?.to_string_lossy().into();
                runtime.save(c).await?;
            }
            Ok(())
        }
        Action::ImportLegacyConfig {} => {
            ensure!(!runtime.snapshot().await.running, "请先停止共享");
            if let Some(file) = pick(app, false).await? {
                runtime.import_config(file).await?;
            }
            Ok(())
        }
        Action::ExportDiagnostics {} => {
            if let Some(file) = pick(app, true).await? {
                crate::store::atomic_write(
                    &file,
                    &serde_json::to_vec_pretty(&runtime.export().await?)?,
                )?;
                runtime
                    .log("success", "诊断日志已导出（不包含配对码或证书私钥）")
                    .await;
            }
            Ok(())
        }
        Action::OpenDataFolder {} | Action::OpenLogFolder {} => {
            let path = runtime.data_directory().await;
            let path = if matches!(action, Action::OpenLogFolder {}) {
                path.join("logs")
            } else {
                path
            };
            app.opener()
                .open_path(path.to_string_lossy().to_string(), None::<&str>)?;
            Ok(())
        }
        Action::OpenAccessibility {} => {
            #[cfg(target_os = "macos")]
            app.opener().open_url(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                None::<&str>,
            )?;
            runtime.check_accessibility().await
        }
    }
}
#[tauri::command]
async fn report_error(
    window: tauri::WebviewWindow,
    runtime: tauri::State<'_, Arc<Runtime>>,
    error: RendererError,
) -> Result<(), String> {
    sender(&window).map_err(|e| e.to_string())?;
    if !["window.error", "window.unhandledrejection", "vue"].contains(&error.source.as_str())
        || error.message.len() > 32768
        || error.stack.as_ref().is_some_and(|s| s.len() > 65536)
    {
        return Err("界面错误数据无效".into());
    }
    runtime
        .trace(
            "renderer.error",
            json!({"source":error.source,"message":error.message,"stack":error.stack}),
        )
        .await;
    Ok(())
}
fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
fn quit(app: &tauri::AppHandle) {
    if app.state::<AtomicBool>().swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let runtime = app.state::<Arc<Runtime>>();
        let _action = runtime.actions.lock().await;
        match runtime.stop().await {
            Ok(()) => {
                runtime.shutdown.cancel();
                app.exit(0)
            }
            Err(e) => {
                runtime
                    .log("error", &format!("退出前停止引擎失败：{e}"))
                    .await;
                app.state::<AtomicBool>().store(false, Ordering::Release);
                show(&app);
            }
        }
    });
}
fn sync_shortcuts(app: tauri::AppHandle, runtime: Arc<Runtime>) {
    tauri::async_runtime::spawn(async move {
        let mut current = String::new();
        let mut delay = Duration::from_secs(1);
        let mut retry = tokio::time::Instant::now();
        let mut wanted_before = String::new();
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = runtime.shutdown.cancelled() => break,
                _ = interval.tick() => {}
            }
            let wanted = runtime.shortcut().await.unwrap_or_default();
            if wanted != wanted_before {
                if !current.is_empty() {
                    let _ = app.global_shortcut().unregister(current.as_str());
                    current.clear();
                }
                wanted_before = wanted.clone();
                retry = tokio::time::Instant::now();
                delay = Duration::from_secs(1);
            }
            if wanted.is_empty() || !current.is_empty() || tokio::time::Instant::now() < retry {
                continue;
            }
            let rt = runtime.clone();
            let registration =
                app.global_shortcut()
                    .on_shortcut(wanted.as_str(), move |_, _, event| {
                        if event.state() == ShortcutState::Pressed {
                            let rt = rt.clone();
                            tauri::async_runtime::spawn(async move {
                                if rt.shortcut().await.is_some() {
                                    if let Err(e) = rt.toggle().await {
                                        rt.log("error", &e.to_string()).await;
                                    }
                                }
                            });
                        }
                    });
            match registration {
                Ok(()) => {
                    current = wanted;
                    delay = Duration::from_secs(1);
                }
                Err(e) => {
                    if delay == Duration::from_secs(1) {
                        runtime
                            .log("error", &format!("快捷键注册失败，将自动重试：{e}"))
                            .await;
                    }
                    retry = tokio::time::Instant::now() + delay;
                    delay = (delay * 2).min(Duration::from_secs(30));
                }
            }
        }
        if !current.is_empty() {
            let _ = app.global_shortcut().unregister(current.as_str());
        }
    });
}
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![snapshot, act, report_error])
        .setup(|app| {
            let resource = if cfg!(debug_assertions) {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources")
            } else {
                app.path().resource_dir()?.join("resources")
            };
            let runtime = match tauri::async_runtime::block_on(Runtime::new(app.path().app_data_dir()?, resource)) {
                Ok(runtime) => runtime,
                Err(error) => {
                    app.dialog().message(format!("无法读取 Preview 配置或系统凭据：{error}\n旧版 Deskflow DDC 的数据未修改。"))
                        .title("Deskflow DDC Preview 启动失败")
                        .kind(tauri_plugin_dialog::MessageDialogKind::Error).blocking_show();
                    return Err(error.into());
                }
            };
            app.manage(runtime.clone());
            app.manage(AtomicBool::new(false));
            let show_item =
                MenuItem::with_id(app, "show", "打开 Deskflow DDC Preview", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出 Preview", true, Some("CmdOrCtrl+Q"))?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
            #[cfg(target_os = "macos")]
            {
                let submenu=tauri::menu::Submenu::with_items(app,"Deskflow DDC Preview",true,&[&show_item,&quit_item])?;
                app.set_menu(Menu::with_items(app,&[&submenu])?)?;
            }
            #[cfg(target_os = "macos")]
            let icon = tauri::image::Image::from_bytes(include_bytes!(
                "../../resources/icons/trayTemplate@2x.png"
            ))?;
            #[cfg(not(target_os = "macos"))]
            let icon =
                tauri::image::Image::from_bytes(include_bytes!("../../resources/icons/app.png"))?;
            let tray = TrayIconBuilder::with_id("preview")
                .icon(icon)
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("Deskflow DDC Preview")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show(app),
                    "quit" => quit(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(event, tauri::tray::TrayIconEvent::DoubleClick { .. }) {
                        show(tray.app_handle());
                    }
                })
                .build(app);
            let has_tray = tray.is_ok();
            if let Err(e) = tray {
                eprintln!("Preview tray unavailable: {e}");
            }
            app.on_menu_event(|app, event| match event.id.as_ref() {
                "show" => show(app),
                "quit" => quit(app),
                _ => {}
            });
            if let Some(window) = app.get_webview_window("main") {
                let handle = app.handle().clone();
                let rt = runtime.clone();
                window.on_window_event(move |event| match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        if has_tray && !handle.state::<AtomicBool>().load(Ordering::Acquire) {
                            if let Some(w) = handle.get_webview_window("main") {
                                let _ = w.hide();
                            }
                        } else {
                            quit(&handle);
                        }
                    }
                    tauri::WindowEvent::Focused(true) => {
                        let rt = rt.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = rt.check_accessibility().await;
                        });
                    }
                    _ => {}
                });
            }
            let handle = app.handle().clone();
            let mut updates = runtime.updates.subscribe();
            tauri::async_runtime::spawn(async move {
                while updates.changed().await.is_ok() {
                    let snapshot = updates.borrow_and_update().clone();
                    let _ = handle.emit_to("main", "bridge-state", snapshot);
                }
            });
            sync_shortcuts(app.handle().clone(), runtime);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("无法启动 Deskflow DDC Preview");
    app.run(|app, event| match event {
        tauri::RunEvent::ExitRequested { api, .. } => {
            if !app.state::<AtomicBool>().load(Ordering::Acquire) {
                api.prevent_exit();
                quit(app);
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => show(app),
        _ => {}
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_actions_and_fields() {
        assert!(
            serde_json::from_value::<Action>(json!({"type":"shell","command":"whoami"})).is_err()
        );
        assert!(
            serde_json::from_value::<Action>(json!({"type":"stop","unexpected":true})).is_err()
        );
    }
}
