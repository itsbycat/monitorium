use std::sync::OnceLock;

use objc2_core_graphics::CGDirectDisplayID;

use super::{open_library, symbol};

type GetBrightness = unsafe extern "C" fn(CGDirectDisplayID, *mut f32) -> i32;
type SetBrightness = unsafe extern "C" fn(CGDirectDisplayID, f32) -> i32;

// the private framework behind the brightness keys; it drives built-in and Apple displays
struct DisplayServices {
    get: GetBrightness,
    set: SetBrightness,
}

fn api() -> Option<&'static DisplayServices> {
    static API: OnceLock<Option<DisplayServices>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let library = open_library(
            c"/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices",
        );
        Some(DisplayServices {
            get: symbol(library, c"DisplayServicesGetBrightness")?,
            set: symbol(library, c"DisplayServicesSetBrightness")?,
        })
    })
    .as_ref()
}

pub fn brightness(display: CGDirectDisplayID) -> Option<u8> {
    let api = api()?;
    let mut value = -1.0f32;
    let status = unsafe { (api.get)(display, &mut value) };
    (status == 0 && (0.0..=1.0).contains(&value)).then(|| (value * 100.0).round() as u8)
}

pub fn set_brightness(display: CGDirectDisplayID, percent: u8) -> Result<(), String> {
    let api = api().ok_or("DisplayServices unavailable")?;
    match unsafe { (api.set)(display, f32::from(percent.min(100)) / 100.0) } {
        0 => Ok(()),
        code => Err(format!("DisplayServices error {code}")),
    }
}
