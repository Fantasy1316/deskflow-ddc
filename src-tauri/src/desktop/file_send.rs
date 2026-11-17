//! A dedicated drop window with a narrow IPC contract. Native drops reuse the
//! existing transfer pipeline; the WebView cannot submit file paths or drops.
use super::*;
use anyhow::Context;
use serde::Serialize;
use std::{path::PathBuf, sync::Mutex};
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use uuid::Uuid;

mod interaction;
#[cfg(target_os = "macos")]
mod macos;
mod position;
mod status;
#[cfg(windows)]
mod windows;

const LABEL: &str = "file-send";
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 356.0;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct View {
    revision: u64,
    peer_name: String,
    peer_id: Option<Uuid>,
    disabled_reason: Option<String>,
    busy: bool,
    message: Option<String>,
    error: bool,
    dragging: bool,
    scanned_entries: Option<usize>,
    activity: Option<status::Activity>,
}

pub(super) struct Panel {
    view: Mutex<View>,
    drops: file_drop::FileDrops,
    drag_peer: Mutex<Option<Uuid>>,
    sending: AtomicBool,
    interaction: Mutex<interaction::Interaction>,
    operation: Mutex<Option<Uuid>>,
    tracked: Mutex<Option<status::Tracked>>,
    height: std::sync::atomic::AtomicU32,
}

impl Panel {
    pub(super) fn new(snapshot: &Snapshot) -> Self {
        let (peer_name, disabled_reason) = availability(snapshot);
        Self {
            view: Mutex::new(View {
                peer_name,
                disabled_reason,
                peer_id: snapshot.peer.as_ref().and_then(|p| p.id.parse().ok()),
                ..View::default()
            }),
            drops: file_drop::FileDrops::default(),
            drag_peer: Mutex::new(None),
            sending: AtomicBool::new(false),
            interaction: Mutex::new(interaction::Interaction::default()),
            operation: Mutex::new(None),
            tracked: Mutex::new(None),
            height: std::sync::atomic::AtomicU32::new(HEIGHT as u32),
        }
    }
    fn snapshot(&self) -> View {
        let mut view = self.view.lock().unwrap().clone();
        view.busy = self.sending.load(Ordering::Acquire);
        view.dragging = self.interaction.lock().unwrap().dragging();
        view
    }
}

fn availability(snapshot: &Snapshot) -> (String, Option<String>) {
    let name = snapshot
        .peer
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let reason = if snapshot.config.mode != crate::model::OperationMode::Sharing {
        Some("仅切换显示器模式不支持文件传输，请在主界面启用完整共享")
    } else if snapshot.emergency_stopping {
        Some("正在停止共享，请稍后重新连接")
    } else if !snapshot.connected || snapshot.peer.is_none() {
        Some("请先在主界面连接已配对的电脑")
    } else if !snapshot.peer.as_ref().is_some_and(|p| p.file_transfer) {
        Some("对端不支持文件传输，请更新两端应用")
    } else {
        None
    };
    (name, reason.map(str::to_owned))
}

fn publish(app: &tauri::AppHandle, message: Option<(String, bool)>) {
    publish_for(app, None, message);
}

fn publish_for(app: &tauri::AppHandle, peer: Option<Uuid>, message: Option<(String, bool)>) {
    let panel = app.state::<Panel>();
    {
        let mut view = panel.view.lock().unwrap();
        if peer.is_some() && peer != view.peer_id {
            return;
        }
        view.revision += 1;
        if let Some((text, error)) = message {
            view.message = Some(text);
            view.error = error;
        }
    }
    let _ = app.emit_to(LABEL, "file-send-state", panel.snapshot());
}

