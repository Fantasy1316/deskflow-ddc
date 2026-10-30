//! File transfers are independent of Deskflow and use per-offer secrets sent
//! only over the authenticated control session. Journals never contain keys.
mod commit;
mod delivery;
pub use delivery::{Conflict, Decision};
mod disk;
mod manifest;
mod metrics;
mod scheduling;
mod stream;
#[cfg(test)]
mod tests;
mod wire;
use anyhow::{ensure, Context, Result};
use manifest::Manifest;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Offer {
    pub id: Uuid,
    pub label: String,
    pub entries: usize,
    pub bytes: u64,
    pub hash: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Invitation {
    pub offer: Offer,
    pub port: u16,
    pub secret: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: Uuid,
    pub label: String,
    pub direction: String,
    pub status: String,
    pub peer_name: String,
    pub entries: usize,
    pub total_bytes: u64,
    pub completed_bytes: u64,
    pub current_file: String,
    pub destination: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub conflict: Option<Conflict>,
    #[serde(default)]
    pub performance: metrics::View,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Record {
    view: View,
    peer: Uuid,
    offer: Offer,
    manifest: Option<Manifest>,
    target: Option<PathBuf>,
}
#[derive(Clone)]
struct Endpoint {
    address: SocketAddr,
    secret: [u8; 32],
}
struct Job {
    record: Mutex<Record>,
    metrics: Mutex<Arc<metrics::Metrics>>,
    endpoint: Mutex<Option<Endpoint>>,
    running: AtomicBool,
    cleaning: AtomicBool,
    cancel: Mutex<CancellationToken>,
    choice: Mutex<Option<(Uuid, tokio::sync::oneshot::Sender<Decision>)>>,
}
pub struct Manager {
    directory: PathBuf,
    jobs: Mutex<HashMap<Uuid, Arc<Job>>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    closing: AtomicBool,
    suspended: AtomicBool,
    preparing: AtomicBool,
    lifecycle: Mutex<()>,
    scan_cancel: CancellationToken,
    warnings: Vec<String>,
    network: Arc<scheduling::Network>,
}
fn terminal(s: &str) -> bool {
    matches!(s, "complete" | "cancelled")
}
impl Manager {
    pub fn new(directory: PathBuf) -> Result<Arc<Self>> {
        scheduling::executor()?;
        std::fs::create_dir_all(&directory)?;
        let mut jobs = HashMap::new();
        let mut warnings = Vec::new();
        let mut loaded_bytes = 0u64;
        for file in std::fs::read_dir(&directory)? {
            let file = file?;
            if file.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let result: Result<Record> = (|| {
                ensure!(jobs.len() < 100, "已达到 100 个任务的加载上限");
                let size = file.metadata()?.len();
                ensure!(
                    size <= 64 * 1024 * 1024 && loaded_bytes + size <= 128 * 1024 * 1024,
                    "任务记录超过加载上限"
                );
                loaded_bytes += size;
                let mut record: Record = serde_json::from_slice(&std::fs::read(file.path())?)?;
                ensure!(
                    record.offer.id.to_string()
                        == file
                            .path()
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or_default(),
                    "任务记录 ID 不一致"
                );
                ensure!(
                    record.view.id == record.offer.id
                        && matches!(record.view.direction.as_str(), "send" | "receive")
                        && matches!(
                            record.view.status.as_str(),
                            "pending"
                                | "waiting"
                                | "connecting"
                                | "transferring"
                                | "conflict"
                                | "paused"
                                | "complete"
                                | "cancelled"
                        )
                        && record.offer.entries > 0
                        && record.offer.entries <= manifest::MAX_ENTRIES
                        && record.offer.bytes <= 9_007_199_254_740_991
                        && crate::model::valid_fingerprint(&record.offer.hash)
                        && record.target.as_ref().is_none_or(|p| p.is_absolute()),
                    "任务记录无效"
                );
                if let Some(m) = &record.manifest {
                    let entries: Vec<_> = m.sources.iter().map(|s| s.entry.clone()).collect();
                    ensure!(
                        manifest::validate(&entries)? == record.offer.bytes
                            && entries.len() == record.offer.entries
                            && manifest::hash(&entries)? == record.offer.hash
                            && m.hash == record.offer.hash
                            && m.bytes == record.offer.bytes
                            && m.sources.iter().all(|s| s.source.is_absolute()),
                        "发送清单损坏"
                    );
                }
                if !terminal(&record.view.status) {
                    record.view.conflict = None;
                    record.view.status = "paused".into();
                    record.view.error =
                        Some("应用已重启，请由发送端重新发送此任务，再在接收端继续接收".into());
                }
                Ok(record)
            })();
            match result {
                Ok(record) => {
                    jobs.insert(
                        record.offer.id,
                        Arc::new(Job {
                            record: Mutex::new(record),
                            metrics: Mutex::new(Arc::new(metrics::Metrics::default())),
                            endpoint: Mutex::new(None),
                            running: AtomicBool::new(false),
                            cleaning: AtomicBool::new(false),
                            cancel: Mutex::new(CancellationToken::new()),
                            choice: Mutex::new(None),
                        }),
                    );
                }
                Err(_) => {
                    if warnings.len() < 10 {
                        warnings.push(format!(
                            "文件任务记录 {} 无法加载，已保留原记录和接收缓存；其他任务仍可使用。",
                            file.file_name().to_string_lossy()
                        ));
                    }
                }
            }
        }
        Ok(Arc::new(Self {
            directory,
            jobs: Mutex::new(jobs),
            tasks: Mutex::new(vec![]),
            closing: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
            preparing: AtomicBool::new(false),
            lifecycle: Mutex::new(()),
            scan_cancel: CancellationToken::new(),
            warnings,
            network: Arc::new(scheduling::Network::default()),
        }))
    }
    pub fn network_snapshot(&self) -> scheduling::NetworkView {
        self.network.snapshot()
    }
    pub fn reset_network(&self) {
        self.network.reset();
    }
    pub fn observe_control_latency(&self, latency: Duration) {
        let active = self.jobs.lock().unwrap().values().any(|j| {
            j.running.load(Ordering::Acquire) && {
                let record = j.record.lock().unwrap();
                record.view.status == "transferring" && record.view.direction == "send"
            }
        });
        self.network.observe(latency, active);
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn snapshot(&self) -> Vec<View> {
        let jobs = self.jobs.lock().unwrap();
        let mut views: Vec<_> = jobs
            .values()
            .map(|j| {
                let mut view = j.record.lock().unwrap().view.clone();
                if j.running.load(Ordering::Acquire) {
                    view.performance = j.metrics.lock().unwrap().snapshot();
                }
                view
            })
            .collect();
        views.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        views
    }
    fn job(&self, id: Uuid) -> Result<Arc<Job>> {
        self.jobs
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .context("文件任务不存在")
    }
    fn save(&self, job: &Job) -> Result<()> {
        let r = job.record.lock().unwrap();
        crate::store::atomic_write(
            &self.directory.join(format!("{}.json", r.offer.id)),
            &serde_json::to_vec(&*r)?,
        )
    }
    fn add(&self, record: Record) -> Result<Arc<Job>> {
        let mut jobs = self.jobs.lock().unwrap();
        ensure!(!self.closing.load(Ordering::Acquire), "应用正在退出");
        ensure!(
            jobs.len() < 100,
            "最多保留 100 个文件任务，请移除已完成任务"
        );
        ensure!(!jobs.contains_key(&record.offer.id), "文件任务已存在");
        let job = Arc::new(Job {
            record: Mutex::new(record),
            metrics: Mutex::new(Arc::new(metrics::Metrics::default())),
            endpoint: Mutex::new(None),
            running: AtomicBool::new(false),
            cleaning: AtomicBool::new(false),
            cancel: Mutex::new(CancellationToken::new()),
            choice: Mutex::new(None),
        });
        self.save(&job)?;
        jobs.insert(job.record.lock().unwrap().offer.id, job.clone());
        Ok(job)
    }
    pub async fn prepare(
        self: &Arc<Self>,
        paths: Vec<PathBuf>,
        peer: Uuid,
        name: String,
    ) -> Result<Uuid> {
        ensure!(!self.closing.load(Ordering::Acquire), "应用正在退出");
        ensure!(
            !self.suspended.load(Ordering::Acquire),
            "仅切换显示器模式不支持文件传输"
        );
        ensure!(
            !self.preparing.swap(true, Ordering::AcqRel),
            "正在扫描文件，请稍候"
        );
        let c = self.scan_cancel.child_token();
        let result = scheduling::executor()?
            .spawn_blocking(move || manifest::scan(paths, &c))
            .await;
        self.preparing.store(false, Ordering::Release);
        let manifest = result??;
        let id = Uuid::new_v4();
        let label = manifest.sources.first().unwrap().entry.path.clone();
        let offer = Offer {
            id,
            label: if manifest
                .sources
                .iter()
                .filter(|s| !s.entry.path.contains('/'))
                .count()
                > 1
            {
                format!("{label} 等多个项目")
            } else {
                label
            },
            entries: manifest.sources.len(),
            bytes: manifest.bytes,
            hash: manifest.hash.clone(),
        };
        let record = Record {
            view: View {
                id,
                label: offer.label.clone(),
                direction: "send".into(),
                status: "paused".into(),
                peer_name: name,
                entries: offer.entries,
                total_bytes: offer.bytes,
                completed_bytes: 0,
                current_file: String::new(),
                destination: None,
                error: None,
                performance: metrics::View::default(),
                created_at: chrono::Utc::now().to_rfc3339(),
                conflict: None,
            },
            peer,
            offer,
            manifest: Some(manifest),
            target: None,
        };
        let _lifecycle = self.lifecycle.lock().unwrap();
        ensure!(
            !self.suspended.load(Ordering::Acquire),
            "仅切换显示器模式不支持文件传输"
        );
        self.add(record)?;
        Ok(id)
    }
    pub fn incoming(
        &self,
        invite: Invitation,
        address: IpAddr,
        peer: Uuid,
        name: String,
    ) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().unwrap();
        ensure!(!self.closing.load(Ordering::Acquire), "应用正在退出");
        ensure!(
            !self.suspended.load(Ordering::Acquire),
            "仅切换显示器模式不支持文件传输"
        );
        let o = &invite.offer;
        ensure!(
            o.entries > 0
                && o.entries <= manifest::MAX_ENTRIES
                && o.bytes <= 9_007_199_254_740_991
                && o.label.len() <= 1024
                && crate::model::valid_fingerprint(&o.hash)
                && invite.secret.len() == 64
                && invite.port >= 1024,
            "文件邀请无效"
        );
        let secret: [u8; 32] = hex::decode(&invite.secret)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("文件认证参数无效"))?;
        let endpoint = Endpoint {
            address: SocketAddr::new(address, invite.port),
            secret,
        };
        let existing = self.jobs.lock().unwrap().get(&o.id).cloned();
        let job = if let Some(job) = existing {
            ensure!(!job.running.load(Ordering::Acquire), "文件任务仍在运行");
            {
                let mut r = job.record.lock().unwrap();
                ensure!(
                    r.peer == peer
                        && r.offer == *o
                        && r.view.direction == "receive"
                        && r.view.status != "cancelled",
                    "文件任务与原配对身份或清单不一致"
                );
                r.view.status = "pending".into();
                r.view.error = None;
            }
            job
        } else {
            self.add(Record {
                view: View {
                    id: o.id,
                    label: o.label.clone(),
                    direction: "receive".into(),
                    status: "pending".into(),
                    peer_name: name,
                    entries: o.entries,
                    total_bytes: o.bytes,
                    completed_bytes: 0,
                    current_file: String::new(),
                    destination: None,
                    error: None,
                    performance: metrics::View::default(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    conflict: None,
                },
                peer,
                offer: o.clone(),
                manifest: None,
                target: None,
            })?
        };
        *job.endpoint.lock().unwrap() = Some(endpoint);
        self.save(&job)?;
        Ok(())
    }
    fn begin(&self, job: &Job, status: &str) -> Result<CancellationToken> {
        ensure!(!self.closing.load(Ordering::Acquire), "应用正在退出");
        ensure!(
            !self.suspended.load(Ordering::Acquire),
            "仅切换显示器模式不支持文件传输"
        );
        ensure!(
            !terminal(&job.record.lock().unwrap().view.status),
            "任务已结束"
        );
        ensure!(
            self.jobs
                .lock()
                .unwrap()
                .values()
                .filter(|j| j.running.load(Ordering::Acquire))
                .count()
                < 4,
            "最多同时运行 4 个文件任务"
        );
        ensure!(
            !job.running.swap(true, Ordering::AcqRel),
            "文件任务仍在运行"
        );
        *job.metrics.lock().unwrap() = Arc::new(metrics::Metrics::default());
        job.metrics.lock().unwrap().start();
        let token = CancellationToken::new();
        *job.cancel.lock().unwrap() = token.clone();
        {
            let mut r = job.record.lock().unwrap();
            r.view.status = status.into();
            r.view.error = None;
            r.view.conflict = None;
        }
        if let Err(e) = self.save(job) {
            job.running.store(false, Ordering::Release);
            return Err(e);
        }
        Ok(token)
    }
    fn progress(job: Arc<Job>) -> stream::Progress {
        Arc::new(move |done, file| {
            let mut r = job.record.lock().unwrap();
            r.view.completed_bytes = done;
            r.view.current_file = file;
        })
    }
    fn spawn<F>(self: &Arc<Self>, job: Arc<Job>, cancel: CancellationToken, work: F)
    where
        F: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let mut tasks = self.tasks.lock().unwrap();
        let manager = self.clone();
        let task = scheduling::executor().expect("initialized file executor").spawn(async move {
            let result = tokio::select! {biased;_=cancel.cancelled()=>Err(anyhow::anyhow!("传输已暂停")),result=work=>result};
            let _lifecycle = manager.lifecycle.lock().unwrap();
            {
                let mut r = job.record.lock().unwrap();
                r.view.performance = job.metrics.lock().unwrap().finish();
                r.view.conflict = None;
                if !terminal(&r.view.status) {
                    match result {
                        Ok(()) => {
                            r.view.status = "complete".into();
                            r.view.error = None;
                            r.view.completed_bytes = r.offer.bytes;
                        }
                        Err(e) => {
                            r.view.status = "paused".into();
                            r.view.error = Some(format!("{e:#}"));
                        }
                    }
                }
            }
            if let Err(e) = manager.save(&job) {
                job.record.lock().unwrap().view.error = Some(format!("任务状态保存失败：{e}"));
            }
            job.running.store(false, Ordering::Release);
        });
        tasks.retain(|t| !t.is_finished());
        tasks.push(task);
    }
    pub async fn offer(
        self: &Arc<Self>,
        id: Uuid,
        peer: Uuid,
        address: IpAddr,
    ) -> Result<Invitation> {
        let job = self.job(id)?;
        {
            let _lifecycle = self.lifecycle.lock().unwrap();
            let r = job.record.lock().unwrap();
            ensure!(
                r.peer == peer && r.view.direction == "send" && !terminal(&r.view.status),
                "只能向原配对设备继续发送未完成任务"
            );
            if job.running.load(Ordering::Acquire) {
                ensure!(r.view.status == "waiting", "任务正在传输，请先暂停");
                job.cancel.lock().unwrap().cancel();
            }
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while job.running.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .context("原传输仍在停止，请稍后重试")?;
        let listener = TcpListener::bind(("0.0.0.0", 0))
            .await
            .context("无法创建文件端口")?;
        let _lifecycle = self.lifecycle.lock().unwrap();
        let record = job.record.lock().unwrap().clone();
        ensure!(
            record.peer == peer
                && record.view.direction == "send"
                && !terminal(&record.view.status),
            "只能向原配对设备继续发送未完成任务"
        );
        let manifest = record.manifest.context("发送清单不可用")?;
        let port = listener.local_addr()?.port();
        let secret_text = crate::protocol::nonce();
        let secret: [u8; 32] = hex::decode(&secret_text)?.try_into().unwrap();
        let cancel = self.begin(&job, "waiting")?;
        let worker_job = job.clone();
        let token = cancel.clone();
        let offer = record.offer.clone();
        let network = self.network.clone();
        self.spawn(job, cancel, async move {
            loop {
                let (socket, remote) = listener.accept().await?;
                if remote.ip() != address {
                    continue;
                }
                let result = tokio::time::timeout(
                    Duration::from_secs(10),
                    wire::Wire::handshake(socket, &secret, true),
                )
                .await;
                let Ok(Ok(mut w)) = result else { continue };
                w.configure(network.clone(), worker_job.metrics.lock().unwrap().clone());
                {
                    let mut r = worker_job.record.lock().unwrap();
                    r.view.status = "transferring".into();
                    r.view.error = None;
                }
                let result = stream::send(
                    &mut w,
                    &offer,
                    &manifest,
                    &token,
                    Self::progress(worker_job.clone()),
                )
                .await;
                match result {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        if e.downcast_ref::<manifest::SourceFailure>().is_some()
                            || !wire::connection_error(&e)
                        {
                            return Err(e);
                        }
                        let mut r = worker_job.record.lock().unwrap();
                        r.view.status = "waiting".into();
                        r.view.error = Some(format!("连接中断，等待接收端继续：{e}"));
                    }
                }
            }
        });
        Ok(Invitation {
            offer: record.offer,
            port,
            secret: secret_text,
        })
    }
    pub async fn accept(self: &Arc<Self>, id: Uuid, destination: Option<PathBuf>) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().unwrap();
        let job = self.job(id)?;
        ensure!(!job.running.load(Ordering::Acquire), "文件任务仍在运行");
        let endpoint = job
            .endpoint
            .lock()
            .unwrap()
            .clone()
            .context("发送端已离线，请先在发送端重新发送此任务")?;
        {
            let mut r = job.record.lock().unwrap();
            ensure!(
                r.view.direction == "receive" && r.view.status != "cancelled",
                "不能接收此任务"
            );
            if r.target.is_none() {
                let path = destination.context("请选择接收目录")?;
                ensure!(path.is_dir(), "接收目录不可用");
                r.target = Some(std::fs::canonicalize(path)?);
            }
        }
        let cancel = self.begin(&job, "connecting")?;
        let record = job.record.lock().unwrap().clone();
        let worker_job = job.clone();
        let token = cancel.clone();
        let network = self.network.clone();
        let journal = self.directory.join(format!("{id}.delivery"));
        self.spawn(job, cancel, async move {
            let mut last = None;
            for attempt in 0..3 {
                if attempt > 0 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                let result: Result<PathBuf> = async {
                    let socket = tokio::time::timeout(
                        Duration::from_secs(10),
                        TcpStream::connect(endpoint.address),
                    )
                    .await
                    .context("文件连接超时")??;
                    let mut w = tokio::time::timeout(
                        Duration::from_secs(10),
                        wire::Wire::handshake(socket, &endpoint.secret, false),
                    )
                    .await
                    .context("文件认证超时")??;
                    w.configure(network.clone(), worker_job.metrics.lock().unwrap().clone());
                    {
                        let mut r = worker_job.record.lock().unwrap();
                        r.view.status = "transferring".into();
                        r.view.error = None;
                    }
                    let committed_job = worker_job.clone();
                    stream::receive(
                        &mut w,
                        &record,
                        &token,
                        Self::progress(worker_job.clone()),
                        Arc::new(move |path| {
                            let mut r = committed_job.record.lock().unwrap();
                            r.view.status = "complete".into();
                            r.view.error = None;
                            r.view.destination = Some(path.to_string_lossy().into());
                            r.view.completed_bytes = r.offer.bytes;
                        }),
                        &delivery::Options {
                            journal: journal.clone(),
                            resolve: Self::conflict_resolver(worker_job.clone()),
                        },
                    )
                    .await
                }
                .await;
                match result {
                    Ok(path) => {
                        worker_job.record.lock().unwrap().view.destination =
                            Some(path.to_string_lossy().into());
                        return Ok(());
                    }
                    Err(e) => {
                        if !wire::connection_error(&e) {
                            return Err(e);
                        }
                        last = Some(e);
                    }
                }
            }
            Err(last.unwrap()).context("传输已暂停，可重新发送并继续接收")
        });
        Ok(())
    }
    fn conflict_resolver(job: Arc<Job>) -> delivery::Resolver {
        Arc::new(move |conflict| {
            let job = job.clone();
            Box::pin(async move {
                let (send, receive) = tokio::sync::oneshot::channel();
                let token = conflict.token;
                {
                    let mut choice = job.choice.lock().unwrap();
                    ensure!(choice.is_none(), "已有待处理的同名冲突");
                    *choice = Some((token, send));
                    let mut r = job.record.lock().unwrap();
                    r.view.status = "conflict".into();
                    r.view.conflict = Some(conflict);
                }
                struct Waiting(Arc<Job>, Uuid);
                impl Drop for Waiting {
                    fn drop(&mut self) {
                        let mut choice = self.0.choice.lock().unwrap();
                        if choice.as_ref().is_some_and(|(id, _)| *id == self.1) {
                            *choice = None;
                        }
                        let mut r = self.0.record.lock().unwrap();
                        if r.view.conflict.as_ref().is_some_and(|c| c.token == self.1) {
                            r.view.conflict = None;
                            if r.view.status == "conflict" {
                                r.view.status = "transferring".into();
                            }
                        }
                    }
                }
                let _waiting = Waiting(job, token);
                receive.await.context("同名冲突选择已结束")
            })
        })
    }
    pub fn resolve_conflict(&self, id: Uuid, token: Uuid, decision: Decision) -> Result<()> {
        let job = self.job(id)?;
        let mut choice = job.choice.lock().unwrap();
        ensure!(
            choice
                .as_ref()
                .is_some_and(|(current, _)| *current == token),
            "此同名冲突已失效，请刷新后重试"
        );
        choice
            .take()
            .unwrap()
            .1
            .send(decision)
            .map_err(|_| anyhow::anyhow!("此同名冲突已结束"))
    }
    pub fn needs_destination(&self, id: Uuid) -> Result<bool> {
        Ok(self.job(id)?.record.lock().unwrap().target.is_none())
    }
    pub fn cancellation_peer(&self, id: Uuid) -> Result<Option<Uuid>> {
        let job = self.job(id)?;
        let r = job.record.lock().unwrap();
        Ok((r.view.status != "complete").then_some(r.peer))
    }
    pub async fn cancel_from_peer(&self, id: Uuid, peer: Uuid) -> Result<()> {
        let job = self.jobs.lock().unwrap().get(&id).cloned();
        let Some(job) = job else { return Ok(()) }; // Idempotent after removal.
        ensure!(
            job.record.lock().unwrap().peer == peer,
            "只能取消原配对设备的文件任务"
        );
        self.cancel(id).await
    }
    pub fn pause(&self, id: Uuid) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().unwrap();
        let job = self.job(id)?;
        if terminal(&job.record.lock().unwrap().view.status) {
            return Ok(());
        }
        job.cancel.lock().unwrap().cancel();
        if !job.running.load(Ordering::Acquire) {
            job.record.lock().unwrap().view.status = "paused".into();
            self.save(&job)?;
        }
        Ok(())
    }
    pub fn pause_all(&self) {
        self.pause_all_with_reason("配对已变更，请连接原配对设备后重新发送此任务");
    }
    fn pause_all_with_reason(&self, reason: &str) {
        let _lifecycle = self.lifecycle.lock().unwrap();
        for job in self.jobs.lock().unwrap().values() {
            job.cancel.lock().unwrap().cancel();
            *job.endpoint.lock().unwrap() = None;
            if !terminal(&job.record.lock().unwrap().view.status)
                && !job.running.load(Ordering::Acquire)
            {
                let mut r = job.record.lock().unwrap();
                r.view.status = "paused".into();
                r.view.error = Some(reason.into());
                drop(r);
                let _ = self.save(job);
            }
        }
    }
    // The caller stops the control session first. The gate rejects stale file
    // actions without blocking emergency stop/quit behind a file dialog or scan.
    // Unlike shutdown(), this leaves the manager usable after returning to sharing.
    pub async fn suspend(&self) {
        {
            let _lifecycle = self.lifecycle.lock().unwrap();
            self.suspended.store(true, Ordering::Release);
        }
        let reason = "仅切换显示器模式已暂停传输；恢复完整共享并连接原配对设备后可继续";
        self.pause_all_with_reason(reason);
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap());
        for task in tasks {
            let _ = task.await;
        }
        self.pause_all_with_reason(reason);
        self.reset_network();
    }
    pub fn enable(&self) {
        let _lifecycle = self.lifecycle.lock().unwrap();
        self.suspended.store(false, Ordering::Release);
    }
    pub async fn cancel(&self, id: Uuid) -> Result<()> {
        let job = self.job(id)?;
        {
            let _lifecycle = self.lifecycle.lock().unwrap();
            if job.record.lock().unwrap().view.status == "complete" {
                return Ok(());
            }
            ensure!(
                !job.cleaning.swap(true, Ordering::AcqRel),
                "正在清理接收缓存，请稍候"
            );
            job.record.lock().unwrap().view.status = "cancelled".into();
            job.cancel.lock().unwrap().cancel();
        }
        struct Cleaning<'a>(&'a AtomicBool);
        impl Drop for Cleaning<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _cleaning = Cleaning(&job.cleaning);
        self.save(&job)?;
        while job.running.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let record = job.record.lock().unwrap().clone();
        if record.view.direction == "receive" && record.target.is_some() {
            let journal = self.directory.join(format!("{id}.delivery"));
            // A process may have stopped after publication but before saving
            // its UI state. A completed durable receipt always wins over cancel.
            if delivery::completed(&record, &journal)? {
                {
                    let mut r = job.record.lock().unwrap();
                    r.view.status = "complete".into();
                    r.view.error = None;
                    r.view.destination = r.target.as_ref().map(|p| p.to_string_lossy().into());
                    r.view.completed_bytes = r.offer.bytes;
                }
                self.save(&job)?;
                return Ok(());
            }
            if let Err(e) = delivery::cancel(&record, &journal) {
                let mut r = job.record.lock().unwrap();
                r.view.status = "paused".into();
                r.view.error = Some(format!("缓存含待恢复的原文件备份，已保留：{e:#}"));
                drop(r);
                self.save(&job)?;
                return Err(e);
            }
            let stage = stream::stage_path(&record)?;
            if stage.try_exists()? {
                stream::real_directory(&stage)?;
                tokio::fs::remove_dir_all(stage).await?;
            }
        }
        if job.record.lock().unwrap().view.status == "cancelled" {
            job.record.lock().unwrap().target = None;
        }
        self.save(&job)?;
        Ok(())
    }
    pub fn remove(&self, id: Uuid) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().unwrap();
        let job = self.job(id)?;
        ensure!(
            !job.running.load(Ordering::Acquire)
                && !job.cleaning.load(Ordering::Acquire)
                && terminal(&job.record.lock().unwrap().view.status),
            "请先取消任务再移除"
        );
        std::fs::remove_file(self.directory.join(format!("{id}.json")))?;
        let receipt = self.directory.join(format!("{id}.delivery"));
        if receipt.try_exists()? {
            std::fs::remove_file(receipt)?;
        }
        self.jobs.lock().unwrap().remove(&id);
        Ok(())
    }
    pub fn destination(&self, id: Uuid) -> Result<PathBuf> {
        let job = self.job(id)?;
        let r = job.record.lock().unwrap();
        ensure!(
            r.view.status == "complete" && r.view.direction == "receive",
            "文件尚未接收完成"
        );
        Ok(PathBuf::from(
            r.view.destination.as_ref().context("接收位置不可用")?,
        ))
    }
    pub async fn shutdown(&self) {
        let tasks = {
            let _lifecycle = self.lifecycle.lock().unwrap();
            self.closing.store(true, Ordering::Release);
            self.scan_cancel.cancel();
            for job in self.jobs.lock().unwrap().values() {
                job.cancel.lock().unwrap().cancel();
            }
            std::mem::take(&mut *self.tasks.lock().unwrap())
        };
        for task in tasks {
            let _ = task.await;
        }
    }
}
