use crate::{
    model::Config,
    runtime::{Runtime, Snapshot},
};
use anyhow::{ensure, Result};
use serde::Deserialize;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;

#[derive(Default)]
struct ExitState(AtomicU8);
impl ExitState {
    fn begin(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    fn requested(&self) -> bool {
        self.0.load(Ordering::Acquire) != 0
    }
    fn ready(&self) -> bool {
        self.0.load(Ordering::Acquire) == 2
    }
    fn complete(&self) {
        self.0.store(2, Ordering::Release);
    }
    fn retry(&self) {
        self.0.store(0, Ordering::Release);
    }
}

#[cfg(target_os = "macos")]
static DOCK_QUIT_APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

#[cfg(target_os = "macos")]
fn install_dock_quit(app: &tauri::AppHandle) -> Result<()> {
    unsafe extern "C" {
        fn sb_install_quit_handler(callback: extern "C" fn()) -> bool;
    }
    extern "C" fn request() {
        if let Some(app) = DOCK_QUIT_APP.get() {
            if app.state::<ExitState>().ready() {
                finish_quit(app, true);
            } else {
                quit_from(app, "macosTermination");
            }
        }
    }
    ensure!(DOCK_QUIT_APP.set(app.clone()).is_ok(), "退出处理器已注册");
    ensure!(
        unsafe { sb_install_quit_handler(request) },
        "无法注册程序坞退出保护"
    );
    Ok(())
}

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
        #[cfg(target_os = "macos")]
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
#[derive(Clone)]
struct TrayControls {
    local: MenuItem<tauri::Wry>,
    peer: MenuItem<tauri::Wry>,
    stop: MenuItem<tauri::Wry>,
    busy: Arc<AtomicBool>,
}
impl TrayControls {
    fn sync(&self, state: &Snapshot) {
        let idle = !self.busy.load(Ordering::Acquire);
        let can_switch = idle
            && state.running
            && state.connected
            && state.engine == "ready"
            && !state.switching
            && state.display_status != "uncertain";
        let _ = self.local.set_enabled(can_switch);
        let _ = self.peer.set_enabled(can_switch);
        let _ = self.stop.set_enabled(idle && state.running);
    }
}
fn tray_action(id: &str) -> Option<Action> {
    match id {
        "switch-local" => Some(Action::Switch {
            target: "local".into(),
        }),
        "switch-peer" => Some(Action::Switch {
            target: "peer".into(),
        }),
        "stop" => Some(Action::Stop {}),
        _ => None,
    }
}
fn handle_menu(app: &tauri::AppHandle, id: &str) {
    match id {
        "show" => show(app),
        "quit" => quit(app),
        _ => {
            let Some(action) = tray_action(id) else {
                return;
            };
            let controls = app.state::<TrayControls>().inner().clone();
            if app.state::<ExitState>().requested() || controls.busy.swap(true, Ordering::AcqRel) {
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let runtime = app.state::<Arc<Runtime>>();
                controls.sync(&runtime.snapshot().await);
                let _action = runtime.actions.lock().await;
                // A quit may have started while this action was queued.
                if !app.state::<ExitState>().requested() {
                    if let Err(e) = dispatch(&app, &runtime, action).await {
                        runtime.log("error", &format!("托盘操作失败：{e}")).await;
                        show(&app);
                    }
                }
                controls.busy.store(false, Ordering::Release);
                controls.sync(&runtime.snapshot().await);
            });
        }
    }
}
fn quit(app: &tauri::AppHandle) {
    quit_from(app, "applicationMenu");
}
fn finish_quit(app: &tauri::AppHandle, stopped: bool) {
    let handle = app.clone();
    // A Dock/system quit can arrive while menu-initiated cleanup is running.
    // Decide whether to reply to Cocoa on its main thread, after cleanup.
    let _ = app.run_on_main_thread(move || {
        if stopped {
            handle.state::<ExitState>().complete();
        }
        #[cfg(target_os = "macos")]
        let replied = unsafe {
            unsafe extern "C" {
                fn sb_complete_quit(allow: bool) -> bool;
            }
            sb_complete_quit(stopped)
        };
        #[cfg(not(target_os = "macos"))]
        let replied = false;
        if stopped {
            if !replied {
                handle.exit(0);
            }
        } else {
            handle.state::<ExitState>().retry();
            show(&handle);
        }
    });
}
fn quit_from(app: &tauri::AppHandle, source: &'static str) {
    if !app.state::<ExitState>().begin() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let runtime = app.state::<Arc<Runtime>>();
        runtime
            .trace("app.quitRequested", json!({"source":source}))
            .await;
        let _action = runtime.actions.lock().await;
        match runtime.stop().await {
            Ok(()) => {
                runtime.shutdown.cancel();
                runtime
                    .trace("app.exitReady", json!({"ownedEngineStopped":true}))
                    .await;
                finish_quit(&app, true);
            }
            Err(e) => {
                runtime
                    .log("error", &format!("退出前停止引擎失败：{e}"))
                    .await;
                finish_quit(&app, false);
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
                    app.dialog().message(format!("无法读取 应用配置或系统凭据：{error}\n旧版 Deskflow DDC 的数据未修改。"))
                        .title("Deskflow DDC 启动失败")
                        .kind(tauri_plugin_dialog::MessageDialogKind::Error).blocking_show();
                    return Err(error.into());
                }
            };
            app.manage(runtime.clone());
            app.manage(ExitState::default());
            #[cfg(target_os = "macos")]
            install_dock_quit(app.handle())?;
            let show_item = MenuItem::with_id(app, "show", "打开主界面", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, Some("CmdOrCtrl+Q"))?;
            let controls = TrayControls {
                local: MenuItem::with_id(app, "switch-local", "切换到本机", false, None::<&str>)?,
                peer: MenuItem::with_id(app, "switch-peer", "切换到另一台", false, None::<&str>)?,
                stop: MenuItem::with_id(app, "stop", "停止共享", false, None::<&str>)?,
                busy: Arc::new(AtomicBool::new(false)),
            };
            controls.sync(&tauri::async_runtime::block_on(runtime.snapshot()));
            app.manage(controls.clone());
            let menu = Menu::with_items(app, &[
                &show_item,
                &PredefinedMenuItem::separator(app)?,
                &controls.local,
                &controls.peer,
                &controls.stop,
                &PredefinedMenuItem::separator(app)?,
                &quit_item,
            ])?;
            #[cfg(target_os = "macos")]
            {
                use tauri::menu::Submenu;
                let submenu = Submenu::with_items(
                    app,
                    "Deskflow DDC",
                    true,
                    &[&show_item, &quit_item],
                )?;
                // WKWebView routes editing key equivalents through Cocoa's
                // responder chain. Use native items rather than global shortcuts
                // so they operate only on the focused editable control.
                let edit = Submenu::with_items(
                    app,
                    "编辑",
                    true,
                    &[
                        &PredefinedMenuItem::undo(app, Some("撤销"))?,
                        &PredefinedMenuItem::redo(app, Some("重做"))?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::cut(app, Some("剪切"))?,
                        &PredefinedMenuItem::copy(app, Some("复制"))?,
                        &PredefinedMenuItem::paste(app, Some("粘贴"))?,
                        &PredefinedMenuItem::select_all(app, Some("全选"))?,
                    ],
                )?;
                app.set_menu(Menu::with_items(app, &[&submenu, &edit])?)?;
            }
            #[cfg(target_os = "macos")]
            let icon = tauri::image::Image::from_bytes(include_bytes!(
                "../../resources/icons/trayTemplate@2x.png"
            ))?;
            #[cfg(not(target_os = "macos"))]
            let icon =
                tauri::image::Image::from_bytes(include_bytes!("../../resources/icons/app.png"))?;
            let tray = TrayIconBuilder::with_id("main-tray")
                .icon(icon)
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("Deskflow DDC")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        tauri::tray::TrayIconEvent::Click {
                            button: tauri::tray::MouseButton::Left,
                            button_state: tauri::tray::MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show(tray.app_handle());
                    }
                })
                .build(app);
            let has_tray = tray.is_ok();
            if let Err(e) = tray {
                eprintln!("Tray unavailable: {e}");
            }
            // The global handler also receives tray events. Register only once
            // so a click cannot dispatch the same switch or stop twice.
            app.on_menu_event(|app, event| handle_menu(app, event.id.as_ref()));
            if let Some(window) = app.get_webview_window("main") {
                let handle = app.handle().clone();
                let rt = runtime.clone();
                window.on_window_event(move |event| match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        if has_tray && !handle.state::<ExitState>().requested() {
                            if let Some(w) = handle.get_webview_window("main") {
                                if w.hide().is_ok() {
                                    // Accessory keeps the tray and sharing alive while
                                    // removing the hidden app from the Dock and Cmd+Tab.
                                    #[cfg(target_os = "macos")]
                                    let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                                }
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
                    controls.sync(&snapshot);
                    let _ = handle.emit_to("main", "bridge-state", snapshot);
                }
            });
            sync_shortcuts(app.handle().clone(), runtime);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("无法启动 Deskflow DDC");
    app.run(|app, event| match event {
        tauri::RunEvent::ExitRequested { api, .. } => {
            if !app.state::<ExitState>().ready() {
                api.prevent_exit();
                quit_from(app, "systemExit");
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
    fn repeated_exit_requests_cannot_bypass_engine_cleanup() {
        let state = ExitState::default();
        assert!(!state.ready());
        assert!(state.begin());
        for _ in 0..4 {
            assert!(state.requested());
            assert!(!state.begin());
            assert!(!state.ready());
        }
        state.retry();
        assert!(!state.requested());
        assert!(state.begin());
        state.complete();
        assert!(state.ready());
        assert!(!state.begin());
    }
    #[test]
    fn rejects_unknown_actions_and_fields() {
        assert!(
            serde_json::from_value::<Action>(json!({"type":"shell","command":"whoami"})).is_err()
        );
        assert!(
            serde_json::from_value::<Action>(json!({"type":"stop","unexpected":true})).is_err()
        );
    }
    #[test]
    fn tray_commands_use_local_and_peer_targets() {
        assert!(
            matches!(tray_action("switch-local"), Some(Action::Switch { target }) if target == "local")
        );
        assert!(
            matches!(tray_action("switch-peer"), Some(Action::Switch { target }) if target == "peer")
        );
        assert!(matches!(tray_action("stop"), Some(Action::Stop {})));
        assert!(tray_action("show").is_none());
        assert!(tray_action("unknown").is_none());
    }
}
