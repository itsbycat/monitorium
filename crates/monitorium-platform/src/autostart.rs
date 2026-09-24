use auto_launch::{AutoLaunch, AutoLaunchBuilder};

// on macOS this names the LaunchAgent, and doubles as the bundle identifier
#[cfg(target_os = "macos")]
const APP_NAME: &str = "com.bycat.monitorium";
#[cfg(not(target_os = "macos"))]
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
    // shows the app's name and icon for the login item in System Settings
    #[cfg(target_os = "macos")]
    builder.set_bundle_identifiers(&[APP_NAME]);
    builder.build().map_err(|e| e.to_string())
}

pub fn is_enabled() -> bool {
    launcher()
        .and_then(|l| l.is_enabled().map_err(|e| e.to_string()))
        .unwrap_or(false)
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if enabled {
        macos::check_location()?;
    }
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

/// Points an existing login item at this copy of the app, which on macOS may have been moved.
pub fn refresh() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if is_enabled() && macos::moved() {
        return set_enabled(true);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::PathBuf;

    use super::APP_NAME;

    fn exe() -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(exe.to_string_lossy().into_owned())
    }

    // a disk image, or a quarantined download macOS runs from a random temporary folder
    pub fn check_location() -> Result<(), String> {
        let exe = exe()?;
        if exe.starts_with("/Volumes/") || exe.contains("/AppTranslocation/") {
            return Err("Move Monitorium to the Applications folder first".into());
        }
        Ok(())
    }

    /// Whether the installed app runs from somewhere else than the LaunchAgent starts it from.
    pub fn moved() -> bool {
        let (Ok(exe), Some(home)) = (exe(), std::env::var_os("HOME")) else {
            return false;
        };
        // only a bundled app takes over the login item, not a build run from the source tree
        if !exe.contains(".app/Contents/MacOS/") {
            return false;
        }
        let plist = PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{APP_NAME}.plist"));
        std::fs::read_to_string(plist)
            .is_ok_and(|plist| !plist.contains(&format!("<string>{exe}</string>")))
    }
}
