use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use wmi::WMIConnection;

use super::display_config::monitor_key;

#[derive(Deserialize)]
#[serde(rename = "WmiMonitorBrightness")]
#[serde(rename_all = "PascalCase")]
struct WmiMonitorBrightness {
    instance_name: String,
    current_brightness: u8,
    active: bool,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct WmiMonitorBrightnessMethods {
    __Path: String,
    InstanceName: String,
    Active: bool,
}

#[derive(Serialize)]
#[allow(non_snake_case)]
struct WmiSetBrightnessParams {
    Timeout: u32,
    Brightness: u8,
}

pub struct WmiBrightness {
    connection: Option<WMIConnection>,
    method_paths: HashMap<String, String>,
}

impl WmiBrightness {
    pub fn new() -> Self {
        let connection = match WMIConnection::with_namespace_path("ROOT\\WMI") {
            Ok(connection) => Some(connection),
            Err(err) => {
                log::warn!("WMI unavailable: {err}");
                None
            }
        };
        Self {
            connection,
            method_paths: HashMap::new(),
        }
    }

    pub fn read_all(&mut self) -> HashMap<String, u8> {
        self.method_paths.clear();
        let Some(connection) = &self.connection else {
            return HashMap::new();
        };

        let levels: Vec<WmiMonitorBrightness> = connection.query().unwrap_or_default();
        let methods: Vec<WmiMonitorBrightnessMethods> = connection.query().unwrap_or_default();

        for method in methods.into_iter().filter(|m| m.Active) {
            if let Some(key) = monitor_key(&method.InstanceName) {
                self.method_paths.insert(key, method.__Path);
            }
        }
        levels
            .into_iter()
            .filter(|l| l.active)
            .filter_map(|l| Some((monitor_key(&l.instance_name)?, l.current_brightness)))
            .filter(|(key, _)| self.method_paths.contains_key(key))
            .collect()
    }

    pub fn set(&self, key: &str, percent: u8) -> Result<(), String> {
        let connection = self.connection.as_ref().ok_or("WMI unavailable")?;
        let path = self.method_paths.get(key).ok_or("panel not found")?;
        connection
            .exec_instance_method::<WmiMonitorBrightnessMethods, ()>(
                path,
                "WmiSetBrightness",
                WmiSetBrightnessParams {
                    Timeout: 0,
                    Brightness: percent.min(100),
                },
            )
            .map_err(|e| e.to_string())
    }
}