pub(super) fn sync(app: &tauri::AppHandle, snapshot: &Snapshot) {
    let (name, reason) = availability(snapshot);
    let panel = app.state::<Panel>();
    let peer_id = snapshot.peer.as_ref().and_then(|p| p.id.parse().ok());
    let changed = {
        let mut view = panel.view.lock().unwrap();
        if view.peer_name == name && view.peer_id == peer_id && view.disabled_reason == reason {
            false
        } else {
            view.peer_name = name;
            view.peer_id = peer_id;
            view.disabled_reason = reason;
            // A previous connection's success must not describe a new peer.
            view.message = None;
            view.error = false;
            view.scanned_entries = None;
            view.activity = None;
            *panel.tracked.lock().unwrap() = None;
            true
        }
    };
    if changed {
        publish(app, None);
    }
}

pub(super) fn hide(app: &tauri::AppHandle) {
    hide_checked(app, None);
}

fn hide_checked(app: &tauri::AppHandle, blur_ticket: Option<u64>) {
    let app = app.clone();
    let handle = app.clone();
    // Close the window and remove its observers as one UI-thread operation;
    // an old async hide must not leave a reopened panel without its monitors.
    let _ = app.run_on_main_thread(move || {
        if let Some(panel) = handle.try_state::<Panel>() {
            // Check and consume dismissal in the same UI operation that hides
            // the window. A picker beginning after the timer fired must still
            // be able to revoke that queued hide.
            let focused = blur_ticket.is_some()
                && handle
                    .get_webview_window(LABEL)
                    .is_some_and(|w| w.is_focused().unwrap_or(false));
            let mut interaction = panel.interaction.lock().unwrap();
            if let Some(ticket) = blur_ticket {
                if !interaction.can_dismiss(ticket) {
                    return;
                }
                if focused {
                    interaction.focused();
                    return;
                }
            }
            interaction.hide();
            drop(interaction);
            *panel.drag_peer.lock().unwrap() = None;
        }
        #[cfg(target_os = "macos")]
        macos::hide();
        if let Some(window) = handle.get_webview_window(LABEL) {
            let _ = window.hide();
        }
    });
}

fn dismiss_on_blur(app: &tauri::AppHandle) {
    let ticket = app.state::<Panel>().interaction.lock().unwrap().blur();
    schedule_blur(app, ticket);
}

fn schedule_blur(app: &tauri::AppHandle, ticket: Option<u64>) {
    let Some(ticket) = ticket else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Allow crossing the small gap between the tray icon and the panel.
        tokio::time::sleep(Duration::from_millis(160)).await;
        hide_checked(&app, Some(ticket));
    });
}

struct PickerFocus {
    app: tauri::AppHandle,
    presentation: u64,
}
impl PickerFocus {
    fn new(app: &tauri::AppHandle) -> Result<Self> {
        let presentation = app
            .state::<Panel>()
            .interaction
            .lock()
            .unwrap()
            .begin_picker()
            .context("发送面板已关闭或正在选择文件，请重新打开后重试")?;
        Ok(Self {
            app: app.clone(),
            presentation,
        })
    }
}
impl Drop for PickerFocus {
    fn drop(&mut self) {
        let app = self.app.clone();
        let presentation = self.presentation;
        // Keep picking=true until this UI operation runs. Clear the dialog's
        // blur history before restoring focus, for success, cancel and errors.
        if self
            .app
            .run_on_main_thread(move || {
                let restore = app
                    .state::<Panel>()
                    .interaction
                    .lock()
                    .unwrap()
                    .finish_picker(presentation);
                if restore && !app.state::<ExitState>().requested() {
                    if let Some(window) = app
                        .get_webview_window(LABEL)
                        .filter(|w| w.is_visible().unwrap_or(false))
                    {
                        let _ = window.set_focus();
                    }
                }
            })
            .is_err()
        {
            // A shut-down UI loop cannot run the restoration callback.
            self.app
                .state::<Panel>()
                .interaction
                .lock()
                .unwrap()
                .finish_picker(presentation);
        }
    }
}

fn drag_changed(app: &tauri::AppHandle, tray: bool, active: bool) {
    let panel = app.state::<Panel>();
    let ticket = {
        let mut interaction = panel.interaction.lock().unwrap();
        if tray {
            interaction.tray_drag = active;
        } else {
            interaction.panel_drag = active;
        }
        interaction.dismiss_ticket()
    };
    publish(app, None);
    schedule_blur(app, ticket);
}

