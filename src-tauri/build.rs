fn main() {
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            println!("cargo:rerun-if-changed=../native/mac-lifecycle.m");
            println!("cargo:rerun-if-changed=../native/mac-source-popover.m");
            cc::Build::new()
                .file("../native/mac-lifecycle.m")
                .file("../native/mac-source-popover.m")
                .flag("-fobjc-arc")
                .compile("deskflow-ddc-lifecycle");
            println!("cargo:rustc-link-lib=framework=AppKit");
        }
        tauri_build::build()
    }
}
