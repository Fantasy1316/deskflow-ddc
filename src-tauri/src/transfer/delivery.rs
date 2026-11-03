//! Durable, per-root publication directly into the selected directory. All
//! conflict decisions precede filesystem changes. Replacements retain backups
//! until the complete delivery receipt is durable; interrupted commits can be
//! recovered or rolled back without guessing ownership from path existence.
use super::{commit, manifest, stream, wire::Wire, Offer, Record};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub token: Uuid,
    pub name: String,
    pub directory: bool,
    pub existing_directory: bool,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Replace,
    KeepBoth,
    Stop,
}
pub type Resolver =
    Arc<dyn Fn(Conflict) -> Pin<Box<dyn Future<Output = Result<Decision>> + Send>> + Send + Sync>;
pub struct Options {
    pub journal: PathBuf,
    pub resolve: Resolver,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Node {
    volume: u64,
    index: u64,
    modified: String,
    length: u64,
    directory: bool,
}
fn node(path: &Path) -> Result<Option<Node>> {
    let m = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        !m.file_type().is_symlink() && (m.is_dir() || m.is_file()),
        "同名目标不是普通文件或目录，请手动处理：{}",
        path.display()
    );
    #[cfg(unix)]
    let (volume, index) = {
        use std::os::unix::fs::MetadataExt;
        (m.dev(), m.ino())
    };
    #[cfg(windows)]
    let (volume, index) = {
        use std::os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        };
        ensure!(m.file_attributes() & 0x400 == 0, "同名目标不能是重解析点");
        #[repr(C)]
        #[derive(Default)]
        struct Information {
            attributes: u32,
            created: [u32; 2],
            accessed: [u32; 2],
            written: [u32; 2],
            volume: u32,
            size_high: u32,
            size_low: u32,
            links: u32,
            index_high: u32,
            index_low: u32,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandle(
                handle: *mut std::ffi::c_void,
                info: *mut Information,
            ) -> i32;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x02000000 | 0x00200000)
            .open(path)?;
        let mut info = Information::default();
        ensure!(
            unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0,
            "无法读取文件标识：{}",
            std::io::Error::last_os_error()
        );
        (
            info.volume as u64,
            ((info.index_high as u64) << 32) | info.index_low as u64,
        )
    };
    Ok(Some(Node {
        volume,
        index,
        modified: manifest::stamp(&m)?,
        length: if m.is_file() { m.len() } else { 0 },
        directory: m.is_dir(),
    }))
}
#[derive(Serialize, Deserialize)]
struct Placement {
    source: String,
    destination: String,
    incoming: Node,
    previous: Option<Node>,
}
#[derive(Serialize, Deserialize)]
struct Receipt {
    offer: Offer,
    target: PathBuf,
    placements: Vec<Placement>,
    done: bool,
}
fn save(path: &Path, receipt: &Receipt) -> Result<()> {
    crate::store::atomic_write(path, &serde_json::to_vec(receipt)?)
}
fn load(path: &Path, record: &Record) -> Result<Option<Receipt>> {
    if !path.try_exists()? {
        return Ok(None);
    }
    manifest::regular(path)?;
    ensure!(path.metadata()?.len() < 1024 * 1024, "接收提交记录过大");
    let receipt: Receipt = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        receipt.offer == record.offer && Some(&receipt.target) == record.target.as_ref(),
        "接收提交记录不一致"
    );
    ensure!(
        !receipt.placements.is_empty() && receipt.placements.len() <= 256,
        "接收提交记录无效"
    );
    let mut sources = std::collections::HashSet::new();
    let mut destinations = std::collections::HashSet::new();
    for p in &receipt.placements {
        manifest::safe_name(&p.source)?;
        manifest::safe_name(&p.destination)?;
        ensure!(
            sources.insert(&p.source) && destinations.insert(p.destination.to_lowercase()),
            "接收提交记录重复"
        );
    }
    Ok(Some(receipt))
}
fn apply(receipt: &Receipt, staging: &Path) -> Result<()> {
    stream::real_directory(&receipt.target)?;
    stream::real_directory(staging)?;
    let tree = staging.join("files");
    stream::real_directory(&tree)?;
    let backups = staging.join("backups");
    if !backups.try_exists()? {
        std::fs::create_dir(&backups)?;
    }
    stream::real_directory(&backups)?;
    for (i, p) in receipt.placements.iter().enumerate() {
        let source = tree.join(&p.source);
        let target = receipt.target.join(&p.destination);
        let backup = backups.join(i.to_string());
        if node(&target)?.as_ref() == Some(&p.incoming) {
            ensure!(
                node(&source)?.is_none(),
                "接收结果和缓存同时存在，已停止提交"
            );
            continue;
        }
        ensure!(
            node(&source)?.as_ref() == Some(&p.incoming),
            "接收缓存已变化，已停止提交"
        );
        if let Some(previous) = &p.previous {
            if node(&backup)?.is_none() {
                ensure!(
                    node(&target)?.as_ref() == Some(previous),
                    "同名目标在确认后发生变化，已停止替换"
                );
                commit::publish(&target, &backup)?;
            }
            ensure!(
                node(&backup)?.as_ref() == Some(previous),
                "原文件备份已变化，已停止提交"
            );
        }
        // Atomic exclusive rename also rejects a conflicting path created after
        // the user's decision. Never use a replacing rename here.
        commit::publish(&source, &target)?;
    }
    Ok(())
}
fn rollback(receipt: &Receipt, staging: &Path) -> Result<()> {
    stream::real_directory(&receipt.target)?;
    stream::real_directory(staging)?;
    let tree = staging.join("files");
    stream::real_directory(&tree)?;
    let backups = staging.join("backups");
    if backups.try_exists()? {
        stream::real_directory(&backups)?;
    }
    for (i, p) in receipt.placements.iter().enumerate().rev() {
        let source = tree.join(&p.source);
        let target = receipt.target.join(&p.destination);
        let backup = backups.join(i.to_string());
        if node(&source)?.is_none() {
            ensure!(
                node(&target)?.as_ref() == Some(&p.incoming),
                "接收结果已变化，保留原文件备份等待手动恢复：{}",
                staging.display()
            );
            commit::publish(&target, &source)?;
        }
        if let Some(previous) = &p.previous {
            if node(&backup)?.is_some() {
                ensure!(
                    node(&backup)?.as_ref() == Some(previous),
                    "原文件备份已变化，保留缓存"
                );
                commit::publish(&backup, &target)?;
            }
        }
    }
    Ok(())
}
fn finish(journal: &Path, receipt: &mut Receipt, staging: &Path) -> Result<()> {
    if receipt.done {
        return Ok(());
    }
    if let Err(error) = apply(receipt, staging) {
        rollback(receipt, staging)
            .context("提交中断，自动恢复失败；保留缓存和原文件备份，请勿删除")?;
        std::fs::remove_file(journal)?;
        return Err(error);
    }
    receipt.done = true;
    save(journal, receipt)?;
    Ok(())
}
// Returns true even after a lost ACK or a crash immediately after a root rename.
pub fn recover(record: &Record, options: &Options) -> Result<bool> {
    let Some(mut receipt) = load(&options.journal, record)? else {
        return Ok(false);
    };
    finish(&options.journal, &mut receipt, &stream::stage_path(record)?)?;
    Ok(true)
}
pub fn completed(record: &Record, journal: &Path) -> Result<bool> {
    Ok(load(journal, record)?.is_some_and(|r| r.done))
}
pub fn cancel(record: &Record, journal: &Path) -> Result<()> {
    if let Some(receipt) = load(journal, record)? {
        ensure!(!receipt.done, "接收结果已提交，请保留已完成文件");
        rollback(&receipt, &stream::stage_path(record)?)?;
        std::fs::remove_file(journal)?;
    }
    Ok(())
}
fn alternate(name: &str, directory: bool, n: usize) -> Result<String> {
    let path = Path::new(name);
    let (stem, extension) = if directory {
        (name, String::new())
    } else {
        let ext = path
            .extension()
            .and_then(|p| p.to_str())
            .filter(|p| !p.is_empty());
        (
            path.file_stem().and_then(|p| p.to_str()).unwrap_or(name),
            ext.map_or(String::new(), |p| format!(".{p}")),
        )
    };
    let tail = format!(" ({n}){extension}");
    ensure!(tail.len() < 255, "文件扩展名过长，无法保留两者");
    let mut end = stem.len().min(255 - tail.len());
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    let name = format!("{}{tail}", &stem[..end]);
    manifest::safe_name(&name)?;
    Ok(name)
}
pub async fn deliver(
    record: &Record,
    entries: &[manifest::Entry],
    options: &Options,
    w: &mut Wire,
    cancel: &CancellationToken,
) -> Result<()> {
    let target = record.target.as_ref().context("尚未选择接收目录")?;
    stream::real_directory(target)?;
    let staging = stream::stage_path(record)?;
    let tree = staging.join("files");
    let roots: Vec<_> = entries.iter().filter(|e| !e.path.contains('/')).collect();
    ensure!(roots.len() <= 256, "一次最多接收 256 个顶层项目");
    // Reserve the original names as well, so `a (2).txt` cannot steal another
    // item's original name while resolving a conflict on `a.txt`.
    let mut names: std::collections::HashSet<String> =
        roots.iter().map(|e| e.path.to_lowercase()).collect();
    let mut placements = Vec::new();
    for entry in roots {
        let previous = node(&target.join(&entry.path))?;
        let mut destination = entry.path.clone();
        let mut replace = false;
        if let Some(old) = &previous {
            let choice = (options.resolve)(Conflict {
                token: Uuid::new_v4(),
                name: entry.path.clone(),
                directory: entry.directory,
                existing_directory: old.directory,
            });
            tokio::pin!(choice);
            let mut keepalive = tokio::time::interval(Duration::from_secs(5));
            let decision = loop {
                tokio::select! {
                    _ = cancel.cancelled() => anyhow::bail!("操作已停止，原有文件未替换"),
                    result = &mut choice => break result?,
                    _ = keepalive.tick() => w.send(&[]).await?,
                }
            };
            match decision {
                Decision::Stop => anyhow::bail!("已停止保存，原有文件未替换；可继续接收后重新选择"),
                Decision::Replace => replace = true,
                Decision::KeepBoth => {
                    let mut found = None;
                    for n in 2..=100_000 {
                        let name = alternate(&entry.path, entry.directory, n)?;
                        if !names.contains(&name.to_lowercase())
                            && node(&target.join(&name))?.is_none()
                        {
                            found = Some(name);
                            break;
                        }
                    }
                    destination = found.context("无法生成可用的文件名")?;
                    names.insert(destination.to_lowercase());
                }
            }
        }
        placements.push(Placement {
            source: entry.path.clone(),
            destination,
            incoming: node(&tree.join(&entry.path))?.context("接收缓存缺失")?,
            previous: if replace { previous } else { None },
        });
    }
    ensure!(!cancel.is_cancelled(), "操作已停止");
    let mut receipt = Receipt {
        offer: record.offer.clone(),
        target: target.clone(),
        placements,
        done: false,
    };
    save(&options.journal, &receipt)?;
    // No await inside the commit/receipt section. Cancellation cannot detach a
    // publication, or delete a backup between a rename and its durable receipt.
    finish(&options.journal, &mut receipt, &staging)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        _dir: tempfile::TempDir,
        record: Record,
        staging: PathBuf,
        journal: PathBuf,
        receipt: Receipt,
    }
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&target).unwrap();
        std::fs::write(source.join("report.txt"), b"incoming").unwrap();
        std::fs::write(target.join("report.txt"), b"original").unwrap();
        let manifest =
            manifest::scan(vec![source.join("report.txt")], &CancellationToken::new()).unwrap();
        let record =
            super::super::tests::receiver_record(&manifest, Uuid::new_v4(), target.clone());
        let staging = stream::stage_path(&record).unwrap();
        std::fs::create_dir(&staging).unwrap();
        let tree = staging.join("files");
        std::fs::create_dir(&tree).unwrap();
        std::fs::copy(source.join("report.txt"), tree.join("report.txt")).unwrap();
        std::fs::create_dir(staging.join("backups")).unwrap();
        let receipt = Receipt {
            offer: record.offer.clone(),
            target: target.clone(),
            done: false,
            placements: vec![Placement {
                source: "report.txt".into(),
                destination: "report.txt".into(),
                incoming: node(&tree.join("report.txt")).unwrap().unwrap(),
                previous: node(&target.join("report.txt")).unwrap(),
            }],
        };
        let journal = dir.path().join("receipt.delivery");
        save(&journal, &receipt).unwrap();
        Fixture {
            _dir: dir,
            record,
            staging,
            journal,
            receipt,
        }
    }
    #[test]
    fn recovers_a_crash_after_backup_or_publication_without_a_second_copy() {
        for publish_new in [false, true] {
            let mut f = fixture();
            let target = f.receipt.target.join("report.txt");
            let backup = f.staging.join("backups/0");
            commit::publish(&target, &backup).unwrap();
            if publish_new {
                commit::publish(&f.staging.join("files/report.txt"), &target).unwrap();
            }
            finish(&f.journal, &mut f.receipt, &f.staging).unwrap();
            assert!(load(&f.journal, &f.record).unwrap().unwrap().done);
            assert_eq!(std::fs::read(&target).unwrap(), b"incoming");
            assert_eq!(std::fs::read(&backup).unwrap(), b"original");
            // Durable receipt works after temporary files have been removed.
            std::fs::remove_dir_all(&f.staging).unwrap();
            let options = Options {
                journal: f.journal.clone(),
                resolve: Arc::new(|_| panic!("already committed")),
            };
            assert!(recover(&f.record, &options).unwrap());
            assert_eq!(std::fs::read(target).unwrap(), b"incoming");
        }
    }
    #[test]
    fn publication_race_rolls_back_already_published_roots_without_touching_new_conflict() {
        let mut f = fixture();
        let second = f.staging.join("files/second.txt");
        std::fs::write(&second, b"second incoming").unwrap();
        f.receipt.placements.push(Placement {
            source: "second.txt".into(),
            destination: "second.txt".into(),
            incoming: node(&second).unwrap().unwrap(),
            previous: None,
        });
        save(&f.journal, &f.receipt).unwrap();
        std::fs::write(
            f.receipt.target.join("second.txt"),
            b"arrived after confirmation",
        )
        .unwrap();
        assert!(finish(&f.journal, &mut f.receipt, &f.staging).is_err());
        assert!(!f.journal.exists());
        assert_eq!(
            std::fs::read(f.receipt.target.join("report.txt")).unwrap(),
            b"original"
        );
        assert_eq!(
            std::fs::read(f.receipt.target.join("second.txt")).unwrap(),
            b"arrived after confirmation"
        );
        assert_eq!(
            std::fs::read(f.staging.join("files/report.txt")).unwrap(),
            b"incoming"
        );
    }
    #[test]
    fn cancelling_an_interrupted_replacement_restores_the_original() {
        for publish_new in [false, true] {
            let f = fixture();
            let target = f.receipt.target.join("report.txt");
            commit::publish(&target, &f.staging.join("backups/0")).unwrap();
            if publish_new {
                commit::publish(&f.staging.join("files/report.txt"), &target).unwrap();
            }
            cancel(&f.record, &f.journal).unwrap();
            assert!(!f.journal.exists());
            assert_eq!(std::fs::read(target).unwrap(), b"original");
            assert_eq!(
                std::fs::read(f.staging.join("files/report.txt")).unwrap(),
                b"incoming"
            );
        }
    }
    #[test]
    fn refuses_to_restore_over_a_foreign_file_and_preserves_the_backup() {
        let f = fixture();
        let target = f.receipt.target.join("report.txt");
        commit::publish(&target, &f.staging.join("backups/0")).unwrap();
        commit::publish(&f.staging.join("files/report.txt"), &target).unwrap();
        std::fs::remove_file(&target).unwrap();
        std::fs::write(&target, b"user data").unwrap();
        assert!(cancel(&f.record, &f.journal).is_err());
        assert!(f.journal.exists());
        assert_eq!(std::fs::read(target).unwrap(), b"user data");
        assert_eq!(
            std::fs::read(f.staging.join("backups/0")).unwrap(),
            b"original"
        );
    }
    #[test]
    fn alternate_names_preserve_extensions_and_fit_portable_name_limits() {
        assert_eq!(alternate("video.zip", false, 2).unwrap(), "video (2).zip");
        assert_eq!(alternate("folder.zip", true, 2).unwrap(), "folder.zip (2)");
        assert_eq!(alternate(".env", false, 3).unwrap(), ".env (3)");
        let name = format!("{}.zip", "文".repeat(83));
        let alternate = alternate(&name, false, 100).unwrap();
        assert!(alternate.len() <= 255);
        assert!(alternate.ends_with(" (100).zip"));
    }
}
