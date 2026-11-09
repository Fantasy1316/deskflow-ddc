//! A private stdin lease keeps recovery independent of the desktop runtime.
//! EOF (including an app crash), an explicit restore, or a pre-commit timeout
//! restores the captured output. Failed restoration retains the baseline for a
//! real retry while the owner lives. Commit never claims monitor readback.
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
};

pub const PROTOCOL: u8 = 4;
const GUARD_SECONDS: u64 = 25;

pub struct Lease {
    child: Child,
    input: Option<ChildStdin>,
    replies: Lines<BufReader<ChildStdout>>,
    terminal: Option<Value>,
    recovering: bool,
}
impl Lease {
    pub async fn start(resources: &Path, output: &str) -> Result<Self> {
        Self::spawn(&std::env::current_exe()?, resources, output).await
    }
    async fn spawn(executable: &Path, resources: &Path, output: &str) -> Result<Self> {
        let mut command = Command::new(executable);
        command
            .arg("--output-guardian")
            .arg(resources)
            .arg(output)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn().context("无法启动显示输出恢复保护")?;
        let mut lease = Self {
            input: child.stdin.take(),
            replies: BufReader::new(child.stdout.take().context("恢复保护确认通道不可用")?).lines(),
            child,
            terminal: None,
            recovering: false,
        };
        lease.reply("ready").await?;
        Ok(lease)
    }
    async fn reply(&mut self, expected: &str) -> Result<()> {
        let value = if let Some(terminal) = &self.terminal {
            terminal.clone()
        } else {
            tokio::time::timeout(Duration::from_secs(12), async {
                loop {
                    let line = self
                        .replies
                        .next_line()
                        .await?
                        .context("显示输出恢复保护已结束，输出将自动恢复")?;
                    ensure!(line.len() < 4096, "恢复保护响应过大");
                    let value: Value = serde_json::from_str(&line)?;
                    if value["event"] == "restored" && value["ok"] == true {
                        self.terminal = Some(value.clone());
                    }
                    // A timed-out command may have left its acknowledgement in
                    // the pipe. Restoration must wait for the terminal result.
                    if expected != "restored" || value["event"] == "restored" {
                        return Ok::<_, anyhow::Error>(value);
                    }
                }
            })
            .await
            .context("显示输出恢复保护响应超时")??
        };
        ensure!(
            value["event"] == expected,
            "恢复保护响应与指令不匹配，输出可能已自动恢复，请重新切换"
        );
        ensure!(
            value["ok"] == true,
            "{}",
            value["error"].as_str().unwrap_or("显示输出操作失败")
        );
        Ok(())
    }
    pub async fn command(&mut self, command: &str) -> Result<()> {
        ensure!(!self.recovering, "显示输出尚未恢复，请先重试恢复输出");
        ensure!(
            self.child.try_wait()?.is_none(),
            "输出已自动恢复，请重新切换"
        );
        self.input
            .as_mut()
            .context("恢复保护通道已关闭")?
            .write_all(format!("{command}\n").as_bytes())
            .await?;
        self.reply(command).await
    }
    pub async fn restore(&mut self) -> Result<()> {
        // The guard may already have reached its deadline. Read its terminal
        // restoration acknowledgement instead of treating process exit or OS
        // enumeration (which remains present during Windows sleep) as success.
        self.recovering = true;
        if let Some(input) = self.input.as_mut() {
            let _ = input.write_all(b"restore\n").await;
        }
        let result = self.reply("restored").await;
        if result.is_ok() {
            self.input.take();
            // Never kill a guardian that may still be restoring the display.
            let _ = tokio::time::timeout(Duration::from_secs(12), self.child.wait()).await;
        }
        // On failure keep stdin open. The guardian retains the original
        // baseline and accepts another restore; owner death still delivers EOF.
        result
    }
}

pub fn run_if_requested() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("--output-guardian") {
        return None;
    }
    let result = (|| -> Result<()> {
        ensure!(args.len() == 4, "无效的显示输出恢复参数");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let result = runtime.block_on(run(args[2].as_ref(), &args[3]));
        // Tokio's stdin worker can still be blocked in a pipe read after the
        // pre-commit deadline. It must not stall process termination.
        runtime.shutdown_background();
        result
    })();
    Some(if result.is_ok() { 0 } else { 2 })
}

