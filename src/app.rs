use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use monitorium_core::worker::{self, WorkerHandle};
use monitorium_core::{
    MonitorId, MonitorInfo, Rect, Settings, ThemePreference, WorkerCmd, WorkerEvent,
};
use monitorium_platform::{self as platform, LogicalSize, PlatformEvent, PlatformHandle};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

use crate::hotkeys::{HotkeyAction, Hotkeys};
use crate::tray::{self, Tray};
use crate::ui::{self, UiState};

const LOCAL_EDIT_HOLD: Duration = Duration::from_millis(1500);
const REOPEN_GUARD: Duration = Duration::from_millis(300);
const REFRESH_ON_OPEN_INTERVAL: Duration = Duration::from_secs(5);
const WHEEL_DELTA: i32 = 120;

enum AppEvent {
    Tray(TrayIconEvent),
    Menu(MenuEvent),
    Hotkey(GlobalHotKeyEvent),
    Platform(PlatformEvent),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Monitors,
    Settings,
}

struct Flyout {
    visible: bool,
    anchor: Option<Rect>,
    size: LogicalSize,
    was_focused: bool,
    hidden_at: Option<Instant>,
}

pub struct App {
    ctx: egui::Context,
    window: isize,
    events: Receiver<AppEvent>,
    worker_events: Receiver<WorkerEvent>,
    worker: Option<WorkerHandle>,
    platform: PlatformHandle,
    tray: Option<Tray>,
    hotkeys: Hotkeys,
    flyout: Flyout,
    local_edits: HashMap<MonitorId, Instant>,
    scroll_hook: Option<Rect>,
    scroll_accum: i32,
    last_refresh: Instant,
    frame_count: u64,
    start_hidden: bool,
    started: bool,

