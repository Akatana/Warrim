//! Lockpicking in the manner of Skyrim: the pick is turned with the mouse to find the lock's hidden sweet spot, then the
//! lock is turned (key or mouse button held). The nearer the pick is to the sweet spot, the further the lock turns; at
//! the end of its turn short of the full one the pick strains and, held there, breaks. Pure state and rules (the drawing is
//! below, in quads), so that it can be tested without a window.

use crate::renderer::Gpu;
use crate::ui::{Quad, Ui, FULL_UV, WHITE};
use serde::Deserialize;

/// How hard a lock is (the original's `Lockpicking` skill levels).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
pub enum Level {
    Novice,
    Apprentice,
    Adept,
    Expert,
    Master,
}

impl Level {
    pub fn name(self) -> &'static str {
        ["Novice", "Apprentice", "Adept", "Expert", "Master"][self as usize]
    }

    /// Degrees either side of the sweet spot where the lock turns all the way, and degrees from it where it will not turn.
    fn window(self) -> (f32, f32) {
        [(11.0, 62.0), (8.0, 52.0), (5.5, 42.0), (3.5, 33.0), (2.0, 26.0)][self as usize]
    }

    /// Seconds a pick holds when the lock cannot turn at all (it breaks sooner the further the lock is from turning).
    fn strength(self) -> f32 {
        [4.0, 3.4, 2.9, 2.5, 2.1][self as usize]
    }
}

/// What happened in a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The pick broke (and a new one, if there is one, goes in).
    Broke,
    /// The lock turned all the way.
    Opened,
}

/// The pick turns this far to either side of straight at most, degrees.
const REACH: f32 = 90.0;
/// Seconds the lock takes to turn all the way, and to fall back when let go.
const TURN_SECONDS: f32 = 0.8;
const FALL_SECONDS: f32 = 0.25;
/// Degrees of pick turn for a pixel the mouse moves.
const DEGREES_PER_PIXEL: f32 = 0.22;

pub struct Lockpick {
    pub level: Level,
    /// Where the sweet spot is, degrees from straight up (hidden from the player).
    sweet: f32,
    /// The pick's angle in degrees: 0 points straight at the lock from below, positive turns it to the left.
    pub angle: f32,
    /// How far the lock is turned, 0 to 1.
    pub turn: f32,
    /// How much the pick has strained, in seconds.
    pub strain: f32,
    /// Whether the lock is being held against its stop this moment (the pick shudders).
    pub straining: bool,
    /// What the skill in lockpicking makes of the sweet spot's width (1 for none).
    pub window_factor: f32,
}

impl Lockpick {
    /// A lock of this level with its sweet spot at `sweet` degrees (random when the game starts it).
    pub fn new(level: Level, sweet: f32) -> Lockpick {
        Lockpick { level, sweet: sweet.clamp(-REACH + 10.0, REACH - 10.0), angle: 0.0, turn: 0.0, strain: 0.0, straining: false, window_factor: 1.0 }
    }

    /// How far the lock can turn with the pick where it is: fully in the sweet spot, less and less away from it, not at all
    /// beyond the reach of the lock.
    pub fn limit(&self) -> f32 {
        let (window, reach) = self.level.window();
        let window = (window * self.window_factor).min(reach - 1.0);
        let off = (self.angle - self.sweet).abs();
        1.0 - ((off - window) / (reach - window)).clamp(0.0, 1.0)
    }

    /// The mouse moved `dx` pixels to the right: the pick turns that way.
    pub fn rotate(&mut self, dx: f32) {
        if self.turn == 0.0 {
            // The pick cannot be moved once the lock has begun to turn (as in the original game).
            self.angle = (self.angle - dx * DEGREES_PER_PIXEL).clamp(-REACH, REACH);
        }
    }

