//! The mouse pointer of the original client: its own pictures (`Interface\Cursor`), drawn by the game where the mouse is, and
//! changing with what is under it: the gauntlet over nothing, the sword over an enemy, the speech bubble over someone to
//! talk to, the bag over a corpse with loot or a ware to buy, the padlock over a locked chest.

use crate::renderer::Gpu;
use crate::ui::{Quad, Ui, FULL_UV};
use anyhow::Result;
use mpq::Mount;
use wowfmt::blp::Blp;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Point,
    Attack,
    Speak,
    Interact,
    LootAll,
    PickLock,
    Buy,
    Mine,
    Herb,
}

const FILES: [&str; 9] = ["Point", "Attack", "Speak", "Interact", "LootAll", "PickLock", "Buy", "Mine", "GatherHerbs"];

/// What the pointer is over in the world (a creature, a chest or a prop to pick up, by number), for the click that follows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hover {
    Creature(usize),
    Chest(usize),
    Pickup(usize),
    Node(usize),
}

/// The pointer's pictures are 32 pixels square, whatever the size of the window, with the tip at the top left.
const SIZE: f32 = 32.0;

/// The picture of the aiming dot (a soft glow of the spells) and of the ring that shows how near creatures are to noticing a sneaking player
/// (the glowing ring of the item borders).
const GLOW: &str = r"SPELLS\White-Circle.blp";
const RING: &str = r"Interface\Buttons\IconBorder-GlowRing.blp";

pub struct Cursors {
    textures: [usize; FILES.len()],
    glow: usize,
    ring: usize,
}

impl Cursors {
    pub fn load(mount: &Mount, ui: &mut Ui, gpu: &Gpu) -> Result<Cursors> {
        let mut textures = [0; FILES.len()];
        for (slot, name) in textures.iter_mut().zip(FILES) {
            let blp = Blp::parse(&mount.read(&format!(r"Interface\Cursor\{name}.blp"))?)?;
            *slot = ui.add_texture(gpu, blp.width, blp.height, &blp.mips[0]);
        }
        let mut add = |file: &str| -> Result<usize> {
            let blp = Blp::parse(&mount.read(file)?)?;
            Ok(ui.add_texture(gpu, blp.width, blp.height, &blp.mips[0]))
        };
        Ok(Cursors { textures, glow: add(GLOW)?, ring: add(RING)? })
    }

    /// Queue the pointer of kind `kind` with its tip at `at` (pixels). Call last, so that it is over everything.
    pub fn draw(&self, ui: &mut Ui, kind: Kind, at: (f32, f32)) {
        ui.quad(Quad { texture: self.textures[kind as usize], rect: [at.0, at.1, SIZE, SIZE], uv: FULL_UV, color: [1.0; 4], additive: false });
    }
}

impl Cursors {
    /// The aim at the middle of the screen, for while the mouse looks around (the pointer is hidden then): a small soft glow, added to the
    /// picture like the lights of the spells. While sneaking it is smaller and cool, with a ring about it that opens and reddens as creatures
    /// come to notice the player (`alert` from 0 to 1).
    pub fn reticle(&self, ui: &mut Ui, size: (u32, u32), sneaking: Option<f32>) {
        let (cx, cy) = (size.0 as f32 / 2.0, size.1 as f32 / 2.0);
        let u = size.1 as f32 / 720.0;
        let mut glow = |texture: usize, side: f32, color: [f32; 4]| {
            let side = side * u;
            ui.quad(Quad { texture, rect: [cx - side / 2.0, cy - side / 2.0, side, side], uv: FULL_UV, color, additive: true });
        };
        let Some(alert) = sneaking else {
            glow(self.glow, 26.0, [1.0, 0.82, 0.1, 1.0]);
            return;
        };
        // From a cool white to red.
        let tint = |a: f32| [0.7 + 0.3 * a, 0.85 - 0.6 * a, 1.0 - 0.8 * a];
        let [r, g, b] = tint(alert);
        glow(self.glow, 16.0 + 6.0 * alert, [r, g, b, 0.8]);
        glow(self.ring, 20.0 + 22.0 * alert, [r, g, b, 0.35 + 0.65 * alert]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointers_are_in_the_game_files_and_the_kinds_name_them_in_order() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        for name in FILES {
            let blp = Blp::parse(&mount.read(&format!(r"Interface\Cursor\{name}.blp")).unwrap()).unwrap();
            assert_eq!((blp.width, blp.height), (32, 32), "{name}");
        }
        assert_eq!((FILES[Kind::Point as usize], FILES[Kind::Attack as usize], FILES[Kind::Buy as usize]), ("Point", "Attack", "Buy"));
    }
}
