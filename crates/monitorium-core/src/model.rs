use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MonitorId(pub String);

impl fmt::Display for MonitorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SoftwareDimMode {
    #[default]
    Overlay,
    Gamma,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerOffMethod {
    #[default]
    System,
    Ddc,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Ddc,
    Native,
    Sdr,
    Overlay,
    Gamma,
    None,
}

impl Method {
    pub fn is_software(self) -> bool {
        matches!(self, Method::Overlay | Method::Gamma)
    }

    pub fn label(self) -> &'static str {
        match self {
            Method::Ddc => "DDC/CI",
            // on macOS this also covers Apple's external displays
            Method::Native if cfg!(target_os = "macos") => "Native",
            Method::Native => "Built-in",
            Method::Sdr => "SDR content brightness (HDR)",
            Method::Overlay => "Software (overlay)",
            Method::Gamma => "Software (gamma)",
            Method::None => "Unsupported",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    pub id: MonitorId,
    pub name: String,
    pub hardware_name: String,
    pub method: Method,
    pub brightness: u8,
    pub internal: bool,
    pub hidden: bool,
    pub has_hardware: bool,
    pub hdr: bool,
}
