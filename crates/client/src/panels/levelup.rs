//! The choice a level brings: each level gained is a point to put into health, mana or stamina. While one is unspent a glowing "+" hint hangs over the
//! bars (a click or the key U opens the panel); the panel is drawn here with generated textures (rounded corners, soft glows) in a modern style, apart from
//! the original's windows, which stay as they were.

use super::*;
use crate::font::GlyphSet;
use crate::ui::WHITE as SOLID;
use std::time::Instant;

/// What a point adds to the maximum of health, mana and stamina.
pub const GAIN: [f32; 3] = [20.0, 20.0, 10.0];
const NAMES: [&str; 3] = ["Health", "Mana", "Stamina"];
/// The attributes a point also adds (strength, agility, stamina, intellect, spirit; "wisdom" is spirit): health gives strength and stamina, mana stamina,
/// intellect and spirit, stamina stamina and agility.
const ATTRIBUTES: [[i32; 5]; 3] = [[1, 0, 1, 0, 0], [0, 0, 1, 1, 1], [0, 1, 1, 0, 0]];
/// The colours of the bars of the same names on the action bar.
const ACCENTS: [[f32; 4]; 3] = [crate::actionbar::HEALTH_COLOR, crate::actionbar::MANA_COLOR, crate::actionbar::STAMINA_COLOR];
/// The game's gold (`GameFontNormal`), the colour of the hint and of the panel's own touches.
pub(super) const ACCENT: [f32; 4] = [1.0, 0.82, 0.0, 1.0];
pub(super) const INK: [f32; 4] = [0.09, 0.08, 0.05, 1.0];
pub(super) const GREY: [f32; 4] = [0.6, 0.6, 0.6, 1.0];
const FONT: &str = r"Fonts\ARIALN.TTF";

/// Size of the panel and of its rows, and of the hint, in interface units.
const PANEL: (f32, f32) = (352.0, 328.0);
const ROW_HEIGHT: f32 = 64.0;
const ROWS_TOP: f32 = 84.0;
const ROW_GAP: f32 = 10.0;
const HINT: (f32, f32) = (236.0, 36.0);
/// How far the hint's bottom is above the screen bottom (over the bars).
const HINT_BOTTOM: f32 = 84.0;
/// The corner of the rounded textures, in texels of their 64.
const CORNER: f32 = 20.0;
const SIDE: f32 = 64.0;

/// The generated pictures and the text sets, made when first needed.
pub(super) struct Art {
    pub rounded: usize,
    pub ring: usize,
    pub disc: usize,
    pub glow: usize,
    pub small: GlyphSet,
    pub body: GlyphSet,
    pub title: GlyphSet,
    pub born: Instant,
}

/// Anti-aliased coverage of a distance (in texels) from an edge, negative inside.
fn cover(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

/// A white picture whose alpha is `alpha(x, y)` for the texel centres (from the middle of the picture).
fn texture(ui: &mut Ui, gpu: &Gpu, side: u32, alpha: impl Fn(f32, f32) -> f32) -> usize {
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let a = alpha(x as f32 + 0.5 - side as f32 / 2.0, y as f32 + 0.5 - side as f32 / 2.0);
            rgba.extend_from_slice(&[255, 255, 255, (a.clamp(0.0, 1.0) * 255.0).round() as u8]);
        }
    }
    ui.add_texture(gpu, side, side, &rgba)
}

/// Signed distance to the rounded square of the textures.
fn rounded_distance(x: f32, y: f32) -> f32 {
    let (qx, qy) = (x.abs() - (SIDE / 2.0 - CORNER), y.abs() - (SIDE / 2.0 - CORNER));
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - CORNER
}

