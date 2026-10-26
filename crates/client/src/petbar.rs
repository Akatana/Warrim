//! The pet command bar of the original (`PetActionBarFrame.xml`): the bar art with 30-unit buttons, shown over the action bar while the player has a pet.
//! The buttons are the ones the original gives every pet in its slots 1-3 and 8-10: attack, follow, stay, and the aggressive, defensive and passive stances.
//! (The slots between are for the pet's own spells, which this pet casts by itself, so the six buttons sit side by side.) The bar stands at the bottom left of the screen,
//! left of the action bar's gryphon, with the art of the original's ends shortened to fit.

use crate::creature::{PetCommand, PetOrders, Stance};
use crate::ui::{Quad, Ui, FULL_UV};
use crate::renderer::Gpu;
use mpq::Mount;
use wowfmt::blp::Blp;

const BAR: &str = r"Interface\PetActionBar\UI-PetBar";
const SLOT_FILLED: &str = r"Interface\Buttons\UI-Quickslot2";
const HIGHLIGHT: &str = r"Interface\Buttons\ButtonHilight-Square";
const CHECKED: &str = r"Interface\Buttons\CheckButtonHilight";

/// The buttons: the slot of the bar they are in (0-5), what they order, and their icon (`PET_*_TEXTURE` of the original).
const BUTTONS: [(usize, PetCommand, &str); 6] = [
    (0, PetCommand::Attack, r"Interface\Icons\Ability_GhoulFrenzy"),
    (1, PetCommand::Follow, r"Interface\Icons\Ability_Tracking"),
    (2, PetCommand::Stay, r"Interface\Icons\Spell_Nature_TimeStop"),
    (3, PetCommand::Stance(Stance::Aggressive), r"Interface\Icons\Ability_Racial_BloodRage"),
    (4, PetCommand::Stance(Stance::Defensive), r"Interface\Icons\Ability_Defend"),
    (5, PetCommand::Stance(Stance::Passive), r"Interface\Icons\Ability_Seal"),
];

const BUTTON: f32 = 30.0;
const STRIDE: f32 = 38.0;
/// The bar's bottom, and where its right end is: left of the action bar's left gryphon (which spans 128 units centred 288 left of the middle).
const BOTTOM: f32 = 0.0;
const RIGHT: f32 = -350.0;
/// The first button is 36 units in and 2 up. The art is the original's: its first piece (256 wide, 44 high), then the last 36 units of its second piece, which are its end.
/// Each is (width, u0, u1, v0, v1) in the texture.
const FIRST_BUTTON: f32 = 36.0;
const WIDTH: f32 = 292.0;
const ART: [(f32, f32, f32, f32, f32); 2] = [(256.0, 0.0, 1.0, 0.015625, 0.359375), (36.0, 0.578125, 0.71875, 0.375, 0.71875)];

pub struct PetBar {
    white: usize,
    bar: Option<usize>,
    slot_filled: Option<usize>,
    highlight: Option<usize>,
    checked: Option<usize>,
    icons: [Option<usize>; 6],
}

impl PetBar {
    pub fn load(mount: &Mount, ui: &mut Ui, gpu: &Gpu) -> PetBar {
        let mut texture = |path: &str| -> Option<usize> {
            let blp = mount.read(&format!("{path}.blp")).map_err(anyhow::Error::from).and_then(|d| Ok(Blp::parse(&d)?)).map_err(|e| log::warn!("interface texture {path}: {e:#}")).ok()?;
            Some(ui.add_texture(gpu, blp.width, blp.height, &blp.mips[0]))
        };
        let (bar, slot_filled, highlight, checked) = (texture(BAR), texture(SLOT_FILLED), texture(HIGHLIGHT), texture(CHECKED));
        let icons = BUTTONS.map(|(_, _, path)| texture(path));
        PetBar { white: ui.add_texture(gpu, 1, 1, &[255; 4]), bar, slot_filled, highlight, checked, icons }
    }

    /// Left edge of the bar (from the screen centre) in UI units: the screen's edge if there is not room for it.
    fn origin(size: (u32, u32)) -> f32 {
        (RIGHT - WIDTH).max(-(size.0 as f32 / size.1 as f32) * crate::uixml::ui_height() / 2.0)
    }

    /// Left edge (from the screen centre) of a slot's button, in UI units.
    fn left(slot: usize, size: (u32, u32)) -> f32 {
        Self::origin(size) + FIRST_BUTTON + slot as f32 * STRIDE
    }

