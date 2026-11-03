//! Explicit smoke test of the actual staged runtime; never starts sharing.
use deskflow_ddc_tauri::bundle;
use std::{path::PathBuf, time::Duration};

#[tokio::test]
#[ignore = "requires pnpm prepare:engine; run with --test bundled_runtime -- --ignored"]
async fn official_runtime_extracts_and_runs_without_an_installed_deskflow() {
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    let test_cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/bundle-smoke");
    std::fs::create_dir_all(&test_cache).unwrap();
    let profile = tempfile::Builder::new()
        .prefix("profile-")
        .tempdir_in(test_cache)
        .unwrap();
    let binary = bundle::install(&resources, profile.path()).unwrap();
    assert!(binary.starts_with(profile.path()));
    #[cfg(target_os = "macos")]
    {
        let app = binary.ancestors().nth(3).unwrap();
        let signature = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::process::Command::new("codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(app)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .expect("signature verification timed out")
        .expect("could not run codesign");
        assert!(
            signature.status.success(),
            "{}",
            String::from_utf8_lossy(&signature.stderr)
        );
    }
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(&binary)
            .arg("--version")
            .kill_on_drop(true)
            .output(),
    )
    .await;
    let output = match output {
        Ok(output) => output.expect("bundled engine did not launch"),
        Err(_) => panic!(
            "bundled engine timed out; diagnostic profile retained at {}",
            profile.keep().display()
        ),
    };
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let version = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(version.contains("1.27.0"), "unexpected engine: {version}");
    assert_eq!(bundle::install(&resources, profile.path()).unwrap(), binary);
}
