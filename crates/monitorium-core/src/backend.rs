use crate::model::{MonitorId, PowerOffMethod, Rect, SoftwareDimMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareKind {
    Ddc,
    Wmi,
}

#[derive(Clone, Debug)]
pub struct Display {
    pub id: MonitorId,
    pub name: String,
    pub bounds: Rect,
    pub internal: bool,
    pub hdr: bool,
    pub hardware: Option<(HardwareKind, u8)>,
    pub sdr: Option<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("monitor not found")]
    NotFound,
    #[error("not supported on this monitor")]
    Unsupported,
    #[error("{0}")]
    Failed(String),
}

pub trait Backend {
    fn enumerate(&mut self) -> Vec<Display>;

    fn set_hardware(&mut self, id: &MonitorId, percent: u8) -> Result<(), BackendError>;

    fn set_sdr(&mut self, _id: &MonitorId, _percent: u8) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }

    fn set_software(
        &mut self,
        id: &MonitorId,
        mode: SoftwareDimMode,
        level: u8,
    ) -> Result<(), BackendError>;

    fn clear_software(&mut self, id: &MonitorId);

    fn clear_all_software(&mut self);

    fn power_off(&mut self, method: PowerOffMethod, ddc_ids: &[MonitorId]);
}

pub fn slider_to_level(value: u8, min: u8, max: u8) -> u8 {
    let (min, max) = ordered(min, max);
    let value = u32::from(value.min(100));
    (u32::from(min) + (value * u32::from(max - min) + 50) / 100) as u8
}

pub fn level_to_slider(level: u8, min: u8, max: u8) -> u8 {
    let (min, max) = ordered(min, max);
    if max == min {
        return 100;
    }
    let level = level.clamp(min, max);
    ((u32::from(level - min) * 100 + u32::from(max - min) / 2) / u32::from(max - min)) as u8
}

fn ordered(min: u8, max: u8) -> (u8, u8) {
    let min = min.min(100);
    let max = max.min(100);
    if min <= max { (min, max) } else { (max, min) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remap_roundtrip() {
        for v in 0..=100u8 {
            assert_eq!(slider_to_level(v, 0, 100), v);
            assert_eq!(level_to_slider(v, 0, 100), v);
        }
        assert_eq!(slider_to_level(0, 20, 80), 20);
        assert_eq!(slider_to_level(100, 20, 80), 80);
        assert_eq!(slider_to_level(50, 20, 80), 50);
        assert_eq!(level_to_slider(20, 20, 80), 0);
        assert_eq!(level_to_slider(80, 20, 80), 100);
        assert_eq!(level_to_slider(10, 20, 80), 0);
        assert_eq!(level_to_slider(50, 50, 50), 100);
    }
}
