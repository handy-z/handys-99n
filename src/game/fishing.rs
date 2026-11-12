use std::sync::Arc;
use std::time::{Duration, Instant};

use handy_sdk::app::updater_thread::Session;
use handy_sdk::roblox::instance::{child_by_class, child_by_name, children, data_model, instance_class, instance_name};
use handy_sdk::roblox::memory::Memory;
use handy_sdk::roblox::offsets::Offsets;
use handy_sdk::roblox::read::{
    VisibleProbe, camera_subject, current_camera, gui_absolute_rect, gui_size, gui_text, local_player, parent_of,
    part_position, player_gui, primary_part, ptr_at, workspace_of,
};

/// One read of PlayerGui/Interface/FishingCatchFrame (screen pixels) and the held item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FishingSample {
    /// FishingCatchFrame.Visible
    pub open: bool,
    /// None = not read (only read while closed) or no character
    pub rod: Option<bool>,
    pub bar_mid: f32,
    pub zone_top: f32,
    pub zone_h: f32,
    /// SuccessBar/Bar Size.Y.Scale, 0..1 (catch at 1)
    pub progress: f32,
    /// hotbar digit of the rod, read with `rod`; -1 = none
    pub rod_key: i32,
    /// our bobber is in the water; read with `rod`
    pub line_out: Option<bool>,
    /// the held item is a sack; read with `rod`
    pub sack: bool,
    /// the cursor is over a Workspace/Items model; read with `rod`
    pub hover_item: bool,
}

impl Default for FishingSample {
    fn default() -> Self {
        Self {
            open: false,
            rod: None,
            bar_mid: 0.0,
            zone_top: 0.0,
            zone_h: 0.0,
            progress: 0.0,
            rod_key: -1,
            line_out: None,
            sack: false,
            hover_item: false,
        }
    }
}

/// (top, height) of a GUI object's absolute rect; None when unreadable, not finite or negative height.
fn y_span(m: &dyn Memory, o: &Offsets, inst: usize) -> Option<(f32, f32)> {
    let r = gui_absolute_rect(m, o, inst)?;
    let (y, h) = (r.pos.y, r.size.y);
    (y.is_finite() && h.is_finite() && h >= 0.0).then_some((y, h))
}

/// The held item is a "ToolHandle" Model in the character (no Tool instance); a rod has Rod and Bobber parts.
pub fn is_rod_handle(m: &dyn Memory, o: &Offsets, th: usize) -> bool {
    th != 0 && child_by_name(m, o, th, "Rod") != 0 && child_by_name(m, o, th, "Bobber") != 0
}

/// A held sack's ToolHandle has a "Sack" part (seen with the Giant Sack).
pub fn is_sack_handle(m: &dyn Memory, o: &Offsets, th: usize) -> bool {
    th != 0 && child_by_name(m, o, th, "Sack") != 0
}

/// Workspace/Highlights/InteractionHighlight outlines the model under the cursor; true if that is a Workspace/Items
/// model. Highlight.Adornee is not in the offsets dump: ADORNEE_OFFSET was found on version-5a21b87bbc2641cf.
pub fn hovering_item(m: &dyn Memory, o: &Offsets, ws: usize) -> bool {
    let hl = child_by_name(m, o, ws, "Highlights");
    let ih = if hl != 0 {
        child_by_name(m, o, hl, "InteractionHighlight")
    } else {
        0
    };
    let items = child_by_name(m, o, ws, "Items");
    ih != 0 && items != 0 && parent_of(m, o, ptr_at(m, ih + ADORNEE_OFFSET)) == items
}
pub const ADORNEE_OFFSET: usize = 0xA0;

/// Interface/Hotbar holds one "Button<item name>" per slot with a "Number" label: the digit key for the first
/// visible button whose item name ends in "Rod" (Old Rod, ...), or -1.
pub fn rod_hotbar_key(m: &dyn Memory, o: &Offsets, vis: &VisibleProbe, hotbar: usize) -> i32 {
    if hotbar == 0 {
        return -1;
    }
    for b in children(m, o, hotbar) {
        let Some(n) = instance_name(m, o, b) else { continue };
        if n.len() < 9 || !n.starts_with("Button") || !n.ends_with("Rod") {
            continue;
        }
        if !vis.read(m, o, b).unwrap_or(false) {
            continue;
        }
        if let Some(t) = gui_text(m, o, child_by_name(m, o, b, "Number"))
            && t.len() == 1
            && t.as_bytes()[0].is_ascii_digit()
        {
            return i32::from(t.as_bytes()[0] - b'0');
        }
    }
    -1
}

/// A cast puts a "Bobber" Model in Workspace/Particles (shared by all players): ours is the one within
/// LINE_OUT_STUDS of our character's root part.
pub fn line_out(m: &dyn Memory, o: &Offsets, ws: usize, ch: usize) -> bool {
    let particles = child_by_name(m, o, ws, "Particles");
    let me = if particles != 0 {
        part_position(m, o, primary_part(m, o, ch))
    } else {
        None
    };
    let Some(me) = me else { return false };
    for c in children(m, o, particles) {
        if instance_name(m, o, c).unwrap_or_default() != "Bobber" {
            continue;
        }
        let Some(b) = part_position(m, o, primary_part(m, o, c)) else {
            continue;
        };
        let (dx, dy, dz) = (b.x - me.x, b.y - me.y, b.z - me.z);
        if dx * dx + dy * dy + dz * dz <= LINE_OUT_STUDS * LINE_OUT_STUDS {
            return true;
        }
    }
    false
}
/// the bobber lands ~5-10 studs out
pub const LINE_OUT_STUDS: f32 = 60.0;

