use std::ptr::NonNull;
use std::sync::OnceLock;

use monitorium_core::{MonitorId, Rect};
use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType, CFUUID, CGPoint, CGRect};
use objc2_core_graphics::{
    CGDirectDisplayID, CGDisplayBounds, CGDisplayIsBuiltin, CGDisplayModelNumber,
    CGDisplaySerialNumber, CGDisplayVendorNumber, CGError, CGGetOnlineDisplayList, CGMainDisplayID,
};

use super::{cf_dictionary, cf_string, open_library, symbol};

const MAX_DISPLAYS: usize = 32;

#[link(name = "ColorSync", kind = "framework")]
unsafe extern "C" {
    fn CGDisplayCreateUUIDFromDisplayID(display: CGDirectDisplayID) -> Option<NonNull<CFUUID>>;
}

type CreateInfoDictionary =
    unsafe extern "C" fn(CGDirectDisplayID) -> Option<NonNull<CFDictionary>>;

pub struct Screen {
    pub display: CGDirectDisplayID,
    pub id: MonitorId,
    pub name: String,
    pub bounds: Rect,
    pub internal: bool,
    pub vendor: u32,
    pub model: u32,
    pub serial: u32,
    /// IORegistry path of the display's framebuffer
    pub location: Option<String>,
}

pub fn online() -> Vec<Screen> {
    let mut displays = [0; MAX_DISPLAYS];
    let mut count = 0u32;
    let status =
        unsafe { CGGetOnlineDisplayList(MAX_DISPLAYS as u32, displays.as_mut_ptr(), &mut count) };
    if status != CGError::Success {
        log::warn!("CGGetOnlineDisplayList failed ({})", status.0);
        return Vec::new();
    }

    let mut unnamed = 0;
    displays[..count as usize]
        .iter()
        .map(|&display| {
            let info = info(display);
            let internal = CGDisplayIsBuiltin(display);
            let (vendor, model, serial) = (
                CGDisplayVendorNumber(display),
                CGDisplayModelNumber(display),
                CGDisplaySerialNumber(display),
            );
            let name = if internal {
                "Built-in display".to_string()
            } else if let Some(name) = info.as_deref().and_then(product_name) {
                name
            } else {
                unnamed += 1;
                format!("Display {unnamed}")
            };
            let id =
                uuid(display).unwrap_or_else(|| format!("{vendor:04X}{model:04X}#{serial:08X}"));
            let b = CGDisplayBounds(display);
            Screen {
                display,
                id: MonitorId(id),
                name,
                bounds: Rect {
                    x: b.origin.x.round() as i32,
                    y: b.origin.y.round() as i32,
                    width: b.size.width.round() as i32,
                    height: b.size.height.round() as i32,
                },
                internal,
                vendor,
                model,
                serial,
                location: info
                    .as_deref()
                    .and_then(|info| cf_string(info, "IODisplayLocation")),
            }
        })
        .collect()
}

/// The display's frame in AppKit coordinates, which start at the bottom left of the main
/// display and grow upwards.
pub fn cocoa_frame(display: CGDirectDisplayID) -> CGRect {
    to_cocoa(CGDisplayBounds(display), main_height())
}

pub fn main_height() -> f64 {
    CGDisplayBounds(CGMainDisplayID()).size.height
}

fn to_cocoa(bounds: CGRect, main_height: f64) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: bounds.origin.x,
            y: main_height - bounds.origin.y - bounds.size.height,
        },
        size: bounds.size,
    }
}

// macOS's own identity for the display, which also tells identical monitors apart
fn uuid(display: CGDirectDisplayID) -> Option<String> {
    let uuid = unsafe { CFRetained::from_raw(CGDisplayCreateUUIDFromDisplayID(display)?) };
    CFUUID::new_string(None, Some(&uuid)).map(|s| s.to_string())
}

fn info(display: CGDirectDisplayID) -> Option<CFRetained<CFDictionary>> {
    static CREATE: OnceLock<Option<CreateInfoDictionary>> = OnceLock::new();
    let create = (*CREATE.get_or_init(|| unsafe {
        symbol(
            open_library(c"/System/Library/Frameworks/CoreDisplay.framework/CoreDisplay"),
            c"CoreDisplay_DisplayCreateInfoDictionary",
        )
    }))?;
    let info = unsafe { create(display) }?;
    Some(unsafe { CFRetained::from_raw(info) })
}

fn product_name(info: &CFDictionary) -> Option<String> {
    // localized names, keyed by locale
    let names = cf_dictionary(info, "DisplayProductName")?;
    if let Some(name) = cf_string(&names, "en_US") {
        return Some(name);
    }
    let (_, values) = unsafe { names.cast_unchecked::<CFString, CFType>() }.to_vecs();
    values
        .into_iter()
        .find_map(|value| value.downcast::<CFString>().ok())
        .map(|name| name.to_string())
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::CGSize;

    use super::*;

    #[test]
    fn flips_to_cocoa_coordinates() {
        let rect = |x, y, width, height| CGRect {
            origin: CGPoint { x, y },
            size: CGSize { width, height },
        };
        // a 1080p display to the left of a 1440p main display, both aligned at the top
        assert_eq!(
            to_cocoa(rect(-1920.0, 0.0, 1920.0, 1080.0), 1440.0),
            rect(-1920.0, 360.0, 1920.0, 1080.0)
        );
        assert_eq!(
            to_cocoa(rect(0.0, 0.0, 2560.0, 1440.0), 1440.0),
            rect(0.0, 0.0, 2560.0, 1440.0)
        );
    }
}
