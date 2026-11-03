//! Cumulative stage timings, not file contents or keys. Stages may overlap
//! between peers; these are diagnostic timings, not a bandwidth benchmark.
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct View {
    pub elapsed_ms: u64,
    pub sent_bytes: u64,
    pub received_bytes: u64,
    pub encrypt_ms: u64,
    pub decrypt_ms: u64,
    pub network_read_ms: u64,
    pub network_write_ms: u64,
    pub file_read_ms: u64,
    pub file_write_ms: u64,
    pub hash_ms: u64,
    pub sync_ms: u64,
    pub pacing_ms: u64,
}
#[derive(Default)]
pub struct Metrics {
    started: std::sync::Mutex<Option<Instant>>,
    pub sent: AtomicU64,
    pub received: AtomicU64,
    pub encrypt: AtomicU64,
    pub decrypt: AtomicU64,
    pub network_read: AtomicU64,
    pub network_write: AtomicU64,
    pub file_read: AtomicU64,
    pub file_write: AtomicU64,
    pub hash: AtomicU64,
    pub sync: AtomicU64,
    pub pacing: AtomicU64,
}
pub fn elapsed(counter: &AtomicU64, start: Instant) {
    counter.fetch_add(start.elapsed().as_micros() as u64, Ordering::Relaxed);
}
impl Metrics {
    pub fn start(&self) {
        *self.started.lock().unwrap() = Some(Instant::now());
    }
    pub fn finish(&self) -> View {
        let v = self.snapshot();
        *self.started.lock().unwrap() = None;
        v
    }
    pub fn snapshot(&self) -> View {
        let ms = |v: &AtomicU64| v.load(Ordering::Relaxed) / 1000;
        View {
            elapsed_ms: self
                .started
                .lock()
                .unwrap()
                .map_or(0, |t| t.elapsed().as_millis() as u64),
            sent_bytes: self.sent.load(Ordering::Relaxed),
            received_bytes: self.received.load(Ordering::Relaxed),
            encrypt_ms: ms(&self.encrypt),
            decrypt_ms: ms(&self.decrypt),
            network_read_ms: ms(&self.network_read),
            network_write_ms: ms(&self.network_write),
            file_read_ms: ms(&self.file_read),
            file_write_ms: ms(&self.file_write),
            hash_ms: ms(&self.hash),
            sync_ms: ms(&self.sync),
            pacing_ms: ms(&self.pacing),
        }
    }
}
