//! Text for the interface: the glyphs of one TrueType font at one height packed into a single texture, drawn as
//! quads. Fonts are the original's (`FRIZQT__.TTF`, `ARIALN.TTF`, ...); outlines are baked into the glyphs, drop
//! shadows are a second black pass.

use crate::renderer::Gpu;
use crate::ui::{Quad, Ui};
use anyhow::{Context, Result};
use std::collections::HashMap;

/// Glyphs are rendered at this multiple of their size, so they stay sharp when the interface is scaled up.
const SUPERSAMPLE: f32 = 2.0;
const ATLAS_WIDTH: usize = 1024;

struct Glyph {
    /// The glyph's cell in the atlas as (u0, v0, u1, v1), and its size in pixels (with the padding around it).
    uv: [f32; 4],
    size: (f32, f32),
    xmin: f32,
    ymin: f32,
    advance: f32,
    pad: f32,
}

pub struct GlyphSet {
    texture: usize,
    glyphs: HashMap<char, Glyph>,
    /// Distance from the top of a line to its baseline, in pixels at the rendered size.
    ascent: f32,
}

impl GlyphSet {
    /// The printable ASCII glyphs of `font` at `height` interface units, with an outline of `outline` units (0, 1, 2).
    pub fn new(ui: &mut Ui, gpu: &Gpu, font: &fontdue::Font, height: f32, outline: usize) -> Result<GlyphSet> {
        let px = height * SUPERSAMPLE;
        let ascent = font.horizontal_line_metrics(px).context("font has line metrics")?.ascent;
        let pad = 1 + outline * SUPERSAMPLE as usize;
        // Rasterise, then pack in rows.
        let mut cells = Vec::new();
        for c in (' '..='~').chain(std::iter::once('?')) {
            let (m, bitmap) = font.rasterize(c, px);
            let (w, h) = (m.width + pad * 2, m.height + pad * 2);
            let mut rgba = vec![0u8; w * h * 4];
            let mut cover = vec![0u8; w * h];
            for y in 0..m.height {
                for x in 0..m.width {
                    cover[(y + pad) * w + x + pad] = bitmap[y * m.width + x];
                }
            }
            let reach = (outline as f32 * SUPERSAMPLE) as i32;
            for y in 0..h {
                for x in 0..w {
                    let fill = cover[y * w + x];
                    let mut edge = fill;
                    if reach > 0 {
                        for dy in -reach..=reach {
                            for dx in -reach..=reach {
                                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                                if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                                    edge = edge.max(cover[ny as usize * w + nx as usize]);
                                }
                            }
                        }
                    }
                    let alpha = edge.max(fill);
                    // Fill white over a black outline, so a tint colours the fill only.
                    let white = if alpha == 0 { 255 } else if reach > 0 { (fill as u32 * 255 / alpha as u32) as u8 } else { 255 };
                    rgba[(y * w + x) * 4..][..4].copy_from_slice(&[white, white, white, alpha]);
                }
            }
            cells.push((c, m, w, h, rgba));
        }
        let (mut x, mut y, mut row) = (0usize, 0usize, 0usize);
        let mut placed = Vec::new();
        for (_, _, w, h, _) in &cells {
            if x + w > ATLAS_WIDTH {
                (x, y, row) = (0, y + row, 0);
            }
            placed.push((x, y));
            x += w;
            row = row.max(*h);
        }
        let atlas_height = (y + row).next_power_of_two().max(16);
        let mut atlas = vec![0u8; ATLAS_WIDTH * atlas_height * 4];
        let mut glyphs = HashMap::new();
        for ((c, m, w, h, rgba), (px_x, px_y)) in cells.into_iter().zip(placed) {
            for row in 0..h {
                atlas[((px_y + row) * ATLAS_WIDTH + px_x) * 4..][..w * 4].copy_from_slice(&rgba[row * w * 4..][..w * 4]);
            }
            let uv = [px_x as f32 / ATLAS_WIDTH as f32, px_y as f32 / atlas_height as f32, (px_x + w) as f32 / ATLAS_WIDTH as f32, (px_y + h) as f32 / atlas_height as f32];
            glyphs.insert(c, Glyph { uv, size: (w as f32, h as f32), xmin: m.xmin as f32, ymin: m.ymin as f32, advance: m.advance_width, pad: pad as f32 });
        }
        let texture = ui.add_texture(gpu, ATLAS_WIDTH as u32, atlas_height as u32, &atlas);
        Ok(GlyphSet { texture, glyphs, ascent })
    }

    fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c).or_else(|| self.glyphs.get(&'?'))
    }

    /// Width of `text` in interface units.
    pub fn width(&self, text: &str) -> f32 {
        text.chars().filter_map(|c| self.glyph(c)).map(|g| g.advance).sum::<f32>() / SUPERSAMPLE
    }

    /// Queue `text` with its left end at pixel `x` and the top of its line at pixel `top`; `scale` is pixels per
    /// interface unit.
    pub fn draw(&self, ui: &mut Ui, text: &str, x: f32, top: f32, scale: f32, color: [f32; 4]) {
        let unit = scale / SUPERSAMPLE;
        let (mut pen, baseline) = (x, top + self.ascent * unit);
        for c in text.chars() {
            let Some(g) = self.glyph(c) else { continue };
            let rect = [pen + (g.xmin - g.pad) * unit, baseline - (g.ymin + g.size.1 - g.pad) * unit, g.size.0 * unit, g.size.1 * unit];
            ui.quad(Quad { texture: self.texture, rect, uv: g.uv, color, additive: false });
            pen += g.advance * unit;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_width_grows_with_the_text_and_the_height() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let font = fontdue::Font::from_bytes(mount.read(r"Fonts\FRIZQT__.TTF").unwrap(), fontdue::FontSettings::default()).unwrap();
        // The widths need no GPU: measure with the font's own advances.
        let width = |text: &str, height: f32| text.chars().map(|c| font.metrics(c, height * SUPERSAMPLE).advance_width).sum::<f32>() / SUPERSAMPLE;
        assert!(width("Strength", 12.0) > width("Str", 12.0) * 2.0);
        assert!((width("Strength", 24.0) / width("Strength", 12.0) - 2.0).abs() < 0.1);
    }
}
