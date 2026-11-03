use anyhow::Result;
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};
use uuid::Uuid;
pub struct Diagnostics {
    pub directory: PathBuf,
    run_id: Uuid,
    sequence: u64,
    secret: String,
}
impl Diagnostics {
    pub fn new(directory: PathBuf, secret: String) -> Result<Self> {
        fs::create_dir_all(&directory)?;
        Ok(Self {
            directory,
            run_id: Uuid::new_v4(),
            sequence: 0,
            secret,
        })
    }
    pub fn protect(&mut self, secret: String) {
        self.secret = secret;
    }
    pub fn record(&mut self, event: &str, details: Value) -> Result<()> {
        let path = self.directory.join("deskflow-ddc.jsonl");
        if fs::metadata(&path)
            .map(|m| m.len() > 2 * 1024 * 1024)
            .unwrap_or(false)
        {
            for i in (1..4).rev() {
                let _ = fs::rename(
                    self.directory.join(format!("deskflow-ddc.{i}.jsonl")),
                    self.directory.join(format!("deskflow-ddc.{}.jsonl", i + 1)),
                );
            }
            fs::rename(&path, self.directory.join("deskflow-ddc.1.jsonl"))?;
        }
        self.sequence += 1;
        let mut text=json!({"time":chrono::Utc::now(),"runId":self.run_id,"sequence":self.sequence,"event":event,"details":details}).to_string();
        if self.secret.len() >= 16 {
            text = text
                .replace(&self.secret, "[REDACTED]")
                .replace(self.secret.trim_start_matches("SB1-"), "[REDACTED]");
        }
        let mut opts = OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        writeln!(opts.open(path)?, "{text}")?;
        Ok(())
    }
}
