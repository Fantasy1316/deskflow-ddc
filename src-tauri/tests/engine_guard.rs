#![cfg(all(target_os = "macos", feature = "desktop"))]

use deskflow_ddc_tauri::engine_guard::Guardian;
use std::{
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::process::Command;

fn alive(pid: u32) -> bool {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let n = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            size,
        )
    };
    n == size && info.pbi_status != libc::SZOMB
}

#[tokio::test]
async fn guardian_owner_fixture() {
    let Some(file) = std::env::var_os("DDC_GUARD_FIXTURE_FILE") else {
        return;
    };
    let mut engine = Command::new("python3")
        .args([
            "-c",
            "import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(60)",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let guardian = Guardian::start(&mut engine, Path::new(env!("CARGO_BIN_EXE_deskflow-ddc")))
        .await
        .unwrap();
    std::fs::write(
        file,
        format!("{} {}", engine.id().unwrap(), guardian.pid().unwrap()),
    )
    .unwrap();
    // The supervising test kills this owner without allowing any Rust Drop or
    // application exit handler to run. The separate guardian must still work.
    std::future::pending::<()>().await;
    guardian.finish().await;
}

#[test]
fn parent_death_reaps_only_its_guarded_engine_without_gui_or_accessibility() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("owned-pids");
    let mut owner = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "guardian_owner_fixture", "--nocapture"])
        .env("DDC_GUARD_FIXTURE_FILE", &file)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !file.exists() && Instant::now() < deadline {
        if let Some(status) = owner.try_wait().unwrap() {
            panic!("Owner failed before guardian ready: {status}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !file.exists() {
        let _ = owner.kill();
        let _ = owner.wait();
        panic!("Guardian did not acknowledge startup");
    }
    let pids: Vec<u32> = std::fs::read_to_string(file)
        .unwrap()
        .split_whitespace()
        .map(|p| p.parse().unwrap())
        .collect();
    let mut unrelated = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    assert!(alive(pids[0]) && alive(pids[1]));
    owner.kill().unwrap();
    owner.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while (alive(pids[0]) || alive(pids[1])) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let engine_alive = alive(pids[0]);
    let guard_alive = alive(pids[1]);
    let untouched = unrelated.try_wait().unwrap().is_none();
    unrelated.kill().unwrap();
    unrelated.wait().unwrap();
    assert!(!engine_alive, "Engine survived abrupt owner death");
    assert!(!guard_alive, "Guardian did not exit after releasing engine");
    assert!(untouched, "Unrelated process was affected");
}

#[tokio::test]
async fn normal_cleanup_reaps_guard_and_rejects_a_reused_process_identity() {
    let mut child = Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_deskflow-ddc"));
    let guardian = Guardian::start(&mut child, binary).await.unwrap();
    let pid = guardian.pid().unwrap();
    child.kill().await.unwrap();
    guardian.finish().await;
    assert!(!alive(pid));

    let mut unrelated = Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let result = Command::new(binary)
        .args([
            "--engine-guardian",
            &std::process::id().to_string(),
            &unrelated.id().unwrap().to_string(),
            "0",
            "0",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(!result.status.success());
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.kill().await.unwrap();
}
