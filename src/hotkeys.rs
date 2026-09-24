use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::HotKey;
use monitorium_core::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyAction {
    BrightnessUp,
    BrightnessDown,
    TurnOff,
}

pub struct Hotkeys {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<(HotKey, HotkeyAction)>,
    pub errors: Vec<String>,
}

impl Hotkeys {
    pub fn new() -> Self {
        let manager = GlobalHotKeyManager::new()
            .inspect_err(|err| log::warn!("global hotkeys unavailable: {err}"))
            .ok();
        Self {
            manager,
            registered: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn apply(&mut self, settings: &Settings) {
        self.clear();
        let Some(manager) = &self.manager else {
            return;
        };
        if !settings.hotkeys_enabled {
            return;
        }
        let wanted = [
            (&settings.hotkey_up, HotkeyAction::BrightnessUp),
            (&settings.hotkey_down, HotkeyAction::BrightnessDown),
            (&settings.hotkey_off, HotkeyAction::TurnOff),
        ];
        for (text, action) in wanted {
            if text.trim().is_empty() {
                continue;
            }
            let hotkey = match text.parse::<HotKey>() {
                Ok(hotkey) => hotkey,
                Err(err) => {
                    self.errors.push(format!("{text}: {err}"));
                    continue;
                }
            };
            match manager.register(hotkey) {
                Ok(()) => self.registered.push((hotkey, action)),
                Err(err) => self.errors.push(format!("{text}: {err}")),
            }
        }
    }

    pub fn clear(&mut self) {
        self.errors.clear();
        if let Some(manager) = &self.manager {
            let keys: Vec<HotKey> = self.registered.iter().map(|(k, _)| *k).collect();
            let _ = manager.unregister_all(&keys);
        }
        self.registered.clear();
    }

    pub fn action(&self, id: u32) -> Option<HotkeyAction> {
        self.registered
            .iter()
            .find(|(key, _)| key.id() == id)
            .map(|(_, action)| *action)
    }
}
