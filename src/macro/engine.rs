use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use handy_sdk::app::session_reader::SessionReader;
use handy_sdk::app::updater_thread::Updater;
use handy_sdk::input::focus::roblox_ready;
use handy_sdk::input::hooks::KeyPoller;
use handy_sdk::input::key::{Key, KeyKind, MOUSE_LEFT, digit_key, key_down};
use handy_sdk::input::send::{release, tap};
use handy_sdk::roblox::events::{RobloxGroups, RootsCache, read_roblox_state, update_roots};
use handy_sdk::roblox::instance::data_model;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_XBUTTON1, VK_XBUTTON2};

use crate::app::config::SettingsStore;
use crate::game::fishing::{FishingReader, FishingSample};
use crate::r#macro::bot::{Action, FishBot};

fn left_click() {
    tap(&MOUSE_LEFT, Duration::from_millis(10)); // Roblox drops a down+up delivered in the same frame
}
fn tap_digit(digit: i32) {
    tap(&digit_key(digit), Duration::from_millis(30));
}
fn tap_key(k: &Key) {
    tap(k, Duration::from_millis(20)); // over one 60 FPS frame, so the game sees the key down
}
const fn keyboard(scan: u16) -> Key {
    Key {
        kind: KeyKind::Keyboard,
        scan,
        extended: false,
        mouse_button: 0,
    }
}
const E: Key = keyboard(0x12);
const F: Key = keyboard(0x21);

/// Left mouse, E, F and the hotbar digits up, unconditionally (a key-up for a key that isn't down does nothing).
/// For the crash path: a crash mid-tap must not leave one held in Roblox.
pub fn release_inputs() {
    release(&MOUSE_LEFT);
    release(&E);
    release(&F);
    (0..=9).for_each(|d| release(&digit_key(d)));
}

#[derive(Clone, Debug, Default)]
pub struct EngineView {
    pub on: bool,
    /// None = no session / fishing UI not found
    pub sample: Option<FishingSample>,
    pub catches: i32,
    pub losses: i32,
    pub clicks: i32,
    pub equip_gave_up: bool,
    /// last action, e.g. "cast" or "hit"
    pub last: String,
}

/// Polls the toggle key, reads the minigame, runs FishBot and clicks. Starts OFF.
pub struct Engine {
    u: Arc<Updater>,
    s: Arc<SettingsStore>,
    toggle_req: AtomicBool,
    view: Mutex<EngineView>,
}

impl Engine {
    pub fn new(u: Arc<Updater>, s: Arc<SettingsStore>) -> Self {
        Self {
            u,
            s,
            toggle_req: AtomicBool::new(false),
            view: Mutex::new(EngineView::default()),
        }
    }

    /// any thread
    pub fn toggle(&self) {
        self.toggle_req.store(true, Ordering::SeqCst);
    }

    pub fn view(&self) -> EngineView {
        self.view.lock().unwrap().clone()
    }

