use monitorium_core::Rect;
use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::icons;

#[cfg(target_os = "macos")]
const AUTOSTART_LABEL: &str = "Open at Login";
#[cfg(not(target_os = "macos"))]
const AUTOSTART_LABEL: &str = "Start with Windows";

pub struct MenuIds {
    pub open: MenuId,
    pub refresh: MenuId,
    pub turn_off: MenuId,
    pub settings: MenuId,
    pub autostart: MenuId,
    pub quit: MenuId,
}

pub struct Tray {
    icon: TrayIcon,
    autostart: CheckMenuItem,
    pub ids: MenuIds,
}

impl Tray {
    pub fn new(light_taskbar: bool, autostart_enabled: bool) -> Result<Self, String> {
        let open = MenuItem::new("Open", true, None);
        let refresh = MenuItem::new("Refresh monitors", true, None);
        let turn_off = MenuItem::new("Turn off displays", true, None);
        let settings = MenuItem::new("Settings", true, None);
        let autostart = CheckMenuItem::new(AUTOSTART_LABEL, true, autostart_enabled, None);
        let quit = MenuItem::new("Quit", true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &open,
            &PredefinedMenuItem::separator(),
            &refresh,
            &turn_off,
            &PredefinedMenuItem::separator(),
            &settings,
            &autostart,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .map_err(|e| e.to_string())?;

        let icon = TrayIconBuilder::new()
            .with_icon(icons::tray_icon(light_taskbar))
            .with_tooltip("Monitorium")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            icon,
            ids: MenuIds {
                open: open.id().clone(),
                refresh: refresh.id().clone(),
                turn_off: turn_off.id().clone(),
                settings: settings.id().clone(),
                autostart: autostart.id().clone(),
                quit: quit.id().clone(),
            },
            autostart,
        })
    }

    pub fn set_light_taskbar(&self, light_taskbar: bool) {
        let _ = self.icon.set_icon(Some(icons::tray_icon(light_taskbar)));
    }

    pub fn set_tooltip(&self, text: &str) {
        // Windows truncates tooltips at 127 characters
        let text: String = text.chars().take(127).collect();
        let _ = self.icon.set_tooltip(Some(text));
    }

    pub fn set_autostart_checked(&self, checked: bool) {
        self.autostart.set_checked(checked);
    }

    pub fn rect(&self) -> Option<Rect> {
        self.icon.rect().map(to_rect)
    }
}

pub fn to_rect(rect: tray_icon::Rect) -> Rect {
    Rect {
        x: rect.position.x.round() as i32,
        y: rect.position.y.round() as i32,
        width: rect.size.width as i32,
        height: rect.size.height as i32,
    }
}
