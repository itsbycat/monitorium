use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::Duration;

use monitorium_core::{
    Backend, BackendError, Display, HardwareKind, MonitorId, PowerOffMethod, Rect, SoftwareDimMode,
};
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitor, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, PHYSICAL_MONITOR,
    SetVCPFeature,
};
use windows::Win32::Foundation::{GetLastError, HANDLE, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::core::BOOL;

use super::display_config::{self, TargetRef, monitor_key};
use super::service::PlatformHandle;
use super::wmi_brightness::WmiBrightness;
use super::{from_wide, gamma};

const VCP_BRIGHTNESS: u8 = 0x10;
const VCP_POWER_MODE: u8 = 0xD6;
const POWER_MODE_OFF: u32 = 4;
const DDC_ATTEMPTS: usize = 3;
const DDC_RETRY_DELAY: Duration = Duration::from_millis(50);

struct PhysicalMonitor(HANDLE);

impl Drop for PhysicalMonitor {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyPhysicalMonitor(self.0);
        }
    }
}

struct Monitor {
    gdi_name: String,
    bounds: Rect,
    ddc: Option<(PhysicalMonitor, u32)>,
    wmi_key: Option<String>,
    sdr_target: Option<TargetRef>,
}

pub struct WindowsBackend {
    platform: PlatformHandle,
    wmi: Option<WmiBrightness>,
    monitors: HashMap<MonitorId, Monitor>,
    overlays: HashSet<MonitorId>,
    gamma: HashMap<MonitorId, String>,
}

impl WindowsBackend {
    pub fn new(platform: PlatformHandle) -> Self {
        Self {
            platform,
            wmi: None,
            monitors: HashMap::new(),
            overlays: HashSet::new(),
            gamma: HashMap::new(),
        }
    }

    fn wmi(&mut self) -> &mut WmiBrightness {
        self.wmi.get_or_insert_with(WmiBrightness::new)
    }

    fn clear_overlay(&mut self, id: &MonitorId) {
        if self.overlays.remove(id) {
            self.platform.remove_overlay(id.clone());
        }
    }

    fn clear_gamma(&mut self, id: &MonitorId) {
        if let Some(gdi) = self.gamma.remove(id) {
            gamma::reset(&gdi);
        }
    }
}

impl Backend for WindowsBackend {
    fn enumerate(&mut self) -> Vec<Display> {
        self.monitors.clear();

        let targets = display_config::active_targets();
        let wmi_levels = self.wmi().read_all();
        let mut used = vec![false; targets.len()];
        let mut displays = Vec::new();
        let mut unnamed = 0;

        for (hmonitor, gdi_name, bounds) in display_monitors() {
            let candidates: Vec<usize> = (0..targets.len())
                .filter(|&t| !used[t] && targets[t].gdi_name.eq_ignore_ascii_case(&gdi_name))
                .collect();
            let mut physical_for: HashMap<usize, PhysicalMonitor> = HashMap::new();

            for (index, (handle, description)) in
                physical_monitors(hmonitor).into_iter().enumerate()
            {
                let free = |t: &&usize| !physical_for.contains_key(t);
                let by_name: Vec<usize> = candidates
                    .iter()
                    .filter(free)
                    .filter(|&&t| targets[t].friendly_name == description)
                    .copied()
                    .collect();
                let pick = if by_name.len() == 1 {
                    Some(by_name[0])
                } else {
                    candidates
                        .get(index)
                        .copied()
                        .filter(|t| !physical_for.contains_key(t))
                };
                if let Some(t) = pick {
                    physical_for.insert(t, handle);
                }
            }

            for t in candidates {
                used[t] = true;
                let target = &targets[t];
                let key = monitor_key(&target.device_path);
                let id = MonitorId(key.clone().unwrap_or_else(|| format!("{gdi_name}#{t}")));
                let name = if !target.friendly_name.is_empty() {
                    target.friendly_name.clone()
                } else if target.internal {
                    "Built-in display".to_string()
                } else {
                    unnamed += 1;
                    format!("Display {unnamed}")
                };

                let wmi_level = key.as_ref().and_then(|k| wmi_levels.get(k).copied());
                let mut ddc = None;
                let hardware = if let Some(level) = wmi_level {
                    Some((HardwareKind::Wmi, level))
                } else if let Some(handle) = physical_for.remove(&t) {
                    match read_vcp(handle.0, VCP_BRIGHTNESS) {
                        Ok((current, max)) if max > 0 => {
                            let percent = ((current.min(max) * 100 + max / 2) / max) as u8;
                            ddc = Some((handle, max));
                            Some((HardwareKind::Ddc, percent))
                        }
                        Ok(_) => None,
                        Err(code) => {
                            log::info!("{name}: no DDC/CI brightness (error {code:#x})");
                            None
                        }
                    }
                } else {
                    None
                };

                log::info!(
                    "monitor {id} \"{name}\" on {gdi_name}: {hardware:?}{}",
                    match target.sdr {
                        Some(sdr) => format!(" (HDR, SDR brightness {sdr}%)"),
                        None if target.hdr => " (HDR)".to_string(),
                        None => String::new(),
                    }
                );
                displays.push(Display {
                    id: id.clone(),
                    name,
                    bounds,
                    internal: target.internal,
                    hdr: target.hdr,
                    hardware,
                    sdr: target.sdr,
                });
                self.monitors.insert(
                    id,
                    Monitor {
                        gdi_name: gdi_name.clone(),
                        bounds,
                        ddc,
                        wmi_key: wmi_level.and(key),
                        sdr_target: target.sdr.map(|_| target.target),
                    },
                );
            }
        }
        displays
    }

