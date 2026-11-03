//! Atomic exclusive rename on the destination volume, for files and directories.
use anyhow::{ensure, Result};
use std::path::Path;
pub fn publish(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn renamex_np(from: *const libc::c_char, to: *const libc::c_char, flags: u32) -> i32;
        }
        let from = std::ffi::CString::new(source.as_os_str().as_bytes())?;
        let to = std::ffi::CString::new(destination.as_os_str().as_bytes())?;
        // RENAME_EXCL is atomic, including when a conflicting directory appears
        // between the earlier receipt check and publication.
        ensure!(
            unsafe { renamex_np(from.as_ptr(), to.as_ptr(), 0x4) } == 0,
            "无法发布接收结果（目标已存在时不会覆盖）：{}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
        }
        let from: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // WRITE_THROUGH, deliberately without REPLACE_EXISTING.
        ensure!(
            unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x8) } != 0,
            "无法发布接收结果（目标已存在时不会覆盖）：{}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let from = std::ffi::CString::new(source.as_os_str().as_bytes())?;
        let to = std::ffi::CString::new(destination.as_os_str().as_bytes())?;
        ensure!(
            unsafe {
                libc::renameat2(
                    libc::AT_FDCWD,
                    from.as_ptr(),
                    libc::AT_FDCWD,
                    to.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            } == 0,
            "无法发布接收结果（目标已存在时不会覆盖）：{}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    anyhow::bail!("当前系统不支持文件结果提交");
    // Directory entries are synced after the rename on Unix. Data files have
    // already been synced before the receipt was created.
    #[cfg(unix)]
    {
        std::fs::File::open(destination.parent().unwrap())?.sync_all()?;
        if source.parent() != destination.parent() {
            std::fs::File::open(source.parent().unwrap())?.sync_all()?;
        }
    }
    Ok(())
}