    /// Advance `dt` seconds with the lock turned (`turning`) or let go.
    pub fn update(&mut self, dt: f32, turning: bool) -> Option<Event> {
        self.straining = false;
        if !turning {
            self.turn = (self.turn - dt / FALL_SECONDS).max(0.0);
            self.strain = (self.strain - 2.0 * dt).max(0.0);
            return None;
        }
        let limit = self.limit();
        self.turn = (self.turn + dt / TURN_SECONDS).min(limit);
        if limit >= 1.0 && self.turn >= 1.0 {
            return Some(Event::Opened);
        }
        if self.turn >= limit {
            // Against the stop: the pick strains faster the less the lock can turn.
            self.straining = true;
            self.strain += dt * (1.0 + (1.0 - limit) * 2.0);
            if self.strain >= self.level.strength() {
                (self.turn, self.strain, self.straining) = (0.0, 0.0, false);
                return Some(Event::Broke);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Drawing, with the original's own graphics.

/// The pictures of the lock: the keyhole plate of a door, two ring pictures of the spells, and the icon of the Pick Lock spell.
const KEYHOLE: &str = r"World\Goober\G_UldamanMapKeyHole.blp";
const BEZEL: &str = r"Spells\DoubleRing128.blp";
const RUNES: &str = r"Spells\AuraRune256.blp";
pub const PICK_ICON: &str = r"Interface\Icons\Spell_Nature_MoonKey";

pub struct LockArt {
    keyhole: usize,
    bezel: usize,
    runes: usize,
}

impl LockArt {
    pub fn load(mount: &mpq::Mount, ui: &mut Ui, gpu: &Gpu) -> anyhow::Result<LockArt> {
        let mut texture = |path: &str| -> anyhow::Result<usize> {
            let blp = wowfmt::blp::Blp::parse(&mount.read(path)?)?;
            Ok(ui.add_texture(gpu, blp.width, blp.height, &blp.mips[0]))
        };
        Ok(LockArt { keyhole: texture(KEYHOLE)?, bezel: texture(BEZEL)?, runes: texture(RUNES)? })
    }
}

impl Lockpick {
    /// Queue the lock for drawing on a `size` screen: the picture dimmed, the runed bezel, the keyhole plate turning with the lock,
    /// and the pick (shuddering when strained; `moment` is any number that changes with time).
    pub fn draw(&self, ui: &mut Ui, art: &LockArt, size: (u32, u32), moment: f32) {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let unit = h / 720.0;
        let r = h * 0.2;
        let quad = |ui: &mut Ui, texture: usize, rect: [f32; 4], color: [f32; 4], additive: bool| ui.quad(Quad { texture, rect, uv: FULL_UV, color, additive });
        quad(ui, WHITE, [0.0, 0.0, w, h], [0.0, 0.0, 0.0, 0.55], false);
        let square = |scale: f32| [cx - r * scale, cy - r * scale, 2.0 * r * scale, 2.0 * r * scale];
        // The plate is round and turns a quarter as the lock does.
        ui.set_round(true);
        ui.set_turn(Some((self.turn * 90f32.to_radians(), cx, cy)));
        quad(ui, art.keyhole, square(1.0), [0.82, 0.72, 0.52, 1.0], false);
        ui.set_turn(None);
        ui.set_round(false);
        // The rings round it stand still, and glow.
        quad(ui, art.bezel, square(1.12), [1.0, 0.85, 0.55, 1.0], true);
        quad(ui, art.runes, square(1.55), [1.0, 0.8, 0.45, 0.85], true);
        // The pick comes from below to the middle of the lock, a bright core on a dark edge; a strained one shakes.
        let shake = if self.straining { (moment * 70.0).sin() * 2.0 * unit } else { 0.0 };
        let (px, py) = (cx + shake, cy + shake * 0.5);
        let core = if self.straining { [1.0, 0.6, 0.4, 1.0] } else { [0.88, 0.9, 0.94, 1.0] };
        ui.set_turn(Some((self.angle.to_radians(), px, py)));
        quad(ui, WHITE, [px - 4.0 * unit, py, 8.0 * unit, r * 1.5], [0.08, 0.07, 0.06, 1.0], false);
        quad(ui, WHITE, [px - 2.0 * unit, py, 4.0 * unit, r * 1.5], core, false);
        ui.set_turn(None);
        // The strain, as a bar under the lock.
        let (bx, by, bw) = (cx - r, cy + r * 1.5 + 40.0 * unit, 2.0 * r);
        quad(ui, WHITE, [bx, by, bw, 6.0 * unit], [0.0, 0.0, 0.0, 0.7], false);
        let share = (self.strain / self.level.strength()).clamp(0.0, 1.0);
        quad(ui, WHITE, [bx, by, bw * share, 6.0 * unit], [0.95, 0.45 + 0.3 * (1.0 - share), 0.2, 1.0], false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pictures_of_the_lock_are_in_the_game_files() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        for path in [KEYHOLE, BEZEL, RUNES] {
            assert!(wowfmt::blp::Blp::parse(&mount.read(path).unwrap()).is_ok(), "{path}");
        }
        assert!(mount.read(&format!("{PICK_ICON}.blp")).is_ok());
    }

    /// Hold the lock for `seconds`, or until something happens, and return what.
    fn hold(l: &mut Lockpick, seconds: f32) -> Option<Event> {
        (0..(seconds * 100.0) as usize).find_map(|_| l.update(0.01, true))
    }

    #[test]
    fn the_lock_turns_all_the_way_with_the_pick_in_the_sweet_spot() {
        let mut l = Lockpick::new(Level::Master, 20.0);
        l.angle = 20.0 + 1.5;
        assert_eq!(l.limit(), 1.0);
        assert_eq!(hold(&mut l, 1.2), Some(Event::Opened));
    }

    #[test]
    fn the_further_from_the_sweet_spot_the_less_the_lock_turns_and_beyond_the_reach_not_at_all() {
        let mut l = Lockpick::new(Level::Novice, 0.0);
        let limits: Vec<f32> = [0.0, 11.0, 30.0, 50.0, 62.0, 80.0].iter().map(|&a| { l.angle = a; l.limit() }).collect();
        assert_eq!(limits[0], 1.0);
        assert_eq!(limits[1], 1.0);
        assert!(limits[2] < 1.0 && limits[2] > limits[3] && limits[3] > 0.0, "{limits:?}");
        assert_eq!((limits[4], limits[5]), (0.0, 0.0));
        // Harder locks forgive less at the same distance.
        let at = |level| { let mut l = Lockpick::new(level, 0.0); l.angle = 20.0; l.limit() };
        assert!(at(Level::Novice) > at(Level::Adept) && at(Level::Adept) > at(Level::Master));
    }

    #[test]
    fn held_against_the_stop_the_pick_strains_and_breaks_and_a_new_one_starts_fresh() {
        let mut l = Lockpick::new(Level::Novice, 0.0);
        l.angle = 40.0;
        let limit = l.limit();
        assert!(limit > 0.0 && limit < 1.0);
        // The lock turns to its stop and no further, shuddering.
        hold(&mut l, 0.5);
        assert!((l.turn - limit).abs() < 1e-4 && l.straining);
        assert_eq!(hold(&mut l, 5.0), Some(Event::Broke));
        assert_eq!((l.turn, l.strain), (0.0, 0.0));
        // Letting go in time relieves the strain, and the lock falls back.
        l.angle = 40.0;
        hold(&mut l, 1.0);
        let strained = l.strain;
        assert!(strained > 0.0);
        for _ in 0..100 {
            l.update(0.01, false);
        }
        assert!(l.strain < strained && l.turn == 0.0);
    }

    #[test]
    fn the_pick_is_moved_with_the_mouse_within_its_reach_and_not_once_the_lock_is_turning() {
        let mut l = Lockpick::new(Level::Adept, 10.0);
        l.rotate(100.0);
        assert!((l.angle + 22.0).abs() < 1e-3, "moving right turns the pick to the right: {}", l.angle);
        l.rotate(-100_000.0);
        assert_eq!(l.angle, REACH);
        l.angle = 10.0;
        l.update(0.1, true);
        assert!(l.turn > 0.0);
        let at = l.angle;
        l.rotate(50.0);
        assert_eq!(l.angle, at);
    }
}