    fn set_hardware(&mut self, id: &MonitorId, percent: u8) -> Result<(), BackendError> {
        let monitor = self.monitors.get(id).ok_or(BackendError::NotFound)?;
        if let Some(key) = monitor.wmi_key.clone() {
            return self.wmi().set(&key, percent).map_err(BackendError::Failed);
        }
        let (handle, max) = monitor.ddc.as_ref().ok_or(BackendError::Unsupported)?;
        let raw = (u32::from(percent.min(100)) * max + 50) / 100;
        write_vcp(handle.0, VCP_BRIGHTNESS, raw)
            .map_err(|code| BackendError::Failed(format!("DDC/CI write failed ({code:#x})")))
    }

    fn set_sdr(&mut self, id: &MonitorId, percent: u8) -> Result<(), BackendError> {
        let monitor = self.monitors.get(id).ok_or(BackendError::NotFound)?;
        let target = monitor.sdr_target.ok_or(BackendError::Unsupported)?;
        display_config::set_sdr_percent(target, percent).map_err(|code| {
            BackendError::Failed(format!("setting SDR white level failed ({code:#x})"))
        })
    }

    fn set_software(
        &mut self,
        id: &MonitorId,
        mode: SoftwareDimMode,
        level: u8,
    ) -> Result<(), BackendError> {
        let monitor = self.monitors.get(id).ok_or(BackendError::NotFound)?;
        let (gdi_name, bounds) = (monitor.gdi_name.clone(), monitor.bounds);
        match mode {
            SoftwareDimMode::Overlay => {
                self.clear_gamma(id);
                if level >= 100 {
                    self.clear_overlay(id);
                } else {
                    self.overlays.insert(id.clone());
                    self.platform.set_overlay(id.clone(), bounds, level);
                }
                Ok(())
            }
            SoftwareDimMode::Gamma => {
                self.clear_overlay(id);
                if level >= 100 {
                    self.clear_gamma(id);
                    return Ok(());
                }
                self.gamma.insert(id.clone(), gdi_name.clone());
                gamma::set_level(&gdi_name, level).map_err(BackendError::Failed)
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
        for (_, gdi) in self.gamma.drain() {
            gamma::reset(&gdi);
        }
    }

    fn power_off(&mut self, method: PowerOffMethod, ddc_ids: &[MonitorId]) {
        if matches!(method, PowerOffMethod::Ddc | PowerOffMethod::Both) {
            for id in ddc_ids {
                if let Some((handle, _)) = self.monitors.get(id).and_then(|m| m.ddc.as_ref())
                    && let Err(code) = write_vcp(handle.0, VCP_POWER_MODE, POWER_MODE_OFF)
                {
                    log::warn!("DDC power off of {id} failed ({code:#x})");
                }
            }
        }
        if matches!(method, PowerOffMethod::System | PowerOffMethod::Both) {
            self.platform.power_off_displays();
        }
    }
}

fn display_monitors() -> Vec<(HMONITOR, String, Rect)> {
    unsafe extern "system" fn collect(
        hmonitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(hmonitor);
        true.into()
    }

    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut _ as isize),
        );
    }

    handles
        .into_iter()
        .filter_map(|hmonitor| {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            let ok = unsafe { GetMonitorInfoW(hmonitor, &mut info as *mut _ as *mut MONITORINFO) };
            if !ok.as_bool() {
                return None;
            }
            let r = info.monitorInfo.rcMonitor;
            let bounds = Rect {
                x: r.left,
                y: r.top,
                width: r.right - r.left,
                height: r.bottom - r.top,
            };
            Some((hmonitor, from_wide(&info.szDevice), bounds))
        })
        .collect()
}

