//! Sun, ambient, fog and sky colours of the original client: `Light.dbc` places light zones on a
//! map, each pointing at `LightParams` whose colours/distances are keyframed over the day in
//! `LightIntBand` (18 per params) and `LightFloatBand` (6 per params).

use crate::heavens;
use crate::renderer::Atmosphere;
use anyhow::{Context, Result};
use glam::Vec3;
use mpq::Mount;
use std::collections::HashMap;
use wowfmt::dbc::Dbc;

/// Light positions and falloffs are stored x36, in file space (render space + 32 tiles).
const LIGHT_SCALE: f32 = 36.0;
const FILE_ORIGIN: f32 = 32.0 * wowfmt::adt::TILE_SIZE;
/// Band times are half-minutes since midnight.
const DAY: f32 = 2880.0;

// Int band slots within one LightParams, confirmed in WoW.exe: FUN_006d64d0 fills the light state,
// FUN_006d37e0 uses slots 0/1 as the light's direct/ambient colours and FUN_006cee30 uses slot 7 as
// the fog colour, float slot 0 as its end and float slot 1 as start / end.
const DIRECT: u32 = 0;
const AMBIENT: u32 = 1;
const SKY_TOP: u32 = 2;
const SKY_MIDDLE: u32 = 3;
const FOG: u32 = 7;
// Bands 4 to 6 are the sky colours on the way down to the horizon, 9 the tint of sun and moon, 10 and 11 the clouds' lit and shaded colour
// (the order of the original's light state, FUN_006d64d0).
const SKY_LOW: [u32; 3] = [4, 5, 6];
const CELESTIAL: u32 = 9;
const CLOUD_LIT: u32 = 10;
const CLOUD_SHADE: u32 = 11;
const CLOUD_BASE: u32 = 12;
// Float band slots.
const FOG_END: u32 = 0;
const FOG_START_SCALE: u32 = 1;
const CLOUD_COVER: u32 = 3;

struct Band {
    times: Vec<f32>,
    values: Vec<Vec3>,
}

impl Band {
    /// Linear interpolation between keyframes, wrapping around midnight.
    fn at(&self, t: f32) -> Vec3 {
        let n = self.times.len();
        if n == 0 {
            return Vec3::ZERO;
        }
        let next = self.times.iter().position(|&x| x > t).unwrap_or(n);
        let (a, b) = ((next + n - 1) % n, next % n);
        let ta = self.times[a] - if next == 0 { DAY } else { 0.0 };
        let tb = self.times[b] + if next == n { DAY } else { 0.0 };
        self.values[a].lerp(self.values[b], ((t - ta) / (tb - ta)).clamp(0.0, 1.0))
    }
}

struct Light {
    /// File-space position and falloff radii, in yards. A radius of 0 marks the map's default light.
    x: f32,
    z: f32,
    start: f32,
    end: f32,
    params: u32,
    /// The params the zone uses while it storms.
    storm_params: u32,
}

pub struct Sky {
    lights: Vec<Light>,
    /// `LightParams` ids whose sky glows at sunrise and sunset.
    highlight: std::collections::HashSet<u32>,
    int_bands: HashMap<u32, Band>,
    float_bands: HashMap<u32, Band>,
    view_distance: f32,
    /// The distance the world was loaded for: the fog does not reach further.
    loaded: f32,
}

fn load_bands(mount: &Mount, name: &str, value: impl Fn(u32) -> Vec3) -> Result<HashMap<u32, Band>> {
    let dbc = Dbc::parse(&mount.read(&format!("DBFilesClient\\{name}.dbc"))?)?;
    // Fields: 0 id, 1 key count, 2..18 times, 18..34 values.
    Ok(dbc
        .iter()
        .map(|r| {
            let n = (r.u32(1) as usize).min(16);
            let times = (0..n).map(|i| r.u32(2 + i) as f32).collect();
            let values = (0..n).map(|i| value(r.u32(18 + i))).collect();
            (r.u32(0), Band { times, values })
        })
        .collect())
}

