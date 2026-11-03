use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    System::Threading::*,
};

pub const SYNCHRONIZE_PROCESS: u32 = 0x00100000;

pub fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    use windows_sys::Win32::Security::Cryptography::*;

    fn check_hash(status: NTSTATUS) -> Result<()> {
        ensure!(
            status >= 0,
            "Windows SHA-256 计算失败：0x{:08x}",
            status as u32
        );
        Ok(())
    }
    struct Algorithm(BCRYPT_ALG_HANDLE);
    impl Drop for Algorithm {
        fn drop(&mut self) {
            unsafe {
                BCryptCloseAlgorithmProvider(self.0, 0);
            }
        }
    }
    struct Hash(BCRYPT_HASH_HANDLE);
    impl Drop for Hash {
        fn drop(&mut self) {
            unsafe {
                BCryptDestroyHash(self.0);
            }
        }
    }

    let mut file = std::fs::File::open(path).context("无法读取服务校验文件")?;
    unsafe {
        let mut algorithm = null_mut();
        check_hash(BCryptOpenAlgorithmProvider(
            &mut algorithm,
            BCRYPT_SHA256_ALGORITHM,
            null(),
            0,
        ))?;
        let algorithm = Algorithm(algorithm);
        let mut hash = null_mut();
        // Windows 7+ lets CNG allocate and free the hash object itself.
        check_hash(BCryptCreateHash(
            algorithm.0,
            &mut hash,
            null_mut(),
            0,
            null(),
            0,
            0,
        ))?;
        let hash = Hash(hash);
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let n = match file.read(&mut buffer) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result.context("无法读取服务校验文件")?,
            };
            if n == 0 {
                break;
            }
            check_hash(BCryptHashData(hash.0, buffer.as_ptr(), n as u32, 0))?;
        }
        let mut digest = [0u8; 32];
        check_hash(BCryptFinishHash(
            hash.0,
            digest.as_mut_ptr(),
            digest.len() as u32,
            0,
        ))?;
        Ok(hex::encode(digest))
    }
}

