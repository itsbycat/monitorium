use std::ffi::c_void;

use windows::Win32::Graphics::Gdi::{CreateDCW, DeleteDC, HDC};
use windows::Win32::UI::ColorSystem::{GetDeviceGammaRamp, SetDeviceGammaRamp};
use windows::core::{PCWSTR, w};

use super::to_wide;

const MIN_FACTOR: f32 = 0.5;

struct DeviceContext(HDC);

impl DeviceContext {
    fn open(gdi_name: &str) -> Option<Self> {
        let name = to_wide(gdi_name);
        let dc = unsafe { CreateDCW(w!("DISPLAY"), PCWSTR(name.as_ptr()), None, None) };
        (!dc.is_invalid()).then_some(Self(dc))
    }
}

impl Drop for DeviceContext {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

fn ramp(factor: f32) -> [u16; 768] {
    let mut ramp = [0u16; 768];
    for i in 0..256 {
        let value = ((i as f32 * 257.0) * factor).round().clamp(0.0, 65535.0) as u16;
        ramp[i] = value;
        ramp[256 + i] = value;
        ramp[512 + i] = value;
    }
    ramp
}

pub fn set_level(gdi_name: &str, level: u8) -> Result<(), String> {
    let dc = DeviceContext::open(gdi_name).ok_or_else(|| format!("CreateDC({gdi_name}) failed"))?;
    let mut factor = MIN_FACTOR + (1.0 - MIN_FACTOR) * f32::from(level.min(100)) / 100.0;
    loop {
        let ramp = ramp(factor);
        let ok = unsafe { SetDeviceGammaRamp(dc.0, ramp.as_ptr() as *const c_void) }.as_bool();
        if ok && verify(&dc, &ramp) {
            return Ok(());
        }
        if factor >= 1.0 {
            return Err(format!("SetDeviceGammaRamp({gdi_name}) rejected"));
        }
        factor = (factor + 0.05).min(1.0);
    }
}

pub fn reset(gdi_name: &str) {
    if let Err(err) = set_level(gdi_name, 100) {
        log::warn!("resetting gamma: {err}");
    }
}

fn verify(dc: &DeviceContext, expected: &[u16; 768]) -> bool {
    let mut actual = [0u16; 768];
    let ok = unsafe { GetDeviceGammaRamp(dc.0, actual.as_mut_ptr() as *mut c_void) }.as_bool();
    if !ok {
        return true;
    }
    expected
        .iter()
        .zip(actual.iter())
        .all(|(e, a)| e.abs_diff(*a) <= 256)
}