impl Sky {
    /// Light zones of `map_name`; `view_distance` caps the fog distance (nothing is loaded beyond it).
    pub fn load(mount: &Mount, map_name: &str, view_distance: f32) -> Result<Sky> {
        let maps = Dbc::parse(&mount.read("DBFilesClient\\Map.dbc")?)?;
        let map_id = maps
            .iter()
            .find(|r| r.string(1).eq_ignore_ascii_case(map_name))
            .map(|r| r.u32(0))
            .with_context(|| format!("map {map_name} is not in Map.dbc"))?;
        let dbc = Dbc::parse(&mount.read("DBFilesClient\\Light.dbc")?)?;
        // Fields: 0 id, 1 map, 2..5 x y z, 5 falloff start, 6 falloff end, 7 params for clear weather, 8 under water, 9 in a storm.
        let lights = dbc
            .iter()
            .filter(|r| r.u32(1) == map_id)
            .map(|r| Light {
                x: r.f32(2) / LIGHT_SCALE,
                z: r.f32(4) / LIGHT_SCALE,
                start: r.f32(5) / LIGHT_SCALE,
                end: r.f32(6) / LIGHT_SCALE,
                params: r.u32(7),
                storm_params: r.u32(9),
            })
            .collect();
        // LightParams field 1: the sky glows at sunrise and sunset.
        let highlight = Dbc::parse(&mount.read(r"DBFilesClient\LightParams.dbc")?)?.iter().filter(|r| r.u32(1) != 0).map(|r| r.u32(0)).collect();
        let rgb = |v: u32| Vec3::new((v >> 16 & 255) as f32, (v >> 8 & 255) as f32, (v & 255) as f32) / 255.0;
        Ok(Sky {
            lights,
            highlight,
            int_bands: load_bands(mount, "LightIntBand", rgb)?,
            float_bands: load_bands(mount, "LightFloatBand", |v| Vec3::new(f32::from_bits(v), 0.0, 0.0))?,
            view_distance,
            loaded: view_distance,
        })
    }

    /// Cap the fog at `yards` (the terrain distance option), within what is loaded.
    pub fn set_view_distance(&mut self, yards: f32) {
        self.view_distance = yards.min(self.loaded);
    }