    pub settings: Settings,
    pub monitors: Vec<MonitorInfo>,
    pub monitors_loaded: bool,
    pub page: Page,
    pub ui: UiState,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, autostart: bool) -> Self {
        let ctx = cc.egui_ctx.clone();
        ui::setup_style(&ctx);

        let window = match cc.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
            Ok(RawWindowHandle::AppKit(handle)) => handle.ns_view.as_ptr() as isize,
            _ => 0,
        };

        let (mut settings, first_run) = Settings::load();
        // debug builds in target/ shouldn't register themselves to run at login
        if first_run
            && settings.start_with_os
            && !cfg!(debug_assertions)
            && let Err(err) = platform::autostart::set_enabled(true)
        {
            log::warn!("enabling autostart failed: {err}");
        }
        if !cfg!(debug_assertions)
            && let Err(err) = platform::autostart::refresh()
        {
            log::warn!("updating autostart failed: {err}");
        }
        settings.start_with_os = platform::autostart::is_enabled();
        if let Err(err) = settings.save() {
            log::warn!("saving settings failed: {err}");
        }
        ctx.set_theme(theme(settings.theme));

        let (tx, events) = unbounded();
        install_event_handlers(&tx, &ctx);

        let platform = platform::start_service({
            let tx = tx.clone();
            let ctx = ctx.clone();
            Arc::new(move |event| {
                let _ = tx.send(AppEvent::Platform(event));
                ctx.request_repaint();
            })
        });

        let (worker_tx, worker_events) = unbounded();
        let worker = worker::spawn(
            {
                let platform = platform.clone();
                move || platform::create_backend(platform)
            },
            settings.clone(),
            worker_tx,
            {
                let ctx = ctx.clone();
                Arc::new(move || ctx.request_repaint())
            },
        );

        let tray = Tray::new(platform::taskbar_uses_light_theme(), settings.start_with_os)
            .inspect_err(|err| log::error!("creating tray icon failed: {err}"))
            .ok();

        let mut hotkeys = Hotkeys::new();
        hotkeys.apply(&settings);

        Self {
            ctx,
            window,
            events,
            worker_events,
            worker: Some(worker),
            platform,
            tray,
            hotkeys,
            flyout: Flyout {
                visible: false,
                anchor: None,
                size: LogicalSize {
                    width: ui::FLYOUT_WIDTH,
                    height: 320.0,
                },
                was_focused: false,
                hidden_at: None,
            },
            local_edits: HashMap::new(),
            scroll_hook: None,
            scroll_accum: 0,
            last_refresh: Instant::now(),
            frame_count: 0,
            start_hidden: autostart,
            started: false,
            settings,
            monitors: Vec::new(),
            monitors_loaded: false,
            page: Page::Monitors,
            ui: UiState::default(),
        }
    }

    pub fn send(&self, cmd: WorkerCmd) {
        if let Some(worker) = &self.worker {
            worker.send(cmd);
        }
    }

    pub fn set_brightness(&mut self, id: &MonitorId, value: u8) {
        let now = Instant::now();
        let link = self.settings.link_levels;
        for monitor in &mut self.monitors {
            if link || &monitor.id == id {
                monitor.brightness = value;
                self.local_edits.insert(monitor.id.clone(), now);
            }
        }
        self.send(WorkerCmd::SetBrightness {
            id: id.clone(),
            value,
        });
    }

    pub fn set_all_brightness(&mut self, value: u8) {
        let now = Instant::now();
        for monitor in self.monitors.iter_mut().filter(|m| !m.hidden) {
            monitor.brightness = value;
            self.local_edits.insert(monitor.id.clone(), now);
        }
        self.send(WorkerCmd::SetAll(value));
    }

    pub fn refresh(&mut self) {
        self.last_refresh = Instant::now();
        self.send(WorkerCmd::Refresh);
    }

    pub fn turn_off_displays(&mut self) {
        self.hide_flyout();
        self.send(WorkerCmd::TurnOffDisplays);
    }

    pub fn settings_changed(&mut self, before: &Settings) {
        let now = &self.settings;
        if now.start_with_os != before.start_with_os {
            match platform::autostart::set_enabled(now.start_with_os) {
                Ok(()) => self.ui.autostart_error = None,
                Err(err) => {
                    log::warn!("changing autostart failed: {err}");
                    self.ui.autostart_error = Some(err);
                    self.settings.start_with_os = platform::autostart::is_enabled();
                }
            }
            if let Some(tray) = &self.tray {
                tray.set_autostart_checked(self.settings.start_with_os);
            }
        }
        let now = &self.settings;
        if now.theme != before.theme {
            self.ctx.set_theme(theme(now.theme));
        }
        let hotkeys_changed = now.hotkeys_enabled != before.hotkeys_enabled
            || now.hotkey_up != before.hotkey_up
            || now.hotkey_down != before.hotkey_down
            || now.hotkey_off != before.hotkey_off;
        if hotkeys_changed && self.ui.recording.is_none() {
            self.hotkeys.apply(&self.settings);
        }
        if !self.settings.scroll_on_tray {
            self.set_scroll_hook(None);
        }

        if let Err(err) = self.settings.save() {
            log::warn!("saving settings failed: {err}");
        }
        self.send(WorkerCmd::UpdateSettings(Box::new(self.settings.clone())));
    }

    pub fn hotkey_errors(&self) -> &[String] {
        &self.hotkeys.errors
    }

    pub fn start_recording(&mut self, action: HotkeyAction) {
        self.ui.recording = Some(action);
        self.hotkeys.clear();
    }

    pub fn stop_recording(&mut self) {
        if self.ui.recording.take().is_some() {
            self.hotkeys.apply(&self.settings);
        }
    }

    pub fn quit(&mut self) {
        log::info!("quitting");
        platform::hide_window(self.window);
        if let Some(worker) = self.worker.take() {
            worker.shutdown(Duration::from_secs(3));
        }
        self.tray.take();
        std::process::exit(0);
    }

    fn toggle_flyout(&mut self, anchor: Option<Rect>) {
        if self.flyout.visible {
            self.hide_flyout();
        } else if self
            .flyout
            .hidden_at
            .is_some_and(|t| t.elapsed() < REOPEN_GUARD)
        {
            // this click is what took focus away from the flyout
        } else {
            self.show_flyout(anchor, Page::Monitors);
        }
    }

    pub fn show_flyout(&mut self, anchor: Option<Rect>, page: Page) {
        self.page = page;
        self.flyout.anchor = anchor.or_else(|| self.tray.as_ref().and_then(Tray::rect));
        self.flyout.visible = true;
        self.flyout.was_focused = false;
        platform::place_flyout(self.window, self.flyout.anchor, self.flyout.size, true);
        if self.last_refresh.elapsed() > REFRESH_ON_OPEN_INTERVAL {
            self.refresh();
        }
        self.ctx.request_repaint();
    }

    pub fn hide_flyout(&mut self) {
        if !self.flyout.visible {
            return;
        }
        self.flyout.visible = false;
        self.flyout.was_focused = false;
        self.flyout.hidden_at = Some(Instant::now());
        self.stop_recording();
        platform::hide_window(self.window);
    }

    fn fit_flyout(&mut self, height: f32) {
        let height = height.ceil();
        if (height - self.flyout.size.height).abs() < 1.0 {
            return;
        }
        self.flyout.size.height = height;
        if self.flyout.visible {
            platform::place_flyout(self.window, self.flyout.anchor, self.flyout.size, false);
        }
    }

    fn process_events(&mut self) {
        while let Ok(event) = self.worker_events.try_recv() {
            match event {
                WorkerEvent::Monitors(monitors) => self.on_monitors(monitors),
            }
        }
        while let Ok(event) = self.events.try_recv() {
            match event {
                AppEvent::Tray(event) => self.on_tray(event),
                AppEvent::Menu(event) => self.on_menu(event),
                AppEvent::Hotkey(event) => self.on_hotkey(event),
                AppEvent::Platform(event) => self.on_platform(event),
            }
        }
    }

    fn on_monitors(&mut self, mut monitors: Vec<MonitorInfo>) {
        self.local_edits
            .retain(|_, t| t.elapsed() < LOCAL_EDIT_HOLD);
        for monitor in &mut monitors {
            if self.local_edits.contains_key(&monitor.id)
                && let Some(current) = self.monitors.iter().find(|m| m.id == monitor.id)
            {
                monitor.brightness = current.brightness;
            }
        }
        self.monitors = monitors;
        self.monitors_loaded = true;
        self.update_tooltip();
        self.ctx.request_repaint();
    }

    fn update_tooltip(&self) {
        let Some(tray) = &self.tray else { return };
        let mut text = String::from("Monitorium");
        for monitor in self.monitors.iter().filter(|m| !m.hidden) {
            text.push_str(&format!("\n{}: {}%", monitor.name, monitor.brightness));
        }
        tray.set_tooltip(&text);
    }

    fn on_tray(&mut self, event: TrayIconEvent) {
        match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } => self.toggle_flyout(Some(tray::to_rect(rect))),
            TrayIconEvent::Enter { rect, .. } | TrayIconEvent::Move { rect, .. } => {
                if self.settings.scroll_on_tray {
                    self.set_scroll_hook(Some(tray::to_rect(rect)));
                }
            }
            TrayIconEvent::Leave { .. } => self.set_scroll_hook(None),
            _ => {}
        }
    }

    fn set_scroll_hook(&mut self, rect: Option<Rect>) {
        if self.scroll_hook != rect {
            self.scroll_hook = rect;
            self.scroll_accum = 0;
            self.platform.set_scroll_hook(rect);
        }
    }

    fn on_menu(&mut self, event: MenuEvent) {
        let Some(tray) = &self.tray else { return };
        let ids = &tray.ids;
        let id = event.id;
        if id == ids.open {
            self.show_flyout(None, Page::Monitors);
        } else if id == ids.settings {
            self.show_flyout(None, Page::Settings);
        } else if id == ids.refresh {
            self.refresh();
        } else if id == ids.turn_off {
            self.turn_off_displays();
        } else if id == ids.autostart {
            let before = self.settings.clone();
            self.settings.start_with_os = !self.settings.start_with_os;
            self.settings_changed(&before);
        } else if id == ids.quit {
            self.quit();
        }
    }

    fn on_hotkey(&mut self, event: GlobalHotKeyEvent) {
        if event.state != HotKeyState::Pressed {
            return;
        }
        let step = i16::from(self.settings.hotkey_step.max(1));
        match self.hotkeys.action(event.id) {
            Some(HotkeyAction::BrightnessUp) => self.send(WorkerCmd::Step(step)),
            Some(HotkeyAction::BrightnessDown) => self.send(WorkerCmd::Step(-step)),
            Some(HotkeyAction::TurnOff) => self.turn_off_displays(),
            None => {}
        }
    }

    fn on_platform(&mut self, event: PlatformEvent) {
        match event {
            PlatformEvent::DisplaysChanged => self.refresh(),
            PlatformEvent::ThemeChanged => {
                if let Some(tray) = &self.tray {
                    tray.set_light_taskbar(platform::taskbar_uses_light_theme());
                }
                self.ctx.request_repaint();
            }
            PlatformEvent::TrayScroll(delta) => {
                self.scroll_accum += delta;
                let notches = self.scroll_accum / WHEEL_DELTA;
                if notches != 0 {
                    self.scroll_accum -= notches * WHEEL_DELTA;
                    let step = i16::from(self.settings.scroll_step.max(1));
                    self.send(WorkerCmd::Step(notches as i16 * step));
                }
            }
            PlatformEvent::Reopen => self.show_flyout(None, Page::Monitors),
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // eframe force-shows the window after the first frame, so place or hide it on the second.
        // This is in logic() because eframe skips ui() for hidden windows, which is how macOS
        // sees the off-screen first frame.
        if self.frame_count >= 1 && !self.started {
            self.started = true;
            if self.start_hidden {
                platform::hide_window(self.window);
            } else {
                self.show_flyout(None, Page::Monitors);
            }
        }
        self.process_events();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame_count += 1;
        if self.frame_count == 1 {
            self.ctx.request_repaint();
        }

        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_flyout();
        }

        if self.flyout.visible {
            if self.frame_count > 2 && !platform::is_window_visible(self.window) {
                self.flyout.visible = false;
            }
            let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
            if focused {
                self.flyout.was_focused = true;
            } else if self.flyout.was_focused {
                self.hide_flyout();
            }
            if self.ui.recording.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.hide_flyout();
            }
        }

        let height = ui::draw(self, ui);
        if self.frame_count > 1 {
            self.fit_flyout(height);
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }
}

fn install_event_handlers(tx: &Sender<AppEvent>, ctx: &egui::Context) {
    let (t, c) = (tx.clone(), ctx.clone());
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = t.send(AppEvent::Tray(event));
        c.request_repaint();
    }));
    let (t, c) = (tx.clone(), ctx.clone());
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = t.send(AppEvent::Menu(event));
        c.request_repaint();
    }));
    let (t, c) = (tx.clone(), ctx.clone());
    GlobalHotKeyEvent::set_event_handler(Some(move |event| {
        let _ = t.send(AppEvent::Hotkey(event));
        c.request_repaint();
    }));
}

fn theme(preference: ThemePreference) -> egui::ThemePreference {
    match preference {
        ThemePreference::System => egui::ThemePreference::System,
        ThemePreference::Light => egui::ThemePreference::Light,
        ThemePreference::Dark => egui::ThemePreference::Dark,
    }
}
