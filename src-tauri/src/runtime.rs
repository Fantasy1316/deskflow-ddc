mod control;
mod engine_session;
mod pointer_bridge;
mod switch_flow;

use crate::{
    diagnostics::Diagnostics,
    discovery::{self, Device, Discovery},
    engine::{self, Engine},
    model::{platform, Config, Identity, Pin},
    platform::Platform,
    pointer::{self, Panel, Point},
    protocol::{self, Wire},
    session::{Incoming, Session},
    store::Store,
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc, watch, Mutex},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub host_side: String,
    pub video: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub fingerprint: String,
    pub input_configured: bool,
    pub retained_display: bool,
}
#[derive(Clone, Serialize)]
pub struct Log {
    pub id: u64,
    pub time: String,
    pub level: String,
    pub message: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub config: Config,
    pub device_id: String,
    pub platform: String,
    pub hostname: String,
    pub addresses: Vec<std::net::IpAddr>,
    pub discovered_devices: Vec<Device>,
    pub discovering: bool,
    pub cursor_displays: Vec<Panel>,
    pub session_layout: Option<Layout>,
    pub monitors: Value,
    pub running: bool,
    pub listening: bool,
    pub connected: bool,
    pub engine: String,
    pub engine_source: String,
    pub peer: Option<Peer>,
    pub active_device_id: Option<String>,
    pub display_device_id: Option<String>,
    pub last_display_device_id: Option<String>,
    pub display_status: String,
    pub switching: bool,
    pub pairing_code: String,
    pub has_saved_pairing: bool,
    pub dependencies: Dependencies,
    pub diagnostics: Value,
    pub logs: Vec<Log>,
    pub last_error: Option<String>,
    pub preview: bool,
    pub connection: Option<Value>,
}
impl Snapshot {
    fn new(store: &Store, diagnostics: &Diagnostics) -> Self {
        Self {
            revision: 0,
            config: store.value.config.clone(),
            device_id: store.value.id.to_string(),
            platform: platform().into(),
            hostname: format!("ddc-{}.local", store.value.id),
            addresses: discovery::local_addresses(),
            discovered_devices: vec![],
            discovering: false,
            cursor_displays: vec![],
            session_layout: None,
            monitors: json!([]),
            running: false,
            listening: false,
            connected: false,
            engine: "stopped".into(),
            engine_source: "bundled".into(),
            peer: None,
            active_device_id: None,
            display_device_id: None,
            last_display_device_id: None,
            display_status: "uninitialized".into(),
            switching: false,
            pairing_code: store.pairing.clone(),
            has_saved_pairing: store.value.trusted_peer.is_some(),
            dependencies: Dependencies {
                deskflow: false,
                display_control: false,
                accessibility: false,
            },
            diagnostics: json!({"directory":diagnostics.directory,"error":null}),
            logs: vec![],
            last_error: None,
            preview: false,
            connection: None,
        }
    }
    fn control_connected(&mut self, host: String) {
        self.connected = true;
        self.active_device_id = Some(host.clone());
        // Only the first connection after an explicit start assumes the host picture.
        // Recreating the input engine does not change the monitor's physical input.
        if self.display_status == "uninitialized"
            && self.display_device_id.is_none()
            && self.last_display_device_id.is_none()
        {
            self.display_device_id = Some(host.clone());
            self.last_display_device_id = Some(host);
        }
    }
    fn control_disconnected(&mut self) {
        if self.switching {
            // A DDC command may have reached the monitor before its acknowledgement
            // was lost. Keep the previous source for diagnostics, never for routing.
            self.display_device_id = None;
            self.display_status = "uncertain".into();
        }
        self.connected = false;
        self.engine = "stopped".into();
        self.peer = None;
        self.session_layout = None;
        self.active_device_id = None;
        self.switching = false;
        self.connection = None;
    }
    fn reset_display(&mut self) {
        self.display_device_id = None;
        self.last_display_device_id = None;
        self.display_status = "uninitialized".into();
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependencies {
    pub deskflow: bool,
    pub display_control: bool,
    pub accessibility: bool,
}
struct Inner {
    store: Store,
    state: Snapshot,
    diagnostics: Diagnostics,
    sequence: u64,
    diagnostic_line: Option<(String, u64, std::time::Instant)>,
}
struct Link {
    id: u64,
    epoch: u64,
    session: Session,
    peer: watch::Sender<Option<Identity>>,
}
#[derive(Clone, Debug)]
struct Transition {
    from: String,
    to: String,
}
#[derive(Clone, Debug)]
struct Completed {
    session: u64,
    to: String,
    error: Option<String>,
}
pub struct Runtime {
    inner: Mutex<Inner>,
    pub platform: Platform,
    discovery: Mutex<Option<Discovery>>,
    fingerprint: String,
    directory: PathBuf,
    pub updates: watch::Sender<Snapshot>,
    dirty: AtomicBool,
    epoch: AtomicU64,
    session_sequence: AtomicU64,
    active_session: AtomicU64,
    network: Mutex<Option<(CancellationToken, tokio::task::JoinHandle<()>)>>,
    pointer_busy: AtomicBool,
    topology_busy: AtomicBool,
    link: Mutex<Option<Arc<Link>>>,
    engine: Mutex<Option<Engine>>,
    pub actions: Mutex<()>,
    commands: Mutex<()>,
    effects: Mutex<()>,
    command_pending: AtomicBool,
    keyboard_pending: AtomicU64,
    start_stop: Mutex<()>,
    observed: broadcast::Sender<(u64, Transition)>,
    completed: broadcast::Sender<Completed>,
    pub shutdown: CancellationToken,
}
impl Runtime {
    pub async fn new(directory: PathBuf, resources: PathBuf) -> Result<Arc<Self>> {
        ensure!(platform() != "unsupported", "当前仅支持 macOS 和 Windows");
        let store = Store::load(directory.clone())?;
        let fingerprint = engine::identity(&directory)?;
        let diagnostics = Diagnostics::new(directory.join("logs"), store.pairing.clone())?;
        let state = Snapshot::new(&store, &diagnostics);
        let (updates, _) = watch::channel(state.clone());
        let (observed, _) = broadcast::channel(32);
        let (completed, _) = broadcast::channel(32);
        let runtime = Arc::new(Self {
            inner: Mutex::new(Inner {
                store,
                state,
                diagnostics,
                sequence: 0,
                diagnostic_line: None,
            }),
            platform: Platform::new(resources),
            discovery: Mutex::new(None),
            fingerprint,
            directory,
            updates,
            dirty: AtomicBool::new(true),
            epoch: AtomicU64::new(0),
            session_sequence: AtomicU64::new(0),
            active_session: AtomicU64::new(0),
            network: Mutex::new(None),
            pointer_busy: AtomicBool::new(false),
            topology_busy: AtomicBool::new(false),
            link: Mutex::new(None),
            engine: Mutex::new(None),
            actions: Mutex::new(()),
            commands: Mutex::new(()),
            effects: Mutex::new(()),
            command_pending: AtomicBool::new(false),
            keyboard_pending: AtomicU64::new(0),
            start_stop: Mutex::new(()),
            observed,
            completed,
            shutdown: CancellationToken::new(),
        });
        runtime.refresh().await?;
        runtime
            .trace(
                "app.started",
                json!({
                    "version":env!("CARGO_PKG_VERSION"),
                    "pid":std::process::id(),
                    "executable":std::env::current_exe().ok(),
                    "sharing":false,
                    "accessibility":runtime.snapshot().await.dependencies.accessibility,
                }),
            )
            .await;
        if let Err(error) = engine::assert_no_running_engine(&runtime.directory).await {
            runtime.log("error", &format!("启动检查：{error}")).await;
        }
        runtime
            .log(
                "info",
                "0.2 使用独立配置与身份；开始共享前请先停止旧版或独立 Deskflow 的共享。",
            )
            .await;
        let this = runtime.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! {
                    _ = this.shutdown.cancelled() => break,
                    _ = interval.tick() => {}
                }
                if this.dirty.swap(false, Ordering::AcqRel) {
                    let mut inner = this.inner.lock().await;
                    inner.state.revision += 1;
                    this.updates.send_replace(inner.state.clone());
                }
            }
        });
        runtime.start_pointer_poll();
        Ok(runtime)
    }
    pub async fn snapshot(&self) -> Snapshot {
        self.inner.lock().await.state.clone()
    }
    fn publish(&self) {
        self.dirty.store(true, Ordering::Release);
    }
    pub async fn log(&self, level: &str, message: &str) {
        let mut i = self.inner.lock().await;
        let message = message.replace(&i.store.pairing, "[REDACTED]");
        if level == "error" {
            i.state.last_error = Some(message.clone());
        }
        i.sequence += 1;
        let id = i.sequence;
        i.state.logs.insert(
            0,
            Log {
                id,
                time: chrono::Utc::now().to_rfc3339(),
                level: level.into(),
                message: message.clone(),
            },
        );
        i.state.logs.truncate(160);
        if let Err(e) = i
            .diagnostics
            .record("activity", json!({"level":level,"message":message}))
        {
            i.state.diagnostics["error"] = e.to_string().into();
        }
        self.publish();
    }
    pub async fn trace(&self, event: &str, details: Value) {
        let mut i = self.inner.lock().await;
        if let Err(e) = i.diagnostics.record(event, details) {
            i.state.diagnostics["error"] = e.to_string().into();
        }
        self.publish();
    }
    pub async fn refresh(&self) -> Result<()> {
        let c = self.snapshot().await.config;
        let path = self.binary(&c);
        let (control, accessibility, screens) = tokio::join!(
            self.platform.available(),
            self.platform.accessibility(),
            self.platform.screens()
        );
        let mut i = self.inner.lock().await;
        i.state.dependencies = Dependencies {
            deskflow: path.is_ok(),
            display_control: control,
            accessibility,
        };
        i.state.engine_source = if c.deskflow_path.is_empty() {
            "bundled"
        } else {
            "external"
        }
        .into();
        if let Ok(screens) = screens {
            i.state.cursor_displays = serde_json::from_value(screens)?;
        }
        i.state.addresses = discovery::local_addresses();
        self.publish();
        Ok(())
    }
    pub async fn check_accessibility(&self) -> Result<()> {
        let trusted = self.platform.accessibility().await;
        self.inner.lock().await.state.dependencies.accessibility = trusted;
        self.publish();
        Ok(())
    }
    fn binary(&self, c: &Config) -> Result<PathBuf> {
        if c.deskflow_path.is_empty() {
            engine::bundled_binary(&self.platform.resources, &self.directory)
        } else {
            let p = PathBuf::from(&c.deskflow_path);
            ensure!(p.is_absolute() && p.is_file(), "外部引擎路径无效");
            Ok(p)
        }
    }
    pub async fn save(&self, c: Config) -> Result<()> {
        c.validate()?;
        {
            let mut i = self.inner.lock().await;
            ensure!(!i.state.running, "请先停止共享，再修改配置");
            let old = i.store.value.config.clone();
            i.store.value.config = c.clone();
            if let Err(e) = i.store.save() {
                i.store.value.config = old;
                return Err(e);
            }
            i.state.config = c;
        }
        self.refresh().await?;
        self.log("success", "设置已保存").await;
        Ok(())
    }
    pub async fn scan(&self) -> Result<()> {
        ensure!(!self.snapshot().await.switching, "正在切换，请稍后扫描");
        let monitors = self.platform.list().await?;
        self.inner.lock().await.state.monitors = monitors;
        self.publish();
        Ok(())
    }
    pub async fn discover(&self) -> Result<()> {
        self.inner.lock().await.state.discovering = true;
        self.publish();
        let result = async {
            let mut discovery = self.discovery.lock().await;
            if discovery.is_none() {
                *discovery = Some(Discovery::new()?);
            }
            discovery
                .as_ref()
                .unwrap()
                .browse(Duration::from_secs(2))
                .await
        }
        .await;
        let mut i = self.inner.lock().await;
        i.state.discovering = false;
        if let Ok(devices) = &result {
            i.state.discovered_devices = devices
                .iter()
                .filter(|d| d.id != i.state.device_id)
                .cloned()
                .collect();
        }
        self.publish();
        result.map(|_| ())
    }
    pub async fn import_config(&self, file: PathBuf) -> Result<()> {
        let mut i = self.inner.lock().await;
        ensure!(!i.state.running, "请先停止共享");
        i.store.import_legacy_config(&file)?;
        i.state.has_saved_pairing = false;
        i.state.config = i.store.value.config.clone();
        self.publish();
        drop(i);
        self.refresh().await?;
        self.log(
            "info",
            "已导入旧版配置；0.2 使用独立身份，请重新配对。旧版文件未修改。",
        )
        .await;
        Ok(())
    }
    pub async fn rotate_pairing(&self) -> Result<()> {
        let mut i = self.inner.lock().await;
        ensure!(!i.state.running, "请先停止共享");
        let code = protocol::new_pairing();
        i.store.set_pairing(code.clone())?;
        i.diagnostics.protect(code.clone());
        i.state.pairing_code = code;
        i.state.has_saved_pairing = false;
        self.publish();
        Ok(())
    }
    fn current(&self, link: &Link) -> bool {
        self.epoch.load(Ordering::Acquire) == link.epoch
            && self.active_session.load(Ordering::Acquire) == link.id
            && !link.session.closed.is_cancelled()
    }
    async fn identity(&self) -> Result<Identity> {
        let i = self.inner.lock().await;
        let c = &i.state.config;
        c.ready()?;
        Ok(Identity {
            app_version: env!("CARGO_PKG_VERSION").into(),
            id: i.store.value.id,
            name: c.device_name.clone(),
            platform: platform().into(),
            fingerprint: self.fingerprint.clone(),
            role: c.role.clone(),
            local_input: c.local_input.unwrap(),
            peer_input: c.peer_input.unwrap(),
            shortcut: c.shortcut.clone(),
            clipboard: c.clipboard,
            clipboard_megabytes: c.clipboard_megabytes,
            deskflow_port: c.deskflow_port,
            layout_version: 1,
            layout_side: c.layout_side.clone(),
            cross_screen_video: c.cross_screen_video.clone(),
            retained_display: pointer::selected(&i.state.cursor_displays, &c.retained_display_id)
                .is_some(),
        })
    }
    pub async fn start(self: &Arc<Self>, pairing: Option<String>) -> Result<()> {
        let _lifecycle = self.start_stop.lock().await;
        ensure!(!self.snapshot().await.running, "共享已经启动");
        self.trace(
            "sharing.startRequested",
            json!({"version":env!("CARGO_PKG_VERSION")}),
        )
        .await;
        engine::assert_no_running_engine(&self.directory).await?;
        self.refresh().await?;
        let state = self.snapshot().await;
        let c = state.config;
        c.ready()?;
        self.binary(&c)?;
        ensure!(
            state.dependencies.display_control,
            "原生显示器桥未准备，请检查安装包"
        );
        ensure!(
            state.dependencies.accessibility,
            "请先在系统设置中授予 Deskflow DDC 辅助功能权限，再点击检测"
        );
        ensure!(
            c.retained_display_id.is_empty()
                || pointer::selected(&state.cursor_displays, &c.retained_display_id).is_some(),
            "所选独立屏幕已断开，请重新选择"
        );
        if c.role == "client" {
            let mut i = self.inner.lock().await;
            let pairing = pairing.filter(|p| !p.trim().is_empty());
            ensure!(
                pairing.is_some() || i.store.value.trusted_peer.is_some(),
                "首次连接请填写提供端配对码"
            );
            let code = pairing.unwrap_or_else(|| i.store.pairing.clone());
            protocol::pairing_key(code.trim())?;
            ensure!(
                !c.peer_address.is_empty()
                    || !c.fallback_ips.is_empty()
                    || (i.store.value.trusted_peer.is_some() && code.trim() == i.store.pairing),
                "请选择发现的电脑，或填写主机名和备用 IP"
            );
            if code.trim() != i.store.pairing {
                i.store.set_pairing(code.trim().into())?;
                i.diagnostics.protect(code.trim().into());
            }
            i.state.pairing_code = i.store.pairing.clone();
            i.state.has_saved_pairing = i.store.value.trusted_peer.is_some();
            self.publish();
        }
        let listener = if c.role == "host" {
            Some(
                TcpListener::bind(("0.0.0.0", c.control_port))
                    .await
                    .context("控制端口已被占用")?,
            )
        } else {
            None
        };
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let cancel = CancellationToken::new();

        {
            let mut i = self.inner.lock().await;
            i.state.running = true;
            i.state.listening = listener.is_some();
            i.state.last_error = None;
        }
        if listener.is_some() {
            let mut d = self.discovery.lock().await;
            if d.is_none() {
                match Discovery::new() {
                    Ok(v) => *d = Some(v),
                    Err(e) => {
                        drop(d);
                        self.log(
                            "error",
                            &format!("设备发现不可用，仍可使用主机名或 IP：{e}"),
                        )
                        .await;
                        d = self.discovery.lock().await;
                    }
                }
            }
            if let Some(d) = d.as_mut() {
                if let Err(e) = d.advertise(&state.device_id, &c) {
                    self.log("error", &format!("广播设备失败：{e}")).await;
                }
            }
        }
        let this = self.clone();
        let token = cancel.clone();
        let task = tokio::spawn(async move {
            if let Some(listener) = listener {
                this.host_loop(listener, epoch, cancel).await
            } else {
                this.client_loop(epoch, cancel).await
            }
        });
        *self.network.lock().await = Some((token, task));
        self.log(
            "info",
            if c.role == "host" {
                "等待另一台电脑连接；请保持提供端画面可见"
            } else {
                "正在解析提供端地址并尝试连接"
            },
        )
        .await;
        self.publish();
        Ok(())
    }
    pub async fn stop(&self) -> Result<()> {
        let _lifecycle = self.start_stop.lock().await;
        self.trace("sharing.stopRequested", json!({})).await;
        self.epoch.fetch_add(1, Ordering::AcqRel);
        let network = self.network.lock().await.take();
        if let Some((cancel, _)) = &network {
            cancel.cancel();
        }
        if let Some(link) = self.link.lock().await.take() {
            link.session.close();
        }
        self.active_session.store(0, Ordering::Release);
        if let Some((_, task)) = network {
            task.await.context("等待网络监听退出失败")?;
        }
        {
            let mut i = self.inner.lock().await;
            i.state.running = false;
            i.state.listening = false;
            i.state.connected = false;
        }
        if let Some(discovery) = self.discovery.lock().await.as_mut() {
            discovery.stop_advertising();
        }
        let _command = self.commands.lock().await;
        let _effects = self.effects.lock().await;
        let mut guard = self.engine.lock().await;
        if let Some(engine) = guard.as_mut() {
            let pid = engine.pid();
            engine.stop().await?;
            self.trace(
                "engine.stopped",
                json!({"pid":pid,"reason":"sharingStopped"}),
            )
            .await;
        }
        *guard = None;
        drop(guard);
        self.platform.close().await;
        {
            let mut i = self.inner.lock().await;
            i.state.engine = "stopped".into();
            i.state.peer = None;
            i.state.session_layout = None;
            i.state.active_device_id = None;
            i.state.reset_display();
            i.state.switching = false;
            i.state.connection = None;
        }
        self.command_pending.store(false, Ordering::Release);
        self.keyboard_pending.store(0, Ordering::Release);
        self.publish();
        Ok(())
    }
    pub async fn export(&self) -> Result<Value> {
        let mut state = serde_json::to_value(self.snapshot().await)?;
        state["pairingCode"] = "[REDACTED]".into();
        state["config"]["deskflowPath"] = "[LOCAL PATH]".into();
        let directory = self.inner.lock().await.diagnostics.directory.clone();
        let mut files = vec![];
        for i in 0..=4 {
            let name = if i == 0 {
                "deskflow-ddc.jsonl".into()
            } else {
                format!("deskflow-ddc.{i}.jsonl")
            };
            if let Ok(text) = std::fs::read_to_string(directory.join(&name)) {
                files.push(json!({"name":name,"content":text}));
            }
        }
        Ok(
            json!({"app":"deskflow-ddc","version":env!("CARGO_PKG_VERSION"),"runtime":"tauri2","exportedAt":chrono::Utc::now(),"state":state,"files":files}),
        )
    }
    pub async fn data_directory(&self) -> PathBuf {
        self.inner.lock().await.store.directory.clone()
    }
    pub async fn shortcut(&self) -> Option<String> {
        let s = self.snapshot().await;
        let manual = s
            .session_layout
            .as_ref()
            .is_some_and(|l| l.video == "manual");
        if !(manual || keep_mode(&s))
            || !s.connected
            || s.engine != "ready"
            || s.active_device_id.as_ref() != Some(&s.device_id)
        {
            return None;
        }
        if s.config.role == "host" {
            Some(s.config.shortcut)
        } else {
            self.link
                .lock()
                .await
                .as_ref()?
                .peer
                .borrow()
                .as_ref()
                .map(|p| p.shortcut.clone())
        }
    }
}
fn c_role(c: &Config) -> bool {
    c.role == "host"
}
fn target_id(v: &Value) -> Result<String> {
    let s = v.as_str().context("切换目标无效")?;
    uuid::Uuid::parse_str(s)?;
    Ok(s.into())
}
fn keep_mode(s: &Snapshot) -> bool {
    s.connected
        && s.session_layout
            .as_ref()
            .is_some_and(|l| l.host_side != "off" && l.video != "follow")
}
fn local_side(s: &Snapshot) -> &str {
    let side = s
        .session_layout
        .as_ref()
        .map_or("off", |l| l.host_side.as_str());
    if s.config.role == "host" {
        side
    } else {
        pointer::opposite(side)
    }
}
fn distance(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}
fn parse_transition(line: &str) -> Option<Transition> {
    let re =
        regex::Regex::new(r#"switch from "sb-([a-f0-9-]{36})" to "sb-([a-f0-9-]{36})""#).ok()?;
    let m = re.captures(line)?;
    Some(Transition {
        from: m[1].into(),
        to: m[2].into(),
    })
}
pub fn validate_peer(c: &Config, id: uuid::Uuid, pin: Option<&Pin>, peer: &Identity) -> Result<()> {
    peer.validate()?;
    ensure!(
        id != peer.id && c.role != peer.role,
        "两台电脑必须选择不同的键鼠角色"
    );
    if let Some(pin) = pin {
        ensure!(
            pin.id == peer.id && pin.fingerprint == peer.fingerprint,
            "对端身份已变化，请停止共享并重新配对"
        );
    }
    let (host_platform, shortcut, keep) = if c.role == "host" {
        (
            platform(),
            c.shortcut.as_str(),
            c.cross_screen_video == "manual"
                || c.layout_side != "off" && c.cross_screen_video == "keep",
        )
    } else {
        (
            peer.platform.as_str(),
            peer.shortcut.as_str(),
            peer.cross_screen_video == "manual"
                || peer.layout_side != "off" && peer.cross_screen_video == "keep",
        )
    };
    ensure!(
        !(keep
            && (host_platform == "win32" || peer.platform == "win32" || platform() == "win32")
            && shortcut.ends_with("F12")),
        "Windows 无法注册 F12 全局快捷键；请在提供端改为 Ctrl + Alt + F10 后重新连接"
    );
    ensure!(
        c.local_input == Some(peer.peer_input) && c.peer_input == Some(peer.local_input),
        "两端输入源映射不对应"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            directory: dir.path().into(),
            value: crate::store::Stored {
                version: 3,
                id: uuid::Uuid::new_v4(),
                config: Config::default(),
                trusted_peer: Some(Pin {
                    id: uuid::Uuid::new_v4(),
                    fingerprint: "a".repeat(64),
                }),
                last_verified_address: None,
            },
            pairing: protocol::new_pairing(),
        };
        let diagnostics = Diagnostics::new(dir.path().join("logs"), store.pairing.clone()).unwrap();
        Snapshot::new(&store, &diagnostics)
    }
    #[test]
    fn reconnect_retains_receiver_picture_independently_of_keyboard_owner() {
        let mut s = snapshot();
        s.control_connected("provider".into());
        s.display_device_id = Some("receiver".into());
        s.last_display_device_id = Some("receiver".into());
        s.display_status = "commanded".into();
        s.active_device_id = Some("receiver".into());
        s.control_disconnected();
        assert!(!s.connected);
        assert!(s.active_device_id.is_none());
        assert!(s.has_saved_pairing);
        s.control_connected("provider".into());
        assert_eq!(s.active_device_id.as_deref(), Some("provider"));
        assert_eq!(s.display_device_id.as_deref(), Some("receiver"));
        assert_eq!(s.last_display_device_id.as_deref(), Some("receiver"));
        assert_eq!(s.display_status, "commanded");
    }
    #[test]
    fn reconnect_preserves_uncertainty_until_explicit_restart() {
        let mut s = snapshot();
        s.last_display_device_id = Some("receiver".into());
        s.display_status = "uncertain".into();
        for _ in 0..2 {
            s.control_disconnected();
            s.control_connected("provider".into());
            assert!(s.display_device_id.is_none());
            assert_eq!(s.display_status, "uncertain");
            assert_eq!(s.last_display_device_id.as_deref(), Some("receiver"));
        }
        s.control_disconnected();
        s.reset_display();
        s.control_connected("provider".into());
        assert_eq!(s.display_device_id.as_deref(), Some("provider"));
    }
    #[test]
    fn disconnect_during_switch_never_assumes_ddc_failed_to_reach_monitor() {
        let mut s = snapshot();
        s.control_connected("provider".into());
        s.switching = true;
        s.control_disconnected();
        s.control_connected("provider".into());
        assert!(!s.switching);
        assert!(s.display_device_id.is_none());
        assert_eq!(s.display_status, "uncertain");
        assert_eq!(s.last_display_device_id.as_deref(), Some("provider"));
    }
    pub(super) fn peer() -> Identity {
        Identity {
            app_version: env!("CARGO_PKG_VERSION").into(),
            id: uuid::Uuid::new_v4(),
            name: "peer".into(),
            platform: "win32".into(),
            fingerprint: "a".repeat(64),
            role: "client".into(),
            local_input: 1,
            peer_input: 17,
            shortcut: "control+alt+F10".into(),
            clipboard: true,
            clipboard_megabytes: 4,
            deskflow_port: 24801,
            layout_version: 1,
            layout_side: "off".into(),
            cross_screen_video: "follow".into(),
            retained_display: false,
        }
    }
    #[test]
    fn pins_are_identity_not_hostname() {
        let p = peer();
        let c = Config {
            local_input: Some(17),
            peer_input: Some(1),
            ..Config::default()
        };
        let id = uuid::Uuid::new_v4();
        assert!(validate_peer(&c, id, None, &p).is_ok());
        assert!(validate_peer(
            &c,
            id,
            Some(&Pin {
                id: uuid::Uuid::new_v4(),
                fingerprint: p.fingerprint.clone()
            }),
            &p
        )
        .is_err());
        assert!(validate_peer(
            &c,
            id,
            Some(&Pin {
                id: p.id,
                fingerprint: "b".repeat(64)
            }),
            &p
        )
        .is_err());
    }
    #[test]
    fn rejects_legacy_or_mismatched_app_before_pairing_or_engine_start() {
        let c = Config {
            local_input: Some(17),
            peer_input: Some(1),
            ..Config::default()
        };
        let id = uuid::Uuid::new_v4();
        let mut p = peer();
        assert!(validate_peer(&c, id, None, &p).is_ok());
        p.app_version = "0.2.0".into();
        let error = validate_peer(&c, id, None, &p).unwrap_err().to_string();
        assert!(error.contains("两端应用版本不一致"));
        assert!(error.contains("阻止启动键鼠引擎"));
        // Published old builds did not send an application version at all.
        let mut legacy = serde_json::to_value(peer()).unwrap();
        legacy.as_object_mut().unwrap().remove("appVersion");
        let legacy: Identity = serde_json::from_value(legacy).unwrap();
        assert!(validate_peer(&c, id, None, &legacy)
            .unwrap_err()
            .to_string()
            .contains("旧版，未提供版本号"));
    }
    #[test]
    fn startup_snapshot_never_restores_a_running_input_session() {
        let s = snapshot();
        assert!(s.has_saved_pairing);
        assert!(!s.running && !s.listening && !s.connected);
        assert_eq!(s.engine, "stopped");
        assert!(s.peer.is_none());
        assert!(s.active_device_id.is_none());
    }
    #[test]
    fn parses_only_engine_screen_names() {
        assert!(parse_transition("switch from \"sb-11111111-1111-1111-1111-111111111111\" to \"sb-22222222-2222-2222-2222-222222222222\" at 5,40").is_some());
        assert!(parse_transition("switch from foo to bar").is_none());
    }
    #[test]
    fn manual_video_shortcut_checks_windows_receiver_even_with_edges_disabled() {
        let p = peer();
        let mut c = Config {
            local_input: Some(17),
            peer_input: Some(1),
            shortcut: "control+alt+F12".into(),
            ..Config::default()
        };
        assert!(validate_peer(&c, uuid::Uuid::new_v4(), None, &p).is_err());
        c.shortcut = "control+alt+F10".into();
        assert!(validate_peer(&c, uuid::Uuid::new_v4(), None, &p).is_ok());
    }
}
