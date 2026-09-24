use std::mem::size_of;

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
    DISPLAYCONFIG_DEVICE_INFO_TYPE, DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, DisplayConfigSetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, LUID};

use super::from_wide;

const SDR_MIN_NITS: u32 = 80;
const SDR_MAX_NITS: u32 = 480;
const SDR_STEP_NITS: u32 = 4;
const SET_SDR_WHITE_LEVEL: DISPLAYCONFIG_DEVICE_INFO_TYPE =
    DISPLAYCONFIG_DEVICE_INFO_TYPE(0xFFFF_FFEE_u32 as i32);

#[derive(Clone, Copy, Debug)]
pub struct TargetRef {
    adapter_id: LUID,
    id: u32,
}

#[repr(C)]
struct SetSdrWhiteLevel {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    sdr_white_level: u32,
    final_value: u8,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub target: TargetRef,
    pub gdi_name: String,
    pub device_path: String,
    pub friendly_name: String,
    pub internal: bool,
    pub hdr: bool,
    pub sdr: Option<u8>,
}

pub fn active_targets() -> Vec<Target> {
    let paths = match query_paths() {
        Some(paths) => paths,
        None => {
            log::warn!("QueryDisplayConfig failed");
            return Vec::new();
        }
    };
    paths.iter().filter_map(target_for_path).collect()
}

fn query_paths() -> Option<Vec<DISPLAYCONFIG_PATH_INFO>> {
    for _ in 0..5 {
        let mut path_count = 0u32;
        let mut mode_count = 0u32;
        let status = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status != ERROR_SUCCESS {
            return None;
        }
        paths.truncate(path_count as usize);
        return Some(paths);
    }
    None
}

fn target_for_path(path: &DISPLAYCONFIG_PATH_INFO) -> Option<Target> {
    let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: header(
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(),
            path.sourceInfo.adapterId,
            path.sourceInfo.id,
        ),
        ..Default::default()
    };
    if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != 0 {
        return None;
    }

    let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: header(
            DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(),
            path.targetInfo.adapterId,
            path.targetInfo.id,
        ),
        ..Default::default()
    };
    if unsafe { DisplayConfigGetDeviceInfo(&mut target.header) } != 0 {
        return None;
    }

    let mut color = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
        header: header(
            DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
            size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(),
            path.targetInfo.adapterId,
            path.targetInfo.id,
        ),
        ..Default::default()
    };
    // bit 0: advanced color supported, bit 1: enabled
    let hdr = unsafe { DisplayConfigGetDeviceInfo(&mut color.header) } == 0
        && unsafe { color.Anonymous.value } & 0b11 == 0b11;

    let tech = target.outputTechnology;
    let internal = tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
        || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
        || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED
        || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS;

    let target_ref = TargetRef {
        adapter_id: path.targetInfo.adapterId,
        id: path.targetInfo.id,
    };
    Some(Target {
        target: target_ref,
        gdi_name: from_wide(&source.viewGdiDeviceName),
        device_path: from_wide(&target.monitorDevicePath),
        friendly_name: from_wide(&target.monitorFriendlyDeviceName),
        internal,
        hdr,
        sdr: if hdr { sdr_percent(target_ref) } else { None },
    })
}

fn sdr_percent(target: TargetRef) -> Option<u8> {
    let mut info = DISPLAYCONFIG_SDR_WHITE_LEVEL {
        header: header(
            DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
            size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>(),
            target.adapter_id,
            target.id,
        ),
        ..Default::default()
    };
    if unsafe { DisplayConfigGetDeviceInfo(&mut info.header) } != 0 {
        return None;
    }
    let nits = (info.SDRWhiteLevel * 80 / 1000).clamp(SDR_MIN_NITS, SDR_MAX_NITS);
    let range = SDR_MAX_NITS - SDR_MIN_NITS;
    Some((((nits - SDR_MIN_NITS) * 100 + range / 2) / range) as u8)
}

pub fn set_sdr_percent(target: TargetRef, percent: u8) -> Result<(), i32> {
    let range = SDR_MAX_NITS - SDR_MIN_NITS;
    let nits = SDR_MIN_NITS + u32::from(percent.min(100)) * range / 100;
    let nits = (nits + SDR_STEP_NITS / 2) / SDR_STEP_NITS * SDR_STEP_NITS;
    let packet = SetSdrWhiteLevel {
        header: header(
            SET_SDR_WHITE_LEVEL,
            size_of::<SetSdrWhiteLevel>(),
            target.adapter_id,
            target.id,
        ),
        sdr_white_level: nits * 1000 / 80,
        final_value: 1,
    };
    match unsafe { DisplayConfigSetDeviceInfo(&packet.header) } {
        0 => Ok(()),
        code => Err(code),
    }
}

fn header(
    kind: windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_TYPE,
    size: usize,
    adapter_id: windows::Win32::Foundation::LUID,
    id: u32,
) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: kind,
        size: size as u32,
        adapterId: adapter_id,
        id,
    }
}

pub fn monitor_key(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(['#', '\\']).filter(|p| !p.is_empty()).collect();
    let display = parts
        .iter()
        .position(|p| p.eq_ignore_ascii_case("DISPLAY"))?;
    let model = parts.get(display + 1)?;
    let instance = parts.get(display + 2)?;
    let instance = instance.split('_').next().unwrap_or(instance);
    Some(format!(
        "{}#{}",
        model.to_ascii_uppercase(),
        instance.to_ascii_uppercase()
    ))
}

#[cfg(test)]
mod tests {
    use super::monitor_key;

    #[test]
    fn keys_match_between_sources() {
        let path =
            r"\\?\DISPLAY#BOE0747#4&2a6e4d4d&0&UID8388688#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
        let wmi = r"DISPLAY\BOE0747\4&2a6e4d4d&0&UID8388688_0";
        assert_eq!(
            monitor_key(path).as_deref(),
            Some("BOE0747#4&2A6E4D4D&0&UID8388688")
        );
        assert_eq!(monitor_key(path), monitor_key(wmi));
        assert_eq!(monitor_key(""), None);
    }
}
