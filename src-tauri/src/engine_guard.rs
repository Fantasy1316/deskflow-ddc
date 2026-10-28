//! A separate macOS process releases the owned engine if its app dies before
//! async cleanup can run. It never scans or terminates another app's engine.
use anyhow::{ensure, Context, Result};
use std::{io::Read, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::AsyncBufReadExt,
    process::{Child, ChildStdin, Command},
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct ProcessIdentity {
    parent: u32,
    uid: u32,
    seconds: u64,
    micros: u64,
}
fn identity(pid: u32) -> Result<ProcessIdentity> {
    ensure!(pid > 1 && pid <= i32::MAX as u32, "无效的引擎进程 ID");
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            size,
        )
    };
    ensure!(read == size && info.pbi_pid == pid, "无法核验引擎进程身份");
    Ok(ProcessIdentity {
        parent: info.pbi_ppid,
        uid: info.pbi_uid,
        seconds: info.pbi_start_tvsec,
        micros: info.pbi_start_tvusec,
    })
}

fn owned_parents(owner: u32, guard_parent: u32, engine_parent: u32) -> bool {
    // These kernel snapshots are not atomic: either sibling can be reparented
    // to launchd first when the owner exits. UID and exact engine start time
    // are checked separately; never require matching parent snapshots.
    owner > 1 && [owner, 1].contains(&guard_parent) && [owner, 1].contains(&engine_parent)
}

pub struct Guardian {
    child: Child,
    lease: Option<ChildStdin>,
}
impl Guardian {
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    pub async fn start(engine: &mut Child, executable: &Path) -> Result<Self> {
        let pid = engine.id().context("引擎已结束")?;
        let fingerprint = identity(pid)?;
        ensure!(
            fingerprint.parent == std::process::id(),
            "引擎不属于当前应用"
        );
        let mut child = Command::new(executable)
            .args([
                "--engine-guardian",
                &std::process::id().to_string(),
                &pid.to_string(),
                &fingerprint.seconds.to_string(),
                &fingerprint.micros.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            // The guardian must outlive a crashed/killed app.
            .kill_on_drop(false)
            .spawn()
            .context("无法启动引擎退出保护")?;
        let output = child.stdout.take().context("退出保护确认通道不可用")?;
        let mut lines = tokio::io::BufReader::new(output).lines();
        let ready = tokio::time::timeout(Duration::from_secs(3), lines.next_line()).await;
        if !matches!(ready, Ok(Ok(Some(ref line))) if line == "ready") {
            let _ = child.kill().await;
            anyhow::bail!("引擎退出保护未就绪，禁止开始共享");
        }
        Ok(Self {
            lease: child.stdin.take(),
            child,
        })
    }

    // Only call once the engine has been reaped. EOF is also delivered by the
    // kernel if the app exits abruptly, so the guard does not depend on Tokio.
    pub async fn finish(mut self) {
        self.lease.take();
        if tokio::time::timeout(Duration::from_secs(2), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
    }
}

pub fn run_if_requested() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("--engine-guardian") {
        return None;
    }
    Some(match run(&args) {
        Ok(()) => 0,
        Err(_) => 2,
    })
}
fn run(args: &[String]) -> Result<()> {
    use std::io::Write;
    ensure!(args.len() == 6, "无效的退出保护参数");
    let parent: u32 = args[2].parse()?;
    let pid: u32 = args[3].parse()?;
    let initial = identity(pid)?;
    let actual_parent = unsafe { libc::getppid() } as u32;
    let owned = owned_parents(parent, actual_parent, initial.parent);
    ensure!(
        owned
            && initial.uid == unsafe { libc::getuid() }
            && initial.seconds == args[4].parse::<u64>()?
            && initial.micros == args[5].parse::<u64>()?,
        "退出保护引擎身份不匹配"
    );
    let mut output = std::io::stdout();
    // A dead owner may have already closed stdout. Its lease still needs to
    // be read so the engine is released rather than abandoned on a broken pipe.
    let _ = writeln!(output, "ready");
    let _ = output.flush();
    // No commands are accepted. The sole trigger is the owner's pipe closing.
    let mut byte = [0u8; 1];
    loop {
        match std::io::stdin().read(&mut byte) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    if identity(pid).is_ok_and(|current| {
        current.uid == initial.uid
            && current.seconds == initial.seconds
            && current.micros == initial.micros
    }) {
        // PID reuse cannot target an unrelated process: match kernel start time.
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_both_orders_of_owner_death_during_parent_snapshots() {
        for guard in [42, 1] {
            for engine in [42, 1] {
                assert!(owned_parents(42, guard, engine));
            }
        }
        assert!(!owned_parents(42, 43, 1));
        assert!(!owned_parents(42, 1, 43));
        assert!(!owned_parents(1, 1, 1));
        assert!(!owned_parents(0, 1, 1));
    }
}
