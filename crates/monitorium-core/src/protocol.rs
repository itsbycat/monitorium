use crate::model::{MonitorId, MonitorInfo};
use crate::settings::Settings;

#[derive(Debug, Clone)]
pub enum WorkerCmd {
    Refresh,
    SetBrightness { id: MonitorId, value: u8 },
    SetAll(u8),
    Step(i16),
    TurnOffDisplays,
    UpdateSettings(Box<Settings>),
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Monitors(Vec<MonitorInfo>),
}
