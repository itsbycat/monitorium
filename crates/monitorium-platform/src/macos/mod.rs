mod backend;
mod ddc;
mod display;
mod gamma;
mod native;
mod service;
mod window;

use std::ffi::{CStr, c_void};

use monitorium_core::Backend;
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType, Type};

pub use service::{PlatformHandle, start_service};
pub use window::{hide_window, is_window_visible, place_flyout};

pub fn create_backend(platform: PlatformHandle) -> Box<dyn Backend> {
    Box::new(backend::MacBackend::new(platform))
}

pub fn init_process() {}

pub fn attach_console() {}

// the tray icon is a template image, which macOS recolors for the menu bar itself
pub fn taskbar_uses_light_theme() -> bool {
    true
}

/// Loads a system library that might not exist on every macOS version; null if it doesn't.
pub(crate) fn open_library(path: &CStr) -> *mut c_void {
    unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LAZY) }
}

/// # Safety
///
/// `T` must be the `extern "C" fn` type of the symbol.
pub(crate) unsafe fn symbol<T: Copy>(library: *mut c_void, name: &CStr) -> Option<T> {
    if library.is_null() {
        return None;
    }
    let ptr = unsafe { libc::dlsym(library, name.as_ptr()) };
    (!ptr.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, T>(&ptr) })
}

pub(crate) fn cf_value(dict: &CFDictionary, key: &str) -> Option<CFRetained<CFType>> {
    let key = CFString::from_str(key);
    // SAFETY: property dictionaries have string keys and CF values, and aren't mutated here
    let dict = unsafe { dict.cast_unchecked::<CFString, CFType>() };
    unsafe { dict.get_unchecked(&key) }.map(|value| value.retain())
}

pub(crate) fn cf_string(dict: &CFDictionary, key: &str) -> Option<String> {
    let value = cf_value(dict, key)?.downcast::<CFString>().ok()?;
    Some(value.to_string())
}

pub(crate) fn cf_number(dict: &CFDictionary, key: &str) -> Option<i64> {
    cf_value(dict, key)?.downcast::<CFNumber>().ok()?.as_i64()
}

pub(crate) fn cf_dictionary(dict: &CFDictionary, key: &str) -> Option<CFRetained<CFDictionary>> {
    cf_value(dict, key)?.downcast::<CFDictionary>().ok()
}
