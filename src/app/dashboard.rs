use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;

use handy_sdk::app::self_update::{SelfUpdate, UpdateState};
use handy_sdk::app::updater_thread::{Updater, status_name, status_tone};
use handy_sdk::input::key::vk_name;
use handy_sdk::ui::dashboard::{Dashboard, Spec, check, note, number, page, section, value};
use handy_sdk::ui::overlay::StatusOverlay;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWDEFAULT;

use crate::app::config::{Settings, SettingsStore};
use crate::app::view::{minigame_text, rod_text, session_text};
use crate::r#macro::engine::{Engine, release_inputs};

pub const DASHBOARD_CLASS: &str = "handys-99n-dashboard";

pub struct DashboardDeps {
    pub updater: Arc<Updater>,
    pub engine: Arc<Engine>,
    pub settings: Arc<SettingsStore>,
    pub config_ini: PathBuf,
    pub update: Arc<SelfUpdate>,
}

const ID_ON: i32 = 2;
const ID_ROD: i32 = 3;
const ID_GAME: i32 = 4;
const ID_SESSION: i32 = 5;
const ID_HINT: i32 = 6;
const ID_SPAM_E: i32 = 7;
const ID_SPAM_CLICK: i32 = 8;
const ID_SPAM_F: i32 = 9;
const ID_LEAD: i32 = 100;
const ID_COOLDOWN: i32 = 101;
const ID_BITE: i32 = 102;
const ID_RECAST: i32 = 103;
const ID_EQUIP: i32 = 104;
const ID_SPAM_E_MS: i32 = 105;
const ID_SPAM_CLICK_MS: i32 = 106;
const ID_SPAM_F_MS: i32 = 107;

const FIELDS: [i32; 8] = [
    ID_LEAD,
    ID_COOLDOWN,
    ID_BITE,
    ID_RECAST,
    ID_EQUIP,
    ID_SPAM_E_MS,
    ID_SPAM_CLICK_MS,
    ID_SPAM_F_MS,
];

/// The Settings field a Number row edits.
fn field(s: &mut Settings, id: i32) -> Option<&mut i32> {
    Some(match id {
        ID_LEAD => &mut s.t.lead_ms,
        ID_COOLDOWN => &mut s.t.cooldown_ms,
        ID_BITE => &mut s.t.bite_wait_s,
        ID_RECAST => &mut s.t.recast_delay_ms,
        ID_EQUIP => &mut s.t.equip_delay_ms,
        ID_SPAM_E_MS => &mut s.spam_e_ms,
        ID_SPAM_CLICK_MS => &mut s.spam_click_ms,
        ID_SPAM_F_MS => &mut s.spam_f_ms,
        _ => return None,
    })
}

/// Shows the dashboard and the "macro: on/off" overlay until the window is closed. UI thread only.
pub fn run_dashboard(deps: &DashboardDeps) -> i32 {
    // Declared before the dashboard, whose callbacks borrow them.
    let overlay = StatusOverlay::new();
    let first = Cell::new(true);
    let d = Dashboard::new(Spec {
        title: "handy's 99n".into(),
        badge: deps.update.badge().text,
        window_class: DASHBOARD_CLASS.into(),
        rows: vec![
            page("Overview"),
            section("Fishing"),
            check(ID_ON, "Macro"),
            value(ID_ROD, "Rod"),
            value(ID_GAME, "Minigame"),
            value(ID_SESSION, "This session"),
            section("Spam"),
            check(ID_SPAM_E, "Spam E\thold Mouse 4"),
            check(ID_SPAM_CLICK, "Spam click (F with a sack)\thold Mouse 5"),
            check(ID_SPAM_F, "Spam F when not over an item\thold G"),
            page("Settings"),
            section("Fishing timings"),
            number(ID_LEAD, "Latency (ms)", 0, 100),
            number(ID_COOLDOWN, "Hit cooldown (ms)", 0, 5000),
            number(ID_BITE, "Bite wait (s)", 5, 300),
            number(ID_RECAST, "Recast delay (ms)", 0, 10000),
            number(ID_EQUIP, "Equip delay (ms)", 0, 2000),
            section("Spam speed"),
            number(ID_SPAM_E_MS, "E every (ms)", 10, 1000),
            number(ID_SPAM_CLICK_MS, "Click every (ms)", 10, 1000),
            number(ID_SPAM_F_MS, "F every (ms)", 10, 1000),
            note(ID_HINT),
        ],
        hint_row: Some(ID_HINT),
        label_width: 120, // 99n's column widths
        value_width: 300,
        ..Default::default()
    });
    overlay.create();
    d.on_tick(|ui| {
        let b = deps.update.badge();
        ui.set_badge(&b.text, b.tone, &b.tip);
        if first.replace(false) {
            // first tick runs right after the window is created
            let mut st = deps.settings.get();
            for id in FIELDS {
                ui.set_number(id, *field(&mut st, id).unwrap());
            }
        }
        let v = deps.engine.view();
        let status = deps.updater.status();
        ui.set_status(status_tone(status), status_name(status), &deps.updater.detail());
        let st = deps.settings.get();
        ui.set_text(ID_ON, &format!("Macro\tpress {}", vk_name(st.toggle_vk)));
        ui.set_check(ID_ON, v.on);
        ui.set_check(ID_SPAM_E, st.spam_e);
        ui.set_check(ID_SPAM_CLICK, st.spam_click);
        ui.set_check(ID_SPAM_F, st.spam_f);
        let gave_up = if v.on && v.equip_gave_up {
            " - couldn't equip; press F1 twice to retry"
        } else {
            ""
        };
        ui.set_text(ID_ROD, &(rod_text(v.sample.as_ref()) + gave_up));
        ui.set_text(ID_GAME, &minigame_text(v.sample.as_ref()));
        ui.set_text(ID_SESSION, &session_text(&v));
        overlay.set(if v.on { "Macro on" } else { "Macro off" }, v.on);
    });
    d.on_badge_click(|ui| {
        if matches!(deps.update.state(), UpdateState::Ready { .. }) && deps.engine.view().on {
            return ui.flash(); // not while the app is working
        }
        if deps.update.request_restart() {
            ui.close();
        } else {
            handy_sdk::ui::open_url(&deps.update.release_url());
        }
    });
    d.on_check(|_, id, checked| {
        if id == ID_ON {
            return deps.engine.toggle();
        }
        deps.settings.update(|s| {
            *(match id {
                ID_SPAM_E => &mut s.spam_e,
                ID_SPAM_CLICK => &mut s.spam_click,
                _ => &mut s.spam_f,
            }) = checked;
        });
        deps.settings.save(&deps.config_ini);
    });
    d.on_number(|_, id, value| {
        deps.settings.update(|s| {
            if let Some(f) = field(s, id) {
                *f = value;
            }
        });
        deps.settings.save(&deps.config_ini);
    });
    d.on_end_session(release_inputs); // logoff/shutdown can end us without WM_DESTROY
    d.run(SW_SHOWDEFAULT)
}
