use crate::game::fishing::FishingSample;

const EQUIP_RETRY_S: f64 = 1.5;

/// Measured on version-5a21b87bbc2641cf: a click with the bar in the zone is a hit (+1 step), outside a miss
/// (-1 step), catch at progress 1. Clicks within ~0.8 s of a counted one are ignored. A click while the
/// minigame is closed casts (its bobber shows up in Workspace/Particles ~0.5 s later); a bite opens the minigame
/// ~1.5-17 s later, and clicking before that reels the line back in. The bobber lingers ~0.6 s after a catch and
/// a cast sent too soon after it is ignored: 1.2 s after the close was taken every time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BotTimings {
    /// click-to-registration latency: the game sees the bar ~1-2 frames after the click
    pub lead_ms: i32,
    /// between minigame clicks
    pub cooldown_ms: i32,
    /// line out this long without a bite: reel in and recast
    pub bite_wait_s: i32,
    /// after the minigame closes, before the next cast
    pub recast_delay_ms: i32,
    /// after pressing the rod's key, before the cast (a cast that doesn't take is retried)
    pub equip_delay_ms: i32,
}

impl Default for BotTimings {
    fn default() -> Self {
        Self {
            lead_ms: 12,
            cooldown_ms: 850,
            bite_wait_s: 45,
            recast_delay_ms: 1200,
            equip_delay_ms: 300,
        }
    }
}

/// Aims at the zone centre: where the bar will be when the click registers (`lead_s` ahead) has reached the zone
/// centre and is still inside the zone (minus a small edge margin). A still bar only needs to be inside.
pub fn should_click(bar_mid: f32, vel: f32, zone_top: f32, zone_h: f32, lead_s: f64) -> bool {
    let (c, half) = (zone_top + zone_h / 2.0, zone_h / 2.0 - 2.0f32.min(zone_h * 0.1));
    let at = bar_mid + vel * lead_s as f32; // bar position when the game reads it
    if (at - c).abs() > half {
        return false;
    }
    vel.abs() < 1.0 || (at - c) * vel >= 0.0 // reached the centre in its direction of travel
}

/// Equip = press the rod's hotbar key (FishingSample::rod_key)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Equip,
    Cast,
    Hit,
}

pub struct FishBot {
    was_open: bool,
    have_last: bool,
    cast_sent: bool,
    line_was_out: bool,
    last_mid: f32,
    vel: f32,
    last_progress: f32,
    last_move_s: f64,
    last_click_s: f64,
    next_cast: f64,
    next_equip: f64,
    cast_at: f64,
    line_since: f64,
    catches: i32,
    losses: i32,
    hits: i32,
    equip_tries: i32,
    cast_fails: i32,
}

impl Default for FishBot {
    fn default() -> Self {
        Self {
            was_open: false,
            have_last: false,
            cast_sent: false,
            line_was_out: false,
            last_mid: 0.0,
            vel: 0.0,
            last_progress: 0.0,
            last_move_s: 0.0,
            last_click_s: -1e9,
            next_cast: 0.0,
            next_equip: 0.0,
            cast_at: 0.0,
            line_since: 0.0,
            catches: 0,
            losses: 0,
            hits: 0,
            equip_tries: 0,
            cast_fails: 0,
        }
    }
}

impl FishBot {
    pub const MAX_EQUIP_TRIES: i32 = 3;
    /// A cast whose bobber has not shown up after CAST_CHECK_S failed: retry at once FAST_RETRIES times, then slowly
    /// (not at the water, or the game moved us).
    pub const CAST_CHECK_S: f64 = 0.8;
    pub const SLOW_RETRY_S: f64 = 5.0;
    /// then stop pressing keys until restart() or the rod shows up
    pub const FAST_RETRIES: i32 = 3;

