use super::{win::*, Command, Launch, Reply, LEASE_SECONDS, MAX_MESSAGE};
use anyhow::{ensure, Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle},
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        OnceLock,
    },
    time::Duration,
};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio_util::codec::{Framed, LinesCodec};
use windows_sys::Win32::{
    Foundation::*,
    Security::*,
    Storage::FileSystem::*,
    System::{
        Diagnostics::ToolHelp::*, JobObjects::*, Pipes::*, RemoteDesktop::*, Services::*,
        Threading::*,
    },
};
static OWNER: OnceLock<String> = OnceLock::new();
static STOP: AtomicBool = AtomicBool::new(false);
static STATUS: AtomicUsize = AtomicUsize::new(0);

pub fn run_if_requested() -> Option<i32> {
    let args: Vec<_> = std::env::args().collect();
    let mode = args.get(1)?.as_str();
    if mode == "--cleanup-windows-service" {
        return Some(if super::install::installer_cleanup().is_ok() {
            0
        } else {
            2
        });
    }
    if ![
        "--windows-service",
        "--install-windows-service",
        "--remove-windows-service",
    ]
    .contains(&mode)
    {
        return None;
    }
    let result = (|| -> Result<()> {
        let sid = args.get(2).context("缺少服务账户")?;
        ensure!(valid_sid(sid), "服务账户无效");
        if mode != "--windows-service" {
            ensure!(
                args.len()
                    == if mode == "--install-windows-service" {
                        4
                    } else {
                        3
                    },
                "无效安装参数"
            );
            return super::install::elevated(sid, args.get(3).map(std::path::Path::new));
        }
        ensure!(args.len() == 3, "无效服务参数");
        OWNER
            .set(sid.clone())
            .map_err(|_| anyhow::anyhow!("服务重复启动"))?;
        let name = wide(service_name(sid)?);
        let table = [
            SERVICE_TABLE_ENTRYW {
                lpServiceName: name.as_ptr() as _,
                lpServiceProc: Some(service_main),
            },
            SERVICE_TABLE_ENTRYW {
                lpServiceName: null_mut(),
                lpServiceProc: None,
            },
        ];
        unsafe {
            check(StartServiceCtrlDispatcherW(table.as_ptr()))?;
        }
        Ok(())
    })();
    // Never log launch messages: they contain an engine private key.
    Some(if result.is_ok() { 0 } else { 2 })
}
fn report(state: u32, error: u32) {
    let handle = STATUS.load(Ordering::Acquire) as SERVICE_STATUS_HANDLE;
    if handle.is_null() {
        return;
    }
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: if state == SERVICE_RUNNING {
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
        } else {
            0
        },
        dwWin32ExitCode: error,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: if state == SERVICE_STOP_PENDING {
            10000
        } else {
            0
        },
    };
    unsafe {
        SetServiceStatus(handle, &status);
    }
}
unsafe extern "system" fn control(
    code: u32,
    _: u32,
    _: *mut std::ffi::c_void,
    _: *mut std::ffi::c_void,
) -> u32 {
    if code == SERVICE_CONTROL_STOP || code == SERVICE_CONTROL_SHUTDOWN {
        STOP.store(true, Ordering::Release);
        report(SERVICE_STOP_PENDING, 0);
    }
    0
}
unsafe extern "system" fn service_main(_: u32, _: *mut *mut u16) {
    let name = wide(service_name(OWNER.get().unwrap()).unwrap());
    let handle = unsafe { RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(control), null_mut()) };
    if handle.is_null() {
        return;
    }
    STATUS.store(handle as usize, Ordering::Release);
    report(SERVICE_START_PENDING, 0);
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(anyhow::Error::from)
        .and_then(|rt| rt.block_on(serve()));
    report(SERVICE_STOPPED, if result.is_ok() { 0 } else { 1 });
}
fn authorize(pipe: &NamedPipeServer, owner: &str) -> Result<(Handle, u32)> {
    unsafe {
        let mut pid = 0;
        check(GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut pid))?;
        let p = Handle::new(OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE_PROCESS,
            0,
            pid,
        ))?;
        ensure!(sid(p.0)? == owner, "后台服务拒绝其他账户");
        let mut session = 0;
        check(ProcessIdToSessionId(pid, &mut session))?;
        ensure!(
            session != 0 && session == WTSGetActiveConsoleSessionId(),
            "后台服务只接受当前控制台用户会话"
        );
        Ok((p, session))
    }
}
async fn reply(pipe: &mut Framed<NamedPipeServer, LinesCodec>, value: Reply) -> Result<()> {
    tokio::time::timeout(
        Duration::from_secs(2),
        pipe.send(serde_json::to_string(&value)?),
    )
    .await??;
    Ok(())
}
async fn serve() -> Result<()> {
    enable_debug_privilege()?;
    // SCM permits only one instance of this per-user service. A previous crash
    // closed its job handle, so leftover private session files can be removed.
    let payload = std::env::current_exe()?.parent().unwrap().to_path_buf();
    for item in std::fs::read_dir(&payload)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if name
            .strip_prefix("session-")
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        {
            no_links(&item.path())?;
            std::fs::remove_dir_all(item.path()).context("无法清理旧服务会话")?;
        }
    }
    let owner = OWNER.get().unwrap();
    let descriptor = Descriptor::new(&format!("D:P(A;;GA;;;SY)(A;;GRGW;;;{owner})"))?;
    let attributes = descriptor.attributes();
    let name = pipe_name(owner)?;
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(true)
        .reject_remote_clients(true);
    let create = || unsafe {
        options.create_with_security_attributes_raw(
            &name,
            (&attributes as *const SECURITY_ATTRIBUTES)
                .cast_mut()
                .cast(),
        )
    };
    let mut pipe = create()?;
    report(SERVICE_RUNNING, 0);
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    while !STOP.load(Ordering::Acquire) {
        tokio::select! { _ = tick.tick() => continue, result = pipe.connect() => result? }
        if let Ok((process, session)) = authorize(&pipe, owner) {
            let mut framed = Framed::new(pipe, LinesCodec::new_with_max_length(MAX_MESSAGE));
            if let Err(error) = session_loop(&mut framed, process, session).await {
                let _ = reply(
                    &mut framed,
                    Reply::Error {
                        message: error.to_string(),
                    },
                )
                .await;
            }
            // Dropping the only server pipe also releases the owned engine lease.
            drop(framed);
            if STOP.load(Ordering::Acquire) {
                break;
            }
            pipe = create()?;
        } else {
            pipe.disconnect()?;
        }
    }
    Ok(())
}
async fn session_loop(
    pipe: &mut Framed<NamedPipeServer, LinesCodec>,
    owner: Handle,
    session: u32,
) -> Result<()> {
    let first = tokio::time::timeout(Duration::from_secs(5), pipe.next())
        .await?
        .context("服务连接已关闭")??;
    let Command::Start { launch } = serde_json::from_str(&first)? else {
        anyhow::bail!("首条请求必须启动接收引擎");
    };
    launch.validate()?;
    ensure!(
        !owner.wait(0)
            && !STOP.load(Ordering::Acquire)
            && session == unsafe { WTSGetActiveConsoleSessionId() },
        "应用或控制台会话已结束，未启动引擎"
    );
    // Refuse existing engines; never attach to/kill a foreign instance.
    crate::engine::assert_no_running_engine(&std::env::temp_dir()).await?;
    let root = std::env::current_exe()?
        .parent()
        .unwrap()
        .join(format!("session-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root)?;
    let result = async {
        let (mut process, mut logs) = Process::start(&launch, &root, session)?;
        reply(pipe, Reply::Started { pid: process.pid }).await?;
        let mut last = tokio::time::Instant::now();
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        let result = loop {
            tokio::select! {
                _ = interval.tick() => {
                    if session != unsafe { WTSGetActiveConsoleSessionId() }
                        || STOP.load(Ordering::Acquire)
                        || last.elapsed() > Duration::from_secs(LEASE_SECONDS)
                        || owner.wait(0)
                    {
                        break Ok(());
                    }
                    if process.child.wait(0) {
                        break Err(anyhow::anyhow!("服务引擎已退出"));
                    }
                }
                message = pipe.next() => {
                    let Some(Ok(line)) = message else { break Ok(()); };
                    match serde_json::from_str::<Command>(&line)? {
                        Command::Ping => {
                            last = tokio::time::Instant::now();
                            reply(pipe, Reply::Pong).await?;
                        }
                        Command::Stop => break Ok(()),
                        Command::Start { .. } => break Err(anyhow::anyhow!("引擎已启动")),
                    }
                }
                Some(line) = logs.recv() => { reply(pipe, Reply::Output { line }).await?; }
            }
        };
        process.stop()?;
        if let Err(e) = &result {
            let _ = reply(
                pipe,
                Reply::Error {
                    message: e.to_string(),
                },
            )
            .await;
        }
        reply(pipe, Reply::Stopped).await?;
        result
    }
    .await;
    // Even an I/O error drops the kill-on-close job before removing session keys.
    let _ = std::fs::remove_dir_all(root);
    result
}
struct Process {
    child: Handle,
    job: Handle,
    pid: u32,
}
impl Process {
    fn start(
        launch: &Launch,
        root: &std::path::Path,
        session: u32,
    ) -> Result<(Self, tokio::sync::mpsc::Receiver<String>)> {
        std::fs::create_dir(root.join("tls"))?;
        crate::store::atomic_write(
            &root.join("tls/identity.pem"),
            launch.certificate.as_bytes(),
        )?;
        crate::store::atomic_write(
            &root.join("tls/trusted-servers"),
            format!("v2:sha256:{}\n", launch.fingerprint).as_bytes(),
        )?;
        crate::store::atomic_write(
            &root.join("Deskflow.conf"),
            launch.settings(root)?.as_bytes(),
        )?;
        let runtime = std::env::current_exe()?.parent().unwrap().join("runtime");
        let binary = runtime.join("deskflow-core.exe");
        unsafe {
            let token = system_token(session)?;
            let mut uiaccess: u32 = 1;
            check(SetTokenInformation(
                token.0,
                TokenUIAccess,
                (&mut uiaccess as *mut u32).cast(),
                4,
            ))?;
            let job = Handle::new(CreateJobObjectW(null(), null()))?;
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            check(SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ))?;
            let mut read = null_mut();
            let mut write = null_mut();
            let sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: null_mut(),
                bInheritHandle: 1,
            };
            check(CreatePipe(&mut read, &mut write, &sa, 0))?;
            let read = Handle::new(read)?;
            let write = Handle::new(write)?;
            check(SetHandleInformation(read.0, HANDLE_FLAG_INHERIT, 0))?;
            let mut info: STARTUPINFOW = std::mem::zeroed();
            info.cb = std::mem::size_of_val(&info) as u32;
            let mut desktop = wide("winsta0\\Default");
            info.lpDesktop = desktop.as_mut_ptr();
            info.dwFlags = STARTF_USESTDHANDLES;
            let input = Handle::new(windows_sys::Win32::Storage::FileSystem::CreateFileW(
                wide("NUL").as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &sa,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            ))?;
            info.hStdInput = input.0;
            info.hStdOutput = write.0;
            info.hStdError = write.0;
            let mut result: PROCESS_INFORMATION = std::mem::zeroed();
            let mut command = wide(format!(
                "\"{}\" client --settings \"{}\"",
                binary.display(),
                root.join("Deskflow.conf").display()
            ));
            let process_sd = Descriptor::new(&format!(
                "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x00101000;;;{})",
                OWNER.get().unwrap()
            ))?;
            let process_sa = process_sd.attributes();
            check(CreateProcessAsUserW(
                token.0,
                wide(&binary).as_ptr(),
                command.as_mut_ptr(),
                &process_sa,
                null(),
                1,
                CREATE_SUSPENDED | CREATE_NO_WINDOW,
                null(),
                wide(&runtime).as_ptr(),
                &info,
                &mut result,
            ))?;
            let child = Handle::new(result.hProcess)?;
            let thread = Handle::new(result.hThread)?;
            if AssignProcessToJobObject(job.0, child.0) == 0 {
                TerminateProcess(child.0, 1);
                WaitForSingleObject(child.0, 5000);
                anyhow::bail!("无法绑定引擎退出保护");
            }
            if ResumeThread(thread.0) == u32::MAX {
                anyhow::bail!("无法启动服务引擎");
            }
            drop(write);
            let raw = read.0;
            std::mem::forget(read);
            let file = std::fs::File::from_raw_handle(raw);
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            tokio::task::spawn_blocking(move || {
                use std::io::Read;
                let mut file = file;
                let mut data = [0u8; 4096];
                let mut line = Vec::new();
                while let Ok(n) = file.read(&mut data) {
                    if n == 0 {
                        break;
                    }
                    for b in &data[..n] {
                        if *b == b'\n' {
                            let text = String::from_utf8_lossy(&line).trim().to_owned();
                            line.clear();
                            if !text.contains("clipboard")
                                && !text.contains("DEBUG")
                                && !text.contains("VERBOSE")
                            {
                                let _ = tx.try_send(text);
                            }
                        } else if line.len() < 16384 {
                            line.push(*b);
                        }
                    }
                }
            });
            Ok((
                Self {
                    child,
                    job,
                    pid: result.dwProcessId,
                },
                rx,
            ))
        }
    }
    fn stop(&mut self) -> Result<()> {
        // The UI sends a verified core IPC stop first. Allow it to release
        // injected keys normally; the job remains the final cleanup guarantee.
        if self.child.wait(1500) {
            return Ok(());
        }
        unsafe {
            check(TerminateJobObject(self.job.0, 0))?;
            ensure!(
                WaitForSingleObject(self.child.0, 5000) == WAIT_OBJECT_0,
                "服务引擎尚未退出"
            );
        }
        Ok(())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
fn system_token(session: u32) -> Result<Handle> {
    unsafe {
        let snapshot = Handle::new(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0))?;
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut next = Process32FirstW(snapshot.0, &mut entry);
        while next != 0 {
            let len = entry
                .szExeFile
                .iter()
                .position(|x| *x == 0)
                .unwrap_or(entry.szExeFile.len());
            let mut id = 0;
            if String::from_utf16_lossy(&entry.szExeFile[..len])
                .eq_ignore_ascii_case("winlogon.exe")
                && ProcessIdToSessionId(entry.th32ProcessID, &mut id) != 0
                && id == session
            {
                let process = Handle::new(OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION,
                    0,
                    entry.th32ProcessID,
                ))?;
                ensure!(sid(process.0)? == "S-1-5-18", "会话进程身份无效");
                let mut token = null_mut();
                check(OpenProcessToken(
                    process.0,
                    TOKEN_DUPLICATE | TOKEN_QUERY,
                    &mut token,
                ))?;
                let token = Handle::new(token)?;
                let mut primary = null_mut();
                check(DuplicateTokenEx(
                    token.0,
                    TOKEN_ALL_ACCESS,
                    null(),
                    SecurityImpersonation,
                    TokenPrimary,
                    &mut primary,
                ))?;
                return Handle::new(primary);
            }
            next = Process32NextW(snapshot.0, &mut entry);
        }
    }
    anyhow::bail!("当前控制台会话尚未准备，未启动引擎")
}
fn enable_debug_privilege() -> Result<()> {
    unsafe {
        let mut token = null_mut();
        check(OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        ))?;
        let token = Handle::new(token)?;
        let mut luid = std::mem::zeroed();
        check(LookupPrivilegeValueW(null(), SE_DEBUG_NAME, &mut luid))?;
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        SetLastError(0);
        check(AdjustTokenPrivileges(
            token.0,
            0,
            &privileges,
            0,
            null_mut(),
            null_mut(),
        ))?;
        ensure!(
            GetLastError() != ERROR_NOT_ALL_ASSIGNED,
            "后台服务缺少会话进程访问权限"
        );
    }
    Ok(())
}
