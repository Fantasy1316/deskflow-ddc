//! File work gets its own bounded executor. Network pacing reacts to control
//! latency, rather than applying an arbitrary permanent transfer speed cap.
use anyhow::Result;
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::Duration,
};
use tokio::time::Instant;

pub fn executor() -> Result<&'static tokio::runtime::Runtime> {
    // One process-wide executor, like Tauri's executor. Jobs are explicitly
    // cancelled/joined by Manager::shutdown; this is never dropped in async code.
    static WORKERS: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    WORKERS
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(4)
                .thread_name("file-transfer")
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| anyhow::anyhow!("无法创建文件传输线程池：{e}"))
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkView {
    pub control_rtt_ms: u64,
    pub baseline_rtt_ms: u64,
    pub send_limit_bytes_per_second: Option<u64>,
}
struct State {
    baseline: Option<Duration>,
    latest: Duration,
    rate: Option<u64>,
    next: Instant,
    sample_time: Instant,
    sample_bytes: u64,
    healthy: u8,
    congested: u8,
}
pub struct Network {
    state: Mutex<State>,
    bytes: AtomicU64,
}
impl Default for Network {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            state: Mutex::new(State {
                baseline: None,
                latest: Duration::ZERO,
                rate: None,
                next: now,
                sample_time: now,
                sample_bytes: 0,
                healthy: 0,
                congested: 0,
            }),
            bytes: AtomicU64::new(0),
        }
    }
}
impl Network {
    pub fn observe(&self, rtt: Duration, active: bool) {
        self.observe_at(rtt, active, Instant::now());
    }
    fn observe_at(&self, rtt: Duration, active: bool, now: Instant) {
        let bytes = self.bytes.load(Ordering::Relaxed);
        let mut s = self.state.lock().unwrap();
        s.latest = rtt;
        let baseline = s.baseline.map_or(rtt, |b| b.min(rtt));
        s.baseline = Some(baseline);
        let elapsed = now.saturating_duration_since(s.sample_time).as_secs_f64();
        let sent = bytes.saturating_sub(s.sample_bytes);
        let throughput = (sent as f64 / elapsed.max(0.1)) as u64;
        s.sample_time = now;
        s.sample_bytes = bytes;
        if !active {
            s.rate = None;
            s.next = now;
            s.healthy = 0;
            s.congested = 0;
        } else if rtt > baseline + Duration::from_millis(20) && rtt > Duration::from_millis(30) {
            s.healthy = 0;
            // Hashing, small-file handshakes and receive acknowledgements do
            // not establish bulk throughput. Never seed a limit from them.
            if sent < 128 * 1024 {
                s.congested = 0;
                return;
            }
            s.congested = s.congested.saturating_add(1);
            // Ignore isolated Wi-Fi/scheduler spikes, but react immediately to
            // severe queueing or two consecutive elevated latency samples.
            if s.congested >= 2 || rtt > baseline + Duration::from_millis(250) {
                let reference = s.rate.unwrap_or(throughput);
                s.rate = Some((reference * 4 / 5).max(128 * 1024));
                s.congested = 0;
            }
        } else if rtt <= baseline + Duration::from_millis(10) {
            s.congested = 0;
            s.healthy = s.healthy.saturating_add(1);
            if s.healthy >= 8 {
                // A long healthy period releases stale limits completely.
                // A fresh sustained latency rise can reinstate input priority.
                s.rate = None;
                s.next = now;
            } else if s.healthy % 2 == 0 {
                s.rate = s
                    .rate
                    .map(|rate| rate.saturating_add((rate / 4).max(256 * 1024)));
            }
        } else {
            s.healthy = 0;
            s.congested = 0;
        }
    }
    pub fn reset(&self) {
        let mut s = self.state.lock().unwrap();
        s.baseline = None;
        s.rate = None;
        s.next = Instant::now();
        s.healthy = 0;
        s.congested = 0;
        s.sample_time = s.next;
        s.sample_bytes = self.bytes.load(Ordering::Relaxed);
    }
    fn reserve_at(&self, bytes: usize, now: Instant) -> Option<Instant> {
        let mut s = self.state.lock().unwrap();
        let rate = s.rate?;
        let at = s.next.max(now);
        s.next = at + Duration::from_secs_f64(bytes as f64 / rate as f64);
        Some(at)
    }
    pub async fn pace(&self, bytes: usize) {
        if let Some(at) = self.reserve_at(bytes, Instant::now()) {
            tokio::time::sleep_until(at).await;
        }
    }
    pub fn sent(&self, bytes: usize) {
        self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }
    pub fn snapshot(&self) -> NetworkView {
        let s = self.state.lock().unwrap();
        NetworkView {
            control_rtt_ms: s.latest.as_millis() as u64,
            baseline_rtt_ms: s.baseline.unwrap_or_default().as_millis() as u64,
            send_limit_bytes_per_second: s.rate,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn congestion_backs_off_aggregate_file_sends_and_recovers_after_healthy_samples() {
        let n = Network::default();
        let now = Instant::now();
        n.observe_at(Duration::from_millis(3), false, now);
        n.sent(2 * 1024 * 1024);
        n.observe_at(
            Duration::from_millis(150),
            true,
            now + Duration::from_secs(1),
        );
        assert!(n.snapshot().send_limit_bytes_per_second.is_none());
        n.sent(2 * 1024 * 1024);
        n.observe_at(
            Duration::from_millis(150),
            true,
            now + Duration::from_secs(2),
        );
        let rate = n.snapshot().send_limit_bytes_per_second.unwrap();
        assert!(rate < 2 * 1024 * 1024);
        let a = n.reserve_at(32768, now).unwrap();
        let b = n.reserve_at(32768, now).unwrap();
        assert!(b > a); // Concurrent jobs share a single allowance.
        for i in 3..=4 {
            n.observe_at(Duration::from_millis(3), true, now + Duration::from_secs(i));
        }
        assert!(n.snapshot().send_limit_bytes_per_second.unwrap() > rate);
        for i in 5..=10 {
            n.observe_at(Duration::from_millis(3), true, now + Duration::from_secs(i));
        }
        assert!(n.snapshot().send_limit_bytes_per_second.is_none());
        n.observe_at(
            Duration::from_millis(3),
            false,
            now + Duration::from_secs(11),
        );
        assert!(n.reserve_at(32768, now).is_none());
    }
    #[test]
    fn intermittent_wifi_spikes_do_not_pin_a_healthy_sender_to_a_low_rate() {
        let n = Network::default();
        let now = Instant::now();
        n.observe_at(Duration::from_millis(3), false, now);
        for i in 1..=60 {
            n.sent(8 * 1024 * 1024);
            let rtt = if i % 5 == 0 { 100 } else { 4 };
            n.observe_at(
                Duration::from_millis(rtt),
                true,
                now + Duration::from_secs(i),
            );
            assert!(n.snapshot().send_limit_bytes_per_second.is_none());
        }
    }
    #[test]
    fn small_protocol_replies_cannot_seed_a_bulk_rate_limit_but_severe_bulk_delay_can() {
        let n = Network::default();
        let now = Instant::now();
        n.observe_at(Duration::from_millis(3), false, now);
        for i in 1..=4 {
            n.sent(1024);
            n.observe_at(
                Duration::from_millis(400),
                true,
                now + Duration::from_secs(i),
            );
            assert!(n.snapshot().send_limit_bytes_per_second.is_none());
        }
        n.sent(4 * 1024 * 1024);
        n.observe_at(
            Duration::from_millis(400),
            true,
            now + Duration::from_secs(5),
        );
        assert!(n.snapshot().send_limit_bytes_per_second.unwrap() < 4 * 1024 * 1024);
    }
    #[test]
    fn a_normally_slow_link_is_not_mistaken_for_new_congestion() {
        let n = Network::default();
        let now = Instant::now();
        n.observe_at(Duration::from_millis(90), false, now);
        n.observe_at(
            Duration::from_millis(95),
            true,
            now + Duration::from_secs(1),
        );
        assert!(n.snapshot().send_limit_bytes_per_second.is_none());
        n.observe_at(
            Duration::from_millis(200),
            true,
            now + Duration::from_secs(2),
        );
        assert!(
            n.snapshot().send_limit_bytes_per_second.is_none(),
            "no sent bytes must not throttle the next job"
        );
        n.reset();
        assert_eq!(n.snapshot().baseline_rtt_ms, 0);
    }
}

#[cfg(test)]
mod executor_tests {
    use super::*;
    #[tokio::test(flavor = "current_thread")]
    async fn file_work_cannot_occupy_the_control_executor_thread() {
        let (started, entered) = tokio::sync::oneshot::channel();
        let (release, wait) = tokio::sync::oneshot::channel();
        let job = executor().unwrap().spawn(async move {
            assert!(std::thread::current()
                .name()
                .unwrap()
                .starts_with("file-transfer"));
            started.send(()).unwrap();
            // Stand in for a slow synchronous hash/filesystem operation while
            // the single-thread control executor must continue scheduling.
            std::thread::sleep(Duration::from_millis(100));
            wait.await.unwrap();
        });
        entered.await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            release.send(()).unwrap();
            job.await.unwrap();
        })
        .await
        .unwrap();
    }
}