    pub fn run(&self, stop: &AtomicBool) {
        let mut bot = FishBot::default();
        let mut readers = SessionReader::for_updater(&self.u, |s| FishingReader::new(s.clone()));
        let mut toggle_key = KeyPoller::default(); // a key held at startup is not a press
        let (mut on, mut was_open) = (false, false);
        let mut rod_held = false; // last known; the held item is not read while the minigame is open
        let mut rod_key = -1;
        let t0 = Instant::now();
        let (mut next_e, mut next_click, mut next_f) = (t0, t0, t0);
        let mut roots = RootsCache::default(); // UserInputService, for the chat focus
        let mut was_chatting = true; // last tick's, for the unequip that uses last tick's session
        while !stop.load(Ordering::Relaxed) {
            let cfg = self.s.get();
            let mut did: Option<&str> = None;
            // Both run every loop: pressed() tracks the key edge and swap() clears a pending dashboard toggle.
            let toggle_key_hit = toggle_key.pressed(cfg.toggle_vk);
            if self.toggle_req.swap(false, Ordering::SeqCst) || toggle_key_hit {
                on = !on;
                bot.restart();
                // Switched off: put the rod away (its hotbar key toggles it). Only into Roblox, never another window.
                if !on
                    && !was_chatting // the hotbar key would be typed into the chat
                    && rod_held
                    && rod_key >= 0
                    && let Some(prev) = readers.session()
                    && roblox_ready(prev.proc.pid())
                {
                    tap_digit(rod_key);
                    did = Some("unequip rod");
                }
            }

            // After the toggle, as before: unequip uses last tick's session.
            // Only read the held item while closed: the minigame loop must stay fast.
            // A missing offset key panics (abort): every key read here is in required_keys(), which the Updater
            // enforces, so none can be missing.
            let smp = readers.get().and_then(|r| r.sample(!was_open));
            let sess = readers.session().cloned();
            // A focused TextBox (the chat) takes the keys; unreadable counts as chatting, so no keys are typed blindly.
            let mut chat = None;
            if let Some(s) = &sess {
                let (p, o) = (&s.proc, &s.offsets);
                update_roots(&mut roots, p, o, data_model(p, o), Instant::now());
                let groups = RobloxGroups {
                    chat: true,
                    character: false,
                    camera: false,
                };
                chat = read_roblox_state(p, o, &roots.roots, groups).chat_focused;
            }
            let chatting = chat.unwrap_or(true);
            was_chatting = chatting;
            was_open = smp.is_some_and(|s| s.open);
            if let Some(s) = &smp {
                if let Some(r) = s.rod {
                    rod_held = r;
                }
                if s.open {
                    rod_held = true;
                }
                if s.rod_key >= 0 {
                    rod_key = s.rod_key;
                }
            }
            if on && let (Some(smp), Some(sess)) = (&smp, &sess) {
                let now = t0.elapsed().as_secs_f64();
                // While chatting, the equip key would be typed and a cast click takes the chat box away; the minigame's
                // clicks still go, or the fish is lost.
                let can_send = roblox_ready(sess.proc.pid()) && (smp.open || !chatting);
                match bot.step(smp, can_send, now, &cfg.t) {
                    Action::Equip => {
                        tap_digit(smp.rod_key);
                        did = Some("equip rod");
                    }
                    Action::Cast => {
                        left_click();
                        did = Some("cast");
                    }
                    Action::Hit => {
                        left_click();
                        did = Some("hit");
                    }
                    Action::None => {}
                }
            }
            // Hold mouse X1 to spam E, X2 to spam left click (F on items with the sack), G to spam F off items, whether
            // or not the macro is on. Paused while the macro plays the minigame (stray clicks are misses) and, for the
            // keys, while chatting (they would be typed).
            let bot_busy = on && smp.is_some_and(|s| s.open);
            let spam_e = !bot_busy && !chatting && cfg.spam_e && key_down(VK_XBUTTON1.0);
            let spam_click = !bot_busy && cfg.spam_click && key_down(VK_XBUTTON2.0);
            let spam_f = !bot_busy && !chatting && cfg.spam_f && key_down(u16::from(b'G'));
            if (spam_e || spam_click || spam_f)
                && let Some(sess) = &sess
                && roblox_ready(sess.proc.pid())
            {
                if spam_e && Instant::now() >= next_e {
                    next_e = Instant::now() + ms(cfg.spam_e_ms);
                    tap_key(&E);
                }
                if spam_click && Instant::now() >= next_click {
                    next_click = Instant::now() + ms(cfg.spam_click_ms);
                    match &smp {
                        Some(s) if s.sack => {
                            if s.hover_item {
                                tap_key(&F); // sack in hand: F bags the item under the cursor
                            }
                        }
                        _ => left_click(),
                    }
                }
                if spam_f && !smp.is_some_and(|s| s.hover_item) && Instant::now() >= next_f {
                    next_f = Instant::now() + ms(cfg.spam_f_ms);
                    tap_key(&F);
                }
            }
            {
                let mut v = self.view.lock().unwrap();
                v.on = on;
                v.sample = smp;
                v.catches = bot.catches();
                v.losses = bot.losses();
                v.clicks = bot.hits();
                v.equip_gave_up = bot.equip_gave_up();
                if let Some(d) = did {
                    d.clone_into(&mut v.last);
                }
            }
            std::thread::sleep(ms(if bot_busy || spam_e || spam_click || spam_f {
                1
            } else {
                15
            }));
        }
    }
}

fn ms(n: i32) -> Duration {
    Duration::from_millis(n.max(0) as u64)
}