pub(super) fn open(app: &tauri::AppHandle, focus: bool) {
    if app.state::<ExitState>().requested() {
        return;
    }
    display_source::close(app);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if handle.state::<ExitState>().requested() {
            return;
        }
        if let Err(error) = present(&handle, focus) {
            let runtime = handle.state::<Arc<Runtime>>().inner().clone();
            tauri::async_runtime::spawn(async move {
                runtime
                    .log("error", &format!("无法打开文件发送面板：{error:#}"))
                    .await;
            });
            show(&handle);
        }
    });
}

fn present(app: &tauri::AppHandle, focus: bool) -> Result<()> {
    app.state::<Panel>().interaction.lock().unwrap().open();
    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => {
            let window = WebviewWindowBuilder::new(
                app,
                LABEL,
                WebviewUrl::App(
                    format!(
                        "index.html?panel=file-send&platform={}",
                        std::env::consts::OS
                    )
                    .into(),
                ),
            )
            .title("Deskflow DDC · 发送文件")
            .inner_size(WIDTH, HEIGHT)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .focused(false)
            .accept_first_mouse(true)
            .on_page_load(|window, payload| {
                #[cfg(windows)]
                if payload.event() == tauri::webview::PageLoadEvent::Finished {
                    windows::install(&window);
                }
                #[cfg(not(windows))]
                let _ = (window, payload);
            })
            .build()?;
            let handle = app.clone();
            window.on_window_event(move |event| match event {
                tauri::WindowEvent::DragDrop(event) => {
                    // Windows uses the panel's late-bound OLE target, which
                    // dispatches directly; never submit both native paths.
                    #[cfg(not(windows))]
                    if !handle.state::<ExitState>().requested() {
                        native_drop(&handle, event);
                    }
                    #[cfg(windows)]
                    let _ = event;
                }
                tauri::WindowEvent::Focused(false) => dismiss_on_blur(&handle),
                tauri::WindowEvent::Focused(true) => handle
                    .state::<Panel>()
                    .interaction
                    .lock()
                    .unwrap()
                    .focused(),
                #[cfg(windows)]
                tauri::WindowEvent::Destroyed => windows::remove(),
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    hide(&handle);
                }
                _ => {}
            });
            window
        }
    };
    position::place(
        app,
        &window,
        app.state::<Panel>().height.load(Ordering::Acquire) as f64,
    )?;
    window.show()?;
    #[cfg(windows)]
    windows::install(&window);
    if focus {
        window.set_focus()?;
    }
    Ok(())
}

fn native_drop(app: &tauri::AppHandle, event: &tauri::DragDropEvent) {
    let panel = app.state::<Panel>();
    match event {
        tauri::DragDropEvent::Enter { .. } => {
            let view = panel.snapshot();
            #[cfg(target_os = "macos")]
            let peer = macos::take_drag_peer().or(view.peer_id);
            #[cfg(not(target_os = "macos"))]
            let peer = view.peer_id;
            *panel.drag_peer.lock().unwrap() = peer;
            drag_changed(app, false, true);
        }
        tauri::DragDropEvent::Leave => {
            *panel.drag_peer.lock().unwrap() = None;
            drag_changed(app, false, false);
        }
        tauri::DragDropEvent::Drop { paths, .. } => {
            // Dispatch native paths directly. The first drop must not be lost
            // while a lazily created WebView is loading its event listeners.
            let peer = panel.drag_peer.lock().unwrap().take();
            panel.interaction.lock().unwrap().dropped();
            publish(app, None);
            let paths = paths.clone();
            file_drop::handle_for(app, &tauri::DragDropEvent::Leave, LABEL, &panel.drops);
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let result = async {
                    let peer = peer.context("请先连接电脑，再重新拖入文件或目录")?;
                    let sending = begin(&app, peer).await?;
                    send(&app, paths, &sending).await
                }
                .await;
                outcome_for(&app, peer, &result);
            });
            return;
        }
        _ => {}
    }
    file_drop::handle_for(app, event, LABEL, &panel.drops);
}

