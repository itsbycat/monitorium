use eframe::egui::{self, Align, Layout, RichText};
use egui_phosphor::regular as icon;
use monitorium_core::Method;

use super::icon_button;
use crate::app::{App, Page};

const POINTS_PER_SCROLL_STEP: f32 = 40.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    header(app, ui);
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(2.0);

    let visible: Vec<usize> = app
        .monitors
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.hidden && m.method != Method::None)
        .map(|(i, _)| i)
        .collect();
    let step = app.settings.scroll_step.max(1);

    if !app.monitors_loaded {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("Looking for monitors…").weak());
        });
    } else if visible.is_empty() {
        ui.label(RichText::new("No adjustable monitors found.").weak());
    } else if app.settings.link_levels && visible.len() > 1 {
        let mut value = app.monitors[visible[0]].brightness;
        let names = visible
            .iter()
            .map(|&i| app.monitors[i].name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if brightness_row(ui, icon::SUN, "All displays", &names, &mut value, step) {
            app.set_all_brightness(value);
        }
    } else {
        for (n, i) in visible.into_iter().enumerate() {
            if n > 0 {
                ui.add_space(6.0);
            }
            let monitor = &app.monitors[i];
            let glyph = if monitor.internal {
                icon::LAPTOP
            } else {
                icon::MONITOR
            };
            let (id, name, method) = (monitor.id.clone(), monitor.name.clone(), monitor.method);
            let mut value = monitor.brightness;
            if brightness_row(ui, glyph, &name, method.label(), &mut value, step) {
                app.set_brightness(&id, value);
            }
        }
    }

    ui.add_space(4.0);
    ui.separator();
    footer(app, ui);
}

fn header(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let logo = app.ui.logo(ui.ctx(), ui.visuals().dark_mode);
        ui.add(egui::Image::new((logo.id(), egui::vec2(18.0, 18.0))));
        ui.label(RichText::new("Monitorium").strong().size(15.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, icon::GEAR_SIX, "Settings").clicked() {
                app.page = Page::Settings;
            }
            if icon_button(ui, icon::ARROWS_CLOCKWISE, "Refresh monitors").clicked() {
                app.refresh();
            }
        });
    });
}

fn footer(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let linked = app.settings.link_levels;
        let label = format!(
            "{}  Link levels",
            if linked { icon::LINK } else { icon::LINK_BREAK }
        );
        if ui
            .add(egui::Button::selectable(linked, label))
            .on_hover_text("Move all monitors together")
            .clicked()
        {
            app.settings.link_levels = !linked;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .button(format!("{}  Turn off", icon::POWER))
                .on_hover_text("Turn off displays")
                .clicked()
            {
                app.turn_off_displays();
            }
        });
    });
}

fn brightness_row(
    ui: &mut egui::Ui,
    glyph: &str,
    title: &str,
    subtitle: &str,
    value: &mut u8,
    scroll_step: u8,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(RichText::new(glyph).size(20.0));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(title).strong());
            ui.label(RichText::new(subtitle).small().weak());
        });
    });

    ui.horizontal(|ui| {
        ui.label(RichText::new(icon::SUN_DIM).size(16.0).weak());
        let percent_width = 40.0;
        ui.spacing_mut().slider_width =
            ui.available_width() - percent_width - ui.spacing().item_spacing.x;
        let response = ui.add(
            egui::Slider::new(value, 0..=100)
                .show_value(false)
                .trailing_fill(true),
        );
        let mut changed = response.changed();

        if response.hovered() {
            let steps = scroll_steps(ui, response.id.with("scroll"));
            if steps != 0 {
                *value = (i16::from(*value) + steps * i16::from(scroll_step)).clamp(0, 100) as u8;
                changed = true;
            }
        }

        ui.add_sized(
            [percent_width, ui.spacing().interact_size.y],
            egui::Label::new(RichText::new(format!("{value}%")).monospace()),
        );
        changed
    })
    .inner
}

// a wheel notch is one step, while trackpads send streams of small point deltas that add up
fn scroll_steps(ui: &egui::Ui, id: egui::Id) -> i16 {
    let (lines, points) = ui.input(|i| {
        i.events
            .iter()
            .fold((0.0f32, 0.0f32), |(lines, points), event| match event {
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta,
                    ..
                } => (lines, points + delta.y),
                egui::Event::MouseWheel { delta, .. } => (lines + delta.y, points),
                _ => (lines, points),
            })
    });
    let point_steps = ui.data_mut(|data| {
        let pending = data.get_temp_mut_or_default::<f32>(id);
        *pending += points;
        let steps = (*pending / POINTS_PER_SCROLL_STEP).trunc();
        *pending -= steps * POINTS_PER_SCROLL_STEP;
        steps as i16
    });
    let line_steps = if lines == 0.0 {
        0
    } else {
        lines.signum() as i16
    };
    line_steps + point_steps
}
