use crate::{
    model::{Config, Pin},
    protocol::{new_pairing, pairing_key},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stored {
    pub version: u32,
    pub id: Uuid,
    pub config: Config,
    pub trusted_peer: Option<Pin>,
    #[serde(default)]
    pub last_verified_address: Option<std::net::IpAddr>,
}
pub struct Store {
    pub directory: PathBuf,
    pub value: Stored,
    pub pairing: String,
}
impl Store {
    pub fn load(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory)?;
        let file = directory.join("settings.json");
        let value = match fs::read(&file) {
            Ok(bytes) => {
                let v: Stored = serde_json::from_slice(&bytes).context("配置损坏；未覆盖原文件")?;
                ensure!(v.version == 2, "配置版本不支持");
                v.config.validate()?;
                v
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Stored {
                version: 2,
                id: Uuid::new_v4(),
                config: Config::default(),
                trusted_peer: None,
                last_verified_address: None,
            },
            Err(e) => return Err(e.into()),
        };
        let vault = keyring::Entry::new("app.deskflow-ddc.preview", &value.id.to_string())?;
        let pairing = match vault.get_password() {
            Ok(code) => {
                pairing_key(&code)?;
                code
            }
            Err(keyring::Error::NoEntry) => {
                ensure!(!file.exists(), "配对密钥不可用；未生成新身份或覆盖旧配置");
                let code = new_pairing();
                vault.set_password(&code).context("无法保存系统配对凭据")?;
                code
            }
            Err(e) => return Err(e).context("无法读取系统配对凭据"),
        };
        let s = Self {
            directory,
            value,
            pairing,
        };
        s.save()?;
        Ok(s)
    }
    pub fn save(&self) -> Result<()> {
        self.value.config.validate()?;
        atomic_write(
            &self.directory.join("settings.json"),
            &serde_json::to_vec_pretty(&self.value)?,
        )
    }
    pub fn set_pairing(&mut self, code: String) -> Result<()> {
        pairing_key(&code)?;
        let vault = keyring::Entry::new("app.deskflow-ddc.preview", &self.value.id.to_string())?;
        let previous = self.value.clone();
        let old_code = self.pairing.clone();
        vault.set_password(&code)?;
        self.pairing = code;
        self.value.trusted_peer = None;
        self.value.last_verified_address = None;
        if let Err(error) = self.save() {
            self.value = previous;
            self.pairing = old_code.clone();
            vault
                .set_password(&old_code)
                .context("配置写入与配对凭据回滚均失败，请恢复 Preview 备份")?;
            return Err(error);
        }
        Ok(())
    }
    /// Read settings only. Electron safeStorage ciphertext is never treated as plaintext.
    pub fn import_legacy_config(&mut self, file: &Path) -> Result<()> {
        let bytes = fs::read(file)?;
        ensure!(bytes.len() < 256 * 1024, "配置文件过大");
        let disk: serde_json::Value = serde_json::from_slice(&bytes)?;
        let c: Config = serde_json::from_value(disk["config"].clone()).context("旧配置格式无效")?;
        c.validate()?;
        let backup = self
            .directory
            .join(format!("settings-before-import-{}.json", Uuid::new_v4()));
        atomic_write(&backup, &serde_json::to_vec_pretty(&self.value)?)?;
        let old = self.value.clone();
        self.value.config = c;
        self.value.trusted_peer = None;
        self.value.last_verified_address = None;
        if let Err(e) = self.save() {
            self.value = old;
            return Err(e);
        }
        Ok(())
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("保存路径无效")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".save-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(not(target_os = "windows"))]
        fs::rename(&temp, path)?;
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> i32;
            }
            let a: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let b: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            ensure!(
                unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 0x1 | 0x8) } != 0,
                "原子保存配置失败：{}",
                std::io::Error::last_os_error()
            );
        }
        Ok(())
    })();
    if temp.exists() {
        let _ = fs::remove_file(temp);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_replaces_without_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        for i in 0..20 {
            atomic_write(&p, format!("{{\"value\":{i}}}").as_bytes()).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
            assert_eq!(v["value"], i);
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn legacy_import_never_writes_source_or_copies_secret() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("legacy.json");
        let bytes = serde_json::to_vec(&serde_json::json!({"config":Config::default(),"secretEncrypted":true,"secret":"unreadable"})).unwrap();
        fs::write(&legacy, &bytes).unwrap();
        let mut store = Store {
            directory: dir.path().join("new"),
            value: Stored {
                version: 2,
                id: Uuid::new_v4(),
                config: Config::default(),
                trusted_peer: None,
                last_verified_address: None,
            },
            pairing: new_pairing(),
        };
        store.import_legacy_config(&legacy).unwrap();
        assert_eq!(fs::read(&legacy).unwrap(), bytes);
        assert!(!fs::read_to_string(store.directory.join("settings.json"))
            .unwrap()
            .contains("unreadable"));
    }
}
