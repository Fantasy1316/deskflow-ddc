fn main() {
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            println!("cargo:rerun-if-changed=../native/mac-lifecycle.m");
            println!("cargo:rerun-if-changed=../native/mac-source-popover.m");
            println!("cargo:rerun-if-changed=../native/mac-tray-file-drop.m");
            cc::Build::new()
                .file("../native/mac-lifecycle.m")
                .file("../native/mac-source-popover.m")
                .file("../native/mac-tray-file-drop.m")
                .flag("-fobjc-arc")
                .compile("deskflow-ddc-lifecycle");
            println!("cargo:rustc-link-lib=framework=AppKit");
            println!("cargo:rustc-link-lib=framework=QuartzCore");
        }
        tauri_build::build()
    }
}