struct Sending {
    app: tauri::AppHandle,
    peer: Uuid,
    operation: Uuid,
}
impl Drop for Sending {
    fn drop(&mut self) {
        self.app
            .state::<Panel>()
            .sending
            .store(false, Ordering::Release);
        publish(&self.app, None);
    }
}

async fn begin(app: &tauri::AppHandle, peer: Uuid) -> Result<Sending> {
    ensure!(!app.state::<ExitState>().requested(), "应用正在退出");
    let snapshot = app.state::<Arc<Runtime>>().snapshot().await;
    sync(app, &snapshot);
    if let Some(reason) = availability(&snapshot).1 {
        anyhow::bail!(reason);
    }
    ensure!(
        snapshot
            .peer
            .as_ref()
            .is_some_and(|p| p.id == peer.to_string()),
        "连接的电脑已更换，请确认目标后重新拖入"
    );
    ensure!(
        app.state::<Panel>()
            .sending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok(),
        "正在准备文件，请稍候"
    );
    app.state::<Panel>().view.lock().unwrap().scanned_entries = None;
    publish_for(app, Some(peer), Some(("正在准备文件…".into(), false)));
    let operation = Uuid::new_v4();
    *app.state::<Panel>().operation.lock().unwrap() = Some(operation);
    Ok(Sending {
        app: app.clone(),
        peer,
        operation,
    })
}

async fn send(app: &tauri::AppHandle, paths: Vec<PathBuf>, sending: &Sending) -> Result<()> {
    ensure!(!app.state::<ExitState>().requested(), "应用正在退出");
    let runtime = app.state::<Arc<Runtime>>();
    let snapshot = runtime.snapshot().await;
    if let Some(reason) = availability(&snapshot).1 {
        anyhow::bail!(reason);
    }
    ensure!(
        snapshot
            .peer
            .as_ref()
            .is_some_and(|peer| peer.id == sending.peer.to_string()),
        "连接的电脑已更换，请确认目标后重新发送"
    );
    // Validate and consume using the same rules as a normal native drop.
    let drops = file_drop::FileDrops::default();
    let token = drops.register(paths)?;
    let paths = drops.take(token)?;
    let count = paths.len();
    let peer = sending.peer;
    let operation = sending.operation;
    {
        let panel = app.state::<Panel>();
        *panel.tracked.lock().unwrap() = None;
        let mut view = panel.view.lock().unwrap();
        view.activity = None;
        view.scanned_entries = Some(0);
    }
    publish_for(app, Some(peer), Some(("正在整理文件…".into(), false)));
    let handle = app.clone();
    runtime
        .send_files_observed(
            paths,
            peer,
            Arc::new(move |progress| {
                let panel = handle.state::<Panel>();
                if *panel.operation.lock().unwrap() != Some(operation) {
                    return;
                }
                let mut view = panel.view.lock().unwrap();
                if view.peer_id != Some(peer) {
                    return;
                }
                match progress {
                    crate::transfer::PreparationProgress::Scanning(entries) => {
                        view.scanned_entries = Some(entries)
                    }
                    crate::transfer::PreparationProgress::Ready(id) => {
                        *panel.tracked.lock().unwrap() =
                            Some(status::Tracked::new(id, peer, count, operation));
                        view.scanned_entries = None;
                        view.message = Some(format!("已添加 {count} 项，正在通知对方接收"));
                    }
                }
                view.revision += 1;
                drop(view);
                let _ = handle.emit_to(LABEL, "file-send-state", panel.snapshot());
            }),
        )
        .await?;
    // A completed preparation for an old connection must not mark the new
    // peer's panel as successful. Transfer records retain their own peer ID.
    publish_for(
        app,
        Some(peer),
        Some((format!("已添加 {count} 项，等待对方接收"), false)),
    );
    Ok(())
}

