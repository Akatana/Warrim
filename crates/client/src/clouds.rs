//! The clouds of the original client: a picture of the sky made on the CPU and laid over the cloud dome (`FUN_006cffc0` makes the noise,
//! `FUN_006cfb00` lights it). Four octaves of three-dimensional value noise, whose lattice is a table of random numbers read through a fixed
//! permutation (the table is in WoW.exe at 0x86f2d0), are summed for every texel of the picture; the noise's x and its z both move with a
//! counter that goes up by one each time the whole picture has been made again, so the clouds drift and change shape. The picture is made in
//! bands of 32 rows, one band every tenth of a second. How many texels are cloud follows the zone's cloud cover; each is lit like a bump
//! picture from the sun's (or the moon's) place on the map. That picture only changes in steps (a band every tenth of a second, a drift of a
//! fraction of a texel for each whole picture), so what is shown is a fade from the last whole picture to the one before it, over the time one
//! whole picture takes to make: the clouds then move smoothly.

use crate::heavens;
use crate::renderer::Atmosphere;
use glam::Vec3;
use std::f32::consts::PI;

/// WoW.exe 0x86f2d0: the permutation of the noise.
const PERM: [u8; 256] = [
    0xe1, 0x9b, 0xd2, 0x6c, 0xaf, 0xc7, 0xdd, 0x90, 0xcb, 0x74, 0x46, 0xd5, 0x45, 0x9e, 0x21, 0xfc, 0x05, 0x52, 0xad, 0x85, 0xde, 0x8b, 0xae, 0x1b, 0x09, 0x47, 0x5a, 0xf6, 0x4b, 0x82, 0x5b, 0xbf,
    0xa9, 0x8a, 0x02, 0x97, 0xc2, 0xeb, 0x51, 0x07, 0x19, 0x71, 0xe4, 0x9f, 0xcd, 0xfd, 0x86, 0x8e, 0xf8, 0x41, 0xe0, 0xd9, 0x16, 0x79, 0xe5, 0x3f, 0x59, 0x67, 0x60, 0x68, 0x9c, 0x11, 0xc9, 0x81,
    0x24, 0x08, 0xa5, 0x6e, 0xed, 0x75, 0xe7, 0x38, 0x84, 0xd3, 0x98, 0x14, 0xb5, 0x6f, 0xef, 0xda, 0xaa, 0xa3, 0x33, 0xac, 0x9d, 0x2f, 0x50, 0xd4, 0xb0, 0xfa, 0x57, 0x31, 0x63, 0xf2, 0x88, 0xbd,
    0xa2, 0x73, 0x2c, 0x2b, 0x7c, 0x5e, 0x96, 0x10, 0x8d, 0xf7, 0x20, 0x0a, 0xc6, 0xdf, 0xff, 0x48, 0x35, 0x83, 0x54, 0x39, 0xdc, 0xc5, 0x3a, 0x32, 0xd0, 0x0b, 0xf1, 0x1c, 0x03, 0xc0, 0x3e, 0xca,
    0x12, 0xd7, 0x99, 0x18, 0x4c, 0x29, 0x0f, 0xb3, 0x27, 0x2e, 0x37, 0x06, 0x80, 0xa7, 0x17, 0xbc, 0x6a, 0x22, 0xbb, 0x8c, 0xa4, 0x49, 0x70, 0xb6, 0xf4, 0xc3, 0xe3, 0x0d, 0x23, 0x4d, 0xc4, 0xb9,
    0x1a, 0xc8, 0xe2, 0x77, 0x1f, 0x7b, 0xa8, 0x7d, 0xf9, 0x44, 0xb7, 0xe6, 0xb1, 0x87, 0xa0, 0xb4, 0x0c, 0x01, 0xf3, 0x94, 0x66, 0xa6, 0x26, 0xee, 0xfb, 0x25, 0xf0, 0x7e, 0x40, 0x4a, 0xa1, 0x28,
    0xb8, 0x95, 0xab, 0xb2, 0x65, 0x42, 0x1d, 0x3b, 0x92, 0x3d, 0xfe, 0x6b, 0x2a, 0x56, 0x9a, 0x04, 0xec, 0xe8, 0x78, 0x15, 0xe9, 0xd1, 0x2d, 0x62, 0xc1, 0x72, 0x4e, 0x13, 0xce, 0x0e, 0x76, 0x7f,
    0x30, 0x4f, 0x93, 0x55, 0x1e, 0xcf, 0xdb, 0x36, 0x58, 0xea, 0xbe, 0x7a, 0x5f, 0x43, 0x8f, 0x6d, 0x89, 0xd6, 0x91, 0x5d, 0x5c, 0x64, 0xf5, 0x00, 0xd8, 0xba, 0x3c, 0x53, 0x69, 0x61, 0xcc, 0x34,
];

