use std::sync::Arc;

use monitorium_core::Rect;

pub mod autostart;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(any(windows, target_os = "macos")))]
mod stub;
#[cfg(not(any(windows, target_os = "macos")))]
pub use stub::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformEvent {
    DisplaysChanged,
    ThemeChanged,
    TrayScroll(i32),
    /// The app was opened again while running (macOS)
    Reopen,
}

pub type EventCallback = Arc<dyn Fn(PlatformEvent) + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogicalSize {
    pub width: f32,
    pub height: f32,
}

pub type TrayAnchor = Option<Rect>;
