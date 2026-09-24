use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::process::Command;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use block2::RcBlock;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use dispatch2::DispatchQueue;
use monitorium_core::{MonitorId, Rect};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSScreenSaverWindowLevel, NSWindow,
    NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowSharingType, NSWindowStyleMask,
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidWakeNotification,
};
use objc2_core_foundation::CGRect;
use objc2_core_graphics::{
    CGDirectDisplayID, CGDisplayChangeSummaryFlags, CGDisplayIsOnline,
    CGDisplayRegisterReconfigurationCallback,
};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSNotification};

use super::{display, gamma, window};
use crate::{EventCallback, PlatformEvent};

const DISPLAY_CHANGE_DEBOUNCE: Duration = Duration::from_millis(1500);
const POWER_OFF_DELAY: Duration = Duration::from_millis(400);
const GAMMA_CHECK_INTERVAL: Duration = Duration::from_secs(3);
const MAX_OVERLAY_ALPHA: f64 = 235.0 / 255.0;
// TrayScroll uses Windows wheel units, 120 per notch; on a trackpad a step is about 40 points
const WHEEL_DELTA: i32 = 120;
const WHEEL_DELTA_PER_POINT: f64 = 3.0;
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const REOPEN_APPLICATION: u32 = u32::from_be_bytes(*b"rapp");

enum Timer {
    DisplayChange,
    PowerOff,
}

#[derive(Clone)]
pub struct PlatformHandle {
    timers: Sender<Timer>,
}

impl PlatformHandle {
    pub fn set_overlay(&self, id: MonitorId, display: CGDirectDisplayID, level: u8) {
        on_main(move |mtm| set_overlay(mtm, id, display, level));
    }

    pub fn remove_overlay(&self, id: MonitorId) {
        on_main(move |_| remove_overlay(&id));
    }

    pub fn clear_overlays(&self) {
        on_main(|_| clear_overlays());
    }

    // tray-icon reports entering and leaving the icon, so the rect itself isn't needed
    pub fn set_scroll_hook(&self, rect: Option<Rect>) {
        SCROLL_HOOK.store(rect.is_some(), Ordering::Relaxed);
        if rect.is_some() {
            on_main(install_scroll_monitor);
        }
    }

    pub fn power_off_displays(&self) {
        let _ = self.timers.send(Timer::PowerOff);
    }
}

static CALLBACK: OnceLock<EventCallback> = OnceLock::new();
static TIMERS: OnceLock<Sender<Timer>> = OnceLock::new();
static SCROLL_HOOK: AtomicBool = AtomicBool::new(false);

struct Overlay {
    window: Retained<NSWindow>,
    display: CGDirectDisplayID,
}

thread_local! {
    // AppKit objects, only ever touched on the main thread
    static OVERLAYS: RefCell<HashMap<MonitorId, Overlay>> = RefCell::new(HashMap::new());
    static SCROLL_MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this has no Drop impl
    #[unsafe(super(NSObject))]
    #[name = "MonitoriumReopenHandler"]
    struct ReopenHandler;

    impl ReopenHandler {
        #[unsafe(method(handleReopen:withReplyEvent:))]
        fn handle_reopen(&self, _event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            emit(PlatformEvent::Reopen);
        }
    }
);

// no NSApplication or windows in here: the command line modes don't have an app
pub fn start_service(on_event: EventCallback) -> PlatformHandle {
    let _ = CALLBACK.set(on_event);
    let timers = TIMERS.get_or_init(|| {
        let (tx, rx) = unbounded();
        thread::Builder::new()
            .name("monitorium-platform".into())
            .spawn(move || run_timers(&rx))
            .expect("failed to spawn platform thread");
        unsafe {
            CGDisplayRegisterReconfigurationCallback(Some(reconfigured), std::ptr::null_mut());
        }
        observe_wake();
        handle_reopen();
        tx
    });
    PlatformHandle {
        timers: timers.clone(),
    }
}

fn emit(event: PlatformEvent) {
    if let Some(callback) = CALLBACK.get() {
        callback(event);
    }
}

