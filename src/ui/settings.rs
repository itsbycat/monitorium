use std::ops::RangeInclusive;

use eframe::egui::{self, RichText};
use egui_phosphor::regular as icon;
use monitorium_core::{PowerOffMethod, SoftwareDimMode, ThemePreference, settings};

use super::{hotkey_field, icon_button};
use crate::app::{App, Page};
use crate::hotkeys::HotkeyAction;

const MAX_HEIGHT: f32 = 480.0;

#[cfg(target_os = "macos")]
const AUTOSTART_LABEL: &str = "Open at login";
#[cfg(not(target_os = "macos"))]
const AUTOSTART_LABEL: &str = "Start with Windows";

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if icon_button(ui, icon::ARROW_LEFT, "Back").clicked() {
            app.stop_recording();
            app.page = Page::Monitors;
        }
        ui.label(RichText::new("Settings").strong().size(15.0));
    });
    ui.add_space(2.0);
    ui.separator();

    // min_scrolled_height lets the page grow past the current window height
    egui::ScrollArea::vertical()
        .max_height(MAX_HEIGHT)
        .min_scrolled_height(MAX_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            general(app, ui);
            brightness(app, ui);
            shortcuts(app, ui);
            monitors(app, ui);
            about(app, ui);
        });
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(title)
            .strong()
            .color(ui.visuals().hyperlink_color),
    );
    ui.add_space(2.0);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().weak());
}

fn slider_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut u8,
    range: RangeInclusive<u8>,
    suffix: &str,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::Slider::new(value, range).suffix(suffix));
    });
}

fn general(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "General");
    ui.checkbox(&mut app.settings.start_with_os, AUTOSTART_LABEL);
    if let Some(err) = &app.ui.autostart_error {
        ui.colored_label(ui.visuals().error_fg_color, err);
    }
    ui.horizontal(|ui| {
        ui.label("Theme");
        let theme = &mut app.settings.theme;
        egui::ComboBox::from_id_salt("theme")
            .selected_text(match theme {
                ThemePreference::System => "System",
                ThemePreference::Light => "Light",
                ThemePreference::Dark => "Dark",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(theme, ThemePreference::System, "System");
                ui.selectable_value(theme, ThemePreference::Light, "Light");
                ui.selectable_value(theme, ThemePreference::Dark, "Dark");
            });
    });
}

fn brightness(app: &mut App, ui: &mut egui::Ui) {
    let s = &mut app.settings;
    section(ui, "Brightness");
    ui.checkbox(&mut s.link_levels, "Link all monitors");
    // macOS has no SDR brightness setting for HDR displays
    if cfg!(windows) {
        ui.checkbox(
            &mut s.hdr_sdr_brightness,
            "Use SDR content brightness on HDR displays",
        );
        hint(
            ui,
            "Most monitors lock their backlight in HDR mode, so DDC/CI has no visible effect \
             there. This adjusts the same setting as Windows' HDR brightness slider instead.",
        );
    }
    ui.checkbox(
        &mut s.software_fallback,
        "Dim monitors without DDC/CI in software",
    );

    ui.horizontal(|ui| {
        ui.label("Software dimming");
        ui.radio_value(
            &mut s.software_dim_mode,
            SoftwareDimMode::Overlay,
            "Overlay",
        );
        ui.radio_value(&mut s.software_dim_mode, SoftwareDimMode::Gamma, "Gamma");
    });
    hint(
        ui,
        match s.software_dim_mode {
            SoftwareDimMode::Overlay => {
                "A dark, click-through layer over the screen. Can dim almost to black."
            }
            SoftwareDimMode::Gamma if cfg!(target_os = "macos") => {
                "Scales the display's color curve. Night Shift and apps like f.lux can undo it \
                 for a moment."
            }
            SoftwareDimMode::Gamma => {
                "Scales the display's color curve. Windows limits it to about half brightness, \
                 and HDR displays use the overlay instead."
            }
        },
    );

    ui.horizontal(|ui| {
        ui.label("Turn off displays with");
        let method = &mut s.power_off_method;
        let label = |m: PowerOffMethod| match m {
            PowerOffMethod::System if cfg!(target_os = "macos") => "macOS",
            PowerOffMethod::System => "Windows",
            PowerOffMethod::Ddc => "DDC/CI",
            PowerOffMethod::Both => "Both",
        };
        egui::ComboBox::from_id_salt("power_off")
            .selected_text(label(*method))
            .show_ui(ui, |ui| {
                for m in [
                    PowerOffMethod::System,
                    PowerOffMethod::Ddc,
                    PowerOffMethod::Both,
                ] {
                    ui.selectable_value(method, m, label(m));
                }
            });
    });
}

