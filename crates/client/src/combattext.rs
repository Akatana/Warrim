//! Floating combat text, after the original's `Blizzard_CombatText` addon (and `CombatFeedback`): text in
//! `CombatTextFont` (FRIZQT__.TTF at height 25, drop shadow) scrolls straight up for 1.9 seconds, fading out
//! over the last 0.6, with the original's constants. What happens to the player ("-8" in red, "Entering Combat",
//! "Leaving Combat", "Health Low") scrolls up from the middle of the screen, stacked so lines do not overlap and
//! staggered a little to the sides like the original's damage; the player's own hits show their damage in white
//! (a physical `CombatFeedback` number) centred on the enemy they hit.

use crate::camera::Camera;
use crate::renderer::Gpu;
use crate::ui::{Quad, Ui, FULL_UV};
use anyhow::{Context, Result};
use glam::Vec3;
use mpq::Mount;
use std::collections::HashMap;

const FONT: &str = r"Fonts\FRIZQT__.TTF";
/// Glyphs are rendered at this multiple of their size, so they stay sharp when the interface is scaled up.
const SUPERSAMPLE: f32 = 2.0;

// Blizzard_CombatText.lua
const HEIGHT: f32 = 25.0;
const SCROLL_TIME: f32 = 1.9;
const FADE_START: f32 = 1.3;
const STAGGER_RANGE: i32 = 20;
const SPACING: f32 = 10.0;
const MAX_OFFSET: f32 = 130.0;
const MAX_LINES: usize = 20;
/// Where messages start (the middle of the screen, from its bottom) and where they scroll to (`COMBAT_TEXT_FLOAT_MODE` 1).
const START_Y: f32 = 384.0;
const END_Y: f32 = 609.0;
/// The line height `CombatText_AddMessage` uses to stack messages.
const LINE: f32 = 16.0;

/// The red of damage and warnings, and the white of the player's own hits.
pub const RED: [f32; 3] = [1.0, 0.1, 0.1];
pub const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
pub const GREEN: [f32; 3] = [0.1, 1.0, 0.1];

struct Line {
    text: String,
    color: [f32; 3],
    /// Seconds scrolled.
    scroll: f32,
    /// Horizontal offset from the middle (the stagger), top edge now and when it started, in UI units from the bottom.
    x: f32,
    y: f32,
    start_y: f32,
    /// Centred on this world position (render space) instead of the screen's middle.
    at: Option<Vec3>,
}

/// The lines on screen and how they scroll: the logic of `CombatText_AddMessage` and `CombatText_OnUpdate`.
#[derive(Default)]
struct Scroller {
    lines: Vec<Line>,
    rng: u32,
}

impl Scroller {
    fn random(&mut self, below: i32) -> i32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as i32 % below
    }

    /// A message from the middle of the screen. `staggered` offsets it randomly to either side.
    fn add_screen(&mut self, text: String, color: [f32; 3], staggered: bool) {
        if self.lines.len() >= MAX_LINES {
            self.lines.remove(0);
        }
        // Start below the lowest message that would overlap; start over from the bottom when that is too far down.
        let own_bottom = START_Y - HEIGHT;
        let mut lowest = own_bottom;
        for l in self.lines.iter().filter(|l| l.at.is_none()) {
            if lowest >= l.y - LINE - SPACING {
                lowest = l.y - LINE - SPACING;
            }
        }
        if lowest < START_Y - MAX_OFFSET {
            lowest = own_bottom;
        }
        let x = if staggered { (self.random(STAGGER_RANGE + 1) - STAGGER_RANGE / 2) as f32 } else { 0.0 };
        self.lines.push(Line { text, color, scroll: 0.0, x, y: lowest, start_y: lowest, at: None });
    }

    /// A message that scrolls up from a place in the world.
    fn add_world(&mut self, at: Vec3, text: String, color: [f32; 3]) {
        if self.lines.len() >= MAX_LINES {
            self.lines.remove(0);
        }
        let x = (self.random(STAGGER_RANGE + 1) - STAGGER_RANGE / 2) as f32;
        self.lines.push(Line { text, color, scroll: 0.0, x, y: 0.0, start_y: 0.0, at: Some(at) });
    }

    fn update(&mut self, dt: f32) {
        for l in &mut self.lines {
            l.scroll += dt;
            // `CombatText_StandardScroll`: straight up at (end - start) / scroll time.
            l.y = l.start_y + (END_Y - START_Y) * l.scroll / SCROLL_TIME;
        }
        self.lines.retain(|l| l.scroll < SCROLL_TIME);
    }
}

