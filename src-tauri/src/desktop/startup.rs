use super::*;
use anyhow::Context;
#[cfg(not(any(windows, target_os = "linux")))]
use tauri_plugin_autostart::ManagerExt;
#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(windows)]
mod windows;

fn enabled(app: &tauri::AppHandle) -> Result<bool> {
    #[cfg(windows)]
    {
        let _ = app;
        windows::enabled()
    }
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        linux::enabled()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Ok(app.autolaunch().is_enabled()?)
    }
}

fn set_enabled(app: &tauri::AppHandle, wanted: bool) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = app;
        windows::set_enabled(wanted)
    }
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        linux::set_enabled(wanted)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        if wanted {
            app.autolaunch().enable()?
        } else {
            app.autolaunch().disable()?
        };
        Ok(())
    }
}

pub(super) async fn refresh(app: &tauri::AppHandle, runtime: &Runtime) -> Result<()> {
    let handle = app.clone();
    let enabled = tauri::async_runtime::spawn_blocking(move || enabled(&handle)).await??;
    runtime.set_startup_registered(enabled).await;
    Ok(())
}

pub(super) async fn save(
    app: &tauri::AppHandle,
    runtime: &Runtime,
    settings: crate::startup::Preferences,
) -> Result<()> {
    runtime.validate_startup(&settings).await?;
    ensure!(
        !tauri::is_dev(),
        "请使用安装包设置开机自启动；开发环境不会修改系统启动项"
    );
    let handle = app.clone();
    let previous = tauri::async_runtime::spawn_blocking(move || enabled(&handle)).await??;
    let wanted = settings.launch_at_login;
    let handle = app.clone();
    let update = tauri::async_runtime::spawn_blocking(move || -> Result<()> {
        if wanted || previous {
            set_enabled(&handle, wanted).context("无法更新系统登录启动项")?;
        }
        ensure!(enabled(&handle)? == wanted, "系统登录启动项未按预期更新");
        Ok(())
    })
    .await?;
    let result = match update {
        Ok(()) => runtime.save_startup(settings, wanted).await,
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || set_enabled(&handle, previous))
            .await?
            .context("启动设置保存失败，系统启动项回滚也失败，请在系统设置中检查登录启动项")?;
        return Err(error).context("启动设置保存失败，系统启动项已恢复");
    }
    Ok(())
}

pub(super) fn initialize(app: tauri::AppHandle, runtime: Arc<Runtime>, has_tray: bool) {
    let background = std::env::args_os().any(|argument| argument == "--autostart") && has_tray;
    if background {
        // The window starts invisible, so login launch never flashes a window.
        #[cfg(target_os = "macos")]
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    } else {
        show(&app);
    }
    if tauri::is_dev() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Err(error) = refresh(&app, &runtime).await {
            runtime
                .log("error", &format!("登录启动项检测失败：{error}"))
                .await;
        }
        if let Err(error) = runtime.start_automatically().await {
            runtime
                .log(
                    "error",
                    &format!("自动共享未启动：{error}；请处理后手动开始共享"),
                )
                .await;
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || show(&handle));
        }
    });
}
