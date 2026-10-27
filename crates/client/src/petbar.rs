//! The pet command bar of the original (`PetActionBarFrame.xml`): the bar art with ten 30-unit slots, shown over the action bar while the player has a pet. Slots 1-3 are
//! the orders every pet has (attack, follow, stay), slots 4-7 the pet's own spells, as many as it was taught, one after the other (the slots left over show nothing),
//! slots 8-10 its stances (aggressive, defensive, passive). The bar is drawn smaller than the original's and stands at the bottom left of the screen, left of the action
//! bar's gryphon.

use crate::creature::{PetCommand, PetOrders, Stance};
use crate::renderer::Gpu;
use crate::ui::{Quad, Ui, FULL_UV};
use mpq::Mount;
use wowfmt::blp::Blp;

const BAR: &str = r"Interface\PetActionBar\UI-PetBar";
const SLOT_FILLED: &str = r"Interface\Buttons\UI-Quickslot2";
const HIGHLIGHT: &str = r"Interface\Buttons\ButtonHilight-Square";
const CHECKED: &str = r"Interface\Buttons\CheckButtonHilight";
const AUTOCASTABLE: &str = r"Interface\Buttons\UI-AutoCastableOverlay";

/// The orders and stances: the slot of the bar they are in (0-9), what they order, and their icon (`PET_*_TEXTURE` of the original).
const BUTTONS: [(usize, PetCommand, &str); 6] = [
    (0, PetCommand::Attack, r"Interface\Icons\Ability_GhoulFrenzy"),
    (1, PetCommand::Follow, r"Interface\Icons\Ability_Tracking"),
    (2, PetCommand::Stay, r"Interface\Icons\Spell_Nature_TimeStop"),
    (7, PetCommand::Stance(Stance::Aggressive), r"Interface\Icons\Ability_Racial_BloodRage"),
    (8, PetCommand::Stance(Stance::Defensive), r"Interface\Icons\Ability_Defend"),
    (9, PetCommand::Stance(Stance::Passive), r"Interface\Icons\Ability_Seal"),
];
/// The first slot for the pet's spells (the fourth).
const FIRST_SPELL: usize = 3;

const BUTTON: f32 = 30.0;
/// The bar is drawn this much smaller than the original's.
const SCALE: f32 = 0.7;
/// Where the slots are from the left edge of the bar (`PetActionButton1`-`10`: the first 36 in, 8 between them, 7 between the sixth and the seventh).
const SLOT_X: [f32; 10] = [36.0, 74.0, 112.0, 150.0, 188.0, 226.0, 263.0, 301.0, 339.0, 377.0];
/// The bar's bottom, where its right end is (left of the action bar's left gryphon, which spans 128 units centred 288 left of the middle), and its width: the art of the
/// original's two pieces, 256 and 184 wide. The buttons stand 2 above the bottom.
const BOTTOM: f32 = 0.0;
const RIGHT: f32 = -350.0;
const WIDTH: f32 = 440.0;
/// Each piece is (width, u0, u1, v0, v1) in the texture.
const ART: [(f32, f32, f32, f32, f32); 2] = [(256.0, 0.0, 1.0, 0.015625, 0.359375), (184.0, 0.0, 0.71875, 0.375, 0.71875)];

pub struct PetBar {
    white: usize,
    bar: Option<usize>,
    slot_filled: Option<usize>,
    highlight: Option<usize>,
    checked: Option<usize>,
    autocastable: Option<usize>,
    icons: [Option<usize>; 6],
    spell_icons: Vec<Option<usize>>,
}

impl PetBar {
    pub fn load(mount: &Mount, ui: &mut Ui, gpu: &Gpu) -> PetBar {
        let mut texture = |path: &str| -> Option<usize> {
            let blp = mount.read(&format!("{path}.blp")).map_err(anyhow::Error::from).and_then(|d| Ok(Blp::parse(&d)?)).map_err(|e| log::warn!("interface texture {path}: {e:#}")).ok()?;
            Some(ui.add_texture(gpu, blp.width, blp.height, &blp.mips[0]))
        };
        let (bar, slot_filled, highlight, checked, autocastable) = (texture(BAR), texture(SLOT_FILLED), texture(HIGHLIGHT), texture(CHECKED), texture(AUTOCASTABLE));
        let icons = BUTTONS.map(|(_, _, path)| texture(path));
        let spell_icons = crate::special::PET_BAR_SPELLS
            .iter()
            .map(|name| crate::content::get().classic_spells.iter().find(|s| s.name == *name).and_then(|s| texture(&format!(r"Interface\Icons\{}", s.icon))))
            .collect();
        PetBar { white: ui.add_texture(gpu, 1, 1, &[255; 4]), bar, slot_filled, highlight, checked, autocastable, icons, spell_icons }
    }

