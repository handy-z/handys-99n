use std::path::Path;

use handy_sdk::util::ini::Ini;
use handy_sdk::util::shared_settings::{SaveSettings, SharedSettings};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_F1;

use crate::r#macro::bot::BotTimings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub toggle_vk: u32,
    /// hold mouse X1 / X2 / G
    pub spam_e: bool,
    pub spam_click: bool,
    pub spam_f: bool,
    /// between presses
    pub spam_e_ms: i32,
    pub spam_click_ms: i32,
    pub spam_f_ms: i32,
    pub t: BotTimings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            toggle_vk: u32::from(VK_F1.0),
            spam_e: true,
            spam_click: true,
            spam_f: true,
            spam_e_ms: 25,
            spam_click_ms: 25,
            spam_f_ms: 25,
            t: BotTimings::default(),
        }
    }
}

/// Every field is clamped to the dashboard's editable range on load.
pub fn load_settings(file: &Path) -> Settings {
    let ini = Ini::new(file);
    let d = Settings::default();
    let flag = |key: &str, def: bool| ini.get_int_clamped(key, i32::from(def), 0, 1) != 0;
    Settings {
        toggle_vk: ini.get_int_clamped("toggleVk", d.toggle_vk as i32, 1, 254) as u32,
        spam_e: flag("spamE", d.spam_e),
        spam_click: flag("spamClick", d.spam_click),
        spam_f: flag("spamF", d.spam_f),
        spam_e_ms: ini.get_int_clamped("spamEMs", d.spam_e_ms, 10, 1000),
        spam_click_ms: ini.get_int_clamped("spamClickMs", d.spam_click_ms, 10, 1000),
        spam_f_ms: ini.get_int_clamped("spamFMs", d.spam_f_ms, 10, 1000),
        t: BotTimings {
            equip_delay_ms: ini.get_int_clamped("equipDelayMs", d.t.equip_delay_ms, 0, 2000),
            lead_ms: ini.get_int_clamped("latencyMs", d.t.lead_ms, 0, 100),
            cooldown_ms: ini.get_int_clamped("hitCooldownMs", d.t.cooldown_ms, 0, 5000),
            bite_wait_s: ini.get_int_clamped("biteWaitS", d.t.bite_wait_s, 5, 300),
            recast_delay_ms: ini.get_int_clamped("recastMs", d.t.recast_delay_ms, 0, 10000),
        },
    }
}

pub fn save_settings(file: &Path, s: &Settings) {
    let ini = Ini::new(file);
    ini.put_int("toggleVk", s.toggle_vk as i32);
    ini.put_bool("spamE", s.spam_e);
    ini.put_bool("spamClick", s.spam_click);
    ini.put_bool("spamF", s.spam_f);
    ini.put_int("equipDelayMs", s.t.equip_delay_ms);
    ini.put_int("spamEMs", s.spam_e_ms);
    ini.put_int("spamClickMs", s.spam_click_ms);
    ini.put_int("spamFMs", s.spam_f_ms);
    ini.put_int("latencyMs", s.t.lead_ms);
    ini.put_int("hitCooldownMs", s.t.cooldown_ms);
    ini.put_int("biteWaitS", s.t.bite_wait_s);
    ini.put_int("recastMs", s.t.recast_delay_ms);
}

impl SaveSettings for Settings {
    fn save(&self, ini: &Path) {
        save_settings(ini, self);
    }
}

/// Settings shared between the engine, dashboard and updater threads.
pub type SettingsStore = SharedSettings<Settings>;

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_F2;

    #[test]
    fn config_roundtrip_and_clamp() {
        let dir = std::env::temp_dir().join(format!("n99_config_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let ini = dir.join("n99_config_test.ini");
        let _ = std::fs::remove_file(&ini);
        let mut s = Settings::default();
        s.t.lead_ms = 30;
        s.t.bite_wait_s = 60;
        s.toggle_vk = u32::from(VK_F2.0);
        s.spam_click = false;
        s.spam_f = false;
        s.spam_f_ms = 60;
        s.spam_e_ms = 50;
        s.t.equip_delay_ms = 100;
        save_settings(&ini, &s);
        assert_eq!(load_settings(&ini), s);
        Ini::new(&ini).put_str("latencyMs", "999");
        assert_eq!(load_settings(&ini).t.lead_ms, 100);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
