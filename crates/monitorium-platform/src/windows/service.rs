use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;
use std::thread;

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use monitorium_core::{MonitorId, Rect};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GetMessageW, HHOOK, HTTRANSPARENT, HWND_TOPMOST, KillTimer, LWA_ALPHA, MSG,
    MSLLHOOKSTRUCT, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PostMessageW, PostQuitMessage,
    RegisterClassExW, SC_MONITORPOWER, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SetLayeredWindowAttributes, SetTimer, SetWindowPos, SetWindowsHookExW, ShowWindow,
    TranslateMessage, UnhookWindowsHookEx, WH_MOUSE_LL, WM_APP, WM_DISPLAYCHANGE, WM_MOUSEWHEEL,
    WM_NCHITTEST, WM_POWERBROADCAST, WM_SETTINGCHANGE, WM_SYSCOMMAND, WM_TIMER, WNDCLASSEXW,
    WS_DISABLED, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_OVERLAPPED, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::{EventCallback, PlatformEvent};

const WM_SERVICE_CMD: u32 = WM_APP + 1;
const TIMER_DISPLAY_CHANGE: usize = 1;
const TIMER_TOPMOST: usize = 2;
const TIMER_POWER_OFF: usize = 3;
const DISPLAY_CHANGE_DEBOUNCE_MS: u32 = 1500;
const TOPMOST_INTERVAL_MS: u32 = 2000;
const POWER_OFF_DELAY_MS: u32 = 400;
const MAX_OVERLAY_ALPHA: u32 = 235;

enum ServiceCmd {
    SetOverlay {
        id: MonitorId,
        bounds: Rect,
        level: u8,
    },
    RemoveOverlay(MonitorId),
    ClearOverlays,
    ScrollHook(Option<Rect>),
    PowerOffDisplays,
}

#[derive(Clone)]
pub struct PlatformHandle {
    tx: Sender<ServiceCmd>,
    hwnd: isize,
}

impl PlatformHandle {
    fn send(&self, cmd: ServiceCmd) {
        if self.tx.send(cmd).is_ok() {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut _)),
                    WM_SERVICE_CMD,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }

    pub fn set_overlay(&self, id: MonitorId, bounds: Rect, level: u8) {
        self.send(ServiceCmd::SetOverlay { id, bounds, level });
    }

    pub fn remove_overlay(&self, id: MonitorId) {
        self.send(ServiceCmd::RemoveOverlay(id));
    }

    pub fn clear_overlays(&self) {
        self.send(ServiceCmd::ClearOverlays);
    }

    pub fn set_scroll_hook(&self, rect: Option<Rect>) {
        self.send(ServiceCmd::ScrollHook(rect));
    }

    pub fn power_off_displays(&self) {
        self.send(ServiceCmd::PowerOffDisplays);
    }
}

struct ServiceState {
    rx: Receiver<ServiceCmd>,
    on_event: EventCallback,
    overlays: HashMap<MonitorId, HWND>,
}

thread_local! {
    static STATE: RefCell<Option<ServiceState>> = const { RefCell::new(None) };
}

struct HookState {
    hook: HHOOK,
    rect: Rect,
}

unsafe impl Send for HookState {}

static HOOK: Mutex<Option<HookState>> = Mutex::new(None);
static HOOK_CALLBACK: Mutex<Option<EventCallback>> = Mutex::new(None);

pub fn start_service(on_event: EventCallback) -> PlatformHandle {
    let (tx, rx) = unbounded();
    let (ready_tx, ready_rx) = bounded(1);
    *HOOK_CALLBACK.lock().unwrap() = Some(on_event.clone());

    thread::Builder::new()
        .name("monitorium-platform".into())
        .spawn(move || run(rx, on_event, ready_tx))
        .expect("failed to spawn platform thread");

    let hwnd = ready_rx.recv().unwrap_or(0);
    PlatformHandle { tx, hwnd }
}

