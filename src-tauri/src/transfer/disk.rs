use anyhow::{ensure, Result};
use std::path::Path;
pub fn available(path: &Path) -> Result<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
        let mut info: libc::statvfs = unsafe { std::mem::zeroed() };
        ensure!(
            unsafe { libc::statvfs(path.as_ptr(), &mut info) } == 0,
            "无法查询接收磁盘空间：{}",
            std::io::Error::last_os_error()
        );
        Ok((info.f_bavail as u64).saturating_mul(info.f_frsize as u64))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetDiskFreeSpaceExW(
                directory: *const u16,
                available: *mut u64,
                total: *mut u64,
                free: *mut u64,
            ) -> i32;
        }
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0u64;
        ensure!(
            unsafe {
                GetDiskFreeSpaceExW(
                    name.as_ptr(),
                    &mut available,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            } != 0,
            "无法查询接收磁盘空间：{}",
            std::io::Error::last_os_error()
        );
        Ok(available)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        anyhow::bail!("当前系统不支持文件传输")
    }
}