async fn reply(
    writer: &mut (impl AsyncWrite + Unpin),
    event: &str,
    result: &Result<()>,
) -> Result<()> {
    let value = match result {
        Ok(()) => json!({"event":event,"ok":true}),
        Err(error) => json!({"event":event,"ok":false,"error":format!("{error:#}")}),
    };
    writer.write_all(format!("{value}\n").as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

trait OutputBackend {
    async fn capture(&self) -> Result<Value>;
    async fn disable(&self, baseline: &Value) -> Result<()>;
    async fn restore(&self, baseline: &Value) -> Result<()>;
}
struct NativeOutput<'a> {
    platform: &'a crate::platform::Platform,
    output: &'a str,
}
impl OutputBackend for NativeOutput<'_> {
    async fn capture(&self) -> Result<Value> {
        self.platform.capture_output(self.output).await
    }
    async fn disable(&self, baseline: &Value) -> Result<()> {
        self.platform.disable_output(self.output, baseline).await
    }
    async fn restore(&self, baseline: &Value) -> Result<()> {
        self.platform.restore_output(self.output, baseline).await
    }
}
async fn run(resources: &Path, output: &str) -> Result<()> {
    let platform = crate::platform::Platform::new(resources.into());
    let backend = NativeOutput {
        platform: &platform,
        output,
    };
    let result = serve(
        &backend,
        tokio::io::stdin(),
        tokio::io::stdout(),
        Duration::from_secs(GUARD_SECONDS),
    )
    .await;
    platform.close().await;
    result
}
async fn serve(
    backend: &impl OutputBackend,
    input: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    timeout: Duration,
) -> Result<()> {
    let baseline = backend.capture().await;
    reply(
        &mut writer,
        "ready",
        &baseline
            .as_ref()
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("{e:#}")),
    )
    .await?;
    let baseline = baseline?;
    let mut lines = BufReader::new(input).lines();
    let mut changed = false;
    let mut deadline = None;
    let mut owner_gone = false;
    let operations: Result<()> = async {
        loop {
            let line = if let Some(until) = deadline {
                match tokio::time::timeout_at(until, lines.next_line()).await {
                    Ok(line) => line?,
                    Err(_) => break,
                }
            } else {
                lines.next_line().await?
            };
            let Some(line) = line else {
                owner_gone = true;
                break;
            };
            ensure!(line.len() < 64, "无效的恢复保护指令");
            match line.as_str() {
                "disable" => {
                    // Even an error can follow a partial native change.
                    changed = true;
                    let result = backend.disable(&baseline).await;
                    // Native calls have their own bound. Give the owner its
                    // complete pre-commit window after the call returns.
                    deadline = Some(tokio::time::Instant::now() + timeout);
                    reply(&mut writer, "disable", &result).await?;
                    result?;
                }
                "confirm" => {
                    ensure!(changed, "没有待确认的切换");
                    deadline = None;
                    reply(&mut writer, "confirm", &Ok(())).await?;
                }
                "restore" => break,
                _ => bail!("不支持的恢复保护指令"),
            }
        }
        Ok(())
    }
    .await;
    loop {
        let restored = if changed {
            let first = backend.restore(&baseline).await;
            // Give a transient link-training failure one automatic retry.
            if first.is_err() {
                tokio::time::sleep(Duration::from_secs(1)).await;
                backend.restore(&baseline).await
            } else {
                first
            }
        } else {
            Ok(())
        };
        let _ = reply(&mut writer, "restored", &restored).await;
        if restored.is_ok() || owner_gone {
            return operations.and(restored);
        }
        // Recovery failed, not the ownership lease. Retain this exact baseline
        // instead of recapturing the disabled topology in a new process.
        loop {
            match lines.next_line().await {
                Ok(Some(line)) if line == "restore" => break,
                Ok(Some(line)) if line.len() < 64 => {
                    let _ = reply(
                        &mut writer,
                        &line,
                        &Err(anyhow::anyhow!("显示输出尚未恢复，只能重试恢复")),
                    )
                    .await;
                }
                // EOF/crash must make one last recovery attempt independently.
                _ => {
                    owner_gone = true;
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };

    #[cfg(unix)]
    pub(crate) async fn retrying_lease(directory: &Path, failures: usize) -> Lease {
        use std::os::unix::fs::PermissionsExt;
        let script = directory.join("retrying-guardian");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
printf '%s\n' '{{"event":"ready","ok":true}}'
read operation
printf '%s\n' '{{"event":"disable","ok":true}}'
remaining={failures}
while read operation; do
    echo "$operation" >> "$0.calls"
    test "$operation" = restore || exit 7
    if [ "$remaining" -gt 0 ]; then
        remaining=$((remaining - 1))
        printf '%s\n' '{{"event":"restored","ok":false,"error":"temporary failure"}}'
    else
        printf '%s\n' '{{"event":"restored","ok":true}}'
        exit 0
    fi
done
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut lease = Lease::spawn(&script, directory, "external").await.unwrap();
        lease.command("disable").await.unwrap();
        lease
    }

    #[derive(Default)]
    struct MockOutput {
        disabled: AtomicBool,
        restored: AtomicUsize,
        fail_disable: bool,
        fail_restore: AtomicBool,
    }
    impl OutputBackend for MockOutput {
        async fn capture(&self) -> Result<Value> {
            Ok(json!({"originalMode":"saved"}))
        }
        async fn disable(&self, _: &Value) -> Result<()> {
            self.disabled.store(true, Ordering::Release);
            ensure!(!self.fail_disable, "partial native failure");
            Ok(())
        }
        async fn restore(&self, baseline: &Value) -> Result<()> {
            assert_eq!(baseline["originalMode"], "saved");
            self.restored.fetch_add(1, Ordering::AcqRel);
            ensure!(
                !self.fail_restore.load(Ordering::Acquire),
                "transient restore failure"
            );
            self.disabled.store(false, Ordering::Release);
            Ok(())
        }
    }
    async fn read_reply(
        reader: &mut Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    ) -> bool {
        let line = reader.next_line().await.unwrap().unwrap();
        serde_json::from_str::<Value>(&line).unwrap()["ok"] == true
    }
    async fn fixture(
        fail: bool,
    ) -> (
        Arc<MockOutput>,
        tokio::io::WriteHalf<tokio::io::DuplexStream>,
        Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
        tokio::task::JoinHandle<Result<()>>,
    ) {
        let backend = Arc::new(MockOutput {
            fail_disable: fail,
            ..Default::default()
        });
        let (owner, guard) = tokio::io::duplex(4096);
        let (input, output) = tokio::io::split(guard);
        let cloned = backend.clone();
        let task = tokio::spawn(async move {
            serve(cloned.as_ref(), input, output, Duration::from_millis(80)).await
        });
        let (input, output) = tokio::io::split(owner);
        let mut replies = BufReader::new(input).lines();
        assert!(read_reply(&mut replies).await);
        (backend, output, replies, task)
    }
    #[tokio::test]
    async fn eof_without_disable_never_changes_output() {
        let (output, owner, replies, task) = fixture(false).await;
        drop((owner, replies));
        task.await.unwrap().unwrap();
        assert_eq!(output.restored.load(Ordering::Acquire), 0);
    }
    #[tokio::test]
    async fn timeout_restores_original_output_without_owner_cooperation() {
        let (output, mut owner, mut replies, task) = fixture(false).await;
        owner.write_all(b"disable\n").await.unwrap();
        assert!(read_reply(&mut replies).await);
        assert!(output.disabled.load(Ordering::Acquire));
        assert!(read_reply(&mut replies).await);
        task.await.unwrap().unwrap();
        assert!(!output.disabled.load(Ordering::Acquire));
        assert_eq!(output.restored.load(Ordering::Acquire), 1);
    }
    #[tokio::test]
    async fn confirmation_preserves_disabled_output_but_owner_death_still_restores() {
        let (output, mut owner, mut replies, mut task) = fixture(false).await;
        owner.write_all(b"disable\n").await.unwrap();
        assert!(read_reply(&mut replies).await);
        owner.write_all(b"confirm\n").await.unwrap();
        assert!(read_reply(&mut replies).await);
        assert!(tokio::time::timeout(Duration::from_millis(160), &mut task)
            .await
            .is_err());
        assert!(output.disabled.load(Ordering::Acquire));
        drop((owner, replies));
        task.await.unwrap().unwrap();
        assert!(!output.disabled.load(Ordering::Acquire));
        assert_eq!(output.restored.load(Ordering::Acquire), 1);
    }
    #[tokio::test]
    async fn partial_native_failure_and_explicit_restore_both_restore_once() {
        for fail in [false, true] {
            let (output, mut owner, mut replies, task) = fixture(fail).await;
            owner.write_all(b"disable\n").await.unwrap();
            assert_eq!(read_reply(&mut replies).await, !fail);
            if !fail {
                owner.write_all(b"restore\n").await.unwrap();
            }
            assert!(read_reply(&mut replies).await);
            assert_eq!(task.await.unwrap().is_ok(), !fail);
            assert_eq!(output.restored.load(Ordering::Acquire), 1);
            assert!(!output.disabled.load(Ordering::Acquire));
        }
    }

    #[tokio::test]
    async fn failed_recovery_keeps_baseline_for_a_real_retry_or_owner_death() {
        for eof in [false, true] {
            let (output, mut owner, mut replies, mut task) = fixture(false).await;
            output.fail_restore.store(true, Ordering::Release);
            owner.write_all(b"disable\n").await.unwrap();
            assert!(read_reply(&mut replies).await);
            if !eof {
                owner.write_all(b"restore\n").await.unwrap();
            }
            // Both the explicit request and the pre-commit deadline can fail.
            assert!(!read_reply(&mut replies).await);
            assert_eq!(output.restored.load(Ordering::Acquire), 2);
            assert!(output.disabled.load(Ordering::Acquire));
            assert!(futures_util::poll!(&mut task).is_pending());
            // A failed lease cannot be committed/reused as another disconnect.
            owner.write_all(b"confirm\n").await.unwrap();
            assert!(!read_reply(&mut replies).await);
            output.fail_restore.store(false, Ordering::Release);
            if eof {
                drop((owner, replies));
            } else {
                owner.write_all(b"restore\n").await.unwrap();
                assert!(read_reply(&mut replies).await);
            }
            task.await.unwrap().unwrap();
            assert_eq!(output.restored.load(Ordering::Acquire), 3);
            assert!(!output.disabled.load(Ordering::Acquire));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lease_retry_sends_another_restore_after_failure_instead_of_caching_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("retrying-guardian");
        std::fs::write(
            &script,
            r#"#!/bin/sh
printf '%s\n' '{"event":"ready","ok":true}'
read operation
printf '%s\n' '{"event":"disable","ok":true}'
read operation
echo "$operation" >> "$0.calls"
printf '%s\n' '{"event":"restored","ok":false,"error":"temporary failure"}'
read operation
echo "$operation" >> "$0.calls"
printf '%s\n' '{"event":"restored","ok":true}'
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut lease = Lease::spawn(&script, dir.path(), "external").await.unwrap();
        lease.command("disable").await.unwrap();
        assert!(lease.restore().await.is_err());
        assert!(lease.command("disable").await.is_err());
        lease.restore().await.unwrap();
        lease.restore().await.unwrap(); // Successful recovery remains idempotent.
        assert_eq!(
            std::fs::read_to_string(script.with_extension("calls")).unwrap(),
            "restore\nrestore\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exited_guard_requires_its_terminal_recovery_acknowledgement() {
        use std::os::unix::fs::PermissionsExt;
        for accepted in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let script = directory.path().join("fake-output-guardian");
            let terminal = if accepted {
                r#"{"event":"restored","ok":true}"#
            } else {
                r#"{"event":"restored","ok":false,"error":"wake input rejected"}"#
            };
            std::fs::write(&script, format!(
                "#!/bin/sh\nprintf '%s\\n' '{{\"event\":\"ready\",\"ok\":true}}'\nread operation\nprintf '%s\\n' '{{\"event\":\"disable\",\"ok\":true}}'\nprintf '%s\\n' '{terminal}'\n"
            )).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut lease = Lease::spawn(&script, directory.path(), "external")
                .await
                .unwrap();
            lease.command("disable").await.unwrap();
            lease.child.wait().await.unwrap();
            assert_eq!(lease.restore().await.is_ok(), accepted);
            // A consumed failure must not become success on a later retry.
            assert_eq!(lease.restore().await.is_ok(), accepted);
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_restoration_reply_cannot_confirm_a_switch() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("fake-output-guardian");
        std::fs::write(
            &script,
            r#"#!/bin/sh
printf '%s\n' '{"event":"ready","ok":true}'
read operation
printf '%s\n' '{"event":"disable","ok":true}'
printf '%s\n' '{"event":"restored","ok":true}'
read operation
read operation
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut lease = Lease::spawn(&script, directory.path(), "external")
            .await
            .unwrap();
        lease.command("disable").await.unwrap();
        assert!(lease.command("confirm").await.is_err());
        lease.restore().await.unwrap();
    }
}