fn alpha(scroll: f32) -> f32 {
    if scroll >= FADE_START { (1.0 - (scroll - FADE_START) / (SCROLL_TIME - FADE_START)).max(0.0) } else { 1.0 }
}

/// A rendered glyph: its texture and metrics in pixels at `SUPERSAMPLE` times the font height.
struct Glyph {
    texture: usize,
    size: (f32, f32),
    xmin: f32,
    ymin: f32,
    advance: f32,
}

pub struct CombatText {
    glyphs: HashMap<char, Glyph>,
    ascent: f32,
    scroller: Scroller,
}

impl CombatText {
    pub fn load(mount: &Mount, ui: &mut Ui, gpu: &Gpu) -> Result<CombatText> {
        let font = fontdue::Font::from_bytes(mount.read(FONT)?, fontdue::FontSettings::default()).map_err(|e| anyhow::anyhow!("{FONT}: {e}")).context("combat text font")?;
        let px = HEIGHT * SUPERSAMPLE;
        let ascent = font.horizontal_line_metrics(px).context("font has line metrics")?.ascent;
        let mut glyphs = HashMap::new();
        for c in (' '..='~').chain(std::iter::once('?')) {
            let (m, bitmap) = font.rasterize(c, px);
            // White with the glyph's coverage as alpha; a one-pixel clear border keeps the filtering off the edges.
            let (w, h) = (m.width.max(1) + 2, m.height.max(1) + 2);
            let mut rgba = vec![255u8; w * h * 4];
            for i in 0..w * h {
                rgba[i * 4 + 3] = 0;
            }
            for y in 0..m.height {
                for x in 0..m.width {
                    rgba[((y + 1) * w + x + 1) * 4 + 3] = bitmap[y * m.width + x];
                }
            }
            let texture = ui.add_texture(gpu, w as u32, h as u32, &rgba);
            glyphs.insert(c, Glyph { texture, size: (w as f32, h as f32), xmin: m.xmin as f32 - 1.0, ymin: m.ymin as f32 - 1.0, advance: m.advance_width });
        }
        Ok(CombatText { glyphs, ascent, scroller: Scroller { rng: 0xC0FFEE, ..Default::default() } })
    }

    /// Damage the player takes: "-8", red, staggered (`DAMAGE`).
    pub fn damage_taken(&mut self, amount: u32) {
        self.scroller.add_screen(format!("-{amount}"), RED, true);
    }

    /// Health the player is given: "+26", green, staggered (`HEAL`).
    pub fn healed(&mut self, amount: u32) {
        self.scroller.add_screen(format!("+{amount}"), GREEN, true);
    }

    /// A message of the original's combat text from the middle of the screen: "Entering Combat", "Health Low", ...
    pub fn message(&mut self, text: &str) {
        self.scroller.add_screen(text.to_string(), RED, false);
    }

    /// Damage the player's hit deals, centred on the enemy at `at`.
    pub fn damage_dealt(&mut self, at: Vec3, amount: u32) {
        self.scroller.add_world(at, amount.to_string(), WHITE);
    }

    pub fn update(&mut self, dt: f32) {
        self.scroller.update(dt);
    }

    fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c).or_else(|| self.glyphs.get(&'?'))
    }

    /// Width of a text in UI units.
    fn width(&self, text: &str) -> f32 {
        text.chars().filter_map(|c| self.glyph(c)).map(|g| g.advance).sum::<f32>() / SUPERSAMPLE
    }

    pub fn draw(&self, ui: &mut Ui, camera: &Camera, size: (u32, u32)) {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let s = h / crate::uixml::ui_height();
        let view_proj = camera.view_proj_relative();
        for line in &self.scroller.lines {
            // The middle of the text, in pixels: the screen's middle column, or the enemy's projected position.
            let (cx, top) = match line.at {
                None => (w / 2.0 + line.x * s, h - line.y * s),
                Some(at) => {
                    let clip = view_proj * (at - camera.pos).extend(1.0);
                    if clip.w <= 0.1 {
                        continue;
                    }
                    let (x, y) = ((clip.x / clip.w * 0.5 + 0.5) * w, (0.5 - clip.y / clip.w * 0.5) * h);
                    (x + line.x * s, y - (HEIGHT / 2.0 + (END_Y - START_Y) * line.scroll / SCROLL_TIME) * s)
                }
            };
            let a = alpha(line.scroll);
            let unit = s / SUPERSAMPLE;
            // The drop shadow of the original's fonts: black, one unit right and down.
            for (shadow, color) in [(true, [0.0, 0.0, 0.0, a]), (false, [line.color[0], line.color[1], line.color[2], a])] {
                let mut pen = cx - self.width(&line.text) * s / 2.0 + if shadow { s } else { 0.0 };
                let baseline = top + self.ascent * unit + if shadow { s } else { 0.0 };
                for c in line.text.chars() {
                    let Some(g) = self.glyph(c) else { continue };
                    let (gx, gy) = (pen + g.xmin * unit, baseline - (g.ymin + g.size.1) * unit);
                    ui.quad(Quad { texture: g.texture, rect: [gx, gy, g.size.0 * unit, g.size.1 * unit], uv: FULL_UV, color, additive: false });
                    pen += g.advance * unit;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scroller() -> Scroller {
        Scroller { rng: 7, ..Default::default() }
    }

    #[test]
    fn messages_scroll_up_linearly_and_fade_over_the_last_part() {
        let mut s = scroller();
        s.add_screen("Entering Combat".into(), RED, false);
        assert_eq!((s.lines[0].y, s.lines[0].x), (START_Y - HEIGHT, 0.0));
        s.update(0.95);
        // Halfway through the scroll: half of the 225 units up.
        assert!((s.lines[0].y - (START_Y - HEIGHT + 112.5)).abs() < 1e-3);
        assert_eq!(alpha(0.95), 1.0);
        assert!((alpha(1.6) - 0.5).abs() < 1e-3 && alpha(1.9) == 0.0);
        s.update(1.0);
        assert!(s.lines.is_empty(), "gone after 1.9 seconds");
    }

    #[test]
    fn simultaneous_messages_stack_downwards_and_start_over_when_too_far() {
        let mut s = scroller();
        for _ in 0..3 {
            s.add_screen("x".into(), RED, false);
        }
        // Each new one starts 16 + 10 units below the lowest it would overlap.
        let ys: Vec<f32> = s.lines.iter().map(|l| l.y).collect();
        assert_eq!(ys, [359.0, 333.0, 307.0]);
        // A message a second later finds the first ones far above and starts at the top again.
        s.update(1.0);
        s.add_screen("y".into(), RED, false);
        assert_eq!(s.lines[3].y, 359.0);
        // Piled up past 130 units below the start, it begins again at the top.
        let mut s = scroller();
        (0..6).for_each(|_| s.add_screen("x".into(), RED, false));
        assert_eq!(s.lines[5].y, 359.0);
    }

    #[test]
    fn damage_is_staggered_within_ten_units_either_side() {
        let mut s = scroller();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            s.add_screen("-8".into(), RED, true);
            seen.insert(s.lines[0].x as i32);
            s.lines.clear();
        }
        assert!(seen.iter().all(|x| (-10..=10).contains(x)) && seen.len() > 10, "{seen:?}");
    }

    #[test]
    fn at_most_twenty_lines_the_oldest_makes_room() {
        let mut s = scroller();
        for i in 0..25 {
            s.add_world(Vec3::ZERO, i.to_string(), WHITE);
        }
        assert_eq!(s.lines.len(), 20);
        assert_eq!(s.lines[0].text, "5");
    }
}
