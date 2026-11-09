use super::*;
use crate::runtime::SourcePrompt;
use std::sync::{Mutex, OnceLock};

#[cfg(target_os = "macos")]
#[path = "display_source/macos.rs"]
mod native;
#[cfg(windows)]
#[path = "display_source/windows.rs"]
mod native;

#[cfg(any(windows, test))]
mod geometry;

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

#[derive(Default)]
pub(super) struct Reminders {
    state: Mutex<PopoverState>,
    // Serialize UI changes, never user input or peer RPCs.
    gate: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct PopoverState {
    tracker: PromptTracker,
    active: Option<Active>,
}

struct Active {
    ticket: u64,
    prompt: Arc<SourcePrompt>,
    busy: bool,
    error: Option<String>,
}

#[derive(Clone)]
struct View {
    ticket: u64,
    local_name: String,
    peer_name: String,
    busy: bool,
    error: Option<String>,
}

impl PopoverState {
    fn view(&self) -> Option<View> {
        self.active.as_ref().map(|a| View {
            ticket: a.ticket,
            local_name: a.prompt.local_name.clone(),
            peer_name: a.prompt.peer_name.clone(),
            busy: a.busy,
            error: a.error.clone(),
        })
    }

    fn dismiss(&mut self, ticket: u64) -> bool {
        if !self.active.as_ref().is_some_and(|a| a.ticket == ticket) {
            return false;
        }
        self.active = None;
        self.tracker.finish(ticket);
        true
    }
}

#[derive(Default)]
struct PromptTracker {
    pending: Option<(u64, u64)>,
    shown: bool,
    hidden: bool,
    entered_tray: bool,
    request: u64,
    open: Option<u64>,
    sequence: u64,
}

impl PromptTracker {
    fn observe(
        &mut self,
        pending: Option<(u64, u64)>,
        available: bool,
        hidden: bool,
        request: u64,
        manual: bool,
    ) -> Option<u64> {
        if hidden && !self.hidden {
            self.entered_tray = true;
        } else if !hidden {
            self.entered_tray = false;
        }
        self.hidden = hidden;
        if self.pending != pending {
            self.pending = pending;
            self.shown = false;
        }
        if self.open.is_some() {
            self.request = request;
            self.entered_tray = false;
            return None;
        }
        if pending.is_none()
            || !available
            || !(hidden || manual)
            || (self.shown && !self.entered_tray && self.request == request && !manual)
        {
            return None;
        }
        self.sequence += 1;
        self.open = Some(self.sequence);
        self.shown = true;
        self.entered_tray = false;
        self.request = request;
        self.open
    }

    fn finish(&mut self, ticket: u64) {
        if self.open == Some(ticket) {
            self.open = None;
        }
    }
}

// Native callbacks carry the exact prompt ticket. They cannot act on a newer
// connection, and never expose a general-purpose action API to another window.
fn event(ticket: u64, choice: i32) {
    let Some(app) = APP.get().cloned() else {
        return;
    };
    if !(0..=2).contains(&choice) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if choice == 0 {
            let reminders = app.state::<Reminders>();
            let _gate = reminders.gate.lock().await;
            dismiss(&app, ticket);
        } else {
            choose(&app, ticket, choice == 1).await;
        }
    });
}

fn dismiss(app: &tauri::AppHandle, ticket: u64) {
    if app
        .state::<Reminders>()
        .state
        .lock()
        .unwrap()
        .dismiss(ticket)
    {
        let _ = app.run_on_main_thread(move || {
            #[cfg(any(target_os = "macos", windows))]
            native::hide(ticket);
        });
    }
}

pub(super) fn close(app: &tauri::AppHandle) {
    let ticket = app
        .state::<Reminders>()
        .state
        .lock()
        .unwrap()
        .active
        .as_ref()
        .map(|a| a.ticket);
    if let Some(ticket) = ticket {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let reminders = app.state::<Reminders>();
            let _gate = reminders.gate.lock().await;
            dismiss(&app, ticket);
        });
    }
}

pub(super) fn start(app: tauri::AppHandle, runtime: Arc<Runtime>) {
    let _ = APP.set(app.clone());
    let mut updates = runtime.updates.subscribe();
    let mut requests = runtime.source_reminders.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            refresh(&app, &runtime, false).await;
            tokio::select! {
                _ = runtime.shutdown.cancelled() => break,
                r = updates.changed() => if r.is_err() { break; },
                r = requests.changed() => if r.is_err() { break; },
                _ = interval.tick() => {},
            }
        }
    });
}

pub(super) fn open(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let runtime = app.state::<Arc<Runtime>>().inner().clone();
        refresh(&app, &runtime, true).await;
    });
}

async fn present(app: &tauri::AppHandle) -> Result<()> {
    let Some(view) = app.state::<Reminders>().state.lock().unwrap().view() else {
        return Ok(());
    };
    let (send, receive) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        #[cfg(any(target_os = "macos", windows))]
        let result = native::show(&handle, view);
        #[cfg(not(any(target_os = "macos", windows)))]
        let result: Result<()> = Err(anyhow::anyhow!("当前平台请在主界面确认画面来源"));
        let _ = send.send(result);
    })?;
    receive
        .await
        .map_err(|_| anyhow::anyhow!("原生浮窗未响应"))?
}

