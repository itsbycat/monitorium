#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod hotkeys;
mod icons;
mod tray;
mod ui;

use std::fs::File;
use std::sync::Arc;

use eframe::egui;
use monitorium_core::settings;
use monitorium_platform as platform;
use single_instance::SingleInstance;

const APP_ID: &str = "com.bycat.monitorium";

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        platform::attach_console();
        list_monitors();
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--set") {
        platform::attach_console();
        match args.get(i + 1).and_then(|v| v.parse::<u8>().ok()) {
            Some(value) => set_all(value.min(100)),
            None => eprintln!("usage: monitorium --set <0-100>"),
        }
        return Ok(());
    }
    let autostart = args.iter().any(|a| a == platform::autostart::AUTOSTART_ARG);

    init_logging();

    // on macOS single-instance takes the name as a file path, relative to the working directory
    let lock = if cfg!(target_os = "macos") {
        settings::data_local_dir()
            .map(|dir| dir.join("monitorium.lock").to_string_lossy().into_owned())
    } else {
        None
    };
    let instance = SingleInstance::new(lock.as_deref().unwrap_or(APP_ID));
    match &instance {
        Ok(instance) if !instance.is_single() => {
            log::info!("another instance is already running");
            return Ok(());
        }
        Err(err) => log::warn!("single-instance check failed: {err}"),
        _ => {}
    }

    platform::init_process();

    let viewport = egui::ViewportBuilder::default()
        .with_title("Monitorium")
        .with_app_id(APP_ID)
        .with_inner_size([ui::FLYOUT_WIDTH, 320.0])
        // eframe shows the window after its first frame no matter what, so start off-screen
        .with_position([-32000.0, -32000.0])
        .with_decorations(false)
        .with_resizable(false)
        .with_always_on_top()
        .with_taskbar(false)
        .with_visible(false)
        .with_icon(icons::window_icon());

    let options = eframe::NativeOptions {
        viewport,
        centered: false,
        persist_window: false,
        event_loop_builder: event_loop_builder(),
        ..Default::default()
    };

    eframe::run_native(
        "Monitorium",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, autostart)))),
    )
}

#[cfg(target_os = "macos")]
fn event_loop_builder() -> Option<eframe::EventLoopBuilderHook> {
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

    // a menu bar app: no Dock icon or menu bar, and no stealing focus when started at login
    Some(Box::new(|builder| {
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_default_menu(false)
            .with_activate_ignoring_other_apps(false);
    }))
}

#[cfg(not(target_os = "macos"))]
fn event_loop_builder() -> Option<eframe::EventLoopBuilderHook> {
    None
}

fn init_logging() {
    use simplelog::{
        ColorChoice, CombinedLogger, Config, LevelFilter, SharedLogger, TermLogger, TerminalMode,
        WriteLogger,
    };

    let mut loggers: Vec<Box<dyn SharedLogger>> = Vec::new();
    if cfg!(debug_assertions) {
        loggers.push(TermLogger::new(
            LevelFilter::Debug,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ));
    }
    if let Some(dir) = settings::data_local_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(file) = File::create(dir.join("monitorium.log")) {
            loggers.push(WriteLogger::new(LevelFilter::Info, Config::default(), file));
        }
    }
    let _ = CombinedLogger::init(loggers);

    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
}

fn set_all(value: u8) {
    let service = platform::start_service(Arc::new(|_| {}));
    let (events, _events_rx) = crossbeam_channel::unbounded();
    let worker = monitorium_core::worker::spawn(
        move || platform::create_backend(service),
        monitorium_core::Settings::load().0,
        events,
        Arc::new(|| {}),
    );
    worker.send(monitorium_core::WorkerCmd::SetAll(value));
    worker.shutdown(std::time::Duration::from_secs(10));
    println!("Brightness set to {value}%.");
}

fn list_monitors() {
    let service = platform::start_service(Arc::new(|_| {}));
    let mut backend = platform::create_backend(service);
    let displays = backend.enumerate();
    if displays.is_empty() {
        println!("No monitors found.");
    }
    for display in displays {
        let control = match display.hardware {
            Some((kind, percent)) => format!("{kind:?} {percent}%"),
            None => "software dimming only".to_string(),
        };
        let hdr = match display.sdr {
            Some(sdr) => format!(", HDR (SDR brightness {sdr}%)"),
            None if display.hdr => ", HDR".to_string(),
            None => String::new(),
        };
        println!(
            "{:<28} {:<40} {}{}{}",
            display.name,
            display.id,
            control,
            if display.internal { ", built-in" } else { "" },
            hdr,
        );
    }
}