/// Caches the minigame instances; re-resolves when they are re-parented or the DataModel changes. One thread only.
pub struct FishingReader {
    s: Arc<Session>,
    dm: usize,
    interface: usize,
    frame: usize,
    timing: usize,
    bar: usize,
    zone: usize,
    success: usize,
    progress: usize,
    vis: VisibleProbe,
    last_resolve: Option<Instant>,
}

impl FishingReader {
    pub fn new(s: Arc<Session>) -> Self {
        Self {
            s,
            dm: 0,
            interface: 0,
            frame: 0,
            timing: 0,
            bar: 0,
            zone: 0,
            success: 0,
            progress: 0,
            vis: VisibleProbe::default(),
            last_resolve: None,
        }
    }

    fn resolve(&mut self) -> bool {
        let (p, o) = (&self.s.proc, &self.s.offsets);
        (
            self.interface,
            self.frame,
            self.timing,
            self.bar,
            self.zone,
            self.success,
            self.progress,
        ) = (0, 0, 0, 0, 0, 0, 0);
        self.dm = data_model(p, o);
        if self.dm == 0 {
            return false;
        }
        let players = child_by_class(p, o, self.dm, "Players");
        let gui = player_gui(p, o, local_player(p, o, players));
        if gui == 0 {
            return false;
        }
        let itf = child_by_name(p, o, gui, "Interface");
        let frame = if itf != 0 {
            child_by_name(p, o, itf, "FishingCatchFrame")
        } else {
            0
        };
        let timing = if frame != 0 {
            child_by_name(p, o, frame, "TimingBar")
        } else {
            0
        };
        let success = if frame != 0 {
            child_by_name(p, o, frame, "SuccessBar")
        } else {
            0
        };
        if timing == 0 || success == 0 {
            return false;
        }
        self.bar = child_by_name(p, o, timing, "Bar");
        self.zone = child_by_name(p, o, timing, "SuccessArea");
        self.progress = child_by_name(p, o, success, "Bar");
        if self.bar == 0 || self.zone == 0 || self.progress == 0 {
            return false;
        }
        (self.interface, self.frame, self.timing, self.success) = (itf, frame, timing, success);
        true
    }

    /// Cached addresses can dangle after a respawn/UI rebuild: each must still sit under its parent.
    fn cache_valid(&self) -> bool {
        let (p, o) = (&self.s.proc, &self.s.offsets);
        self.frame != 0
            && data_model(p, o) == self.dm
            && parent_of(p, o, self.bar) == self.timing
            && parent_of(p, o, self.zone) == self.timing
            && parent_of(p, o, self.timing) == self.frame
            && parent_of(p, o, self.progress) == self.success
    }

    /// 0 = no Humanoid camera subject
    fn character(&self) -> usize {
        let (p, o) = (&self.s.proc, &self.s.offsets);
        let hum = camera_subject(p, o, current_camera(p, o, self.dm));
        if hum == 0 || instance_class(p, o, hum).as_deref() != Some("Humanoid") {
            return 0;
        }
        parent_of(p, o, hum)
    }

    pub fn sample(&mut self, want_rod: bool) -> Option<FishingSample> {
        if !self.cache_valid() {
            let now = Instant::now();
            if self.last_resolve.is_some_and(|t| now - t < Duration::from_millis(500)) {
                return None; // lobby / loading: retry twice a second
            }
            self.last_resolve = Some(now);
            if !self.resolve() {
                return None;
            }
        }
        let (p, o) = (&self.s.proc, &self.s.offsets);
        // the hotbar is always shown and the catch frame hidden between catches: that tells a wrong dumped Visible offset
        let hotbar = if self.interface != 0 {
            child_by_name(p, o, self.interface, "Hotbar")
        } else {
            0
        };
        self.vis.calibrate(p, o, &[hotbar], &[self.frame], &[]);
        let mut s = FishingSample {
            open: self.vis.read(p, o, self.frame)?,
            ..Default::default()
        };
        let (by, bh) = y_span(p, o, self.bar)?;
        (s.zone_top, s.zone_h) = y_span(p, o, self.zone)?;
        s.bar_mid = by + bh / 2.0;
        s.progress = gui_size(p, o, self.progress).unwrap_or_default().y_scale;
        let ch = if want_rod { self.character() } else { 0 };
        if ch != 0 {
            let (th, ws) = (child_by_name(p, o, ch, "ToolHandle"), workspace_of(p, o, self.dm));
            let rod = is_rod_handle(p, o, th);
            s.rod = Some(rod);
            if rod {
                s.line_out = Some(line_out(p, o, ws, ch));
            }
            s.sack = is_sack_handle(p, o, th);
            s.hover_item = hovering_item(p, o, ws);
        }
        if s.rod.is_some() {
            s.rod_key = rod_hotbar_key(p, o, &self.vis, hotbar);
        }
        Some(s)
    }
}
