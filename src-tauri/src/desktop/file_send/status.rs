use serde::Serialize;
use std::time::Instant;
use uuid::Uuid;

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Activity {
    pub id: Uuid,
    pub label: String,
    pub status: String,
    pub items: usize,
    pub total_bytes: u64,
    pub completed_bytes: u64,
    pub bytes_per_second: u64,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn speed_uses_recent_bytes_and_restarts_sampling_after_pause_or_rewind() {
        let id = Uuid::new_v4();
        let mut task = Tracked::new(id, Uuid::new_v4(), 2, Uuid::new_v4());
        let mut job: crate::transfer::View = serde_json::from_value(serde_json::json!({
            "id":id,"label":"directory","direction":"send","status":"transferring",
            "peerName":"peer","entries":150,"totalBytes":10000,"completedBytes":1000,
            "currentFile":"directory/file.zip","destination":null,"error":null,"createdAt":""
        }))
        .unwrap();
        let now = Instant::now();
        assert_eq!(task.update_at(&job, now).bytes_per_second, 0);
        job.completed_bytes = 3000;
        let activity = task.update_at(&job, now + Duration::from_secs(1));
        assert_eq!(activity.bytes_per_second, 2000);
        assert_eq!(activity.items, 2); // Root selections, not recursive entries.
        job.status = "paused".into();
        assert_eq!(
            task.update_at(&job, now + Duration::from_secs(2))
                .bytes_per_second,
            0
        );
        job.status = "transferring".into();
        assert_eq!(
            task.update_at(&job, now + Duration::from_secs(30))
                .bytes_per_second,
            0
        );
        job.completed_bytes = 2000; // Prefix repair can rewind confirmed bytes.
        assert_eq!(
            task.update_at(&job, now + Duration::from_secs(31))
                .bytes_per_second,
            0
        );
        job.completed_bytes = 4000;
        assert_eq!(
            task.update_at(&job, now + Duration::from_secs(32))
                .bytes_per_second,
            2000
        );
        job.status = "complete".into();
        assert_eq!(
            task.update_at(&job, now + Duration::from_secs(33))
                .bytes_per_second,
            0
        );
    }
}
pub(super) struct Tracked {
    pub id: Uuid,
    pub peer: Uuid,
    pub items: usize,
    pub operation: Uuid,
    sample: Option<(Instant, u64)>,
}
impl Tracked {
    pub fn new(id: Uuid, peer: Uuid, items: usize, operation: Uuid) -> Self {
        Self {
            id,
            peer,
            items,
            operation,
            sample: None,
        }
    }
    pub fn update(&mut self, job: &crate::transfer::View) -> Activity {
        self.update_at(job, Instant::now())
    }
    fn update_at(&mut self, job: &crate::transfer::View, now: Instant) -> Activity {
        let speed =
            self.sample
                .filter(|_| job.status == "transferring")
                .map_or(0, |(time, bytes)| {
                    (job.completed_bytes.saturating_sub(bytes) as f64
                        / now.duration_since(time).as_secs_f64().max(0.001))
                        as u64
                });
        self.sample = (job.status == "transferring").then_some((now, job.completed_bytes));
        Activity {
            id: job.id,
            label: job.label.clone(),
            status: job.status.clone(),
            items: self.items,
            total_bytes: job.total_bytes,
            completed_bytes: job.completed_bytes,
            bytes_per_second: speed,
            error: job.error.clone(),
        }
    }
}