    /// now: monotonic seconds. can_click: Roblox is foreground with the cursor over it.
    pub fn step(&mut self, s: &FishingSample, can_click: bool, now: f64, t: &BotTimings) -> Action {
        if !s.open {
            if self.was_open {
                // The last counted step before closing: near the top means the final hit caught it.
                if self.last_progress >= 0.5 {
                    self.catches += 1;
                } else {
                    self.losses += 1;
                }
                self.next_cast = now + f64::from(t.recast_delay_ms) / 1000.0;
            }
            self.was_open = false;
            self.have_last = false;
            if s.rod == Some(false) {
                // Pressing the key again would unequip, so wait for the equip to show up before retrying.
                if s.rod_key < 0 || !can_click || now < self.next_equip || self.equip_gave_up() {
                    return Action::None;
                }
                self.equip_tries += 1;
                self.next_equip = now + EQUIP_RETRY_S;
                self.next_cast = now + f64::from(t.equip_delay_ms) / 1000.0; // the line is reeled in once the rod is put away
                return Action::Equip;
            }
            if s.rod != Some(true) {
                return Action::None;
            }
            self.equip_tries = 0;
            if s.line_out == Some(true) {
                // waiting for a bite; a click now would reel in
                if !self.line_was_out {
                    self.line_since = now;
                }
                self.line_was_out = true;
                self.cast_fails = 0;
                self.cast_sent = false;
                if !can_click || now - self.line_since < f64::from(t.bite_wait_s) {
                    return Action::None;
                }
                self.line_since = now; // no bite for too long: reel in, then the next pass recasts
                self.next_cast = now + f64::from(t.recast_delay_ms) / 1000.0;
                self.last_click_s = now;
                return Action::Cast;
            }
            self.line_was_out = false;
            if self.cast_sent {
                // unknown: old wait
                let wait = if s.line_out.is_some() {
                    Self::CAST_CHECK_S
                } else {
                    f64::from(t.bite_wait_s)
                };
                if now - self.cast_at < wait {
                    return Action::None;
                }
                self.cast_sent = false;
                self.cast_fails += 1;
                self.next_cast = now
                    + if self.cast_fails <= Self::FAST_RETRIES {
                        0.0
                    } else {
                        Self::SLOW_RETRY_S
                    };
            }
            if !can_click || now < self.next_cast {
                return Action::None;
            }
            self.cast_sent = true;
            self.cast_at = now;
            self.last_click_s = now;
            return Action::Cast;
        }
        self.was_open = true;
        self.cast_sent = false;
        self.line_was_out = false;
        self.cast_fails = 0;
        self.last_progress = s.progress;
        // The GUI moves once per rendered frame: velocity is measured between real moves, then extrapolated.
        if !self.have_last || s.bar_mid != self.last_mid {
            self.vel = if self.have_last && now > self.last_move_s {
                (s.bar_mid - self.last_mid) / (now - self.last_move_s) as f32
            } else {
                0.0
            };
            self.last_mid = s.bar_mid;
            self.last_move_s = now;
            self.have_last = true;
        }
        let mid = s.bar_mid + self.vel * (now - self.last_move_s) as f32;
        if can_click
            && now - self.last_click_s > f64::from(t.cooldown_ms) / 1000.0
            && should_click(mid, self.vel, s.zone_top, s.zone_h, f64::from(t.lead_ms) / 1000.0)
        {
            self.last_click_s = now;
            self.hits += 1;
            return Action::Hit;
        }
        Action::None
    }

