use super::{win::*, Status};
use anyhow::{ensure, Context, Result};
use std::{
    path::Path,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::*,
    System::{Services::*, Threading::*},
    UI::Shell::*,
};
struct Service(SC_HANDLE);
impl Service {
    fn new(h: SC_HANDLE) -> Result<Self> {
        if h.is_null() {
            // Preserve the OS code before another service handle is dropped.
            return Err(std::io::Error::last_os_error()).context("服务访问失败");
        }
        Ok(Self(h))
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        unsafe {
            CloseServiceHandle(self.0);
        }
    }
}
fn open(sid: &str, access: u32) -> Result<Service> {
    let scm = Service::new(unsafe { OpenSCManagerW(null(), null(), SC_MANAGER_CONNECT) })?;
    Service::new(unsafe { OpenServiceW(scm.0, wide(service_name(sid)?).as_ptr(), access) })
}
pub fn status() -> Result<Status> {
    let sid = current_sid()?;
    let installed = match open(&sid, SERVICE_QUERY_STATUS) {
        Ok(_) => true,
        Err(error) if missing(&error) => false,
        Err(error) => return Err(error),
    };
    let marker = std::fs::read_to_string(install_root(&sid)?.join("version")).unwrap_or_default();
    // Compare binary digest as well: test builds may retain the same version.
    let current = installed && marker.trim() == current_digest()?;
    Ok(Status { installed, current })
}
fn missing(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST as i32))
}
/// Bind the pipe peer to the SCM-owned service, without needing access to a
/// SYSTEM process token from the unprivileged UI.
pub fn verify_server(pid: u32) -> Result<()> {
    let owner = current_sid()?;
    let service = open(&owner, SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG)?;
    let mut state: SERVICE_STATUS_PROCESS = unsafe { std::mem::zeroed() };
    let mut size = 0;
    unsafe {
        check(QueryServiceStatusEx(
            service.0,
            SC_STATUS_PROCESS_INFO,
            (&mut state as *mut SERVICE_STATUS_PROCESS).cast(),
            std::mem::size_of_val(&state) as u32,
            &mut size,
        ))?;
    }
    ensure!(
        state.dwCurrentState == SERVICE_RUNNING && state.dwProcessId == pid,
        "服务管道身份不可信，未发送配对凭据"
    );
    // SCM configuration is writable only by administrators. Verify its fixed
    // protected executable and LocalSystem account; ordinary users need not
    // open a SYSTEM process/token just to authenticate the pipe server.
    unsafe {
        QueryServiceConfigW(service.0, null_mut(), 0, &mut size);
    }
    ensure!((1..=65536).contains(&size), "服务配置大小无效");
    let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    let config = buffer.as_mut_ptr().cast::<QUERY_SERVICE_CONFIGW>();
    unsafe {
        check(QueryServiceConfigW(service.0, config, size, &mut size))?;
        let string = |p: *const u16| -> Result<String> {
            ensure!(!p.is_null(), "服务配置缺少字段");
            let length = (0..32768)
                .find(|i| *p.add(*i) == 0)
                .context("服务配置字段过长")?;
            Ok(String::from_utf16(std::slice::from_raw_parts(p, length))?)
        };
        let command = string((*config).lpBinaryPathName)?;
        let account = string((*config).lpServiceStartName)?;
        ensure!(
            (*config).dwServiceType == SERVICE_WIN32_OWN_PROCESS
                && (account.eq_ignore_ascii_case("LocalSystem")
                    || account.eq_ignore_ascii_case("NT AUTHORITY\\SYSTEM")),
            "后台服务运行账户不匹配"
        );
        let (binary, args) = command
            .strip_prefix('"')
            .and_then(|s| s.split_once('"'))
            .context("服务启动路径无效")?;
        let binary = Path::new(binary);
        ensure!(
            binary.starts_with(install_root(&owner)?)
                && binary.file_name().is_some_and(|name| name == "service.exe")
                && args == format!(" --windows-service {owner}"),
            "服务程序路径不匹配，未发送配对凭据"
        );
    }
    Ok(())
}
fn current_digest() -> Result<String> {
    // Debug executables are hundreds of MB. Unoptimized Rust SHA-256 can
    // outlast the peer's engine deadline on every status/start check. CNG
    // retains the full digest check with native performance and bounded memory.
    sha256_file(&std::env::current_exe()?)
}
pub fn start_service() -> Result<()> {
    ensure!(
        status()?.current,
        "后台服务未安装或需要更新，请停止共享后在共享设置中安装/更新服务"
    );
    let s = open(&current_sid()?, SERVICE_START | SERVICE_QUERY_STATUS)?;
    unsafe {
        if StartServiceW(s.0, 0, null()) == 0 {
            ensure!(
                GetLastError() == ERROR_SERVICE_ALREADY_RUNNING,
                "启动后台服务失败：{}",
                std::io::Error::last_os_error()
            );
        }
    }
    Ok(())
}
pub async fn manage(resources: std::path::PathBuf, remove: bool) -> Result<()> {
    let sid = current_sid()?;
    let binary = if remove {
        None
    } else {
        Some(crate::bundle::install(
            &resources,
            &std::env::temp_dir().join(format!("ddc-verify-{}", uuid::Uuid::new_v4())),
        )?)
    };
    // Source runtime is only an input to the elevated installer. Every file is
    // checked against hashes pinned in this executable before privileged copying.
    let source = binary
        .as_ref()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf);
    let result = tokio::task::spawn_blocking(move || -> Result<()> {
        let mode = if remove {
            "--remove-windows-service"
        } else {
            "--install-windows-service"
        };
        let mut args = format!("{mode} {sid}");
        if let Some(source) = &source {
            args.push_str(&format!(" \"{}\"", source.display()));
        }
        let exe = wide(std::env::current_exe()?);
        let params = wide(args);
        let verb = wide("runas");
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of_val(&info) as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = exe.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = 0;
        unsafe {
            check(ShellExecuteExW(&mut info)).context("管理员授权未完成，普通共享模式仍可使用")?;
            let process = Handle::new(info.hProcess)?;
            check((WaitForSingleObject(process.0, 180_000) == WAIT_OBJECT_0) as i32)?;
            let mut code = 1;
            check(GetExitCodeProcess(process.0, &mut code))?;
            ensure!(
                code == 0,
                "后台服务安装/移除失败（{code}），请检查服务目录权限及其他共享实例"
            );
        }
        Ok(())
    })
    .await?;
    // Best-effort removal of the verified temporary runtime, never user data.
    if let Some(binary) = binary {
        if let Some(root) = binary.ancestors().find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("ddc-verify-"))
        }) {
            let _ = std::fs::remove_dir_all(root);
        }
    }
    result
}
fn stop(s: &Service) -> Result<()> {
    let mut state: SERVICE_STATUS = unsafe { std::mem::zeroed() };
    unsafe {
        if ControlService(s.0, SERVICE_CONTROL_STOP, &mut state) == 0 {
            ensure!(GetLastError() == ERROR_SERVICE_NOT_ACTIVE, "停止服务失败");
        }
    }
    let start = Instant::now();
    loop {
        unsafe {
            check(QueryServiceStatus(s.0, &mut state))?;
        }
        if state.dwCurrentState == SERVICE_STOPPED {
            return Ok(());
        }
        ensure!(
            start.elapsed() < Duration::from_secs(20),
            "服务尚未退出，未替换文件"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub fn elevated(sid: &str, source: Option<&Path>) -> Result<()> {
    let name = service_name(sid)?;
    let root = install_root(sid)?;
    no_links(&root)?;
    if source.is_none() {
        match open(sid, SERVICE_STOP | SERVICE_QUERY_STATUS | 0x00010000) {
            Ok(service) => {
                stop(&service)?;
                unsafe {
                    check(DeleteService(service.0))?;
                }
            }
            Err(error) => {
                if !missing(&error) {
                    return Err(error);
                }
            }
        }
        if root.exists() {
            std::fs::remove_dir_all(root)?;
        }
        return Ok(());
    }
    let source = source.unwrap();
    std::fs::create_dir_all(base()?)?;
    protect(&base()?)?;
    std::fs::create_dir_all(&root)?;
    protect(&root)?;
    let staging = root.join(format!(".install-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&staging)?;
    let result = (|| -> Result<()> {
        // Finish verifying/staging the replacement before stopping the old service.
        pinned_runtime(source, &staging.join("runtime"))?;
        app_dependencies(&staging)?;
        std::fs::copy(std::env::current_exe()?, staging.join("service.exe"))?;
        let revision = current_digest()?;
        let payload = root.join(format!("build-{}", uuid::Uuid::new_v4()));
        std::fs::rename(&staging, &payload)?;
        let command = wide(format!(
            "\"{}\" --windows-service {sid}",
            payload.join("service.exe").display()
        ));
        let service = match open(sid, SERVICE_ALL_ACCESS) {
            Ok(service) => {
                stop(&service)?;
                unsafe {
                    check(ChangeServiceConfigW(
                        service.0,
                        SERVICE_NO_CHANGE,
                        SERVICE_DEMAND_START,
                        SERVICE_NO_CHANGE,
                        command.as_ptr(),
                        null(),
                        null_mut(),
                        null(),
                        null(),
                        null(),
                        null(),
                    ))?;
                }
                service
            }
            Err(error) => {
                if !missing(&error) {
                    return Err(error);
                }
                let scm = Service::new(unsafe {
                    OpenSCManagerW(null(), null(), SC_MANAGER_CREATE_SERVICE)
                })?;
                Service::new(unsafe {
                    CreateServiceW(
                        scm.0,
                        wide(&name).as_ptr(),
                        wide(format!(
                            "Deskflow DDC 安全桌面 ({})",
                            &name[name.len() - 8..]
                        ))
                        .as_ptr(),
                        SERVICE_ALL_ACCESS,
                        SERVICE_WIN32_OWN_PROCESS,
                        SERVICE_DEMAND_START,
                        SERVICE_ERROR_NORMAL,
                        command.as_ptr(),
                        null(),
                        null_mut(),
                        null(),
                        null(),
                        null(),
                    )
                })?
            }
        };
        let d = Descriptor::new(&format!(
            "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;CCLCSWRPWPLO;;;{sid})"
        ))?;
        unsafe {
            check(SetServiceObjectSecurity(
                service.0,
                DACL_SECURITY_INFORMATION,
                d.0,
            ))?;
        }
        std::fs::write(root.join("version"), revision)?;
        let marker = root.join("version");
        let d = Descriptor::new(&format!("D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FR;;;{sid})"))?;
        unsafe {
            check(SetFileSecurityW(
                wide(marker).as_ptr(),
                DACL_SECURITY_INFORMATION,
                d.0,
            ))?;
        }
        // The old service has stopped; retain only the newly activated runtime.
        for item in std::fs::read_dir(&root)? {
            let item = item?;
            if item.file_name().to_string_lossy().starts_with("build-") && item.path() != payload {
                no_links(&item.path())?;
                std::fs::remove_dir_all(item.path())?;
            }
        }
        Ok(())
    })();
    if staging.exists() {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

/// Installer/uninstaller hook. It never installs/starts a service, and does not
/// invoke the UI or restore a sharing session.
pub fn installer_cleanup() -> Result<()> {
    let sid = current_sid()?;
    match open(&sid, SERVICE_QUERY_STATUS) {
        Ok(_) => (),
        Err(error) => {
            if missing(&error) {
                return Ok(());
            }
            return Err(error);
        }
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(manage(std::path::PathBuf::new(), true))
}