#[cfg(windows)]
fn outcome(app: &tauri::AppHandle, result: &Result<()>) {
    outcome_for(app, None, result);
}
fn outcome_for(app: &tauri::AppHandle, peer: Option<Uuid>, result: &Result<()>) {
    if let Err(error) = result {
        let text = format!("{error:#}");
        publish_for(app, peer, Some((text.clone(), true)));
        let runtime = app.state::<Arc<Runtime>>().inner().clone();
        tauri::async_runtime::spawn(async move {
            runtime.log("error", &format!("托盘文件发送：{text}")).await;
        });
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum PanelAction {
    ChooseFiles {
        #[serde(rename = "peerId")]
        peer_id: Uuid,
    },
    ChooseFolder {
        #[serde(rename = "peerId")]
        peer_id: Uuid,
    },
    Retry {
        id: Uuid,
        #[serde(rename = "peerId")]
        peer_id: Uuid,
    },
    Resize {
        height: u32,
    },
    ShowMain {},
    Close {},
}

#[tauri::command]
pub(super) async fn file_send_snapshot(
    window: WebviewWindow,
    app: tauri::AppHandle,
) -> Result<View, String> {
    sender_label(&window, LABEL).map_err(|e| e.to_string())?;
    Ok(app.state::<Panel>().snapshot())
}

#[tauri::command]
pub(super) async fn file_send_action(
    window: WebviewWindow,
    app: tauri::AppHandle,
    action: PanelAction,
) -> Result<(), String> {
    sender_label(&window, LABEL).map_err(|e| e.to_string())?;
    let peer = match &action {
        PanelAction::ChooseFiles { peer_id }
        | PanelAction::ChooseFolder { peer_id }
        | PanelAction::Retry { peer_id, .. } => Some(*peer_id),
        _ => None,
    };
    let result = async {
        ensure!(!app.state::<ExitState>().requested(), "应用正在退出");
        match action {
            PanelAction::Close {} => hide(&app),
            PanelAction::ShowMain {} => show(&app),
            PanelAction::Resize { height } => {
                ensure!((260..=480).contains(&height), "发送面板高度无效");
                app.state::<Panel>().height.store(height, Ordering::Release);
                let app = app.clone();
                let handle = app.clone();
                app.run_on_main_thread(move || {
                    if let Some(window) = handle
                        .get_webview_window(LABEL)
                        .filter(|window| window.is_visible().unwrap_or(false))
                    {
                        let _ = position::place(&handle, &window, height as f64);
                    }
                })?;
            }
            PanelAction::Retry { id, peer_id } => {
                let sending = begin(&app, peer_id).await?;
                {
                    let panel = app.state::<Panel>();
                    let mut tracked = panel.tracked.lock().unwrap();
                    let task = tracked
                        .as_mut()
                        .filter(|tracked| tracked.id == id && tracked.peer == peer_id)
                        .context("这项任务已更换，请在主界面查看")?;
                    task.operation = sending.operation;
                }
                app.state::<Arc<Runtime>>().resume_transfer(id).await?;
                publish_for(
                    &app,
                    Some(peer_id),
                    Some(("已请求继续发送，等待对方接收".into(), false)),
                );
            }
            PanelAction::ChooseFiles { peer_id } | PanelAction::ChooseFolder { peer_id } => {
                // Protect the panel before begin() yields to connection checks
                // and publishes a resize-triggering status update.
                let picker_focus = PickerFocus::new(&app)?;
                let sending = begin(&app, peer_id).await?;
                publish_for(
                    &app,
                    Some(peer_id),
                    Some(("请选择要发送的文件或目录".into(), false)),
                );
                let handle = app.clone();
                let folder = matches!(action, PanelAction::ChooseFolder { .. });
                let paths = tauri::async_runtime::spawn_blocking(move || {
                    let picker = handle
                        .dialog()
                        .file()
                        .set_parent(&window)
                        .set_title(if folder {
                            "选择要发送的目录"
                        } else {
                            "选择要发送的文件"
                        });
                    let selected = if folder {
                        picker.blocking_pick_folder().map(|p| vec![p])
                    } else {
                        picker.blocking_pick_files()
                    };
                    selected
                        .map(|paths| {
                            paths
                                .into_iter()
                                .map(|p| p.into_path().map_err(anyhow::Error::from))
                                .collect::<Result<Vec<_>>>()
                        })
                        .transpose()
                })
                .await??;
                drop(picker_focus);
                if let Some(paths) = paths {
                    send(&app, paths, &sending).await?;
                } else {
                    publish_for(
                        &app,
                        Some(peer_id),
                        Some(("已取消选择，可以继续拖入文件或目录".into(), false)),
                    );
                }
            }
        }
        Ok(())
    }
    .await;
    outcome_for(&app, peer, &result);
    result.map_err(|e| format!("{e:#}"))
}

pub(super) fn install(app: &tauri::AppHandle) -> Result<()> {
    #[cfg(target_os = "macos")]
    macos::install(app)?;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let runtime = app.state::<Arc<Runtime>>().inner().clone();
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        let mut previous_hint = None;
        loop {
            tokio::select! {
                _ = runtime.shutdown.cancelled() => break,
                _ = interval.tick() => {},
            }
            refresh_activity(&app, &runtime);
            let hint = tooltip_suffix(&app);
            if hint != previous_hint {
                previous_hint = hint;
                super::sync_tray_tooltip(&app, &runtime.snapshot().await);
            }
        }
    });
    Ok(())
}

