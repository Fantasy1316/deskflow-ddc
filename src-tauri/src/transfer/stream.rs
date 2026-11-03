//! Streaming content with verified resume prefixes, per-file SHA-256 and a
//! durable per-root commit. Data is never published as a complete file early.
use super::{
    manifest::{self, Entry, Manifest},
    metrics,
    wire::{Wire, CHUNK, SEND_CHUNK},
    Offer, Record,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
};
use tokio_util::sync::CancellationToken;

pub type Progress = Arc<dyn Fn(u64, String) + Send + Sync>;
pub type Committed = Arc<dyn Fn(PathBuf) + Send + Sync>;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Resume {
    offset: u64,
    hash: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    id: uuid::Uuid,
    hash: String,
    count: usize,
    bytes: u64,
}
async fn prefix(
    file: &mut File,
    len: u64,
    cancel: &CancellationToken,
    w: &mut Wire,
    source: bool,
) -> Result<Sha256> {
    let mut last_ping = std::time::Instant::now();
    let mut h = Sha256::new();
    let mut left = len;
    let mut buffer = vec![0; CHUNK];
    while left > 0 {
        ensure!(!cancel.is_cancelled(), "传输已暂停");
        let n = left.min(CHUNK as u64) as usize;
        let io = Instant::now();
        let read = file.read_exact(&mut buffer[..n]).await;
        metrics::elapsed(&w.metrics.file_read, io);
        if source {
            read.context(manifest::SourceFailure)?;
        } else {
            read?;
        }
        let hashing = Instant::now();
        h.update(&buffer[..n]);
        metrics::elapsed(&w.metrics.hash, hashing);
        tokio::task::yield_now().await;
        left -= n as u64;
        if last_ping.elapsed() >= std::time::Duration::from_secs(1) {
            w.send(&[]).await?;
            last_ping = std::time::Instant::now();
        }
    }
    Ok(h)
}
pub async fn send(
    w: &mut Wire,
    offer: &Offer,
    manifest: &Manifest,
    cancel: &CancellationToken,
    progress: Progress,
) -> Result<()> {
    w.json(&Header {
        id: offer.id,
        hash: manifest.hash.clone(),
        count: manifest.sources.len(),
        bytes: manifest.bytes,
    })
    .await?;
    let ready: String = w.read_json().await?;
    if ready == "complete" {
        progress(manifest.bytes, String::new());
        return Ok(());
    }
    ensure!(ready == "manifest", "接收端未就绪");
    for source in &manifest.sources {
        w.json(&source.entry).await?;
    }
    ensure!(w.read_json::<String>().await? == "ready", "接收清单未确认");
    let mut done = 0u64;
    for source in &manifest.sources {
        let e = &source.entry;
        if e.directory {
            continue;
        }
        manifest::unchanged(source).context(manifest::SourceFailure)?;
        let resume: Resume = w.read_json().await?;
        ensure!(resume.offset <= e.size, "续传位置无效");
        let mut file = File::open(&source.source)
            .await
            .context(manifest::SourceFailure)?;
        let mut hash = prefix(&mut file, resume.offset, cancel, w, true).await?;
        let same = hex::encode(hash.clone().finalize()) == resume.hash;
        w.json(&same).await?;
        let offset = if same {
            resume.offset
        } else {
            file = File::open(&source.source)
                .await
                .context(manifest::SourceFailure)?;
            hash = Sha256::new();
            0
        };
        progress(done + offset, e.path.clone());
        let mut left = e.size - offset;
        let mut buffer = vec![0; SEND_CHUNK];
        while left > 0 {
            ensure!(!cancel.is_cancelled(), "传输已暂停");
            let n = left.min(SEND_CHUNK as u64) as usize;
            let io = Instant::now();
            file.read_exact(&mut buffer[..n])
                .await
                .context(manifest::SourceFailure)?;
            metrics::elapsed(&w.metrics.file_read, io);
            let hashing = Instant::now();
            hash.update(&buffer[..n]);
            metrics::elapsed(&w.metrics.hash, hashing);
            w.send(&buffer[..n]).await?;
            left -= n as u64;
            progress(done + e.size - left, e.path.clone());
        }
        manifest::unchanged(source).context(manifest::SourceFailure)?;
        w.json(&hex::encode(hash.finalize())).await?;
        ensure!(
            w.read_json::<String>().await? == "verified",
            "文件完整性确认失败"
        );
        done += e.size;
    }
    w.json(&"finish").await?;
    ensure!(
        w.read_json::<String>().await? == "complete",
        "接收端尚未完成落盘"
    );
    progress(manifest.bytes, String::new());
    Ok(())
}
pub(super) fn real_directory(path: &Path) -> Result<()> {
    let m = std::fs::symlink_metadata(path)?;
    ensure!(
        m.is_dir() && !m.file_type().is_symlink(),
        "接收目录已改变或不是实际目录"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(m.file_attributes() & 0x400 == 0, "接收目录不能是重解析点");
    }
    Ok(())
}
pub fn stage_path(record: &Record) -> Result<PathBuf> {
    Ok(record
        .target
        .as_ref()
        .context("尚未选择接收目录")?
        .join(format!(".deskflow-ddc-{}.partial", record.offer.id)))
}
pub fn final_path(record: &Record) -> Result<PathBuf> {
    Ok(record
        .target
        .as_ref()
        .context("尚未选择接收目录")?
        .join(format!("Deskflow-{}", record.offer.id)))
}
pub async fn receive(
    w: &mut Wire,
    record: &Record,
    cancel: &CancellationToken,
    progress: Progress,
    committed: Committed,
    delivery: &super::delivery::Options,
) -> Result<PathBuf> {
    let header: Header = w.read_json().await?;
    ensure!(
        header.id == record.offer.id
            && header.hash == record.offer.hash
            && header.count == record.offer.entries
            && header.bytes == record.offer.bytes,
        "文件请求与已确认清单不一致"
    );
    let destination = record.target.as_ref().context("尚未选择接收目录")?;
    real_directory(destination)?;
    let staging = stage_path(record)?;
    let final_dir = final_path(record)?;
    if super::delivery::recover(record, delivery)? {
        committed(destination.clone());
        if staging.try_exists()? {
            real_directory(&staging)?;
            let _ = fs::remove_dir_all(&staging).await;
        }
        w.json(&"complete").await?;
        progress(header.bytes, String::new());
        return Ok(destination.clone());
    }
    // Recognize already-completed results from the earlier test build. New
    // transfers keep their durable receipt in application storage instead.
    if record.view.destination.as_deref() == final_dir.to_str() && final_dir.try_exists()? {
        real_directory(&final_dir)?;
        let receipt: Offer =
            serde_json::from_slice(&fs::read(final_dir.join(".deskflow-receipt.json")).await?)?;
        ensure!(
            receipt == record.offer,
            "目标目录已存在且不是此传输的完整结果"
        );
        committed(final_dir.clone());
        w.json(&"complete").await?;
        progress(header.bytes, String::new());
        return Ok(final_dir);
    }
    if !staging.try_exists()? {
        #[cfg(unix)]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(not(unix))]
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&staging)?;
    }
    real_directory(&staging)?;
    let tree = staging.join("files");
    let layout_ready = staging.join("layout.ready");
    w.json(&"manifest").await?;
    let mut entries: Vec<Entry> = Vec::with_capacity(header.count);
    let mut path_bytes = 0usize;
    for _ in 0..header.count {
        let entry: Entry = w.read_json().await?;
        ensure!(
            entry.path.len() <= 2048 && entry.modified.len() <= 40,
            "文件路径过长"
        );
        path_bytes += entry.path.len();
        ensure!(path_bytes <= 16 * 1024 * 1024, "文件清单过大");
        entries.push(entry);
    }
    ensure!(
        manifest::validate(&entries)? == header.bytes && manifest::hash(&entries)? == header.hash,
        "文件清单校验失败"
    );
    let metadata = staging.join("manifest.json");
    if metadata.try_exists()? {
        manifest::regular(&metadata)?;
        let saved: Vec<Entry> = serde_json::from_slice(&fs::read(&metadata).await?)?;
        ensure!(saved == entries, "续传清单与缓存不一致");
    } else {
        crate::store::atomic_write(&metadata, &serde_json::to_vec(&entries)?)?;
    }
    // Reserve directories and files with exclusive creation, detecting even
    // filesystem-specific Unicode/case collisions before receiving any bytes.
    if layout_ready.try_exists()? {
        manifest::regular(&layout_ready)?;
    }
    if tree.try_exists()? && !layout_ready.try_exists()? {
        real_directory(&tree)?;
        fs::remove_dir_all(&tree).await?;
    }
    if !tree.try_exists()? {
        fs::create_dir(&tree).await?;
        let result: Result<()> = async {
            for e in &entries {
                let path = tree.join(&e.path);
                if e.directory {
                    fs::create_dir(path).await?;
                } else {
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)
                        .await?;
                }
            }
            Ok(())
        }
        .await;
        if let Err(e) = result {
            let _ = fs::remove_dir_all(&tree).await;
            return Err(e).context("文件名冲突或接收目录无法创建");
        }
        crate::store::atomic_write(&layout_ready, b"ready")?;
    }
    real_directory(&tree)?;
    let mut remaining = 0u64;
    for e in &entries {
        if !e.directory {
            let m = fs::symlink_metadata(tree.join(&e.path)).await?;
            remaining = remaining
                .checked_add(e.size.saturating_sub(m.len()))
                .context("接收大小溢出")?;
        }
    }
    ensure!(
        super::disk::available(destination)? >= remaining.saturating_add(1024 * 1024),
        "接收磁盘空间不足，请释放空间后继续"
    );
    w.json(&"ready").await?;
    let mut done = 0u64;
    for e in &entries {
        let path = tree.join(&e.path);
        // Check every ancestor so a replaced cache symlink cannot escape staging.
        let mut cursor = tree.clone();
        let parts: Vec<_> = e.path.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            cursor.push(part);
            real_directory(&cursor)?;
        }
        if e.directory {
            real_directory(&path)?;
            continue;
        }
        manifest::regular(&path)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .await?;
        let offset = file.metadata().await?.len();
        ensure!(offset <= e.size, "续传缓存大小异常");
        let mut hash = prefix(&mut file, offset, cancel, w, false).await?;
        w.json(&Resume {
            offset,
            hash: hex::encode(hash.clone().finalize()),
        })
        .await?;
        let offset = if w.read_json::<bool>().await? {
            offset
        } else {
            file.set_len(0).await?;
            use tokio::io::AsyncSeekExt;
            file.rewind().await?;
            hash = Sha256::new();
            0
        };
        let mut left = e.size - offset;
        progress(done + offset, e.path.clone());
        while left > 0 {
            let data = w.receive().await?;
            ensure!(
                !data.is_empty() && data.len() as u64 <= left && data.len() <= CHUNK,
                "文件数据大小无效"
            );
            let io = Instant::now();
            file.write_all(&data).await?;
            metrics::elapsed(&w.metrics.file_write, io);
            let hashing = Instant::now();
            hash.update(&data);
            metrics::elapsed(&w.metrics.hash, hashing);
            left -= data.len() as u64;
            progress(done + e.size - left, e.path.clone());
        }
        let expected: String = w.read_json().await?;
        if expected != hex::encode(hash.finalize()) {
            file.set_len(0).await?;
            file.sync_all().await?;
            anyhow::bail!("文件 SHA-256 校验失败：{}", e.path);
        }
        let sync = Instant::now();
        file.sync_all().await?;
        metrics::elapsed(&w.metrics.sync, sync);
        w.json(&"verified").await?;
        done += e.size;
    }
    ensure!(w.read_json::<String>().await? == "finish", "传输未完成");
    super::delivery::deliver(record, &entries, delivery, w, cancel).await?;
    committed(destination.clone());
    let _ = fs::remove_dir_all(&staging).await;
    w.json(&"complete").await?;
    progress(header.bytes, String::new());
    Ok(destination.clone())
}