    /// The order of the button under a pixel position.
    pub fn button_at(pos: (f32, f32), size: (u32, u32)) -> Option<PetCommand> {
        let s = size.1 as f32 / crate::uixml::ui_height();
        let (x, y) = ((pos.0 - size.0 as f32 / 2.0) / s, (size.1 as f32 - pos.1) / s);
        BUTTONS.iter().find(|(slot, ..)| (Self::left(*slot, size)..Self::left(*slot, size) + BUTTON).contains(&x) && (BOTTOM + 2.0..BOTTOM + 2.0 + BUTTON).contains(&y)).map(|b| b.1)
    }

    /// Whether a pixel position is on the bar (a click there is not a click on the world).
    pub fn covers(pos: (f32, f32), size: (u32, u32)) -> bool {
        let s = size.1 as f32 / crate::uixml::ui_height();
        let (x, y) = ((pos.0 - size.0 as f32 / 2.0) / s, (size.1 as f32 - pos.1) / s);
        (Self::origin(size)..Self::origin(size) + WIDTH).contains(&x) && (BOTTOM..BOTTOM + 44.0).contains(&y)
    }

    /// Whether `command` is the one in force.
    fn active(command: PetCommand, orders: PetOrders) -> bool {
        match command {
            PetCommand::Attack => orders.attack.is_some(),
            PetCommand::Stay => orders.stay,
            PetCommand::Follow => false,
            PetCommand::Stance(stance) => orders.stance == stance,
        }
    }

    /// Queue the bar for drawing; a dead pet's buttons are dark.
    pub fn draw(&self, ui: &mut Ui, size: (u32, u32), orders: PetOrders, dead: bool, cursor: Option<(f32, f32)>) {
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
            quad(self.bar, rect(x, BOTTOM, art_w, 44.0), [u0, v0, u1, v1], [1.0; 4], false);
            x += art_w;
        }
        let hover = cursor.and_then(|c| Self::button_at(c, size));
        for (i, (slot, command, _)) in BUTTONS.iter().enumerate() {
            let button = rect(Self::left(*slot, size), BOTTOM + 2.0, BUTTON, BUTTON);
            quad(self.icons[i], button, FULL_UV, [1.0; 4], false);
            if dead {
                quad(Some(self.white), button, FULL_UV, [0.0, 0.0, 0.0, 0.6], false);
            }
            // The slot frame is 54 units, one unit below the button's centre.
            quad(self.slot_filled, rect(Self::left(*slot, size) + BUTTON / 2.0 - 27.0, BOTTOM + 2.0 + BUTTON / 2.0 - 1.0 - 27.0, 54.0, 54.0), FULL_UV, [1.0; 4], false);
            if hover == Some(*command) {
                quad(self.highlight, button, FULL_UV, [1.0; 4], true);
            }
            if Self::active(*command, orders) {
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
        // At 1920x1080 a unit is 1.40625 pixels: the bar is 292 units wide ending 350 left of the middle, its first button 36 units in and 4 up.
        let size = (1920, 1080);
        let (s, left) = (1080.0 / crate::uixml::ui_height(), -350.0 - 292.0);
        let at = |slot: f32, y: f32| (960.0 + (left + 36.0 + slot * 38.0 + 15.0) * s, 1080.0 - y * s);
        assert_eq!(PetBar::button_at(at(0.0, 20.0), size), Some(PetCommand::Attack));
        assert_eq!(PetBar::button_at(at(5.0, 20.0), size), Some(PetCommand::Stance(Stance::Passive)));
        // The gap between buttons, above the buttons and the rest of the screen hit nothing; the bar's art covers its buttons.
        assert_eq!(PetBar::button_at((at(0.0, 20.0).0 + 21.0 * s, at(0.0, 20.0).1), size), None);
        assert_eq!(PetBar::button_at(at(0.0, 60.0), size), None);
        assert_eq!(PetBar::button_at((960.0, 1060.0), size), None);
        assert!(PetBar::covers(at(2.0, 20.0), size) && !PetBar::covers((960.0, 1060.0), size));
        // A narrow screen has no room: the bar starts at its edge.
        assert_eq!(PetBar::origin((1024, 768)), -(1024.0 / 768.0) * crate::uixml::ui_height() / 2.0);
    }
}