fn refresh_activity(app: &tauri::AppHandle, runtime: &Runtime) {
    let panel = app.state::<Panel>();
    // Lock order matches peer sync and the preparation observer (view, tracked).
    let changed = {
        let mut view = panel.view.lock().unwrap();
        let mut tracked = panel.tracked.lock().unwrap();
        let Some(task) = tracked.as_mut() else {
            return;
        };
        if view.peer_id != Some(task.peer) {
            return;
        }
        let activity = runtime
            .transfers
            .snapshot()
            .iter()
            .find(|job| job.id == task.id)
            .map(|job| task.update(job));
        if activity == view.activity {
            false
        } else {
            // A paused task may have been resumed from the main window.
            // A previous task must not erase a later picker/drop's message.
            let current_operation = *panel.operation.lock().unwrap() == Some(task.operation);
            if current_operation
                && !panel.sending.load(Ordering::Acquire)
                && activity.as_ref().is_some_and(|a| {
                    matches!(
                        a.status.as_str(),
                        "waiting" | "connecting" | "transferring" | "complete"
                    )
                })
            {
                view.error = false;
                view.message = None;
            }
            view.activity = activity;
            true
        }
    };
    if changed {
        publish(app, None);
    }
}

pub(super) fn tooltip_suffix(app: &tauri::AppHandle) -> Option<&'static str> {
    let panel = app.try_state::<Panel>()?;
    let view = panel.view.lock().unwrap();
    if view.error {
        return Some("文件发送需要处理");
    }
    match view.activity.as_ref()?.status.as_str() {
        "complete" => Some("文件已发送完成"),
        "paused" => Some("文件发送已暂停"),
        "cancelled" => Some("文件发送已取消"),
        "waiting" => Some("文件等待对方接收"),
        "transferring" | "connecting" => Some("文件发送中"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mini_window_rejects_renderer_paths_drops_and_unrelated_operations() {
        for value in [
            json!({"type":"sendDropped","token":Uuid::new_v4(),"peerId":Uuid::new_v4()}),
            json!({"type":"sendDropped","token":"bad"}),
            json!({"type":"chooseFiles","paths":["/etc/passwd"]}),
            json!({"type":"chooseFiles"}),
            json!({"type":"chooseFiles","peerId":Uuid::new_v4(),"paths":["/etc/passwd"]}),
            json!({"type":"switch","target":"peer"}),
        ] {
            assert!(serde_json::from_value::<PanelAction>(value).is_err());
        }
        assert!(serde_json::from_value::<PanelAction>(
            json!({"type":"chooseFiles","peerId":Uuid::new_v4()})
        )
        .is_ok());
    }
}
