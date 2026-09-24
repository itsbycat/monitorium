use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::model::{MonitorId, PowerOffMethod, SoftwareDimMode};

const SETTINGS_FILE: &str = "settings.json";
const STATE_FILE: &str = "state.json";

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from_path(PathBuf::from("Monitorium"))
}

pub fn config_dir() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.config_dir().to_path_buf())
}

pub fn data_local_dir() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.data_local_dir().to_path_buf())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitorSettings {
    pub name: Option<String>,
    pub min: u8,
    pub max: u8,
    pub hidden: bool,
    pub force_software: bool,
}

impl Default for MonitorSettings {
    fn default() -> Self {
        Self {
            name: None,
            min: 0,
            max: 100,
            hidden: false,
            force_software: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemePreference,
    pub start_with_os: bool,
    pub link_levels: bool,
    pub hdr_sdr_brightness: bool,
    pub software_fallback: bool,
    pub software_dim_mode: SoftwareDimMode,
    pub power_off_method: PowerOffMethod,
    pub scroll_on_tray: bool,
    pub scroll_step: u8,
    pub hotkeys_enabled: bool,
    pub hotkey_up: String,
    pub hotkey_down: String,
    pub hotkey_off: String,
    pub hotkey_step: u8,
    pub monitors: BTreeMap<MonitorId, MonitorSettings>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            start_with_os: true,
            link_levels: false,
            hdr_sdr_brightness: true,
            software_fallback: true,
            software_dim_mode: SoftwareDimMode::Overlay,
            power_off_method: PowerOffMethod::System,
            scroll_on_tray: true,
            scroll_step: 5,
            hotkeys_enabled: true,
            hotkey_up: "Ctrl+Alt+ArrowUp".into(),
            hotkey_down: "Ctrl+Alt+ArrowDown".into(),
            hotkey_off: String::new(),
            hotkey_step: 10,
            monitors: BTreeMap::new(),
        }
    }
}

impl Settings {
    pub fn load() -> (Self, bool) {
        let Some(path) = config_dir().map(|dir| dir.join(SETTINGS_FILE)) else {
            return (Self::default(), true);
        };
        if !path.exists() {
            return (Self::default(), true);
        }
        (read_json(&path).unwrap_or_default(), false)
    }

    pub fn save(&self) -> io::Result<()> {
        let dir = config_dir().ok_or_else(|| io::Error::other("no config directory"))?;
        write_json(&dir.join(SETTINGS_FILE), self)
    }

    pub fn monitor(&self, id: &MonitorId) -> MonitorSettings {
        self.monitors.get(id).cloned().unwrap_or_default()
    }

    pub fn monitor_mut(&mut self, id: &MonitorId) -> &mut MonitorSettings {
        self.monitors.entry(id.clone()).or_default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub software_levels: BTreeMap<MonitorId, u8>,
}

impl State {
    pub fn load() -> Self {
        config_dir()
            .and_then(|dir| read_json(&dir.join(STATE_FILE)))
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let dir = config_dir().ok_or_else(|| io::Error::other("no config directory"))?;
        write_json(&dir.join(STATE_FILE), self)
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        Err(err) => {
            log::warn!("ignoring invalid {}: {err}", path.display());
            None
        }
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(value).map_err(io::Error::other)?,
    )?;
    fs::rename(&tmp, path)
}