async fn refresh(app: &tauri::AppHandle, runtime: &Arc<Runtime>, manual: bool) {
    let reminders = app.state::<Reminders>();
    let _gate = reminders.gate.lock().await;
    if app.state::<ExitState>().requested() || app.tray_by_id("main-tray").is_none() {
        let ticket = reminders
            .state
            .lock()
            .unwrap()
            .active
            .as_ref()
            .map(|a| a.ticket);
        if let Some(ticket) = ticket {
            dismiss(app, ticket);
        }
        return;
    }
    let prompt = runtime.signal_source_prompt().await;
    let snapshot = runtime.snapshot().await;
    let stale = reminders
        .state
        .lock()
        .unwrap()
        .active
        .as_ref()
        .filter(|active| {
            !snapshot.can_confirm_signal_source()
                || !prompt
                    .as_ref()
                    .is_some_and(|p| active.prompt.same_context(p))
        })
        .map(|a| a.ticket);
    if let Some(ticket) = stale {
        dismiss(app, ticket);
        reminders.state.lock().unwrap().tracker.pending = None;
    }
    let hidden = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().ok() == Some(false) || w.is_minimized().ok() == Some(true));
    let ticket = reminders.state.lock().unwrap().tracker.observe(
        prompt.as_ref().map(|p| p.key()),
        snapshot.can_confirm_signal_source(),
        hidden,
        *runtime.source_reminders.borrow(),
        manual,
    );
    let (Some(prompt), Some(ticket)) = (prompt, ticket) else {
        return;
    };
    reminders.state.lock().unwrap().active = Some(Active {
        ticket,
        prompt: Arc::new(prompt),
        busy: false,
        error: None,
    });
    if let Err(error) = present(app).await {
        dismiss(app, ticket);
        runtime
            .log("error", &format!("无法打开原生托盘确认：{error:#}"))
            .await;
        show(app);
    }
}

async fn choose(app: &tauri::AppHandle, ticket: u64, local: bool) {
    let reminders = app.state::<Reminders>();
    let prompt = {
        let _gate = reminders.gate.lock().await;
        if app.state::<ExitState>().requested() {
            return;
        }
        let prompt = {
            let mut state = reminders.state.lock().unwrap();
            let Some(active) = state
                .active
                .as_mut()
                .filter(|a| a.ticket == ticket && !a.busy)
            else {
                return;
            };
            active.busy = true;
            active.error = None;
            active.prompt.clone()
        };
        let _ = present(app).await;
        prompt
    };
    let runtime = app.state::<Arc<Runtime>>();
    let result = async {
        let _action = runtime
            .actions
            .try_lock()
            .map_err(|_| anyhow::anyhow!("正在处理其他操作，请稍后重试"))?;
        runtime.confirm_signal_source(&prompt, local).await
    }
    .await;
    let _gate = reminders.gate.lock().await;
    match result {
        Ok(_) => dismiss(app, ticket),
        Err(error) => {
            {
                let mut state = reminders.state.lock().unwrap();
                if let Some(active) = state.active.as_mut().filter(|a| a.ticket == ticket) {
                    active.busy = false;
                    active.error = Some(format!("{error:#}"));
                }
            }
            let _ = present(app).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unresolved_prompts_reopen_on_every_request_or_tray_entry_without_timer_spam() {
        let mut state = PromptTracker::default();
        // Main window visible; engine not ready yet. Neither consumes a prompt.
        assert_eq!(state.observe(Some((1, 1)), false, true, 0, false), None);
        assert_eq!(state.observe(Some((1, 1)), true, false, 0, false), None);
        let first = state.observe(Some((1, 1)), true, true, 0, false).unwrap();
        state.finish(first); // Explicit close, still unresolved.
        for _ in 0..10 {
            assert_eq!(state.observe(Some((1, 1)), true, true, 0, false), None);
        }
        let second = state.observe(Some((1, 1)), true, true, 1, false).unwrap();
        // More requests while already open don't stack windows.
        assert_eq!(state.observe(Some((1, 1)), true, true, 2, false), None);
        state.finish(second);
        assert_eq!(state.observe(Some((1, 1)), true, true, 2, false), None);
        assert_eq!(state.observe(Some((1, 1)), true, false, 2, false), None);
        let third = state.observe(Some((1, 1)), true, true, 2, false).unwrap();
        state.finish(third);
        assert_eq!(state.observe(Some((1, 1)), true, false, 2, false), None);
        assert_eq!(state.observe(Some((1, 1)), false, true, 2, false), None);
        let fourth = state.observe(Some((1, 1)), true, true, 2, false).unwrap();
        state.finish(fourth);
        assert!(state.observe(Some((1, 1)), true, true, 1, true).is_some());
    }

    #[test]
    fn busy_requests_wait_until_available_and_stale_closes_cannot_dismiss_new_tickets() {
        let mut state = PromptTracker::default();
        let first = state.observe(Some((1, 1)), true, true, 0, false).unwrap();
        state.finish(first);
        assert_eq!(state.observe(Some((1, 1)), false, true, 1, false), None);
        let second = state.observe(Some((1, 1)), true, true, 1, false).unwrap();
        state.finish(first);
        assert_eq!(state.open, Some(second));
        state.finish(second);
        assert_eq!(state.observe(None, true, true, 1, false), None);
        assert!(state.observe(Some((1, 2)), true, true, 1, false).is_some());
    }
}