fn shortcuts(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "Tray & shortcuts");
    ui.checkbox(
        &mut app.settings.scroll_on_tray,
        "Scroll over the tray icon to change brightness",
    );
    ui.add_enabled_ui(app.settings.scroll_on_tray, |ui| {
        slider_row(
            ui,
            "Scroll step",
            &mut app.settings.scroll_step,
            1..=25,
            "%",
        );
    });

    ui.add_space(4.0);
    ui.checkbox(&mut app.settings.hotkeys_enabled, "Keyboard shortcuts");
    let enabled = app.settings.hotkeys_enabled;
    ui.add_enabled_ui(enabled, |ui| {
        hotkey_row(app, ui, "Brightness up", HotkeyAction::BrightnessUp);
        hotkey_row(app, ui, "Brightness down", HotkeyAction::BrightnessDown);
        hotkey_row(app, ui, "Turn off displays", HotkeyAction::TurnOff);
        slider_row(
            ui,
            "Shortcut step",
            &mut app.settings.hotkey_step,
            1..=25,
            "%",
        );
    });
    if enabled {
        for err in app.hotkey_errors().to_vec() {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
    }
}

fn hotkey_row(app: &mut App, ui: &mut egui::Ui, label: &str, action: HotkeyAction) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let recording = app.ui.recording == Some(action);
            let current = hotkey_field(&mut app.settings, action).clone();
            if !current.is_empty() && !recording && icon_button(ui, icon::X, "Clear").clicked() {
                hotkey_field(&mut app.settings, action).clear();
            }
            let text = if recording {
                "Press keys… (Esc to cancel)".to_string()
            } else if current.is_empty() {
                "Not set".to_string()
            } else {
                current
            };
            if ui
                .add(egui::Button::selectable(recording, text))
                .on_hover_text("Click, then press the new shortcut")
                .clicked()
            {
                if recording {
                    app.stop_recording();
                } else {
                    app.start_recording(action);
                }
            }
        });
    });
}

fn monitors(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "Monitors");
    if app.monitors.is_empty() {
        hint(ui, "No monitors detected.");
    }
    for monitor in app.monitors.clone() {
        let glyph = if monitor.internal {
            icon::LAPTOP
        } else {
            icon::MONITOR
        };
        egui::CollapsingHeader::new(format!("{glyph}  {}", monitor.name))
            .id_salt(&monitor.id)
            .show(ui, |ui| {
                let ms = app.settings.monitor_mut(&monitor.id);

                ui.horizontal(|ui| {
                    ui.label("Name");
                    let mut name = ms.name.clone().unwrap_or_default();
                    let edit = egui::TextEdit::singleline(&mut name)
                        .hint_text(&monitor.hardware_name)
                        .desired_width(f32::INFINITY);
                    if ui.add(edit).changed() {
                        ms.name = (!name.trim().is_empty()).then_some(name);
                    }
                });
                ui.checkbox(&mut ms.hidden, "Hide from flyout");
                ui.add_enabled(
                    monitor.has_hardware,
                    egui::Checkbox::new(
                        &mut ms.force_software,
                        "Use software dimming instead of hardware brightness",
                    ),
                );
                slider_row(ui, "Minimum", &mut ms.min, 0..=99, "%");
                slider_row(ui, "Maximum", &mut ms.max, 1..=100, "%");
                if ms.min >= ms.max {
                    ms.max = (ms.min + 1).min(100);
                }
                hint(
                    ui,
                    &format!(
                        "{}{} · {}",
                        monitor.method.label(),
                        if monitor.hdr { " · HDR" } else { "" },
                        monitor.id
                    ),
                );
            });
    }
}

fn about(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "About");
    ui.label(format!("Monitorium {}", env!("CARGO_PKG_VERSION")));
    ui.horizontal(|ui| {
        if ui
            .button(format!("{}  Settings folder", icon::FOLDER_OPEN))
            .clicked()
            && let Some(dir) = settings::config_dir()
        {
            let _ = std::fs::create_dir_all(&dir);
            open_folder(&dir);
        }
        if ui.button(format!("{}  Quit", icon::SIGN_OUT)).clicked() {
            app.quit();
        }
    });
}

fn open_folder(dir: &std::path::Path) {
    let program = if cfg!(windows) { "explorer" } else { "open" };
    if let Err(err) = std::process::Command::new(program).arg(dir).spawn() {
        log::warn!("opening {} failed: {err}", dir.display());
    }
}
