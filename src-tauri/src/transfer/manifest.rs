use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use tokio_util::sync::CancellationToken;

pub const MAX_ENTRIES: usize = 100_000;
#[derive(Debug)]
pub struct SourceFailure;
impl std::fmt::Display for SourceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("源文件不可用或已改变，请重新选择文件发送")
    }
}
impl std::error::Error for SourceFailure {}
fn no_reparse(m: &fs::Metadata) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(m.file_attributes() & 0x400 == 0, "暂不支持重解析点");
    }
    let _ = m;
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub directory: bool,
    pub size: u64,
    pub modified: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Source {
    pub entry: Entry,
    pub source: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub sources: Vec<Source>,
    pub hash: String,
    pub bytes: u64,
}
pub fn safe_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name != "."
            && name != ".."
            && name.len() <= 255
            && !name.ends_with([' ', '.'])
            && !name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)),
        "文件名无法跨系统使用：{name}"
    );
    let upper = name.split('.').next().unwrap().to_ascii_uppercase();
    ensure!(
        !["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
            && !(upper.len() == 4
                && (upper.starts_with("COM") || upper.starts_with("LPT"))
                && matches!(upper.as_bytes()[3], b'1'..=b'9')),
        "Windows 保留文件名：{name}"
    );
    Ok(())
}
pub fn validate(entries: &[Entry]) -> Result<u64> {
    ensure!(
        !entries.is_empty() && entries.len() <= MAX_ENTRIES,
        "文件数量超出限制（最多 100000 项）"
    );
    let mut seen = HashSet::new();
    let mut dirs = HashSet::new();
    let mut total = 0u64;
    let mut path_bytes = 0usize;
    for e in entries {
        path_bytes += e.path.len();
        ensure!(path_bytes <= 16 * 1024 * 1024, "文件清单过大");
        ensure!(
            e.path.to_lowercase() != ".deskflow-receipt.json",
            "文件名与接收凭据冲突"
        );
        ensure!(
            e.path.len() <= 2048 && e.modified.len() <= 40,
            "文件路径过长"
        );
        let parts: Vec<_> = e.path.split('/').collect();
        ensure!(parts.len() <= 64, "目录层级过深");
        for p in &parts {
            safe_name(p)?;
        }
        let folded = e.path.to_lowercase();
        ensure!(
            seen.insert(folded.clone()),
            "文件名存在大小写冲突：{}",
            e.path
        );
        if let Some((parent, _)) = folded.rsplit_once('/') {
            ensure!(dirs.contains(parent), "目录结构无效");
        }
        if e.directory {
            ensure!(e.size == 0, "目录大小无效");
            dirs.insert(folded);
        } else {
            total = total.checked_add(e.size).context("文件总大小溢出")?;
        }
    }
    ensure!(total <= 9_007_199_254_740_991, "文件总大小超过支持范围");
    Ok(total)
}
pub fn hash(entries: &[Entry]) -> Result<String> {
    let mut h = Sha256::new();
    for e in entries {
        let b = serde_json::to_vec(e)?;
        h.update((b.len() as u64).to_be_bytes());
        h.update(b);
    }
    Ok(hex::encode(h.finalize()))
}
pub fn stamp(meta: &fs::Metadata) -> Result<String> {
    Ok(meta
        .modified()?
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_string())
}
pub fn scan(paths: Vec<PathBuf>, cancel: &CancellationToken) -> Result<Manifest> {
    ensure!(
        !paths.is_empty() && paths.len() <= 256,
        "一次最多选择 256 个文件或目录"
    );
    fn visit(
        source: PathBuf,
        path: String,
        out: &mut Vec<Source>,
        cancel: &CancellationToken,
        path_bytes: &mut usize,
    ) -> Result<()> {
        ensure!(!cancel.is_cancelled(), "文件扫描已取消");
        ensure!(
            out.len() < MAX_ENTRIES && path.split('/').count() <= 64,
            "文件数量或目录层级超出限制"
        );
        *path_bytes += path.len() + source.as_os_str().len();
        ensure!(
            *path_bytes <= 16 * 1024 * 1024 && path.len() <= 2048,
            "文件清单或路径过长"
        );
        let m = fs::symlink_metadata(&source).with_context(|| format!("无法读取：{path}"))?;
        ensure!(
            !m.file_type().is_symlink() && (m.is_dir() || m.is_file()),
            "暂不支持符号链接或特殊文件：{path}"
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                m.file_attributes() & 0x400 == 0,
                "暂不支持目录联接或重解析点：{path}"
            );
        }
        let e = Entry {
            path: path.clone(),
            directory: m.is_dir(),
            size: if m.is_file() { m.len() } else { 0 },
            modified: stamp(&m)?,
        };
        out.push(Source {
            entry: e,
            source: source.clone(),
        });
        if m.is_dir() {
            let mut children = Vec::new();
            for child in fs::read_dir(&source)? {
                ensure!(!cancel.is_cancelled(), "文件扫描已取消");
                ensure!(
                    children.len() + out.len() < MAX_ENTRIES,
                    "文件数量超过 100000 项"
                );
                children.push(child?);
            }
            children.sort_by_key(|e| e.file_name());
            for child in children {
                let name = child
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("文件名不是有效 Unicode"))?;
                safe_name(&name)?;
                visit(
                    child.path(),
                    format!("{path}/{name}"),
                    out,
                    cancel,
                    path_bytes,
                )?;
            }
        }
        Ok(())
    }
    let mut sources = Vec::new();
    let mut path_bytes = 0;
    for path in paths {
        ensure!(path.is_absolute(), "请选择绝对文件路径");
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .context("无法发送磁盘根目录")?
            .to_owned();
        safe_name(&name)?;
        visit(path, name, &mut sources, cancel, &mut path_bytes)?;
    }
    let entries: Vec<_> = sources.iter().map(|s| s.entry.clone()).collect();
    Ok(Manifest {
        bytes: validate(&entries)?,
        hash: hash(&entries)?,
        sources,
    })
}
pub fn unchanged(source: &Source) -> Result<()> {
    let m = fs::symlink_metadata(&source.source)?;
    no_reparse(&m)?;
    ensure!(
        m.is_file()
            && !m.file_type().is_symlink()
            && m.len() == source.entry.size
            && stamp(&m)? == source.entry.modified,
        "源文件已变化，请重新选择发送：{}",
        source.entry.path
    );
    Ok(())
}
pub fn regular(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    no_reparse(&m)?;
    ensure!(
        m.is_file() && !m.file_type().is_symlink(),
        "接收缓存不是普通文件"
    );
    Ok(())
}
