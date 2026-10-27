//! Install a verified bundled runtime once into the Preview's private data tree.
//! An archive avoids duplicate Qt frameworks when packagers dereference symlinks.
use anyhow::{ensure, Context, Result};
use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: String,
    platform: String,
    arch: String,
    binary: String,
    sha256: String,
    archive_sha256: String,
    upstream_archive_sha256: String,
}
fn digest(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}
fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}
fn safe_link(path: &Path, target: &Path) -> bool {
    if target.is_absolute() {
        return false;
    }
    let mut depth = path.parent().map_or(0, |p| p.components().count());
    for c in target.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            _ => return false,
        }
    }
    true
}
pub fn install(resources: &Path, data: &Path) -> Result<PathBuf> {
    let root = resources.join("deskflow");
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(root.join("manifest.json"))
            .context("内置 Deskflow 未准备，请运行 pnpm prepare:engine")?,
    )?;
    ensure!(
        manifest.version == "1.27.0" && manifest.platform == crate::model::platform(),
        "内置引擎版本或平台不匹配"
    );
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        "unsupported"
    };
    ensure!(manifest.arch == arch, "内置引擎架构不匹配");
    ensure!(
        [
            &manifest.sha256,
            &manifest.archive_sha256,
            &manifest.upstream_archive_sha256
        ]
        .iter()
        .all(|s| crate::model::valid_fingerprint(s)),
        "内置引擎清单校验值无效"
    );
    let binary = Path::new(&manifest.binary);
    ensure!(
        relative(binary) && manifest.binary.len() <= 1024,
        "内置引擎路径无效"
    );
    // The upstream digest keeps the engine path stable across app-only updates,
    // even if the local tar archive's timestamps change during packaging.
    let destination = data.join("engine").join(format!(
        "{}-{}",
        manifest.version,
        &manifest.upstream_archive_sha256[..16]
    ));
    if !destination.exists() {
        let archive = root.join("runtime.tar.gz");
        ensure!(
            digest(&archive)? == manifest.archive_sha256,
            "内置引擎压缩包校验失败"
        );
        let parent = destination.parent().unwrap();
        fs::create_dir_all(parent)?;
        let staging = parent.join(format!(".prepare-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&staging)?;
        let result = (|| -> Result<()> {
            let mut tar = tar::Archive::new(GzDecoder::new(File::open(archive)?));
            let mut total = 0u64;
            for entry in tar.entries()? {
                let mut entry = entry?;
                let path = entry.path()?.into_owned();
                ensure!(relative(&path), "引擎压缩包包含无效路径");
                let kind = entry.header().entry_type();
                ensure!(
                    kind.is_file() || kind.is_dir() || kind.is_symlink() || kind.is_hard_link(),
                    "引擎压缩包包含不支持的文件类型"
                );
                if let Some(target) = entry.link_name()? {
                    ensure!(
                        if kind.is_hard_link() {
                            relative(&target)
                        } else {
                            safe_link(&path, &target)
                        },
                        "引擎压缩包包含越界链接"
                    );
                }
                total = total
                    .checked_add(entry.size())
                    .context("引擎压缩包大小无效")?;
                ensure!(total <= 500 * 1024 * 1024, "引擎压缩包过大");
                ensure!(entry.unpack_in(&staging)?, "引擎压缩包路径越界");
            }
            ensure!(
                digest(&staging.join(binary))? == manifest.sha256,
                "解压后的引擎校验失败"
            );
            fs::rename(&staging, &destination).context("保存内置引擎缓存失败")?;
            Ok(())
        })();
        if staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        result?;
    }
    let executable = destination.join(binary);
    ensure!(
        digest(&executable)? == manifest.sha256,
        "缓存引擎校验失败；未覆盖已有文件，请停止共享后重新准备"
    );
    Ok(executable)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> PathBuf {
        let bundle = root.join("resources/deskflow");
        fs::create_dir_all(&bundle).unwrap();
        let archive = bundle.join("runtime.tar.gz");
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            File::create(&archive).unwrap(),
            flate2::Compression::default(),
        ));
        let bytes = b"test executable";
        let mut h = tar::Header::new_gnu();
        h.set_size(bytes.len() as u64);
        h.set_mode(0o755);
        h.set_cksum();
        tar.append_data(&mut h, "runtime/core", &bytes[..]).unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        fs::write(bundle.join("manifest.json"),serde_json::to_vec(&serde_json::json!({"version":"1.27.0","platform":crate::model::platform(),"arch":if cfg!(target_arch="aarch64"){"arm64"}else{"x64"},"binary":"runtime/core","sha256":hex::encode(Sha256::digest(bytes)),"archiveSha256":digest(&archive).unwrap(),"upstreamArchiveSha256":"a".repeat(64)})).unwrap()).unwrap();
        root.join("resources")
    }
    #[test]
    fn caches_once_and_never_overwrites_a_changed_binary() {
        let dir = tempfile::tempdir().unwrap();
        let resources = fixture(dir.path());
        let data = dir.path().join("profile");
        let binary = install(&resources, &data).unwrap();
        fs::remove_file(resources.join("deskflow/runtime.tar.gz")).unwrap();
        assert_eq!(install(&resources, &data).unwrap(), binary);
        fs::write(&binary, b"changed").unwrap();
        assert!(install(&resources, &data).is_err());
        assert_eq!(fs::read(binary).unwrap(), b"changed");
    }
    #[test]
    fn rejects_damaged_download_before_extracting() {
        let dir = tempfile::tempdir().unwrap();
        let resources = fixture(dir.path());
        fs::write(resources.join("deskflow/runtime.tar.gz"), b"bad").unwrap();
        assert!(install(&resources, &dir.path().join("profile")).is_err());
        assert!(!dir.path().join("profile/engine").exists());
    }
    #[test]
    fn paths_and_links_stay_inside_extraction_root() {
        assert!(!relative(Path::new("../core")));
        assert!(!safe_link(
            Path::new("runtime/link"),
            Path::new("../../outside")
        ));
        assert!(!safe_link(Path::new("runtime/link"), Path::new("/outside")));
        assert!(safe_link(
            Path::new("QtCore.framework/QtCore"),
            Path::new("Versions/Current/QtCore")
        ));
    }
}