fn physical_monitors(hmonitor: HMONITOR) -> Vec<(PhysicalMonitor, String)> {
    let mut count = 0u32;
    if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count) }.is_err()
        || count == 0
    {
        return Vec::new();
    }
    let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
    if unsafe { GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut monitors) }.is_err() {
        return Vec::new();
    }
    monitors
        .into_iter()
        .map(|m| {
            let handle = m.hPhysicalMonitor;
            let description = m.szPhysicalMonitorDescription;
            (PhysicalMonitor(handle), from_wide(&description))
        })
        .collect()
}

fn is_transient(code: u32) -> bool {
    matches!(
        code,
        0xC026_2582 // ERROR_GRAPHICS_I2C_ERROR_TRANSMITTING_DATA
            | 0xC026_2583 // ERROR_GRAPHICS_I2C_ERROR_RECEIVING_DATA
            | 0xC026_2585 // ERROR_GRAPHICS_DDCCI_INVALID_DATA
            | 0xC026_2586 // ERROR_GRAPHICS_DDCCI_MONITOR_RETURNED_INVALID_TIMING_STATUS_BYTE
            | 0xC026_2589 // ERROR_GRAPHICS_DDCCI_INVALID_MESSAGE_COMMAND
            | 0xC026_258A // ERROR_GRAPHICS_DDCCI_INVALID_MESSAGE_LENGTH
            | 0xC026_258B // ERROR_GRAPHICS_DDCCI_INVALID_MESSAGE_CHECKSUM
    )
}

fn with_retries<T>(mut op: impl FnMut() -> Option<T>) -> Result<T, u32> {
    let mut attempt = 0;
    loop {
        if let Some(value) = op() {
            return Ok(value);
        }
        let code = unsafe { GetLastError() }.0;
        attempt += 1;
        if attempt >= DDC_ATTEMPTS || !is_transient(code) {
            return Err(code);
        }
        thread::sleep(DDC_RETRY_DELAY);
    }
}

fn read_vcp(handle: HANDLE, code: u8) -> Result<(u32, u32), u32> {
    with_retries(|| {
        let (mut current, mut max) = (0u32, 0u32);
        let ok = unsafe {
            GetVCPFeatureAndVCPFeatureReply(handle, code, None, &mut current, Some(&mut max))
        };
        (ok != 0).then_some((current, max))
    })
}

fn write_vcp(handle: HANDLE, code: u8, value: u32) -> Result<(), u32> {
    with_retries(|| (unsafe { SetVCPFeature(handle, code, value) } != 0).then_some(()))
}
