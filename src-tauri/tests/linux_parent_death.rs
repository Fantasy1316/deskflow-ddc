#![cfg(target_os = "linux")]
use std::{
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn alive(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    // A container's init can leave a dead child as a zombie; it cannot hold input.
    stat.rsplit_once(") ")
        .is_some_and(|(_, state)| !state.starts_with('Z'))
}
#[test]
fn abrupt_owner_death_releases_only_its_child() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("child.pid");
    let mut owner = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "linux_owner_fixture", "--ignored", "--nocapture"])
        .env("DDC_LINUX_OWNER_FIXTURE", &file)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if !file.exists() {
        let _ = owner.kill();
        let _ = owner.wait();
        panic!("owner fixture did not initialize");
    }
    let pid: u32 = std::fs::read_to_string(file).unwrap().parse().unwrap();
    let mut foreign = Command::new("sleep").arg("30").spawn().unwrap();
    owner.kill().unwrap();
    owner.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let released = !alive(pid);
    let untouched = foreign.try_wait().unwrap().is_none();
    let _ = foreign.kill();
    let _ = foreign.wait();
    if !released {
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
    assert!(released, "owned child survived abrupt owner death");
    assert!(untouched, "unrelated child was affected");
}
#[test]
#[ignore = "subprocess fixture for abrupt_owner_death_releases_only_its_child"]
fn linux_owner_fixture() {
    let Ok(file) = std::env::var("DDC_LINUX_OWNER_FIXTURE") else {
        return;
    };
    let owner = std::process::id();
    let mut command = Command::new("sleep");
    command.arg("30");
    unsafe {
        command.pre_exec(move || deskflow_ddc_tauri::linux::arm_parent_death_signal(owner));
    }
    let mut child = command.spawn().unwrap();
    // Wait until the executable has completed pre_exec and is alive.
    assert!(child.try_wait().unwrap().is_none());
    std::fs::write(file, child.id().to_string()).unwrap();
    let _ = child.wait();
}
