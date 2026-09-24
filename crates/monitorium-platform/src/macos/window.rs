use std::ptr::NonNull;

use monitorium_core::Rect;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSEvent, NSScreen, NSScreenSaverWindowLevel, NSStatusWindowLevel, NSView,
    NSWindow, NSWindowCollectionBehavior,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGDisplayPixelsHigh, CGMainDisplayID};

use crate::{LogicalSize, TrayAnchor};

const MENU_BAR_GAP: f64 = 6.0;
const SCREEN_MARGIN: f64 = 8.0;
// menu bars of displays with a notch are taller than the usual 24 points
const MENU_BAR_MAX_HEIGHT: f64 = 40.0;

// the raw handle is the content view of the app's window; callers check for the main thread
fn window(raw: isize) -> Option<Retained<NSWindow>> {
    let view = NonNull::new(raw as *mut NSView)?;
    unsafe { view.as_ref() }.window()
}

pub fn place_flyout(raw: isize, anchor: TrayAnchor, size: LogicalSize, show: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = window(raw) else {
        return;
    };
    let (width, height) = (f64::from(size.width), f64::from(size.height));
    if !show {
        // resizing an open panel: keep its top edge
        let frame = window.frame();
        let top = frame.origin.y + frame.size.height;
        let frame = CGRect::new(
            CGPoint::new(frame.origin.x, top - height),
            CGSize::new(width, height),
        );
        window.setFrame_display(frame, true);
        return;
    }

    let screens: Vec<(CGRect, CGRect)> = NSScreen::screens(mtm)
        .iter()
        .map(|screen| (screen.frame(), screen.visibleFrame()))
        .collect();
    let item = anchor.and_then(|anchor| status_item_frame(mtm, anchor));
    let Some(frame) = flyout_frame(item, NSEvent::mouseLocation(), &screens, width, height) else {
        return;
    };
    window.setFrame_display(frame, true);
    // above the dimming overlays, so the panel stays readable on a dimmed screen
    window.setLevel(NSScreenSaverWindowLevel + 1);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Transient
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    let app = NSApplication::sharedApplication(mtm);
    app.unhideWithoutActivation();
    activate(&app);
    window.makeKeyAndOrderFront(None);
}

pub fn hide_window(raw: isize) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = window(raw) else {
        return;
    };
    let was_key = window.isKeyWindow();
    window.orderOut(None);
    let app = NSApplication::sharedApplication(mtm);
    // gives the focus back to the app that had it before the panel opened
    if was_key && app.isActive() {
        app.hide(None);
    }
}

pub fn is_window_visible(raw: isize) -> bool {
    MainThreadMarker::new().is_some() && window(raw).is_some_and(|window| window.isVisible())
}

pub(super) fn is_status_item(window: &NSWindow) -> bool {
    window.level() == NSStatusWindowLevel
}

