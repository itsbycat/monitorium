use std::collections::{HashMap, HashSet};

use monitorium_core::{
    Backend, BackendError, Display, HardwareKind, MonitorId, PowerOffMethod, SoftwareDimMode,
};
use objc2_core_graphics::CGDirectDisplayID;

use super::ddc::{self, Channel};
use super::display::{self, Screen};
use super::service::PlatformHandle;
use super::{gamma, native};

const APPLE_VENDOR: u32 = 0x610;
const VCP_BRIGHTNESS: u8 = 0x10;
const VCP_POWER_MODE: u8 = 0xD6;
const POWER_MODE_OFF: u16 = 4;
// DDC/CI through Apple silicon's display controller often fails for a moment after wake
const DDC_GRACE_REFRESHES: u8 = 2;

enum Control {
    Native,
    Ddc { channel: Channel, max: u16 },
    None,
}

struct Monitor {
    display: CGDirectDisplayID,
    control: Control,
}

struct DdcReading {
    percent: u8,
    max: u16,
    failures: u8,
}

pub struct MacBackend {
    platform: PlatformHandle,
    monitors: HashMap<MonitorId, Monitor>,
    overlays: HashSet<MonitorId>,
    gamma: HashMap<MonitorId, CGDirectDisplayID>,
    ddc_readings: HashMap<MonitorId, DdcReading>,
}

impl MacBackend {
    pub fn new(platform: PlatformHandle) -> Self {
        Self {
            platform,
            monitors: HashMap::new(),
            overlays: HashSet::new(),
            gamma: HashMap::new(),
            ddc_readings: HashMap::new(),
        }
    }

    fn control(
        &mut self,
        screen: &Screen,
        services: &mut Vec<ddc::Service>,
    ) -> (Control, Option<(HardwareKind, u8)>) {
        // DisplayServices also claims third-party monitors it can't drive, so only ask it about
        // Apple's own
        if (screen.internal || screen.vendor == APPLE_VENDOR)
            && let Some(level) = native::brightness(screen.display)
        {
            return (Control::Native, Some((HardwareKind::Native, level)));
        }

        let identity = (screen.vendor, screen.model, screen.serial);
        let Some(mut channel) =
            ddc::take(services, screen.location.as_deref(), identity).and_then(ddc::Service::open)
        else {
            return (Control::None, None);
        };
        if let Some((current, max)) = channel.read(VCP_BRIGHTNESS) {
            let percent =
                ((u32::from(current.min(max)) * 100 + u32::from(max) / 2) / u32::from(max)) as u8;
            self.ddc_readings.insert(
                screen.id.clone(),
                DdcReading {
                    percent,
                    max,
                    failures: 0,
                },
            );
            return (
                Control::Ddc { channel, max },
                Some((HardwareKind::Ddc, percent)),
            );
        }
        match self.ddc_readings.get_mut(&screen.id) {
            Some(reading) if reading.failures < DDC_GRACE_REFRESHES => {
                reading.failures += 1;
                log::info!(
                    "{}: DDC/CI read failed, keeping the last level",
                    screen.name
                );
                (
                    Control::Ddc {
                        channel,
                        max: reading.max,
                    },
                    Some((HardwareKind::Ddc, reading.percent)),
                )
            }
            _ => {
                self.ddc_readings.remove(&screen.id);
                log::info!("{}: no DDC/CI brightness", screen.name);
                (Control::None, None)
            }
        }
    }

    fn clear_overlay(&mut self, id: &MonitorId) {
        if self.overlays.remove(id) {
            self.platform.remove_overlay(id.clone());
        }
    }

    fn clear_gamma(&mut self, id: &MonitorId) {
        if let Some(display) = self.gamma.remove(id) {
            gamma::reset(display);
        }
    }
}

impl Backend for MacBackend {
    fn enumerate(&mut self) -> Vec<Display> {
        self.monitors.clear();
        let mut services = ddc::services();
        let mut displays = Vec::new();

        for screen in display::online() {
            let (control, hardware) = self.control(&screen, &mut services);
            log::info!(
                "monitor {} \"{}\" (display {}): {hardware:?}",
                screen.id,
                screen.name,
                screen.display
            );
            displays.push(Display {
                id: screen.id.clone(),
                name: screen.name,
                bounds: screen.bounds,
                internal: screen.internal,
                hdr: false,
                hardware,
                sdr: None,
            });
            self.monitors.insert(
                screen.id,
                Monitor {
                    display: screen.display,
                    control,
                },
            );
        }
        self.ddc_readings
            .retain(|id, _| self.monitors.contains_key(id));
        displays
    }

    fn set_hardware(&mut self, id: &MonitorId, percent: u8) -> Result<(), BackendError> {
        let monitor = self.monitors.get_mut(id).ok_or(BackendError::NotFound)?;
        match &mut monitor.control {
            Control::Native => {
                native::set_brightness(monitor.display, percent).map_err(BackendError::Failed)
            }
            Control::Ddc { channel, max } => {
                let raw = ((u32::from(percent.min(100)) * u32::from(*max) + 50) / 100) as u16;
                if !channel.write(VCP_BRIGHTNESS, raw) {
                    return Err(BackendError::Failed("DDC/CI write failed".into()));
                }
                if let Some(reading) = self.ddc_readings.get_mut(id) {
                    reading.percent = percent;
                }
                Ok(())
            }
            Control::None => Err(BackendError::Unsupported),
        }
    }

    fn set_software(
        &mut self,
        id: &MonitorId,
        mode: SoftwareDimMode,
        level: u8,
    ) -> Result<(), BackendError> {
        let display = self.monitors.get(id).ok_or(BackendError::NotFound)?.display;
        match mode {
            SoftwareDimMode::Overlay => {
                self.clear_gamma(id);
                if level >= 100 {
                    self.clear_overlay(id);
                } else {
                    self.overlays.insert(id.clone());
                    self.platform.set_overlay(id.clone(), display, level);
                }
                Ok(())
            }
            SoftwareDimMode::Gamma => {
                self.clear_overlay(id);
                if level >= 100 {
                    self.clear_gamma(id);
                    return Ok(());
                }
                self.gamma.insert(id.clone(), display);
                gamma::set_level(display, level).map_err(BackendError::Failed)
            }
        }
    }

    fn clear_software(&mut self, id: &MonitorId) {
        self.clear_overlay(id);
        self.clear_gamma(id);
    }

    fn clear_all_software(&mut self) {
        self.platform.clear_overlays();
        self.overlays.clear();
        self.gamma.clear();
        gamma::reset_all();
    }

    fn power_off(&mut self, method: PowerOffMethod, ddc_ids: &[MonitorId]) {
        if matches!(method, PowerOffMethod::Ddc | PowerOffMethod::Both) {
            for id in ddc_ids {
                if let Some(Monitor {
                    control: Control::Ddc { channel, .. },
                    ..
                }) = self.monitors.get_mut(id)
                    && !channel.write(VCP_POWER_MODE, POWER_MODE_OFF)
                {
                    log::warn!("DDC power off of {id} failed");
                }
            }
        }
        if matches!(method, PowerOffMethod::System | PowerOffMethod::Both) {
            self.platform.power_off_displays();
        }
    }
}