/// The octaves' steps along the picture in 256ths of a lattice cell per texel, for the pictures of 128 and 256 texels (WoW.exe 0x86f3dc): the
/// first octave has 8 cells across the picture, each next one twice as many.
const OCTAVE_STEPS: [[u16; 4]; 2] = [[16, 32, 64, 128], [8, 16, 32, 64]];
const OCTAVES: usize = 4;
/// Rows made at once, and seconds between bands.
const BAND: usize = 32;
const BAND_SECONDS: f32 = 0.1;

pub struct Clouds {
    size: usize,
    lod: usize,
    /// The next row to make, and the counter that moves the noise.
    row: usize,
    counter: u16,
    timer: f32,
    lattice: [f32; 256],
    /// The weight of the next lattice point by how far along the cell (WoW.exe `FUN_006d0c90`).
    fade: [f32; 256],
    /// How thick the cloud is by how far the noise is over the threshold: `255 - 255 * 0.96^(i * 0.5977)`, as made at start.
    thick: [u8; 256],
    /// The noise summed over the octaves, per texel, and the rate it changes by in x and y (of the first three octaves).
    sum: Vec<f32>,
    slope: Vec<[f32; 2]>,
    density: Vec<u8>,
    above: Vec<f32>,
    /// The picture being made, RGBA, and the last two whole ones with the seconds since the newer was finished.
    pub pixels: Vec<u8>,
    prev: Vec<u8>,
    cur: Vec<u8>,
    since: f32,
    shown: Vec<u8>,
}

fn rng(state: &mut u64) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    ((*state >> 33) & 0x7fff) as u32
}