impl Art {
    fn new(ui: &mut Ui, gpu: &Gpu, mount: &Mount) -> Result<Art> {
        let font = fontdue::Font::from_bytes(mount.read(FONT)?, fontdue::FontSettings::default()).map_err(|e| anyhow::anyhow!("{FONT}: {e}"))?;
        Ok(Art {
            rounded: texture(ui, gpu, SIDE as u32, |x, y| cover(rounded_distance(x, y))),
            ring: texture(ui, gpu, SIDE as u32, |x, y| cover(rounded_distance(x, y)) * cover(-rounded_distance(x, y) - 2.0)),
            disc: texture(ui, gpu, 128, |x, y| cover(x.hypot(y) - 63.5)),
            glow: texture(ui, gpu, 128, |x, y| (1.0 - x.hypot(y) / 64.0).clamp(0.0, 1.0).powf(2.2)),
            small: GlyphSet::new(ui, gpu, &font, 11.0, 0)?,
            body: GlyphSet::new(ui, gpu, &font, 13.0, 0)?,
            title: GlyphSet::new(ui, gpu, &font, 22.0, 0)?,
            born: Instant::now(),
        })
    }
}

pub(super) fn tint(color: [f32; 4], alpha: f32) -> [f32; 4] {
    [color[0], color[1], color[2], color[3] * alpha]
}

pub(super) fn quad(ui: &mut Ui, texture: usize, rect: [f32; 4], color: [f32; 4], additive: bool) {
    ui.quad(Quad { texture, rect, uv: FULL_UV, color, additive });
}

/// A rounded rectangle (or its outline) of the generated picture, cut in nine so the corners keep their shape at any size; `radius` in pixels.
pub(super) fn slice(ui: &mut Ui, texture: usize, [x, y, w, h]: [f32; 4], radius: f32, color: [f32; 4]) {
    let r = radius.min(w / 2.0).min(h / 2.0);
    let (xs, ys, c) = ([x, x + r, x + w - r, x + w], [y, y + r, y + h - r, y + h], CORNER / SIDE);
    let us = [0.0, c, 1.0 - c, 1.0];
    for j in 0..3 {
        for i in 0..3 {
            let rect = [xs[i], ys[j], xs[i + 1] - xs[i], ys[j + 1] - ys[j]];
            if rect[2] > 0.0 && rect[3] > 0.0 {
                ui.quad(Quad { texture, rect, uv: [us[i], us[j], us[i + 1], us[j + 1]], color, additive: false });
            }
        }
    }
}

/// A plus (`turn` 0) or a cross (a quarter of a turn about the same middle) of two bars, `size` pixels across.
pub(super) fn plus(ui: &mut Ui, art: &Art, [x, y, w, h]: [f32; 4], size: f32, color: [f32; 4], turn: f32) {
    let (cx, cy, t) = (x + w / 2.0, y + h / 2.0, size / 7.0);
    ui.set_turn((turn != 0.0).then_some((turn, cx, cy)));
    slice(ui, art.rounded, [cx - size / 2.0, cy - t / 2.0, size, t], t / 2.0, color);
    slice(ui, art.rounded, [cx - t / 2.0, cy - size / 2.0, t, size], t / 2.0, color);
    ui.set_turn(None);
}

/// A level is one point: the points of a character of `level` that are not spent yet.
fn unspent(level: u32, spent: [u32; 3]) -> u32 {
    (level - 1).saturating_sub(spent.iter().sum())
}

fn attributes_of(spent: [u32; 3]) -> [i32; 5] {
    let mut sum = [0; 5];
    for (n, row) in spent.iter().zip(ATTRIBUTES) {
        sum.iter_mut().zip(row).for_each(|(s, r)| *s += *n as i32 * r);
    }
    sum
}

/// 1234 as "1,234".
fn grouped(n: i32) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Where the panel's parts are on a `screen` (pixels): the panel, the three rows and the close button.
struct Layout {
    panel: [f32; 4],
    rows: [[f32; 4]; 3],
    close: [f32; 4],
}