    /// switched on: equip / cast at once
    pub fn restart(&mut self) {
        self.next_cast = 0.0;
        self.next_equip = 0.0;
        self.equip_tries = 0;
        self.cast_fails = 0;
        self.cast_sent = false;
    }
    pub fn equip_gave_up(&self) -> bool {
        self.equip_tries >= Self::MAX_EQUIP_TRIES
    }
    pub fn catches(&self) -> i32 {
        self.catches
    }
    pub fn losses(&self) -> i32 {
        self.losses
    }
    pub fn hits(&self) -> i32 {
        self.hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_click_aims_at_zone_centre() {
        // zone 492..516, centre 504, usable half-width 10
        assert!(should_click(504.0, 0.0, 492.0, 24.0, 0.012)); // still bar inside
        assert!(!should_click(480.0, 0.0, 492.0, 24.0, 0.012));
        assert!(!should_click(490.0, 1000.0, 492.0, 24.0, 0.012)); // registers at 502: before the centre, wait
        assert!(should_click(493.0, 1000.0, 492.0, 24.0, 0.012)); // registers at 505: just past the centre
        assert!(!should_click(510.0, 1000.0, 492.0, 24.0, 0.012)); // registers at 522: outside
        assert!(should_click(515.0, -1000.0, 492.0, 24.0, 0.012)); // moving up: registers at 503
        assert!(!should_click(518.0, -1000.0, 492.0, 24.0, 0.012)); // registers at 506: before the centre going up
        assert!(!should_click(560.0, -500.0, 486.0, 105.0, 0.012)); // big zone, moving up: registers at 554, before centre 538.5
        assert!(should_click(535.0, -500.0, 486.0, 105.0, 0.012)); // registers at 529: past the centre
    }

    fn closed(rod: bool) -> FishingSample {
        FishingSample {
            rod: Some(rod),
            ..Default::default()
        }
    }
    fn open(mid: f32, progress: f32) -> FishingSample {
        FishingSample {
            open: true,
            bar_mid: mid,
            zone_top: 492.0,
            zone_h: 24.0,
            progress,
            ..Default::default()
        }
    }
    fn open5(mid: f32) -> FishingSample {
        open(mid, 0.5)
    }

    #[test]
    fn bot_casts_once_then_waits_for_bite() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        assert_eq!(b.step(&closed(true), true, 0.0, &t), Action::Cast);
        assert_eq!(b.step(&closed(true), true, 1.0, &t), Action::None); // clicking again would reel in
        assert_eq!(b.step(&closed(true), true, 44.0, &t), Action::None);
        assert_eq!(b.step(&closed(true), true, 45.1, &t), Action::Cast); // no bite: recast
    }