impl Clouds {
    /// `lod` 0 or 1: a picture of 128 or 256 texels (the original's `SkyCloudLOD`).
    pub fn new(lod: usize, seed: u64) -> Self {
        let size = 128 << lod.min(1);
        let mut state = seed | 1;
        let mut lattice = [0.0; 256];
        for v in &mut lattice {
            *v = 1.0 - (rng(&mut state) as f32 / 32768.0 * 2.0);
        }
        let mut fade = [0.0; 256];
        for (i, f) in fade.iter_mut().enumerate() {
            *f = (1.0 - (i as f32 * PI / 256.0).cos()) * 0.5;
        }
        // The table is made once, for the threshold of the start (0.6 cover): the original does not make it again when the cover changes.
        let step = (255.0 - ((1.0f32 - 0.6) * 255.0).floor()) / 256.0;
        let mut thick = [0u8; 256];
        for (i, t) in thick.iter_mut().enumerate() {
            *t = (255.0 - 255.0 * 0.96f32.powf(i as f32 * step)) as u8;
        }
        Clouds {
            size,
            lod: lod.min(1),
            row: 0,
            counter: 0,
            timer: 0.0,
            lattice,
            fade,
            thick,
            sum: vec![0.0; size * size],
            slope: vec![[0.0; 2]; size * size],
            density: vec![0; size * size],
            above: vec![0.0; size],
            pixels: vec![0; size * size * 4],
            prev: vec![0; size * size * 4],
            cur: vec![0; size * size * 4],
            since: 0.0,
            shown: vec![0; size * size * 4],
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// The picture to show: the last two whole ones, faded by how far along the making of the next is.
    pub fn blended(&mut self) -> &[u8] {
        let w = (self.since / (self.size / BAND) as f32 / BAND_SECONDS).clamp(0.0, 1.0);
        let w = (w * 256.0) as u32;
        for ((o, p), c) in self.shown.iter_mut().zip(&self.prev).zip(&self.cur) {
            *o = ((*p as u32 * (256 - w) + *c as u32 * w) >> 8) as u8;
        }
        &self.shown
    }

    /// Moves time on by `dt`; every tenth of a second a band of rows is made again.
    pub fn update(&mut self, dt: f32, a: &Atmosphere) {
        self.since += dt;
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.timer = BAND_SECONDS;
            self.make_band(a);
        }
    }

    /// Makes the whole picture at once (the start).
    pub fn fill(&mut self, a: &Atmosphere) {
        for _ in 0..self.size / BAND {
            self.make_band(a);
        }
        self.prev.copy_from_slice(&self.cur);
    }

    fn make_band(&mut self, a: &Atmosphere) {
        let size = self.size;
        let first = self.row;
        let steps = OCTAVE_STEPS[self.lod];
        let (z_cell, z_frac) = ((self.counter >> 8) as u8, (self.counter & 0xff) as usize);
        let perm = |i: usize| PERM[i & 0xff] as usize;
        // The threshold of the cover: the more cover, the lower.
        let threshold = ((1.0 - a.cloud_cover) * 255.0) as usize;
        self.sum[first * size..(first + BAND) * size].fill(0.0);

        // Where each octave is along y at the first row of the band.
        let mut y: [u16; OCTAVES] = std::array::from_fn(|n| (first as u16).wrapping_mul(steps[n]));
        let gain = (size / 128) as f32;
        for r in 0..BAND {
            let row = first + r;
            // The four hashes of the lattice (y, z) of each octave's row.
            let hashes: [[usize; 4]; OCTAVES] = std::array::from_fn(|n| {
                let ycell = (y[n] >> 8) as u8 as usize;
                let (pz0, pz1) = (perm(z_cell as usize), perm(z_cell as usize + 1));
                [perm(pz0 + ycell), perm(pz0 + ycell + 1), perm(pz1 + ycell), perm(pz1 + ycell + 1)]
            });
            let mut x: [u16; OCTAVES] = [self.counter; OCTAVES];
            // What each octave's cell holds: base and difference to the next cell, for the four (y, z) corners.
            let mut cached: [Option<u8>; OCTAVES] = [None; OCTAVES];
            let mut corner = [[(0.0f32, 0.0f32); 4]; OCTAVES];
            let mut left = 0.0f32;
            for col in 0..size {
                let mut total = 0.0f32;
                for n in 0..OCTAVES {
                    let xcell = (x[n] >> 8) as u8;
                    if cached[n] != Some(xcell) {
                        cached[n] = Some(xcell);
                        for (k, h) in hashes[n].iter().enumerate() {
                            let base = self.lattice[perm(h + xcell as usize)];
                            corner[n][k] = (base, self.lattice[perm(h + xcell as usize + 1)] - base);
                        }
                    }
                    let fx = self.fade[(x[n] & 0xff) as usize];
                    let (wy, wz) = (self.fade[(y[n] & 0xff) as usize], self.fade[z_frac]);
                    let at = |k: usize| corner[n][k].1.mul_add(fx, corner[n][k].0);
                    let (c00, c10, c01, c11) = (at(0), at(1), at(2), at(3));
                    let low = c00 + (c10 - c00) * wy;
                    let high = c01 + (c11 - c01) * wy;
                    total += (low + (high - low) * wz) / (1 << n) as f32;
                    if n == 2 {
                        // The slope of what the first three octaves make: the clouds are lit as if they were a relief.
                        self.slope[row * size + col] = [(left - total) * gain, (self.above[col] - total) * gain];
                        self.above[col] = total;
                        left = total;
                    }
                    x[n] = x[n].wrapping_add(steps[n]);
                }
                self.sum[row * size + col] = total;
            }
            for n in 0..OCTAVES {
                y[n] = y[n].wrapping_add(steps[n]);
            }
        }
        for texel in first * size..(first + BAND) * size {
            let index = ((self.sum[texel] * 64.0 + 128.0).floor() as i32 & 0xff) as usize;
            self.density[texel] = index.checked_sub(threshold).map_or(0, |j| self.thick[j]);
        }
        self.light(first, a);
        self.row += BAND;
        if self.row >= size {
            self.row = 0;
            self.counter = self.counter.wrapping_add(1);
            std::mem::swap(&mut self.prev, &mut self.cur);
            self.cur.copy_from_slice(&self.pixels);
            self.since = 0.0;
        }
    }

    /// Colours the band's rows (`FUN_006cfb00`): the clouds' own colour (band 11, darker where they are thick) plus the shade (band 12),
    /// plus the colour of the light (band 10) where the relief faces it.
    fn light(&mut self, first: usize, a: &Atmosphere) {
        let size = self.size;
        // The light: the sun from just before dawn to just after dusk, else the moon, where it is on the picture.
        let day = (0.229167 - 0.0277778..=0.895833 + 0.0277778).contains(&(a.hour / 24.0));
        let source = if day { heavens::sun_direction(a.hour) } else { heavens::moon_direction(a.hour) };
        let spot = map_position(source) * size as f32;
        let height = a.storm * 192.0 + 64.0;
        // Lit by the sun or the moon, less when it storms; the original's table dips for a while at dawn and after dusk.
        let strength = (1.0 - a.storm * 0.75) * lit_by_day(a.hour);
        let (own, shade, light) = (a.cloud_shade, a.cloud_base, a.cloud_lit);
        for row in first..first + BAND {
            for col in 0..size {
                let t = row * size + col;
                let d = self.density[t];
                let out = &mut self.pixels[t * 4..t * 4 + 4];
                if d == 0 {
                    out[3] = 0;
                    continue;
                }
                let b = (((255 - d as i32) as u8 >> 1) as f32 + 64.0) / 255.0;
                let slope = self.slope[t];
                let to_light = Vec3::new(spot.x - col as f32, spot.y - row as f32, height);
                let normal = Vec3::new(slope[0], slope[1], 1.0);
                let lit = to_light.dot(normal) / (normal.length() * to_light.length());
                let mut c = own * b + shade;
                if lit > 0.0 {
                    c += light * (lit * strength);
                }
                for (o, v) in out.iter_mut().zip([c.x, c.y, c.z]) {
                    *o = (v.min(1.0) * 255.0) as u8;
                }
                out[3] = d;
            }
        }
        // Texels without cloud take the colour of the texel on their left (the first of a row, of the one on its right), so that the edge of a
        // cloud does not go dark where the picture is looked at between texels.
        for row in first..first + BAND {
            for col in 0..size {
                let t = row * size + col;
                if self.density[t] == 0 {
                    let from = if col != 0 { t - 1 } else { (t + 1).min(size * size - 1) };
                    let rgb = [self.pixels[from * 4], self.pixels[from * 4 + 1], self.pixels[from * 4 + 2]];
                    self.pixels[t * 4..t * 4 + 3].copy_from_slice(&rgb);
                }
            }
        }
    }
}

/// The original's factor for how much the clouds are lit by the time of day (table 0xce9ab8): 1, but for a dip between 4:40 and 5:30 and
/// round 22:00.
fn lit_by_day(hour: f32) -> f32 {
    heavens::curve(&[(0.166667, 1.0), (0.194444, 0.0), (0.201389, 0.0), (0.229167, 1.0), (0.895833, 1.0), (0.923611, 0.0), (0.888889, 0.0), (0.916667, 1.0)], hour / 24.0)
}

/// Where a direction (original world space) is on the cloud picture, 0 to 1 each way (`FUN_006cf9c0`, `FUN_006cf870`): the ray from the camera
/// meets the cloud cap, a sphere of radius 1 round a point 0.707 below the camera, and its angle from the cap's apex is laid out linearly to
/// the rim at 45 degrees.
fn map_position(dir: Vec3) -> glam::Vec2 {
    let c = (0.25 * PI).cos();
    // |k d + (0, 0, c)|^2 = 1 for the far root k.
    let (a, b, cc) = (dir.length_squared(), 2.0 * c * dir.z, c * c - 1.0);
    let k = (-b + (b * b - 4.0 * a * cc).max(0.0).sqrt()) / (2.0 * a);
    let p = dir * k;
    let lat = (p.z + c).clamp(-1.0, 1.0).acos().min(0.25 * PI);
    let flat = p.truncate().length();
    let toward = if flat < 1e-3 { glam::Vec2::ZERO } else { p.truncate() / flat };
    glam::Vec2::splat(0.5) + toward * (lat / (0.25 * PI) * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atmosphere(cover: f32) -> Atmosphere {
        Atmosphere { cloud_cover: cover, cloud_base: Vec3::ZERO, ..Atmosphere::daytime(500.0) }
    }

    fn share(clouds: &Clouds) -> f32 {
        clouds.pixels.chunks(4).filter(|p| p[3] > 0).count() as f32 / (clouds.size * clouds.size) as f32
    }

    #[test]
    fn the_picture_is_made_in_bands_of_32_rows_a_tenth_of_a_second_apart() {
        let a = atmosphere(0.5);
        let mut clouds = Clouds::new(0, 1);
        clouds.update(0.0, &a);
        assert_eq!(clouds.row, 32);
        clouds.update(0.05, &a);
        assert_eq!(clouds.row, 32);
        clouds.update(0.06, &a);
        assert_eq!(clouds.row, 64);
        // The whole picture takes four bands, and the counter that moves the noise goes up once.
        clouds.update(0.1, &a);
        clouds.update(0.1, &a);
        assert_eq!(clouds.counter, 1);
    }

    #[test]
    fn more_cover_means_more_cloud() {
        let share_at = |cover| {
            let mut c = Clouds::new(1, 5);
            c.fill(&atmosphere(cover));
            share(&c)
        };
        let (clear, half, storm) = (share_at(0.1), share_at(0.5), share_at(0.95));
        assert!(clear < half && half < storm, "{clear} {half} {storm}");
        assert!(clear < 0.35 && storm > 0.8, "{clear} {storm}");
    }

    #[test]
    fn the_clouds_drift_and_change_as_the_counter_goes_up() {
        let a = atmosphere(0.5);
        let mut clouds = Clouds::new(0, 3);
        clouds.fill(&a);
        let before = clouds.density.clone();
        for _ in 0..4 * 200 {
            clouds.update(BAND_SECONDS, &a);
        }
        assert!(clouds.counter >= 200);
        let moved = before.iter().zip(&clouds.density).filter(|(a, b)| a != b).count();
        assert!(moved > before.len() / 4, "{moved} texels changed");
    }

    #[test]
    fn the_noise_is_smooth_between_neighbours() {
        let mut clouds = Clouds::new(1, 9);
        clouds.fill(&atmosphere(0.5));
        let size = clouds.size;
        let worst = (0..size * size - 1).filter(|t| t % size != size - 1).map(|t| (clouds.sum[t] - clouds.sum[t + 1]).abs()).fold(0.0, f32::max);
        assert!(worst < 0.35, "{worst}");
    }

    #[test]
    fn the_sun_lands_where_it_stands() {
        // Overhead it is at the middle of the picture, on the horizon towards the north-west at its rim.
        let up = map_position(Vec3::Z);
        assert!((up - glam::Vec2::splat(0.5)).length() < 1e-3);
        let low = map_position(Vec3::new(1.0, 1.0, 0.0).normalize());
        assert!((low - glam::Vec2::splat(0.5)).length() > 0.45);
    }
}

#[cfg(test)]
mod blend_tests {
    use super::*;

    #[test]
    fn the_shown_picture_fades_from_the_older_whole_picture_to_the_newer() {
        let mut c = Clouds::new(0, 1);
        c.prev.fill(0);
        c.cur.fill(200);
        c.since = 0.0;
        assert_eq!(c.blended()[0], 0);
        c.since = 0.2;
        assert_eq!(c.blended()[0], 100);
        c.since = 5.0;
        assert_eq!(c.blended()[0], 200);
    }
}