fn layout(screen: (u32, u32)) -> Layout {
    let s = screen.1 as f32 / ui_height();
    let (x, y) = ((screen.0 as f32 - PANEL.0 * s) / 2.0, (screen.1 as f32 - PANEL.1 * s) / 2.0 - 20.0 * s);
    Layout {
        panel: [x, y, PANEL.0 * s, PANEL.1 * s],
        rows: [0, 1, 2].map(|i| [x + 16.0 * s, y + (ROWS_TOP + i as f32 * (ROW_HEIGHT + ROW_GAP)) * s, (PANEL.0 - 32.0) * s, ROW_HEIGHT * s]),
        close: [x + (PANEL.0 - 16.0 - 26.0) * s, y + 16.0 * s, 26.0 * s, 26.0 * s],
    }
}

fn hint_rect(screen: (u32, u32)) -> [f32; 4] {
    let s = screen.1 as f32 / ui_height();
    [(screen.0 as f32 - HINT.0 * s) / 2.0, screen.1 as f32 - (HINT_BOTTOM + HINT.1) * s, HINT.0 * s, HINT.1 * s]
}

pub(super) fn inside(rect: [f32; 4], (px, py): (f32, f32)) -> bool {
    px >= rect[0] && px < rect[0] + rect[2] && py >= rect[1] && py < rect[1] + rect[3]
}

/// The button a position is over: a row (0 to 2) or the close button (3).
fn target(screen: (u32, u32), pos: (f32, f32)) -> Option<usize> {
    let l = layout(screen);
    l.rows.iter().position(|r| inside(*r, pos)).or_else(|| inside(l.close, pos).then_some(3))
}

impl Panels {
    pub fn level_open(&self) -> bool {
        self.level_open
    }

    /// The key U: show or hide the panel.
    pub fn toggle_level(&mut self) {
        let on = !self.level_open;
        if on {
            self.close_all();
        }
        self.level_open = on;
        self.level_press = None;
        self.after_close();
    }

    /// Whether the hint (a button while the mouse is free) is shown at this pixel.
    pub fn level_hint_at(&self, pos: (f32, f32), screen: (u32, u32)) -> bool {
        self.points > 0 && !self.any_open() && inside(hint_rect(screen), pos)
    }

    /// A button of the panel went down.
    pub(super) fn level_press(&mut self, pos: (f32, f32), screen: (u32, u32)) {
        self.level_press = target(screen, pos);
    }

    /// The button came up: on the same button that is a click. A row spends a point on its stat, the cross closes the panel.
    pub(super) fn level_release(&mut self, pos: (f32, f32), screen: (u32, u32)) {
        let down = self.level_press.take();
        match down.filter(|d| Some(*d) == target(screen, pos)) {
            Some(3) => self.toggle_level(),
            Some(which) if self.points > 0 => {
                self.points -= 1;
                self.spent[which] += 1;
                self.stats_dirty = true;
                self.refresh();
                self.events.push(Sfx::Named("igMainMenuOptionCheckBoxOn"));
            }
            _ => {}
        }
    }

    /// A level is one point, whenever it was gained (an old save, the console): what is unspent is the levels reached less the points spent.
    pub(super) fn sync_points(&mut self) {
        self.points = unspent(self.progress.level, self.spent);
    }

    /// What the points spent add to the maximum health, mana and stamina.
    pub(super) fn spent_bonus(&self) -> [f32; 3] {
        [0, 1, 2].map(|i| self.spent[i] as f32 * GAIN[i])
    }

    /// What the points spent add to the five attributes (strength, agility, stamina, intellect, spirit), one point of each listed in `ATTRIBUTES`.
    pub(super) fn spent_attributes(&self) -> [i32; 5] {
        attributes_of(self.spent)
    }

    /// The points spent on health, mana and stamina (for the save).
    pub fn level_spent(&self) -> [u32; 3] {
        self.spent
    }