    /// Left edge of the bar (from the screen centre) in UI units: the screen's edge if there is not room for it.
    fn origin(size: (u32, u32)) -> f32 {
        (RIGHT - WIDTH * SCALE).max(-(size.0 as f32 / size.1 as f32) * crate::uixml::ui_height() / 2.0)
    }

    /// Left edge (from the screen centre) of a slot's button, in UI units.
    fn left(slot: usize, size: (u32, u32)) -> f32 {
        Self::origin(size) + SLOT_X[slot] * SCALE
    }

    /// Every button there is: its slot and what it orders. `spells` are the places (in `PET_BAR_SPELLS`) of the spells the pet has, which fill the slots after the orders.
    fn slots(spells: &[usize]) -> impl Iterator<Item = (usize, PetCommand)> + '_ {
        BUTTONS.iter().map(|b| (b.0, b.1)).chain(spells.iter().enumerate().map(|(n, &spell)| (FIRST_SPELL + n, PetCommand::Spell(spell))))
    }

    /// The order of the button under a pixel position.
    pub fn button_at(pos: (f32, f32), size: (u32, u32), spells: &[usize]) -> Option<PetCommand> {
        let s = size.1 as f32 / crate::uixml::ui_height();
        let (x, y) = ((pos.0 - size.0 as f32 / 2.0) / s, (size.1 as f32 - pos.1) / s);
        Self::slots(spells).find(|&(slot, _)| (Self::left(slot, size)..Self::left(slot, size) + BUTTON * SCALE).contains(&x) && (BOTTOM + 2.0 * SCALE..BOTTOM + (2.0 + BUTTON) * SCALE).contains(&y)).map(|b| b.1)
    }

    /// Whether a pixel position is on the bar (a click there is not a click on the world).
    pub fn covers(pos: (f32, f32), size: (u32, u32)) -> bool {
        let s = size.1 as f32 / crate::uixml::ui_height();
        let (x, y) = ((pos.0 - size.0 as f32 / 2.0) / s, (size.1 as f32 - pos.1) / s);
        (Self::origin(size)..Self::origin(size) + WIDTH * SCALE).contains(&x) && (BOTTOM..BOTTOM + 44.0 * SCALE).contains(&y)
    }

    /// Whether `command` is the one in force (`phased`: the pet is out of phase).
    fn active(command: PetCommand, orders: PetOrders, phased: bool) -> bool {
        match command {
            PetCommand::Spell(n) => phased && crate::special::PET_BAR_SPELLS[n] == "Phase Shift",
            PetCommand::Attack => orders.attack.is_some(),
            PetCommand::Stay => orders.stay,
            PetCommand::Follow => false,
            PetCommand::Stance(stance) => orders.stance == stance,
        }
    }

    /// Queue the bar for drawing; a dead pet's buttons are dark.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, ui: &mut Ui, size: (u32, u32), orders: PetOrders, dead: bool, spells: &[usize], phased: bool, cursor: Option<(f32, f32)>) {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let s = h / crate::uixml::ui_height();
        let rect = |x: f32, y: f32, sw: f32, sh: f32| [w / 2.0 + x * s, h - (y + sh) * s, sw * s, sh * s];
        let mut quad = |texture: Option<usize>, rect: [f32; 4], uv: [f32; 4], color: [f32; 4], additive: bool| {
            if let Some(texture) = texture {
                ui.quad(Quad { texture, rect, uv, color, additive });
            }
        };
        let mut x = Self::origin(size);
        for (art_w, u0, u1, v0, v1) in ART {
            quad(self.bar, rect(x, BOTTOM, art_w * SCALE, 44.0 * SCALE), [u0, v0, u1, v1], [1.0; 4], false);
            x += art_w * SCALE;
        }
        let hover = cursor.and_then(|c| Self::button_at(c, size, spells));
        for (slot, command) in Self::slots(spells) {
            let button = rect(Self::left(slot, size), BOTTOM + 2.0 * SCALE, BUTTON * SCALE, BUTTON * SCALE);
            let icon = match command {
                PetCommand::Spell(n) => self.spell_icons.get(n).copied().flatten(),
                _ => BUTTONS.iter().position(|b| b.1 == command).and_then(|i| self.icons[i]),
            };
            quad(icon, button, FULL_UV, [1.0; 4], false);
            if dead {
                quad(Some(self.white), button, FULL_UV, [0.0, 0.0, 0.0, 0.6], false);
            }
            // The slot frame is 54 units, one unit below the button's centre.
            quad(self.slot_filled, rect(Self::left(slot, size) + (BUTTON / 2.0 - 27.0) * SCALE, BOTTOM + (2.0 + BUTTON / 2.0 - 1.0 - 27.0) * SCALE, 54.0 * SCALE, 54.0 * SCALE), FULL_UV, [1.0; 4], false);
            // The firebolt is a spell that the pet casts by itself: it wears the overlay of those that can be (`$parentAutoCastable`, 58 units).
            if command == PetCommand::Spell(0) {
                quad(self.autocastable, rect(Self::left(slot, size) + (BUTTON / 2.0 - 29.0) * SCALE, BOTTOM + (2.0 + BUTTON / 2.0 - 29.0) * SCALE, 58.0 * SCALE, 58.0 * SCALE), FULL_UV, [1.0; 4], false);
            }
            if hover == Some(command) {
                quad(self.highlight, button, FULL_UV, [1.0; 4], true);
            }
            if Self::active(command, orders, phased) {
                quad(self.checked, button, FULL_UV, [1.0; 4], true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_find_the_button_under_the_pointer() {
        // At 1920x1080 a unit is 1.40625 pixels: the bar is 440 units wide (drawn at `SCALE`) ending 350 left of the middle, its buttons 36 units in, 2 up and 30 wide.
        let size = (1920, 1080);
        let s = 1080.0 / crate::uixml::ui_height();
        let at = |slot: usize, y: f32| (960.0 + (-350.0 - WIDTH * SCALE + (SLOT_X[slot] + 15.0) * SCALE) * s, 1080.0 - y * s);
        assert_eq!(PetBar::button_at(at(0, 10.0), size, &[]), Some(PetCommand::Attack));
        assert_eq!(PetBar::button_at(at(9, 10.0), size, &[]), Some(PetCommand::Stance(Stance::Passive)));
        assert_eq!(PetBar::button_at(at(7, 10.0), size, &[]), Some(PetCommand::Stance(Stance::Aggressive)));
        // The gap between buttons, above and below the buttons and the rest of the screen hit nothing; the bar's art covers its buttons; an unfilled slot is nothing.
        assert_eq!(PetBar::button_at((at(0, 10.0).0 + 21.0 * SCALE * s, at(0, 10.0).1), size, &[]), None);
        assert_eq!(PetBar::button_at(at(0, 60.0), size, &[]), None);
        assert_eq!(PetBar::button_at(at(3, 10.0), size, &[]), None);
        assert_eq!(PetBar::button_at((960.0, 1060.0), size, &[]), None);
        assert!(PetBar::covers(at(2, 10.0), size) && PetBar::covers(at(4, 20.0), size) && !PetBar::covers((960.0, 1060.0), size));
        // The spells the pet has fill the slots after the orders, in the order of the bar.
        assert_eq!(PetBar::button_at(at(3, 10.0), size, &[0, 3]), Some(PetCommand::Spell(0)));
        assert_eq!(PetBar::button_at(at(4, 10.0), size, &[0, 3]), Some(PetCommand::Spell(3)));
        assert_eq!(PetBar::button_at(at(5, 10.0), size, &[0, 3]), None);
        // The bar ends left of the gryphon; a screen too narrow for it has the bar start at its edge.
        assert_eq!(PetBar::origin(size), -350.0 - WIDTH * SCALE);
        assert_eq!(PetBar::origin((800, 800)), -400.0 * crate::uixml::ui_height() / 800.0);
    }
}