fn run(rx: Receiver<ServiceCmd>, on_event: EventCallback, ready: Sender<isize>) {
    let hwnd = match unsafe { create_windows() } {
        Some(hwnd) => hwnd,
        None => {
            log::error!("could not create platform window");
            let _ = ready.send(0);
            return;
        }
    };
    STATE.with(|s| {
        *s.borrow_mut() = Some(ServiceState {
            rx,
            on_event,
            overlays: HashMap::new(),
        })
    });
    let _ = ready.send(hwnd.0 as isize);

    unsafe {
        SetTimer(Some(hwnd), TIMER_TOPMOST, TOPMOST_INTERVAL_MS, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

const SERVICE_CLASS: PCWSTR = w!("MonitoriumService");
const OVERLAY_CLASS: PCWSTR = w!("MonitoriumOverlay");

unsafe fn create_windows() -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let service = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(service_proc),
            hInstance: instance.into(),
            lpszClassName: SERVICE_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&service);

        let overlay = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(overlay_proc),
            hInstance: instance.into(),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: OVERLAY_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&overlay);

        // not a message-only window: those don't receive WM_DISPLAYCHANGE broadcasts
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            SERVICE_CLASS,
            w!("Monitorium service"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()
    }
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCHITTEST {
        return LRESULT(HTTRANSPARENT as isize);
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

unsafe extern "system" fn service_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SERVICE_CMD => {
            handle_commands(hwnd);
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            schedule_display_change(hwnd);
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }
        WM_POWERBROADCAST => {
            let event = wp.0 as u32;
            if event == PBT_APMRESUMEAUTOMATIC || event == PBT_APMRESUMESUSPEND {
                schedule_display_change(hwnd);
            }
            LRESULT(1)
        }
        WM_SETTINGCHANGE => {
            if lp.0 == 0 {
                return LRESULT(0);
            }
            let name = unsafe { PCWSTR(lp.0 as *const u16).to_string() }.unwrap_or_default();
            if name == "ImmersiveColorSet" {
                emit(PlatformEvent::ThemeChanged);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            match wp.0 {
                TIMER_DISPLAY_CHANGE => {
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_DISPLAY_CHANGE);
                    }
                    emit(PlatformEvent::DisplaysChanged);
                }
                TIMER_TOPMOST => reassert_topmost(),
                TIMER_POWER_OFF => unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_POWER_OFF);
                    // lParam 2 = power off
                    DefWindowProcW(
                        hwnd,
                        WM_SYSCOMMAND,
                        WPARAM(SC_MONITORPOWER as usize),
                        LPARAM(2),
                    );
                },
                _ => {}
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn emit(event: PlatformEvent) {
    let callback = STATE.with(|s| s.borrow().as_ref().map(|s| s.on_event.clone()));
    if let Some(callback) = callback {
        callback(event);
    }
}

fn schedule_display_change(hwnd: HWND) {
    // re-arming an existing timer id restarts it, which debounces
    unsafe {
        SetTimer(
            Some(hwnd),
            TIMER_DISPLAY_CHANGE,
            DISPLAY_CHANGE_DEBOUNCE_MS,
            None,
        );
    }
}

fn handle_commands(hwnd: HWND) {
    let commands: Vec<ServiceCmd> = STATE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|s| s.rx.try_iter().collect())
            .unwrap_or_default()
    });
    for cmd in commands {
        match cmd {
            ServiceCmd::SetOverlay { id, bounds, level } => set_overlay(id, bounds, level),
            ServiceCmd::RemoveOverlay(id) => remove_overlay(&id),
            ServiceCmd::ClearOverlays => {
                let ids: Vec<MonitorId> = STATE.with(|s| {
                    s.borrow()
                        .as_ref()
                        .map(|s| s.overlays.keys().cloned().collect())
                        .unwrap_or_default()
                });
                for id in ids {
                    remove_overlay(&id);
                }
            }
            ServiceCmd::ScrollHook(rect) => set_scroll_hook(rect),
            ServiceCmd::PowerOffDisplays => unsafe {
                SetTimer(Some(hwnd), TIMER_POWER_OFF, POWER_OFF_DELAY_MS, None);
            },
        }
    }
}

fn set_overlay(id: MonitorId, bounds: Rect, level: u8) {
    if level >= 100 {
        remove_overlay(&id);
        return;
    }
    let alpha = (u32::from(100 - level) * MAX_OVERLAY_ALPHA / 100) as u8;
    let existing = STATE.with(|s| {
        s.borrow()
            .as_ref()
            .and_then(|s| s.overlays.get(&id).copied())
    });
    let hwnd = match existing {
        Some(hwnd) => hwnd,
        None => {
            let Some(hwnd) = (unsafe { create_overlay() }) else {
                log::warn!("could not create overlay for {id}");
                return;
            };
            STATE.with(|s| {
                if let Some(s) = s.borrow_mut().as_mut() {
                    s.overlays.insert(id, hwnd);
                }
            });
            hwnd
        }
    };
    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            SWP_NOACTIVATE,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

unsafe fn create_overlay() -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            OVERLAY_CLASS,
            w!(""),
            WS_POPUP | WS_DISABLED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()
    }
}

fn remove_overlay(id: &MonitorId) {
    let hwnd = STATE.with(|s| s.borrow_mut().as_mut().and_then(|s| s.overlays.remove(id)));
    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn reassert_topmost() {
    let overlays: Vec<HWND> = STATE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|s| s.overlays.values().copied().collect())
            .unwrap_or_default()
    });
    for hwnd in overlays {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

fn set_scroll_hook(rect: Option<Rect>) {
    let mut hook = HOOK.lock().unwrap();
    match (rect, hook.as_mut()) {
        (Some(rect), Some(state)) => state.rect = rect,
        (Some(rect), None) => {
            let installed = unsafe {
                let instance = GetModuleHandleW(None).ok();
                SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), instance.map(Into::into), 0)
            };
            match installed {
                Ok(h) => *hook = Some(HookState { hook: h, rect }),
                Err(err) => log::warn!("mouse hook failed: {err}"),
            }
        }
        (None, Some(_)) => {
            if let Some(state) = hook.take() {
                unsafe {
                    let _ = UnhookWindowsHookEx(state.hook);
                }
            }
        }
        (None, None) => {}
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && wp.0 as u32 == WM_MOUSEWHEEL {
        let info = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
        let inside = HOOK
            .try_lock()
            .ok()
            .and_then(|h| h.as_ref().map(|h| h.rect.contains(info.pt.x, info.pt.y)))
            .unwrap_or(false);
        if inside {
            let delta = (info.mouseData >> 16) as u16 as i16;
            let callback = HOOK_CALLBACK.try_lock().ok().and_then(|c| c.clone());
            if let Some(callback) = callback {
                callback(PlatformEvent::TrayScroll(i32::from(delta)));
            }
            // swallow it so the window behind the taskbar doesn't scroll too
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}