    #[test]
    fn bot_needs_rod_and_focus() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        assert_eq!(b.step(&closed(false), true, 0.0, &t), Action::None);
        assert_eq!(b.step(&closed(true), false, 0.0, &t), Action::None);
        assert_eq!(b.step(&FishingSample::default(), true, 0.0, &t), Action::None); // rod unknown
    }

    #[test]
    fn bot_hits_in_zone_with_cooldown() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        assert_eq!(b.step(&open5(450.0), true, 0.0, &t), Action::None); // outside
        let mut c = FishBot::default();
        assert_eq!(c.step(&open5(504.0), true, 0.0, &t), Action::Hit); // first sample: velocity 0, inside
        assert_eq!(c.step(&open5(505.0), true, 0.5, &t), Action::None); // cooldown
        assert_eq!(c.step(&open5(505.0), true, 1.0, &t), Action::Hit);
        assert_eq!(c.hits(), 2);
    }

    #[test]
    fn bot_waits_for_centre_on_a_fast_bar() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        assert_eq!(b.step(&open5(470.0), true, 0.000, &t), Action::None);
        assert_eq!(b.step(&open5(478.0), true, 0.008, &t), Action::None); // 1000 px/s, registers at 490: outside
        assert_eq!(b.step(&open5(486.0), true, 0.016, &t), Action::None); // registers at 498: before centre
        assert_eq!(b.step(&open5(494.0), true, 0.024, &t), Action::Hit); // registers at 506
    }

    #[test]
    fn bot_counts_catch_and_recasts_after_delay() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        b.step(&open(300.0, 0.83), true, 0.0, &t);
        assert_eq!(b.step(&closed(true), true, 0.1, &t), Action::None); // just closed: recast delay
        assert_eq!(b.catches(), 1);
        assert_eq!(b.step(&closed(true), true, 2.2, &t), Action::Cast);
        b.step(&open(300.0, 0.1), true, 20.0, &t);
        b.step(&closed(true), true, 20.1, &t);
        assert_eq!(b.losses(), 1);
    }

    #[test]
    fn bot_restart_casts_immediately() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        assert_eq!(b.step(&closed(true), true, 0.0, &t), Action::Cast);
        b.restart();
        assert_eq!(b.step(&closed(true), true, 1.0, &t), Action::Cast);
    }

    fn no_rod_on_key_3() -> FishingSample {
        FishingSample {
            rod_key: 3,
            ..closed(false)
        }
    }

    #[test]
    fn bot_equips_rod_then_casts() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        let no_rod = no_rod_on_key_3();
        assert_eq!(b.step(&no_rod, true, 0.0, &t), Action::Equip);
        assert_eq!(b.step(&no_rod, true, 1.0, &t), Action::None); // equip not visible yet: pressing again would unequip
        assert_eq!(b.step(&no_rod, true, 1.6, &t), Action::Equip); // retry
        assert_eq!(b.step(&closed(true), true, 1.8, &t), Action::None); // let the equip settle
        assert_eq!(b.step(&closed(true), true, 2.5, &t), Action::Cast);
    }

    #[test]
    fn bot_no_equip_without_hotbar_rod_or_focus() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        assert_eq!(b.step(&closed(false), true, 0.0, &t), Action::None); // rod_key -1
        assert_eq!(b.step(&no_rod_on_key_3(), false, 0.0, &t), Action::None);
    }

    #[test]
    fn bot_gives_up_equipping_after_three_tries() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        let no_rod = no_rod_on_key_3();
        assert_eq!(b.step(&no_rod, true, 0.0, &t), Action::Equip);
        assert_eq!(b.step(&no_rod, true, 1.6, &t), Action::Equip);
        assert_eq!(b.step(&no_rod, true, 3.2, &t), Action::Equip);
        assert_eq!(b.step(&no_rod, true, 4.8, &t), Action::None);
        assert!(b.equip_gave_up());
        b.restart();
        assert_eq!(b.step(&no_rod, true, 5.0, &t), Action::Equip);
    }

    fn line(out: bool) -> FishingSample {
        FishingSample {
            line_out: Some(out),
            ..closed(true)
        }
    }

    #[test]
    fn bot_retries_a_cast_that_did_not_take() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        assert_eq!(b.step(&line(false), true, 0.0, &t), Action::Cast);
        assert_eq!(b.step(&line(false), true, 0.5, &t), Action::None); // bobber not out yet
        assert_eq!(b.step(&line(false), true, 0.9, &t), Action::Cast); // no bobber after 0.8 s: retry at once
        assert_eq!(b.step(&line(true), true, 1.2, &t), Action::None); // taken: wait for the bite
        assert_eq!(b.step(&line(true), true, 30.0, &t), Action::None);
    }

    #[test]
    fn bot_backs_off_when_casts_keep_failing() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        let mut now = 0.0;
        for _ in 0..=FishBot::FAST_RETRIES {
            assert_eq!(b.step(&line(false), true, now, &t), Action::Cast);
            now += 0.9;
        }
        assert_eq!(b.step(&line(false), true, now, &t), Action::None); // 4th failure: slow retry
        assert_eq!(
            b.step(&line(false), true, now + FishBot::SLOW_RETRY_S + 0.01, &t),
            Action::Cast
        );
    }

    #[test]
    fn bot_reels_in_after_bite_wait_counted_from_line_out() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        assert_eq!(b.step(&line(true), true, 100.0, &t), Action::None); // line already out when switched on: keep it
        assert_eq!(b.step(&line(true), true, 144.0, &t), Action::None);
        assert_eq!(b.step(&line(true), true, 145.1, &t), Action::Cast); // no bite for 45 s: reel in
    }

    #[test]
    fn bot_waits_for_lingering_bobber_after_catch() {
        let mut b = FishBot::default();
        let t = BotTimings::default();
        b.step(&open(300.0, 0.9), true, 0.0, &t);
        assert_eq!(b.step(&line(true), true, 0.1, &t), Action::None); // catch: bobber still there
        assert_eq!(b.step(&line(false), true, 0.7, &t), Action::None); // gone, recast delay not over
        assert_eq!(b.step(&line(false), true, 1.31, &t), Action::Cast);
    }
}