    /// The panel when it is open, else the hint while a point is unspent.
    pub fn draw_level(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32)) {
        // Another window opened: the panel gives way.
        if self.level_open && self.windows_open() {
            (self.level_open, self.level_press) = (false, None);
        }
        if !self.level_open && (self.points == 0 || self.any_open()) {
            return;
        }
        let Some(art) = self.take_art(ui, gpu) else { return };
        if self.level_open {
            self.draw_level_panel(ui, &art, screen);
        } else {
            self.draw_level_hint(ui, &art, screen);
        }
        self.level_art = Some(art);
    }

    /// The pictures and text of the custom-drawn windows (made when first needed); give it back with `self.level_art = Some(art)` when done drawing.
    pub(super) fn take_art(&mut self, ui: &mut Ui, gpu: &Gpu) -> Option<Art> {
        if self.level_art.is_none() {
            let mount = self.mount.clone();
            match Art::new(ui, gpu, &mount) {
                Ok(art) => self.level_art = Some(art),
                Err(e) => log::warn!("no custom interface art: {e:#}"),
            }
        }
        self.level_art.take()
    }

    fn draw_level_hint(&self, ui: &mut Ui, art: &Art, screen: (u32, u32)) {
        let s = screen.1 as f32 / ui_height();
        let pill = hint_rect(screen);
        let pulse = 0.5 + 0.5 * (art.born.elapsed().as_secs_f32() * 3.0).sin();
        let over = self.cursor.is_some_and(|c| inside(pill, c));
        let around = 26.0 * s;
        quad(ui, art.glow, [pill[0] - around, pill[1] - around, pill[2] + 2.0 * around, pill[3] + 2.0 * around], tint(ACCENT, 0.16 + 0.18 * pulse), true);
        slice(ui, art.rounded, pill, pill[3] / 2.0, if over { [0.13, 0.13, 0.15, 0.94] } else { [0.06, 0.06, 0.07, 0.90] });
        slice(ui, art.ring, pill, pill[3] / 2.0, tint(ACCENT, 0.55 + 0.35 * pulse));
        // The round "+" at the left, breathing with the glow.
        let d = 24.0 * s * (1.0 + 0.05 * pulse);
        let button = [pill[0] + 6.0 * s + (24.0 * s - d) / 2.0, pill[1] + (pill[3] - d) / 2.0, d, d];
        quad(ui, art.glow, [button[0] - d * 0.7, button[1] - d * 0.7, d * 2.4, d * 2.4], tint(ACCENT, 0.35 + 0.3 * pulse), true);
        quad(ui, art.disc, button, ACCENT, false);
        plus(ui, art, button, 11.0 * s, INK, 0.0);
        let points = if self.points == 1 { "1 point to spend".to_string() } else { format!("{} points to spend", self.points) };
        art.body.draw(ui, "Level up!", pill[0] + 40.0 * s, pill[1] + 3.0 * s, s, [1.0; 4]);
        art.small.draw(ui, &format!("{points} - press U"), pill[0] + 40.0 * s, pill[1] + 19.0 * s, s, GREY);
    }

    fn draw_level_panel(&self, ui: &mut Ui, art: &Art, screen: (u32, u32)) {
        let s = screen.1 as f32 / ui_height();
        let l = layout(screen);
        let [x, y, w, h] = l.panel;
        let hover = self.cursor.and_then(|c| target(screen, c));
        // The world dims behind the panel, which floats on a soft shadow.
        quad(ui, SOLID, [0.0, 0.0, screen.0 as f32, screen.1 as f32], [0.0, 0.0, 0.0, 0.38], false);
        let spread = 34.0 * s;
        quad(ui, art.glow, [x - spread, y - spread + 10.0 * s, w + 2.0 * spread, h + 2.0 * spread], [0.0, 0.0, 0.0, 0.55], false);
        slice(ui, art.rounded, l.panel, 18.0 * s, [0.07, 0.07, 0.08, 0.95]);
        slice(ui, art.ring, l.panel, 18.0 * s, [1.0, 1.0, 1.0, 0.11]);

        art.title.draw(ui, &format!("Level {}", self.progress.level), x + 24.0 * s, y + 16.0 * s, s, [1.0; 4]);
        let (text, color) = match self.points {
            0 => ("All points spent".to_string(), GREY),
            1 => ("1 point to spend".to_string(), ACCENT),
            n => (format!("{n} points to spend"), ACCENT),
        };
        art.body.draw(ui, &text, x + 24.0 * s, y + 49.0 * s, s, color);
        // The close button: a round chip with a cross.
        quad(ui, art.disc, l.close, [1.0, 1.0, 1.0, if hover == Some(3) { 0.16 } else { 0.07 }], false);
        plus(ui, art, l.close, 10.0 * s, if hover == Some(3) { [1.0; 4] } else { GREY }, std::f32::consts::FRAC_PI_4);

        let current = [self.stats.max_health, self.stats.max_mana, self.stats.max_stamina];
        for (i, card) in l.rows.iter().enumerate() {
            let [cx, cy, cw, ch] = *card;
            let (accent, active) = (ACCENTS[i], self.points > 0);
            let lit = active && hover == Some(i);
            slice(ui, art.rounded, *card, 14.0 * s, [1.0, 1.0, 1.0, if lit { 0.10 } else { 0.045 }]);
            slice(ui, art.ring, *card, 14.0 * s, tint(if lit { accent } else { [1.0; 4] }, if lit { 0.55 } else { 0.07 }));
            // The name with its colour dot, the number, and what a point adds to it.
            quad(ui, art.disc, [cx + 16.0 * s, cy + 13.0 * s, 8.0 * s, 8.0 * s], accent, false);
            art.body.draw(ui, NAMES[i], cx + 30.0 * s, cy + 9.0 * s, s, GREY);
            let number = grouped(current[i].round() as i32);
            art.title.draw(ui, &number, cx + 16.0 * s, cy + 26.0 * s, s, [1.0; 4]);
            let after = art.title.width(&number) * s;
            art.body.draw(ui, &format!("+{}", GAIN[i]), cx + 16.0 * s + after + 8.0 * s, cy + 33.0 * s, s, tint(ACCENT, if active { 1.0 } else { 0.4 }));
            // The round "+" button at the right: lit while there is a point to spend, small while pressed.
            let pressed = self.level_press == Some(i) && lit;
            let d = 34.0 * s * if pressed { 0.92 } else { 1.0 };
            let button = [cx + cw - 14.0 * s - 34.0 * s + (34.0 * s - d) / 2.0, cy + (ch - d) / 2.0, d, d];
            if active {
                quad(ui, art.glow, [button[0] - d * 0.6, button[1] - d * 0.6, d * 2.2, d * 2.2], tint(accent, if lit { 0.55 } else { 0.28 }), true);
                quad(ui, art.disc, button, accent, false);
                plus(ui, art, button, 14.0 * s, if accent[0] + accent[1] > 0.5 { INK } else { [1.0; 4] }, 0.0);
            } else {
                quad(ui, art.disc, button, [1.0, 1.0, 1.0, 0.07], false);
                plus(ui, art, button, 14.0 * s, [1.0, 1.0, 1.0, 0.25], 0.0);
            }
        }
        let note = "Press U or Esc to close";
        art.small.draw(ui, note, x + (w - art.small.width(note) * s) / 2.0, y + h - 24.0 * s, s, tint(GREY, 0.7));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_adds_its_attributes() {
        assert_eq!(attributes_of([1, 0, 0]), [1, 0, 1, 0, 0]);
        assert_eq!(attributes_of([1, 2, 3]), [1, 3, 6, 2, 2]);
    }

    #[test]
    fn every_level_gained_is_a_point_less_those_spent() {
        assert_eq!(unspent(1, [0; 3]), 0);
        assert_eq!(unspent(5, [0; 3]), 4);
        assert_eq!(unspent(5, [1, 0, 2]), 1);
        // A level set lower than the points spent leaves none, not a wrapped number.
        assert_eq!(unspent(2, [3, 0, 0]), 0);
    }
}
