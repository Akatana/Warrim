//! The loading screen of the original client (WoW.exe, LoadingScreen.cpp: FUN_00406a60 draws it, FUN_00407150 the bar, FUN_00406920
//! the progress, FUN_00406e20 the picture): the picture of the map (Map.dbc field 38 = a LoadingScreens.dbc id, whose field 2 is
//! the file; `Interface\Glues\loading` when the map has none) over a 4:3 viewport in the middle of the window (black bars on the
//! sides of a wider window, above and below a narrower one), and the bar of `Interface\Glues\LoadingBar` in that viewport.

use crate::renderer::Gpu;
use crate::ui::{Quad, Ui, FULL_UV};
use anyhow::{Context, Result};
use mpq::Mount;
use wowfmt::blp::Blp;
use wowfmt::dbc::Dbc;

/// Bar rectangles in the viewport (x from the left, y from the bottom, both 0 to 1): the data table at 0x7ffd34 holds the centre
/// (0.5, 0.075) and the size of each, the fill is drawn first, and only as wide as the progress.
const FILL: [f32; 4] = [0.2375, 0.0625, 0.525, 0.025];
const BORDER: [f32; 4] = [0.2, 0.05, 0.6, 0.05];

/// What the screen waits for, each reporting 0 to 1 (the original's three callbacks): `Interface` is the interface files
/// (FUN_00407fd0, 50% of the bar once it reports), `Terrain` and `Objects` the two steps of making the world (FUN_00407f70 and
/// FUN_00407fa0, 35% and 15% of the bar while the interface reports, 70% and 30% before).
#[derive(Clone, Copy)]
pub enum Stage {
    Interface,
    Terrain,
    Objects,
}

pub struct Loading {
    screen: usize,
    fill: usize,
    border: usize,
    stages: [f32; 3],
    /// The most tiles that were waiting at once, to tell how far the terrain has come.
    pub peak: usize,
    /// Seconds the screen stays up at least (a short trip would otherwise flash it for a frame).
    pub hold: f32,
}

impl Loading {
    /// The screen of `map` ("Azeroth", "Kalimdor").
    pub fn load(mount: &Mount, map: &str, ui: &mut Ui, gpu: &Gpu) -> Result<Loading> {
        let table = |name: &str| -> Result<Dbc> { Ok(Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc"))?)?) };
        let maps = table("Map")?;
        let id = maps.iter().find(|r| r.string(1).eq_ignore_ascii_case(map)).map(|r| r.u32(38)).with_context(|| format!("no map {map}"))?;
        let screens = table("LoadingScreens")?;
        let file = screens.find_by_id(id).map(|r| r.string(2).to_string()).filter(|f| mount.read(f).is_ok()).unwrap_or_else(|| r"Interface\Glues\loading.blp".into());
        let mut texture = |path: &str| -> Result<usize> {
            let blp = Blp::parse(&mount.read(path)?)?;
            Ok(ui.add_texture(gpu, blp.width, blp.height, &blp.mips[0]))
        };
        Ok(Loading {
            screen: texture(&file)?,
            fill: texture(r"Interface\Glues\LoadingBar\Loading-BarFill.blp")?,
            border: texture(r"Interface\Glues\LoadingBar\Loading-BarBorder.blp")?,
            stages: [0.0; 3],
            peak: 0,
            hold: 0.0,
        })
    }

    /// Put the screen up again for a trip: the interface and the objects are there already, the terrain is not.
    pub fn restart(&mut self, hold: f32) {
        (self.stages, self.peak, self.hold) = ([1.0, 0.0, 1.0], 0, hold);
    }

    /// A stage got as far as `value` (0 to 1); a stage never goes back.
    pub fn report(&mut self, stage: Stage, value: f32) {
        let s = &mut self.stages[stage as usize];
        *s = s.max(value);
    }

    /// How full the bar is (FUN_00406920).
    pub fn progress(&self) -> f32 {
        let [interface, terrain, objects] = self.stages;
        let (head, terrain_share, objects_share) = if interface != 0.0 { (interface * 0.5, 0.35, 0.15) } else { (0.0, 0.7, 0.3) };
        (head + objects * objects_share + terrain * terrain_share).clamp(0.0, 1.0)
    }

    /// The 4:3 area of a window of `size` that the screen is drawn in: x, y, width, height in pixels (FUN_00406a60: the window's
    /// aspect times 0.75 is above 1 for a wide window, below for a narrow one).
    pub fn viewport(size: (u32, u32)) -> [f32; 4] {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let aspect = w / h * 0.75;
        if aspect > 1.0 {
            let k = 1.0 / aspect;
            [(1.0 - k) * 0.5 * w, 0.0, k * w, h]
        } else {
            [0.0, (1.0 - aspect) * 0.5 * h, w, aspect * h]
        }
    }

    /// Queue the screen over everything, the rest of the window being black (the caller clears it).
    pub fn draw(&self, ui: &mut Ui, size: (u32, u32)) {
        let [vx, vy, vw, vh] = Self::viewport(size);
        let mut quad = |texture: usize, rect: [f32; 4]| {
            // The rectangles are measured from the bottom of the viewport.
            let rect = [vx + rect[0] * vw, vy + (1.0 - rect[1] - rect[3]) * vh, rect[2] * vw, rect[3] * vh];
            ui.quad(Quad { texture, rect, uv: FULL_UV, color: [1.0; 4], additive: false });
        };
        quad(self.screen, [0.0, 0.0, 1.0, 1.0]);
        quad(self.fill, [FILL[0], FILL[1], FILL[2] * self.progress(), FILL[3]]);
        quad(self.border, BORDER);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_continents_have_their_screens_in_the_game_files() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let table = |name: &str| Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc")).unwrap()).unwrap();
        let (maps, screens) = (table("Map"), table("LoadingScreens"));
        for (map, file) in [("Azeroth", "LoadScreenEasternKingdom"), ("Kalimdor", "LoadScreenKalimdor")] {
            let id = maps.iter().find(|r| r.string(1) == map).unwrap().u32(38);
            let path = screens.find_by_id(id).unwrap().string(2).to_string();
            assert!(path.ends_with(&format!("{file}.blp")), "{map}: {path}");
            assert!(mount.read(&path).is_ok());
        }
        assert!(mount.read(r"Interface\Glues\loading.blp").is_ok());
    }

    #[test]
    fn the_bar_adds_up_like_the_originals() {
        let mut l = Loading { screen: 0, fill: 0, border: 0, stages: [0.0; 3], peak: 0, hold: 0.0 };
        l.report(Stage::Terrain, 1.0);
        assert!((l.progress() - 0.7).abs() < 1e-6);
        l.report(Stage::Objects, 1.0);
        assert!((l.progress() - 1.0).abs() < 1e-6);
        l.report(Stage::Interface, 1.0);
        assert!((l.progress() - 1.0).abs() < 1e-6);
        l.report(Stage::Interface, 0.5);
        assert_eq!(l.stages[0], 1.0);
        // 16:9 is pillarboxed to 4:3; 5:4 is letterboxed.
        let [x, y, w, h] = Loading::viewport((1920, 1080));
        assert_eq!((x, y, w, h), (240.0, 0.0, 1440.0, 1080.0));
        let [x, y, w, h] = Loading::viewport((1280, 1024));
        assert!((x, w) == (0.0, 1280.0) && (y - 32.0).abs() < 1e-3 && (h - 960.0).abs() < 1e-3);
    }
}