pub fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn check(ok: i32) -> Result<()> {
    ensure!(
        ok != 0,
        "Windows 调用失败：{}",
        std::io::Error::last_os_error()
    );
    Ok(())
}
pub struct Handle(pub HANDLE);
impl Handle {
    pub fn wait(&self, millis: u32) -> bool {
        unsafe { WaitForSingleObject(self.0, millis) == WAIT_OBJECT_0 }
    }
    pub fn new(h: HANDLE) -> Result<Self> {
        ensure!(
            !h.is_null() && h != INVALID_HANDLE_VALUE,
            "Windows 句柄失败：{}",
            std::io::Error::last_os_error()
        );
        Ok(Self(h))
    }
}
unsafe impl Send for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub fn sid(process: HANDLE) -> Result<String> {
    unsafe {
        let mut token = null_mut();
        check(OpenProcessToken(process, TOKEN_QUERY, &mut token))?;
        let token = Handle::new(token)?;
        let mut n = 0;
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut n);
        ensure!(n > 0 && n < 65536, "无法读取账户标识");
        let mut data = vec![0u64; (n as usize).div_ceil(8)];
        check(GetTokenInformation(
            token.0,
            TokenUser,
            data.as_mut_ptr().cast(),
            n,
            &mut n,
        ))?;
        let mut text = null_mut();
        check(ConvertSidToStringSidW(
            (*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid,
            &mut text,
        ))?;
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        let out = String::from_utf16(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        Ok(out?)
    }
}
pub fn current_sid() -> Result<String> {
    sid(unsafe { GetCurrentProcess() })
}
pub fn valid_sid(s: &str) -> bool {
    s.starts_with("S-1-5-")
        && s.len() < 184
        && s[6..].chars().all(|c| c.is_ascii_digit() || c == '-')
}
pub fn service_name(sid: &str) -> Result<String> {
    ensure!(valid_sid(sid), "账户标识无效");
    Ok(format!(
        "DeskflowDDC-{}",
        &hex::encode(Sha256::digest(sid.as_bytes()))[..16]
    ))
}
pub fn pipe_name(sid: &str) -> Result<String> {
    Ok(format!(r"\\.\pipe\{}-v1", service_name(sid)?))
}
pub struct Descriptor(pub PSECURITY_DESCRIPTOR);
impl Descriptor {
    pub fn new(sddl: &str) -> Result<Self> {
        let mut p = null_mut();
        unsafe {
            check(ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide(sddl).as_ptr(),
                1,
                &mut p,
                null_mut(),
            ))?;
        }
        Ok(Self(p))
    }
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
pub fn protect(path: &Path) -> Result<()> {
    // Only SYSTEM and administrators can modify/read the service runtime and keys.
    let d = Descriptor::new("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")?;
    unsafe {
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = null_mut();
        check(GetSecurityDescriptorDacl(
            d.0,
            &mut present,
            &mut acl,
            &mut defaulted,
        ))?;
        let err = SetNamedSecurityInfoW(
            wide(path).as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        );
        ensure!(err == 0, "保护服务目录失败：{err}");
    }
    Ok(())
}
pub fn base() -> Result<PathBuf> {
    use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath};
    unsafe {
        let mut p = null_mut();
        ensure!(
            SHGetKnownFolderPath(&FOLDERID_ProgramFiles, 0, null_mut(), &mut p) >= 0,
            "无法定位 Program Files"
        );
        let mut len = 0;
        while *p.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16(std::slice::from_raw_parts(p, len));
        windows_sys::Win32::System::Com::CoTaskMemFree(p.cast());
        Ok(PathBuf::from(text?).join("Deskflow DDC Service"))
    }
}
pub fn install_root(sid: &str) -> Result<PathBuf> {
    Ok(base()?.join(service_name(sid)?))
}
pub fn no_links(path: &Path) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    let mut p = Some(path);
    while let Some(item) = p {
        if item.exists() {
            ensure!(
                std::fs::symlink_metadata(item)?.file_attributes() & 0x400 == 0,
                "服务路径不允许重解析点"
            );
        }
        p = item.parent();
    }
    Ok(())
}
pub fn pinned_runtime(source: &Path, dest: &Path) -> Result<()> {
    let files: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!(
        "../../../resources/windows-engine-hashes.json"
    ))?;
    for (relative, hash) in files {
        let path = source.join(&relative);
        no_links(&path)?;
        let bytes =
            std::fs::read(&path).with_context(|| format!("缺少内置运行文件：{relative}"))?;
        ensure!(
            hex::encode(Sha256::digest(&bytes)) == hash,
            "内置运行文件校验失败：{relative}"
        );
        let out = dest.join(relative);
        std::fs::create_dir_all(out.parent().unwrap())?;
        std::fs::write(out, bytes)?;
    }
    Ok(())
}
/// GNU builds dynamically import this Microsoft loader before main(), even
/// when invoked only as a service. MSVC links it statically. Pin the loader
/// shipped by webview2-com-sys 0.39.1 rather than elevating an arbitrary DLL.
pub fn app_dependencies(dest: &Path) -> Result<()> {
    #[cfg(target_env = "gnu")]
    {
        let source = std::env::current_exe()?
            .parent()
            .context("无法定位应用目录")?
            .to_path_buf();
        let hashes: std::collections::BTreeMap<String, String> =
            serde_json::from_str(include_str!("../../../resources/windows-app-hashes.json"))?;
        for (name, hash) in hashes {
            let path = source.join(&name);
            no_links(&path)?;
            let bytes = std::fs::read(&path).with_context(|| format!("缺少应用运行库：{name}"))?;
            ensure!(
                hex::encode(Sha256::digest(&bytes)) == hash,
                "应用运行库校验失败：{name}"
            );
            std::fs::write(dest.join(name), bytes)?;
        }
    }
    #[cfg(not(target_env = "gnu"))]
    let _ = dest;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sha256_matches_reference_across_chunks_and_file_changes() {
        let path = std::env::temp_dir().join(format!("ddc-hash-{}", uuid::Uuid::new_v4()));
        for bytes in [Vec::new(), b"abc".to_vec(), vec![0xa5; 3 * 64 * 1024 + 7]] {
            std::fs::write(&path, &bytes).unwrap();
            assert_eq!(
                sha256_file(&path).unwrap(),
                hex::encode(Sha256::digest(&bytes))
            );
        }
        std::fs::remove_file(&path).unwrap();
        assert!(sha256_file(&path).is_err());
    }
}
