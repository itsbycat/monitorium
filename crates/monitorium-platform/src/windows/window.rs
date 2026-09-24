use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, HWND_TOPMOST, IsWindowVisible, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetForegroundWindow, SetWindowPos, ShowWindow,
};

use crate::{LogicalSize, TrayAnchor};

const MARGIN: f32 = 12.0;

fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut _)
}

pub fn place_flyout(raw: isize, anchor: TrayAnchor, size: LogicalSize, show: bool) {
    if raw == 0 {
        return;
    }
    let point = match anchor {
        Some(r) => POINT {
            x: r.x + r.width / 2,
            y: r.y + r.height / 2,
        },
        None => {
            let mut p = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut p);
            }
            p
        }
    };

    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = dpi_x as f32 / 96.0;

        let width = (size.width * scale).round() as i32;
        let height = (size.height * scale).round() as i32;
        let margin = (MARGIN * scale).round() as i32;
        let work = info.rcWork;
        let full = info.rcMonitor;

        let (x, y) = if work.top > full.top {
            (work.right - width - margin, work.top + margin)
        } else if work.left > full.left {
            (work.left + margin, work.bottom - height - margin)
        } else {
            (work.right - width - margin, work.bottom - height - margin)
        };
        let x = x.max(work.left);
        let y = y.max(work.top);

        let mut flags = SWP_NOACTIVATE;
        if show {
            flags |= SWP_SHOWWINDOW;
        }
        let _ = SetWindowPos(hwnd(raw), Some(HWND_TOPMOST), x, y, width, height, flags);
        if show {
            let _ = SetForegroundWindow(hwnd(raw));
        }
    }
}

pub fn hide_window(raw: isize) {
    if raw != 0 {
        unsafe {
            let _ = ShowWindow(hwnd(raw), SW_HIDE);
        }
    }
}

pub fn is_window_visible(raw: isize) -> bool {
    raw != 0 && unsafe { IsWindowVisible(hwnd(raw)) }.as_bool()
}