fn activate(app: &NSApplication) {
    if objc2::available!(macos = 14.0) {
        app.activate();
    } else {
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

// the menu bar window tray-icon measured the anchor from
fn status_item_frame(mtm: MainThreadMarker, anchor: Rect) -> Option<CGRect> {
    // what tray-icon flips AppKit's coordinates with
    let main_height = CGDisplayPixelsHigh(CGMainDisplayID()) as f64;
    NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .filter(|window| is_status_item(window))
        .map(|window| (window.frame(), window.backingScaleFactor()))
        .find(|&(frame, scale)| same_rect(tray_space(frame, scale, main_height), anchor))
        .map(|(frame, _)| frame)
}

/// tray-icon's coordinates: a top-left origin like on Windows, in pixels of the window's display.
fn tray_space(frame: CGRect, scale: f64, main_height: f64) -> Rect {
    Rect {
        x: (frame.origin.x * scale).round() as i32,
        y: ((main_height - frame.origin.y - frame.size.height) * scale).round() as i32,
        width: (frame.size.width * scale) as i32,
        height: (frame.size.height * scale) as i32,
    }
}

fn same_rect(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() <= 1
        && (a.y - b.y).abs() <= 1
        && (a.width - b.width).abs() <= 1
        && (a.height - b.height).abs() <= 1
}

/// Below the status item. AppKit places the item a moment after launch, and the copies of it
/// in other displays' menu bars stay parked at their left edge, so otherwise below the pointer
/// if it is on a menu bar (it just clicked one of those copies), or else in the top right
/// corner of the main display. `screens` holds (frame, visible frame), main display first.
fn flyout_frame(
    item: Option<CGRect>,
    mouse: CGPoint,
    screens: &[(CGRect, CGRect)],
    width: f64,
    height: f64,
) -> Option<CGRect> {
    let below_item = item.and_then(|item| {
        let screen = screens
            .iter()
            .find(|(frame, _)| placed_in_menu_bar(item, *frame))?;
        Some((screen, Some(item.origin.x + item.size.width / 2.0)))
    });
    let below_mouse = || {
        screens
            .iter()
            .find(|(frame, _)| menu_bar_contains(*frame, mouse))
            .map(|screen| (screen, Some(mouse.x)))
    };
    let (&(_, visible), center) = below_item
        .or_else(below_mouse)
        .or_else(|| screens.first().map(|screen| (screen, None)))?;

    let left = visible.origin.x + SCREEN_MARGIN;
    let right = visible.origin.x + visible.size.width - SCREEN_MARGIN - width;
    let x = center.map_or(right, |center| center - width / 2.0);
    let top = visible.origin.y + visible.size.height - MENU_BAR_GAP;
    Some(CGRect::new(
        CGPoint::new(x.min(right).max(left), top - height),
        CGSize::new(width, height),
    ))
}

fn placed_in_menu_bar(item: CGRect, screen: CGRect) -> bool {
    let center = item.origin.x + item.size.width / 2.0;
    (item.origin.y + item.size.height - (screen.origin.y + screen.size.height)).abs() <= 1.0
        && item.origin.x > screen.origin.x + 1.0
        && center < screen.origin.x + screen.size.width
}

fn menu_bar_contains(screen: CGRect, point: CGPoint) -> bool {
    let top = screen.origin.y + screen.size.height;
    point.x >= screen.origin.x
        && point.x <= screen.origin.x + screen.size.width
        && point.y <= top
        && point.y >= top - MENU_BAR_MAX_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> CGRect {
        CGRect::new(CGPoint::new(x, y), CGSize::new(width, height))
    }

    // a 1440p main display with a 1080p one to its left, aligned at the top
    const SCREENS: [(CGRect, CGRect); 2] = [
        (
            CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(2560.0, 1440.0)),
            CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(2560.0, 1410.0)),
        ),
        (
            CGRect::new(CGPoint::new(-1920.0, 360.0), CGSize::new(1920.0, 1080.0)),
            CGRect::new(CGPoint::new(-1920.0, 360.0), CGSize::new(1920.0, 1050.0)),
        ),
    ];

    #[test]
    fn tray_space_matches_tray_icon() {
        // status items on a 1x main display 1440 points high, and on a 2x one 982 points high
        let expected = |x, width, height| Rect {
            x,
            y: 0,
            width,
            height,
        };
        assert_eq!(
            tray_space(rect(2152.0, 1410.0, 38.0, 30.0), 1.0, 1440.0),
            expected(2152, 38, 30)
        );
        assert_eq!(
            tray_space(rect(1200.0, 945.0, 40.0, 37.0), 2.0, 982.0),
            expected(2400, 80, 74)
        );
    }

    #[test]
    fn flyout_goes_below_the_status_item() {
        let nowhere = CGPoint::new(1000.0, 700.0);
        let frame = |item| flyout_frame(item, nowhere, &SCREENS, 360.0, 320.0);
        assert_eq!(
            frame(Some(rect(2152.0, 1410.0, 38.0, 30.0))),
            Some(rect(1991.0, 1084.0, 360.0, 320.0))
        );
        // clamped to the screen
        assert_eq!(
            frame(Some(rect(2540.0, 1410.0, 20.0, 30.0))),
            Some(rect(2192.0, 1084.0, 360.0, 320.0))
        );
    }

    #[test]
    fn flyout_falls_back_to_the_pointer_or_the_corner() {
        let size = (360.0, 320.0);
        // the copy of the item on the left display, still parked at its edge; clicked at x -500
        let parked = Some(rect(-1920.0, 1410.0, 38.0, 30.0));
        assert_eq!(
            flyout_frame(
                parked,
                CGPoint::new(-500.0, 1425.0),
                &SCREENS,
                size.0,
                size.1
            ),
            Some(rect(-680.0, 1084.0, 360.0, 320.0))
        );
        // right after launch the item sits at the origin, and the pointer is somewhere else
        let unplaced = Some(rect(0.0, 0.0, 32.0, 30.0));
        assert_eq!(
            flyout_frame(
                unplaced,
                CGPoint::new(1000.0, 700.0),
                &SCREENS,
                size.0,
                size.1
            ),
            Some(rect(2192.0, 1084.0, 360.0, 320.0))
        );
        assert_eq!(flyout_frame(None, CGPoint::ZERO, &[], size.0, size.1), None);
    }
}
