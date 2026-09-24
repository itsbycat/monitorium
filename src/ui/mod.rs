mod flyout;
mod settings;

use eframe::egui::{self, FontId, RichText, TextStyle, TextureHandle, TextureOptions};
use global_hotkey::hotkey::HotKey;

use crate::app::{App, Page};
use crate::hotkeys::HotkeyAction;
use crate::icons;

pub const FLYOUT_WIDTH: f32 = 360.0;
const MARGIN: i8 = 14;

#[derive(Default)]
pub struct UiState {
    pub recording: Option<HotkeyAction>,
    pub autostart_error: Option<String>,
    logo: Option<(bool, TextureHandle)>,
}

impl UiState {
    fn logo(&mut self, ctx: &egui::Context, dark_mode: bool) -> TextureHandle {
        match &self.logo {
            Some((dark, texture)) if *dark == dark_mode => texture.clone(),
            _ => {
                let texture =
                    ctx.load_texture("logo", icons::logo_image(dark_mode), TextureOptions::LINEAR);
                self.logo = Some((dark_mode, texture.clone()));
                texture
            }
        }
    }
}

pub fn setup_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.interact_size.y = 24.0;
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(13.5));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(13.5));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(11.5));
    });
}

pub fn draw(app: &mut App, ui: &mut egui::Ui) -> f32 {
    let before = app.settings.clone();
    capture_hotkey(app, ui.ctx());

    let visuals = ui.visuals().clone();
    let response = egui::Frame::new()
        .fill(visuals.panel_fill)
        .stroke(visuals.window_stroke)
        .inner_margin(egui::Margin::same(MARGIN))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            match app.page {
                Page::Monitors => flyout::show(app, ui),
                Page::Settings => settings::show(app, ui),
            }
        })
        .response;

    if app.settings != before {
        app.settings_changed(&before);
    }
    response.rect.height()
}

fn icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(icon).size(17.0)).frame(false))
        .on_hover_text(tooltip)
}

fn capture_hotkey(app: &mut App, ctx: &egui::Context) {
    let Some(action) = app.ui.recording else {
        return;
    };
    let events = ctx.input(|i| i.events.clone());
    for event in events {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            continue;
        };
        if key == egui::Key::Escape {
            app.stop_recording();
            return;
        }
        if let Some(text) = hotkey_text(key, modifiers) {
            *hotkey_field(&mut app.settings, action) = text;
            app.stop_recording();
            return;
        }
    }
}

pub(crate) fn hotkey_field(
    settings: &mut monitorium_core::Settings,
    action: HotkeyAction,
) -> &mut String {
    match action {
        HotkeyAction::BrightnessUp => &mut settings.hotkey_up,
        HotkeyAction::BrightnessDown => &mut settings.hotkey_down,
        HotkeyAction::TurnOff => &mut settings.hotkey_off,
    }
}

fn hotkey_text(key: egui::Key, modifiers: egui::Modifiers) -> Option<String> {
    let name = key.name();
    let is_function_key =
        name.len() > 1 && name.starts_with('F') && name[1..].chars().all(|c| c.is_ascii_digit());
    let cmd = cfg!(target_os = "macos") && modifiers.mac_cmd;
    if !(modifiers.ctrl || modifiers.alt || modifiers.shift || cmd) && !is_function_key {
        return None;
    }
    let key_name = [key.name(), key.symbol_or_name()]
        .into_iter()
        .find(|n| n.parse::<HotKey>().is_ok())?;

    let mut parts = Vec::new();
    if modifiers.ctrl {
        parts.push("Ctrl");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    if cmd {
        parts.push("Cmd");
    }
    parts.push(key_name);
    let text = parts.join("+");
    text.parse::<HotKey>().ok().map(|_| text)
}
