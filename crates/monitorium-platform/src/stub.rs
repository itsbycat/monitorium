use monitorium_core::{
    Backend, BackendError, Display, MonitorId, PowerOffMethod, Rect, SoftwareDimMode,
};

use crate::{EventCallback, LogicalSize, TrayAnchor};

struct StubBackend;

impl Backend for StubBackend {
    fn enumerate(&mut self) -> Vec<Display> {
        Vec::new()
    }

    fn set_hardware(&mut self, _id: &MonitorId, _percent: u8) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }

    fn set_software(
        &mut self,
        _id: &MonitorId,
        _mode: SoftwareDimMode,
        _level: u8,
    ) -> Result<(), BackendError> {
        Err(BackendError::Unsupported)
    }

    fn clear_software(&mut self, _id: &MonitorId) {}

    fn clear_all_software(&mut self) {}

    fn power_off(&mut self, _method: PowerOffMethod, _ddc_ids: &[MonitorId]) {}
}

pub fn create_backend(_platform: PlatformHandle) -> Box<dyn Backend> {
    Box::new(StubBackend)
}

pub fn init_process() {}

pub fn attach_console() {}

pub fn taskbar_uses_light_theme() -> bool {
    false
}

#[derive(Clone)]
pub struct PlatformHandle;

impl PlatformHandle {
    pub fn set_overlay(&self, _id: MonitorId, _bounds: Rect, _level: u8) {}
    pub fn remove_overlay(&self, _id: MonitorId) {}
    pub fn clear_overlays(&self) {}
    pub fn set_scroll_hook(&self, _rect: Option<Rect>) {}
    pub fn power_off_displays(&self) {}
}

pub fn start_service(_on_event: EventCallback) -> PlatformHandle {
    PlatformHandle
}

pub fn place_flyout(_raw: isize, _anchor: TrayAnchor, _size: LogicalSize, _show: bool) {}

pub fn hide_window(_raw: isize) {}

pub fn is_window_visible(_raw: isize) -> bool {
    false
}