fn display_changed() {
    if let Some(timers) = TIMERS.get() {
        let _ = timers.send(Timer::DisplayChange);
    }
}

// never waits for the main thread, which may itself be waiting for the worker (quitting, --set)
fn on_main(work: impl FnOnce(MainThreadMarker) + Send + 'static) {
    match MainThreadMarker::new() {
        Some(mtm) => work(mtm),
        None => DispatchQueue::main().exec_async(move || {
            if let Some(mtm) = MainThreadMarker::new() {
                work(mtm);
            }
        }),
    }
}

fn run_timers(rx: &Receiver<Timer>) {
    let mut display_change = None;
    let mut power_off = None;
    let mut gamma_check = Instant::now() + GAMMA_CHECK_INTERVAL;
    loop {
        let now = Instant::now();
        if display_change.is_some_and(|at| at <= now) {
            display_change = None;
            emit(PlatformEvent::DisplaysChanged);
        }
        if power_off.is_some_and(|at| at <= now) {
            power_off = None;
            sleep_displays();
        }
        if gamma_check <= now {
            gamma_check = now + GAMMA_CHECK_INTERVAL;
            gamma::reassert();
        }

        let next = display_change
            .into_iter()
            .chain(power_off)
            .fold(gamma_check, Instant::min);
        match rx.recv_timeout(next.saturating_duration_since(Instant::now())) {
            // restarting the timer on every change debounces
            Ok(Timer::DisplayChange) => {
                display_change = Some(Instant::now() + DISPLAY_CHANGE_DEBOUNCE);
            }
            // otherwise the click or key release that asked for it wakes the displays again
            Ok(Timer::PowerOff) => power_off = Some(Instant::now() + POWER_OFF_DELAY),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn sleep_displays() {
    // by absolute path, since apps started at login get a minimal PATH
    match Command::new("/usr/bin/pmset")
        .arg("displaysleepnow")
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => log::warn!("pmset displaysleepnow failed ({status})"),
        Err(err) => log::warn!("running pmset failed: {err}"),
    }
}

unsafe extern "C-unwind" fn reconfigured(
    _: CGDirectDisplayID,
    flags: CGDisplayChangeSummaryFlags,
    _: *mut c_void,
) {
    if flags.contains(CGDisplayChangeSummaryFlags::BeginConfigurationFlag) {
        return;
    }
    display_changed();
    // move overlays along right away instead of after the debounced refresh
    on_main(|_| place_overlays());
}

fn observe_wake() {
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let block = RcBlock::new(|_: NonNull<NSNotification>| display_changed());
    for name in unsafe {
        [
            NSWorkspaceDidWakeNotification,
            NSWorkspaceScreensDidWakeNotification,
        ]
    } {
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        // observes for the rest of the process
        std::mem::forget(observer);
    }
}

// opening the app again shows the panel, for when the menu bar has no room for the icon
fn handle_reopen() {
    let handler: Retained<ReopenHandler> = unsafe { msg_send![ReopenHandler::class(), new] };
    let target: &AnyObject = &handler;
    let manager = NSAppleEventManager::sharedAppleEventManager();
    // the typed binding needs all of CoreServices for the two event code types
    let _: () = unsafe {
        msg_send![
            &*manager,
            setEventHandler: target,
            andSelector: sel!(handleReopen:withReplyEvent:),
            forEventClass: CORE_EVENT_CLASS,
            andEventID: REOPEN_APPLICATION,
        ]
    };
    // the manager doesn't retain its handlers
    std::mem::forget(handler);
}

fn set_overlay(mtm: MainThreadMarker, id: MonitorId, display: CGDirectDisplayID, level: u8) {
    if level >= 100 {
        remove_overlay(&id);
        return;
    }
    let window = OVERLAYS.with_borrow_mut(|overlays| {
        let overlay = overlays.entry(id).or_insert_with(|| Overlay {
            window: create_overlay(mtm),
            display,
        });
        overlay.display = display;
        overlay.window.clone()
    });
    window.setAlphaValue(f64::from(100 - level) / 100.0 * MAX_OVERLAY_ALPHA);
    place_overlay(&window, display);
}

fn create_overlay(mtm: MainThreadMarker) -> Retained<NSWindow> {
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            CGRect::ZERO,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // the Retained owns the window, so closing it must not release it too
    unsafe { window.setReleasedWhenClosed(false) };
    window.setOpaque(false);
    window.setHasShadow(false);
    window.setBackgroundColor(Some(&NSColor::blackColor()));
    window.setIgnoresMouseEvents(true);
    // above the menu bar and the Dock
    window.setLevel(NSScreenSaverWindowLevel);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    // hiding the app when the panel closes must not take the dimming with it
    window.setCanHide(false);
    window.setSharingType(NSWindowSharingType::None);
    window.setAnimationBehavior(NSWindowAnimationBehavior::None);
    window
}

fn place_overlay(window: &NSWindow, display: CGDirectDisplayID) {
    if CGDisplayIsOnline(display) {
        window.setFrame_display(display::cocoa_frame(display), false);
        window.orderFrontRegardless();
    } else {
        window.orderOut(None);
    }
}

fn place_overlays() {
    let overlays: Vec<(Retained<NSWindow>, CGDirectDisplayID)> = OVERLAYS.with_borrow(|overlays| {
        overlays
            .values()
            .map(|overlay| (overlay.window.clone(), overlay.display))
            .collect()
    });
    for (window, display) in overlays {
        place_overlay(&window, display);
    }
}

fn remove_overlay(id: &MonitorId) {
    if let Some(overlay) = OVERLAYS.with_borrow_mut(|overlays| overlays.remove(id)) {
        overlay.window.orderOut(None);
    }
}

fn clear_overlays() {
    for (_, overlay) in OVERLAYS.with_borrow_mut(std::mem::take) {
        overlay.window.orderOut(None);
    }
}

fn install_scroll_monitor(_: MainThreadMarker) {
    SCROLL_MONITOR.with_borrow_mut(|monitor| {
        if monitor.is_some() {
            return;
        }
        let block = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
            if on_scroll(unsafe { event.as_ref() }) {
                std::ptr::null_mut()
            } else {
                event.as_ptr()
            }
        });
        *monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::ScrollWheel, &block)
        };
    });
}

