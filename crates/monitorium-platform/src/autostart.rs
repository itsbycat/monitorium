use auto_launch::{AutoLaunch, AutoLaunchBuilder};

const APP_NAME: &str = "Monitorium";

pub const AUTOSTART_ARG: &str = "--autostart";

fn launcher() -> Result<AutoLaunch, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_string_lossy();
    // auto-launch doesn't quote the path when joining it with the args
    let path = if cfg!(windows) {
        format!("\"{exe}\"")
    } else {
        exe.into_owned()
    };

    let mut builder = AutoLaunchBuilder::new();
    builder
        .set_app_name(APP_NAME)
        .set_app_path(&path)
        .set_args(&[AUTOSTART_ARG]);
    #[cfg(windows)]
    builder.set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);
    builder.build().map_err(|e| e.to_string())
}

pub fn is_enabled() -> bool {
    launcher()
        .and_then(|l| l.is_enabled().map_err(|e| e.to_string()))
        .unwrap_or(false)
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let launcher = launcher()?;
    let result = if enabled {
        launcher.enable()
    } else if launcher.is_enabled().unwrap_or(false) {
        launcher.disable()
    } else {
        Ok(())
    };
    result.map_err(|e| e.to_string())
}