    /// Atmosphere at a render-space position at `hour` (0..24), `storm` of the way (0..1) from the clear light of the zone to its storm light;
    /// `None` if the map has no default light.
    pub fn atmosphere(&self, pos: Vec3, hour: f32, storm: f32) -> Option<Atmosphere> {
        let (x, z) = (pos.x + FILE_ORIGIN, pos.z + FILE_ORIGIN);
        let default = self.lights.iter().find(|l| l.end == 0.0)?;

        // Zones whose falloff reaches us, smallest first: each takes its share of what is left,
        // the map's default light gets the remainder.
        let mut near: Vec<_> = self
            .lights
            .iter()
            .filter(|l| l.end > 0.0)
            .filter_map(|l| {
                let d = ((l.x - x).powi(2) + (l.z - z).powi(2)).sqrt();
                (d < l.end).then(|| (l.end, (l.params, l.storm_params), if d <= l.start { 1.0 } else { (l.end - d) / (l.end - l.start) }))
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut remaining = 1.0;
        let mut mix: Vec<((u32, u32), f32)> = near
            .iter()
            .map(|&(_, params, weight)| {
                let w = weight * remaining;
                remaining -= w;
                (params, w)
            })
            .collect();
        mix.push(((default.params, default.storm_params), remaining));

        let t = hour * DAY / 24.0;
        let storm = storm.clamp(0.0, 1.0);
        let int = |slot| mix.iter().map(|&((clear, wet), w)| w * self.band(&self.int_bands, clear, 18, slot, t).lerp(self.band(&self.int_bands, wet, 18, slot, t), storm)).sum::<Vec3>();
        let float = |slot| mix.iter().map(|&((clear, wet), w)| w * (1.0 - storm).mul_add(self.band(&self.float_bands, clear, 6, slot, t).x, storm * self.band(&self.float_bands, wet, 6, slot, t).x)).sum::<f32>();

        let fog_end = (float(FOG_END) / LIGHT_SCALE).clamp(1.0, self.view_distance);
        let sun = light_source(hour);
        Some(Atmosphere {
            sun_dir: -sun,
            sun_color: int(DIRECT),
            ambient: int(AMBIENT),
            fog_color: int(FOG),
            sky_top: int(SKY_TOP),
            sky_mid: int(SKY_MIDDLE),
            fog_start: fog_end * float(FOG_START_SCALE).clamp(0.0, 0.9),
            fog_end,
            sky_low: SKY_LOW.map(int),
            celestial: int(CELESTIAL),
            cloud_lit: int(CLOUD_LIT),
            cloud_shade: int(CLOUD_SHADE),
            cloud_base: int(CLOUD_BASE),
            cloud_cover: float(CLOUD_COVER).clamp(0.0, 1.0),
            // The glow comes with the zones' clear light, and goes with the storm.
            highlight: mix.iter().filter(|((clear, _), _)| self.highlight.contains(clear)).map(|(_, w)| w).sum::<f32>() * (1.0 - storm),
            storm,
            hour,
        })
    }

    fn band(&self, bands: &HashMap<u32, Band>, params: u32, per_params: u32, slot: u32, t: f32) -> Vec3 {
        let id = params.saturating_sub(1) * per_params + slot + 1;
        bands.get(&id).map_or(Vec3::ZERO, |b| b.at(t))
    }
}

/// Where the light that casts the shadows comes from (render space, unit): the sun while it is up, the moon after it, in between a mix. It
/// stands where the sun and the moon are drawn (`heavens`), but never lower than about nine degrees, so that the ground is lit at all.
pub fn light_source(hour: f32) -> Vec3 {
    let sun = heavens::to_render(heavens::sun_direction(hour));
    let moon = heavens::to_render(heavens::moon_direction(hour));
    let day = ((sun.y + 0.12) / 0.2).clamp(0.0, 1.0);
    let from = moon.lerp(sun, day);
    Vec3::new(from.x, from.y.max(0.15), from.z).normalize()
}

/// How strongly the windows of buildings that glow at night (`MOMT` flag 0x10) glow at `hour`, 0 to 1: WoW.exe's curve (the keys at
/// `0xce9a34`, set by `FUN_006ce670`, read by `FUN_006cf6c0`), and the factor `FUN_006b4090` multiplies their colour with, `k / 256` for
/// `k` = the curve times 255 less a half, rounded. Full from half past nine in the evening until six, gone at seven, back at half past eight.
pub fn window_glow(hour: f32) -> f32 {
    const KEYS: [(f32, f32); 4] = [(0.25, 1.0), (0.291_666_67, 0.0), (0.854_166_7, 0.0), (0.895_833_3, 1.0)];
    let t = (hour / 24.0).rem_euclid(1.0);
    let next = KEYS.iter().position(|k| k.0 >= t).unwrap_or(KEYS.len());
    let (to, from) = match next {
        n if n == KEYS.len() => (0, KEYS.len() - 1),
        0 => (0, KEYS.len() - 1),
        n => (n, n - 1),
    };
    let (a, b) = (KEYS[from], KEYS[to]);
    let mut span = b.0 - a.0;
    let curve = if span.abs() < 0.001 {
        a.1
    } else {
        span += if span < 0.0 { 1.0 } else { 0.0 };
        let mut at = t - a.0;
        at += if at < 0.0 { 1.0 } else { 0.0 };
        a.1 + (b.1 - a.1) * at / span
    };
    (curve * 255.0 - 0.5).round_ties_even().max(0.0) / 256.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_glow_from_the_evening_to_the_morning() {
        let full = 254.0 / 256.0;
        assert!([22.0, 0.0, 3.0, 5.9].iter().all(|&h| (window_glow(h) - full).abs() < 1e-6), "night");
        assert!([7.5, 12.0, 17.0, 20.0].iter().all(|&h| window_glow(h) == 0.0), "day");
        // Seven in the morning is gone, and the glow comes up in the hour before half past nine and is half there at nine.
        assert_eq!(window_glow(7.0), 0.0);
        assert!((window_glow(6.5) - 127.0 / 256.0).abs() < 1.5 / 256.0);
        assert!((window_glow(21.0) - 127.0 / 256.0).abs() < 1.5 / 256.0);
    }

    #[test]
    fn the_light_comes_from_the_sun_by_day_and_the_moon_by_night() {
        assert!((light_source(12.0) - heavens::to_render(heavens::sun_direction(12.0))).length() < 1e-4);
        assert!((light_source(0.0) - heavens::to_render(heavens::moon_direction(0.0))).length() < 1e-4);
        // Between the two, and with the sun below the horizon and no moon, it still comes from above.
        assert!((8..=11).all(|h| light_source(h as f32 * 0.5).y >= 0.14));
    }

    #[test]
    fn bands_interpolate_and_wrap_around_midnight() {
        let b = Band { times: vec![0.0, 1440.0], values: vec![Vec3::ZERO, Vec3::ONE] };
        assert!((b.at(720.0) - Vec3::splat(0.5)).length() < 1e-5);
        // 2160 is halfway from the noon key back round to the midnight key at 2880.
        assert!((b.at(2160.0) - Vec3::splat(0.5)).length() < 1e-5);
        let one = Band { times: vec![500.0], values: vec![Vec3::X] };
        assert_eq!(one.at(0.0), Vec3::X);
        assert_eq!(one.at(2000.0), Vec3::X);
    }
}