/// Turns a scroll over the tray icon into `TrayScroll`, and swallows it.
fn on_scroll(event: &NSEvent) -> bool {
    // by window rather than position: the copies of the icon in other displays' menu bars
    // don't report where they are
    let over_icon = SCROLL_HOOK.load(Ordering::Relaxed)
        && MainThreadMarker::new()
            .and_then(|mtm| event.window(mtm))
            .is_some_and(|window| window::is_status_item(&window));
    if !over_icon {
        return false;
    }
    let momentum = !event.momentumPhase().is_empty();
    if let Some(delta) = wheel_delta(
        event.scrollingDeltaY(),
        event.hasPreciseScrollingDeltas(),
        momentum,
    ) {
        emit(PlatformEvent::TrayScroll(delta));
    }
    true
}

fn wheel_delta(delta: f64, precise: bool, momentum: bool) -> Option<i32> {
    if momentum || delta == 0.0 {
        return None;
    }
    Some(if precise {
        (delta * WHEEL_DELTA_PER_POINT).round() as i32
    } else {
        // macOS accelerates wheel deltas, so every event from a wheel counts as one notch
        WHEEL_DELTA * delta.signum() as i32
    })
}

#[cfg(test)]
mod tests {
    use super::wheel_delta;

    #[test]
    fn wheel_deltas() {
        assert_eq!(wheel_delta(1.0, false, false), Some(120));
        assert_eq!(wheel_delta(-7.5, false, false), Some(-120));
        assert_eq!(wheel_delta(40.0, true, false), Some(120));
        assert_eq!(wheel_delta(-2.0, true, false), Some(-6));
        assert_eq!(wheel_delta(12.0, true, true), None);
        assert_eq!(wheel_delta(0.0, false, false), None);
    }
}
