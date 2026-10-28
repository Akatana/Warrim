//! The action buttons of the original main bar, laid out from `MainMenuBar.xml` and `ActionButtonTemplate.xml`:
//! the left half of the bar art (the part behind the buttons) and twelve 36-unit action buttons (empty or filled
//! slot frame, icon, pressed / hover / active states, hotkey text), centred on the screen, with the gryphon end
//! caps at its ends and the frame of the experience bar above it, which in turn has the health, mana and
//! stamina bars above it between the gryphons. The right half of the bar is left out. Keys 1-0, - and = (or a click while the cursor is
//! free) use the slots; the buttons hold the two things the player can do so far, the sword swing and the shield
//! block (held for as long as the button is down).

use crate::audio::Sfx;
use crate::font::GlyphSet;
use crate::levels::Progress;
use crate::player::Player;
use crate::renderer::Gpu;
use crate::ui::{Quad, Ui, FULL_UV};
use anyhow::{Context, Result};
use mpq::Mount;
use std::collections::HashMap;
use wowfmt::blp::Blp;

const BAR: &str = r"Interface\MainMenuBar\UI-MainMenuBar-Human";
const END_CAP: &str = r"Interface\MainMenuBar\UI-MainMenuBar-EndCap-Human";
const STATUS_BAR: &str = r"Interface\TargetingFrame\UI-StatusBar";
const SLOT_FILLED: &str = r"Interface\Buttons\UI-Quickslot2";
const SLOT_EMPTY: &str = r"Interface\Buttons\UI-Quickslot";
const PUSHED: &str = r"Interface\Buttons\UI-Quickslot-Depress";
const HIGHLIGHT: &str = r"Interface\Buttons\ButtonHilight-Square";
const CHECKED: &str = r"Interface\Buttons\CheckButtonHilight";
const FONT: &str = r"Fonts\ARIALN.TTF";

const SLOTS: usize = 12;
/// The pages of buttons (the original has six); Shift and the mouse wheel turn through them.
const PAGES: usize = 6;
const BUTTON: f32 = 36.0;
const BUTTON_STRIDE: f32 = 42.0;
/// The bar's left half spans 512 units; centred on the screen, its first button starts 8 units in.
const FIRST_BUTTON: f32 = -256.0 + 8.0;
const BUTTON_BOTTOM: f32 = 4.0;
/// Hotkey text is rendered at this multiple of its 12 unit size so it stays sharp when the UI is scaled up.
const TEXT_SCALE: f32 = 2.0;
/// The bar art behind the buttons: for each of its two 256-unit segments, the offset of its centre from the
/// screen centre and its v range in the atlas (the bar is 43 units high).
const BAR_SEGMENTS: [(f32, f32, f32); 2] = [(-128.0, 0.83203125, 1.0), (128.0, 0.58203125, 0.75)];
/// The colour of the experience bar's fill (`MainMenuExpBar`: purple when not rested).
const XP_COLOR: [f32; 4] = [0.58, 0.0, 0.55, 1.0];
/// The same for the experience bar's frame above it (10 units high).
const XP_SEGMENTS: [(f32, f32, f32); 2] = [(-128.0, 0.79296875, 0.83203125), (128.0, 0.54296875, 0.58203125)];

/// The health, mana and stamina bars (119 x 12 like the original's): width and height in units, gap between them, height of their bottom above
/// the screen bottom (just over the experience bar), and the fill colours.
const VITAL_SIZE: (f32, f32) = (119.0, 12.0);
const VITAL_GAP: f32 = 8.0;
const VITAL_BOTTOM: f32 = 57.0;
// The original's player frame bars: health green (`HealthBar_OnValueChanged`), mana blue (`ManaBarColor[0]`), and the
// third bar in the energy yellow (`ManaBarColor[3]`).
pub const HEALTH_COLOR: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
pub const MANA_COLOR: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
pub const STAMINA_COLOR: [f32; 4] = [1.0, 1.0, 0.0, 1.0];
/// The text on a bar is `TextStatusBarText`: ARIALN at height 14 with an outline.
const BAR_TEXT_HEIGHT: f32 = 14.0;
/// The count on an item's button is a little smaller than the text on the bars.
const COUNT_SCALE: f32 = 0.85;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Attack,
    Block,
    /// A heavy swing at once (holding the attack button does the same).
    Heavy,
    /// The bow or gun: drawn for as long as the button is held, and let go when it comes up.
    Shoot,
    /// A spell (an index into the content's spells).
    Spell(usize),
    /// A usable item (an index into the items): the button uses it from the backpack, like a right click.
    Item(usize),
    /// A profession the spellbook lists: a click opens its recipes. It is not put on the bar.
    Trade(crate::skills::Skill),
    /// Breaking an item of the pack into dust and shards (Enchanting). Not put on the bar either.
    Disenchant,
}

/// The actions every character has, which the spellbook lists before its spells.
pub const BASIC: [Action; 4] = [Action::Attack, Action::Block, Action::Heavy, Action::Shoot];

impl Action {
    /// Where its icon is in the bar's list of them: the sword, the shield, then the spells in the order of the content.
    fn icon_index(self) -> Option<usize> {
        Some(match self {
            Action::Attack => 0,
            Action::Block => 1,
            Action::Heavy => 2,
            Action::Shoot => 3,
            Action::Spell(i) => 4 + i,
            Action::Item(_) | Action::Trade(_) | Action::Disenchant => return None,
        })
    }

    /// The icon of a basic action (a spell has the one its content gives).
    pub fn basic_icon(self) -> Option<&'static str> {
        ICONS.get(self.icon_index()?).copied()
    }

    /// The name and what it does, for the spellbook; `None` for a spell, which has its own.
    pub fn basic_info(self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Action::Attack => ("Attack", "Swing your weapon. Hold the button for a heavy swing."),
            Action::Block => ("Block", "Raise your shield for as long as the button is held."),
            Action::Heavy => ("Heavy Attack", "A slow, powerful swing."),
            Action::Shoot => ("Shoot", "Draw your ranged weapon for as long as the button is held, and let fly when it comes up."),
            Action::Spell(_) | Action::Item(_) | Action::Trade(_) | Action::Disenchant => return None,
        })
    }

    /// The id it is saved by: a spell's own, or the name of a basic action.
    fn id(self) -> String {
        match self {
            Action::Attack => "attack".into(),
            Action::Block => "block".into(),
            Action::Heavy => "heavy".into(),
            Action::Shoot => "shoot".into(),
            Action::Spell(i) => crate::content::spell(i).id.clone(),
            Action::Item(i) => format!("item:{}", crate::items::get(i).id),
            Action::Trade(s) => format!("trade:{}", s.id()),
            Action::Disenchant => "disenchant".into(),
        }
    }

    fn from_id(id: &str) -> Option<Action> {
        if let Some(item) = id.strip_prefix("item:") {
            return crate::items::all().iter().position(|i| i.id == item).map(Action::Item);
        }
        BASIC.into_iter().find(|a| a.id() == id).or_else(|| crate::content::get().spells.iter().position(|s| s.id == id).map(Action::Spell))
    }
}

const ICONS: [&str; 4] =[r"Interface\Icons\Ability_SteelMelee", r"Interface\Icons\Ability_Defend", r"Interface\Icons\Ability_Warrior_SavageBlow", r"Interface\Icons\Ability_Marksmanship"];

/// A hotkey label: its texture and size in UI units.
struct Label {
    texture: usize,
    size: (f32, f32),
}

pub struct ActionBar {
    slots: [[Option<Action>; SLOTS]; PAGES],
    /// The page the buttons shown and used belong to.
    page: usize,
    /// Slots whose key or mouse button is down.
    held: [bool; SLOTS],
    /// The mouse pointer in pixels, while it is free to use the interface.
    cursor: Option<(f32, f32)>,
    /// The action being dragged from the spellbook, drawn at the pointer.
    dragging: Option<Action>,
    white: usize,
    status_bar: Option<usize>,
    bar: Option<usize>,
    end_cap: Option<usize>,
    slot_filled: Option<usize>,
    slot_empty: Option<usize>,
    pushed: Option<usize>,
    highlight: Option<usize>,
    checked: Option<usize>,
    icons: Vec<Option<usize>>,
    /// The icons of the items that can be used, by item.
    item_icons: HashMap<usize, Option<usize>>,
    /// The item a button asked to use, for the backpack to use.
    item_use: Option<usize>,
    labels: Vec<Option<Label>>,
    font: fontdue::Font,
    bar_text: GlyphSet,
}

/// The page after `page` (or before it), round the ends.
fn turned(page: usize, forward: bool) -> usize {
    (page + if forward { 1 } else { PAGES - 1 }) % PAGES
}

/// The pictures of the keys written on the buttons.
fn hotkey_labels(font: &fontdue::Font, ui: &mut Ui, gpu: &Gpu, keys: &[String; SLOTS]) -> Vec<Option<Label>> {
    keys.iter()
        .map(|text| {
            let (w, h, rgba) = render_text(font, text, 12.0 * TEXT_SCALE);
            Some(Label { texture: ui.add_texture(gpu, w, h, &rgba), size: (w as f32 / TEXT_SCALE, h as f32 / TEXT_SCALE) })
        })
        .collect()
}

impl ActionBar {
    /// The keys changed: write the new ones on the buttons.
    pub fn set_hotkeys(&mut self, ui: &mut Ui, gpu: &Gpu, keys: &[String; SLOTS]) {
        self.labels = hotkey_labels(&self.font, ui, gpu, keys);
    }

    pub fn load(mount: &Mount, ui: &mut Ui, gpu: &Gpu) -> Result<ActionBar> {
        let mut texture = |path: &str| -> Option<usize> {
            let blp = mount.read(&format!("{path}.blp")).map_err(anyhow::Error::from).and_then(|d| Ok(Blp::parse(&d)?)).map_err(|e| log::warn!("interface texture {path}: {e:#}")).ok()?;
            Some(ui.add_texture(gpu, blp.width, blp.height, &blp.mips[0]))
        };
        let (bar, end_cap, slot_filled, slot_empty, pushed, highlight, checked) = (texture(BAR), texture(END_CAP), texture(SLOT_FILLED), texture(SLOT_EMPTY), texture(PUSHED), texture(HIGHLIGHT), texture(CHECKED));
        let names: Vec<String> = ICONS.iter().map(|p| p.to_string()).chain(crate::content::get().spells.iter().map(|s| format!(r"Interface\Icons\{}", s.icon))).collect();
        let icons = names.iter().map(|p| texture(p)).collect();
        let item_icons = crate::items::all().iter().enumerate().filter(|(i, _)| crate::consume::Consumed::can_use(*i)).map(|(i, d)| (i, texture(d.icon))).collect();
        let status_bar = texture(STATUS_BAR);
        let font = fontdue::Font::from_bytes(mount.read(FONT)?, fontdue::FontSettings::default()).map_err(|e| anyhow::anyhow!("{FONT}: {e}")).context("action bar font")?;
        let labels = hotkey_labels(&font, ui, gpu, &crate::keys::Binds::load().action_labels());
        let white = ui.add_texture(gpu, 1, 1, &[255; 4]);
        let mut slots = [[None; SLOTS]; PAGES];
        for (slot, a) in slots[0].iter_mut().zip(BASIC) {
            *slot = Some(a);
        }
        let bar_text = GlyphSet::new(ui, gpu, &font, BAR_TEXT_HEIGHT, 1)?;
        Ok(ActionBar { slots, page: 0, held: [false; SLOTS], cursor: None, dragging: None, white, status_bar, bar, end_cap, slot_filled, slot_empty, pushed, highlight, checked, icons, item_icons, item_use: None, labels, font, bar_text })
    }

    /// Put a spell in the first free button.
    pub fn learn(&mut self, spell: usize) {
        if let Some(free) = self.slots.iter_mut().flatten().find(|s| s.is_none()) {
            *free = Some(Action::Spell(spell));
        }
    }

    /// Put `action` on a button, in place of what was there.
    pub fn place(&mut self, slot: usize, action: Action) {
        if slot < SLOTS {
            self.slots[self.page][slot] = Some(action);
        }
    }

    pub fn set_drag(&mut self, dragging: Option<Action>) {
        self.dragging = dragging;
    }

    /// The action being dragged (from the spellbook or off a button), which a mouse button coming up puts down.
    pub fn take_drag(&mut self) -> Option<Action> {
        self.dragging.take()
    }

    /// Lift the action off a button to drag it: it is not used, and the button is empty until it is put down.
    pub fn pick_up(&mut self, slot: usize) {
        if let Some(a) = self.slots[self.page].get_mut(slot).and_then(Option::take) {
            self.dragging = Some(a);
        }
    }

    /// What is on each button, by id ("" for none), for the saved game.
    pub fn layout(&self) -> Vec<String> {
        self.slots.iter().flatten().map(|s| s.map_or(String::new(), Action::id)).collect()
    }

    /// Put the buttons as saved; ids the content no longer has leave their button empty.
    pub fn restore(&mut self, layout: &[String]) {
        self.slots = [[None; SLOTS]; PAGES];
        for (slot, id) in self.slots.iter_mut().flatten().zip(layout) {
            *slot = Action::from_id(id);
        }
    }

    /// The key or mouse button of `slot` went down. A spell is not cast here: it is returned.
    pub fn press(&mut self, slot: usize, player: &mut Player) -> Option<usize> {
        if slot >= SLOTS || std::mem::replace(&mut self.held[slot], true) {
            return None;
        }
        let mut cast = None;
        match self.slots[self.page][slot] {
            Some(Action::Attack) => player.attack_press(),
            Some(Action::Heavy) => player.heavy_attack(),
            Some(Action::Shoot) => player.shoot_press(),
            Some(Action::Block) => player.set_block(true),
            Some(Action::Spell(i)) => cast = Some(i),
            Some(Action::Item(i)) => self.item_use = Some(i),
            Some(Action::Trade(_) | Action::Disenchant) | None => return None,
        }
        player.emit(Sfx::ActionButton);
        cast
    }

    /// Turn to the next page (`forward`) or the one before, round the ends; held buttons are let go first, since they
    /// would not find their action on the new page.
    pub fn turn_page(&mut self, forward: bool, player: &mut Player) {
        self.release_all(player);
        self.page = turned(self.page, forward);
    }

    /// The item a button asked to use since the last call.
    pub fn take_item_use(&mut self) -> Option<usize> {
        self.item_use.take()
    }

    fn icon(&self, action: Action) -> Option<usize> {
        match action {
            Action::Item(i) => self.item_icons.get(&i).copied().flatten(),
            _ => self.icons.get(action.icon_index()?).copied().flatten(),
        }
    }

    /// The key or mouse button of `slot` went up: a held shield comes down, and a tap on the attack makes its swing.
    pub fn release(&mut self, slot: usize, player: &mut Player) {
        if slot < SLOTS && std::mem::take(&mut self.held[slot]) {
            match self.slots[self.page][slot] {
                Some(Action::Block) => player.set_block(false),
                Some(Action::Attack) => player.attack_release(),
                Some(Action::Shoot) => player.shoot_release(),
                _ => {}
            }
        }
    }

    pub fn release_all(&mut self, player: &mut Player) {
        (0..SLOTS).for_each(|slot| self.release(slot, player));
    }

    /// The pointer position in pixels, or `None` while the mouse looks around instead.
    pub fn set_cursor(&mut self, cursor: Option<(f32, f32)>) {
        self.cursor = cursor;
    }

    /// Left edge (from the screen centre) of a button in UI units.
    fn button_left(slot: usize) -> f32 {
        FIRST_BUTTON + slot as f32 * BUTTON_STRIDE
    }

    /// The button under a pixel position.
    pub fn slot_at(pos: (f32, f32), size: (u32, u32)) -> Option<usize> {
        let s = size.1 as f32 / crate::uixml::ui_height();
        let (x, y) = ((pos.0 - size.0 as f32 / 2.0) / s, (size.1 as f32 - pos.1) / s);
        (0..SLOTS).find(|&i| (Self::button_left(i)..Self::button_left(i) + BUTTON).contains(&x) && (BUTTON_BOTTOM..BUTTON_BOTTOM + BUTTON).contains(&y))
    }

    /// Queue the bar for drawing on a `size` screen. `player` says which actions are active.
    /// `texts`: show the values on the bars (they also show while the mouse is over a bar), as the original does
    /// while the character sheet is open.
    pub fn draw(&self, ui: &mut Ui, size: (u32, u32), player: &Player, texts: bool, progress: Progress, wait: &dyn Fn(usize) -> f32, items: &dyn Fn(usize) -> (u32, f32)) {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let s = h / crate::uixml::ui_height();
        let cx = w / 2.0;
        // A rectangle `sw` x `sh` units whose bottom centre is `ox` from the screen centre and `oy` above the bottom.
        let rect = |ox: f32, oy: f32, sw: f32, sh: f32| [cx + (ox - sw / 2.0) * s, h - (oy + sh) * s, sw * s, sh * s];
        let mut quad = |texture: Option<usize>, rect: [f32; 4], uv: [f32; 4], color: [f32; 4], additive: bool| {
            if let Some(texture) = texture {
                ui.quad(Quad { texture, rect, uv, color, additive });
            }
        };
        const WHITE: [f32; 4] = [1.0; 4];

        // The experience bar: a dark strip, filled in purple up to the experience gathered, with the bar's border over it.
        quad(Some(self.white), rect(0.0, 40.0, 512.0, 13.0), FULL_UV, [0.0, 0.0, 0.0, 0.5], false);
        let mut bar_texts = Vec::new();
        let mut counts = Vec::new();
        if !progress.at_max() {
            let [x, y, bw, bh] = rect(0.0, 40.0, 512.0, 13.0);
            let fill = progress.xp as f32 / progress.to_next() as f32;
            if fill > 0.0 {
                quad(self.status_bar, [x, y, bw * fill, bh], [0.0, 0.0, fill, 1.0], XP_COLOR, false);
            }
            let over = self.cursor.is_some_and(|(cx, cy)| cx >= x && cx < x + bw && cy >= y && cy < y + bh);
            if texts || over {
                bar_texts.push((format!("XP {} / {}", progress.xp, progress.to_next()), x + bw / 2.0, y + bh / 2.0));
            }
        }
        for (x, v0, v1) in XP_SEGMENTS {
            quad(self.bar, rect(x, 43.0, 256.0, 10.0), [0.0, v0, 1.0, v1], WHITE, false);
        }
        for (x, v0, v1) in BAR_SEGMENTS {
            quad(self.bar, rect(x, 0.0, 256.0, 43.0), [0.0, v0, 1.0, v1], WHITE, false);
        }
        // The gryphons sit 32 units outside the bar's ends, the right one mirrored.
        quad(self.end_cap, rect(-256.0 - 32.0, 0.0, 128.0, 128.0), FULL_UV, WHITE, false);
        quad(self.end_cap, rect(256.0 + 32.0, 0.0, 128.0, 128.0), [1.0, 0.0, 0.0, 1.0], WHITE, false);

        // Health, mana and stamina side by side above the experience bar: the original's dark backing, then the fill
        // (the status bar texture up to the value, in the bar's colour), then, when asked for, "Health 100 / 100".
        let vitals = player.vitals;
        for (i, (name, resource, color)) in [("Health", vitals.health, HEALTH_COLOR), ("Mana", vitals.mana, MANA_COLOR), ("Stamina", vitals.stamina, STAMINA_COLOR)].into_iter().enumerate() {
            let (w, h) = VITAL_SIZE;
            let centre = (i as f32 - 1.0) * (w + VITAL_GAP);
            let [x, y, bw, bh] = rect(centre, VITAL_BOTTOM, w, h);
            quad(Some(self.white), [x, y, bw, bh], FULL_UV, [0.0, 0.0, 0.0, 0.5], false);
            let fill = resource.fraction();
            if fill > 0.0 {
                quad(self.status_bar, [x, y, bw * fill, bh], [0.0, 0.0, fill, 1.0], color, false);
            }
            let over = self.cursor.is_some_and(|(cx, cy)| cx >= x && cx < x + bw && cy >= y && cy < y + bh);
            if texts || over {
                let text = format!("{name} {} / {}", resource.current.round() as i32, resource.max.round() as i32);
                bar_texts.push((text, x + bw / 2.0, y + bh / 2.0));
            }
        }

        let hover = self.cursor.and_then(|c| Self::slot_at(c, size));
        for slot in 0..SLOTS {
            let centre = Self::button_left(slot) + BUTTON / 2.0;
            let button = rect(centre, BUTTON_BOTTOM, BUTTON, BUTTON);
            let action = self.slots[self.page][slot];
            if let Some(a) = action {
                quad(self.icon(a), button, FULL_UV, WHITE, false);
                // A spell that cannot be cast yet is dark.
                if matches!(a, Action::Spell(i) if wait(i) > 0.0) {
                    quad(Some(self.white), button, FULL_UV, [0.0, 0.0, 0.0, 0.6], false);
                }
                // An item is dark while it is out of the backpack or not ready, and shows how many are left.
                if let Action::Item(i) = a {
                    let (count, cooldown) = items(i);
                    if count == 0 || cooldown > 0.0 {
                        quad(Some(self.white), button, FULL_UV, [0.0, 0.0, 0.0, 0.6], false);
                    }
                    counts.push((count.to_string(), button[0] + button[2] - 2.0 * s, button[1] + button[3] - 9.0 * s));
                }
                // A swing is as dark as it is short of its full damage.
                if matches!(a, Action::Attack | Action::Heavy) {
                    quad(Some(self.white), button, FULL_UV, [0.0, 0.0, 0.0, 0.6 * (1.0 - player.swing_ready())], false);
                }
            }
            // The slot frame is 66 units, one unit below the button's centre.
            let frame = rect(centre, BUTTON_BOTTOM + BUTTON / 2.0 - 1.0 - 33.0, 66.0, 66.0);
            quad(if action.is_some() { self.slot_filled } else { self.slot_empty }, frame, FULL_UV, WHITE, false);
            if self.held[slot] {
                quad(self.pushed, button, FULL_UV, WHITE, false);
            }
            if hover == Some(slot) {
                quad(self.highlight, button, FULL_UV, WHITE, true);
            }
            let active = match action {
                Some(Action::Attack) => player.is_attacking() && !player.is_heavy_swinging(),
                Some(Action::Heavy) => player.is_heavy_swinging(),
                Some(Action::Block) => player.is_blocking(),
                Some(Action::Shoot) => player.is_drawing(),
                Some(Action::Spell(_) | Action::Item(_) | Action::Trade(_) | Action::Disenchant) | None => false,
            };
            if active {
                quad(self.checked, button, FULL_UV, WHITE, true);
            }
            // The hotkey, right-aligned two units in from the button's top right corner.
            if let Some(label) = &self.labels[slot] {
                let (lw, lh) = label.size;
                let colour = if action.is_some() { WHITE } else { [0.6, 0.6, 0.6, 1.0] };
                let x = cx + (centre - BUTTON / 2.0 + 34.0 - lw) * s;
                let y = h - (BUTTON_BOTTOM + BUTTON - 2.0 - (10.0 - lh) / 2.0) * s;
                quad(Some(label.texture), [x, y, lw * s, lh * s], FULL_UV, colour, false);
            }
        }
        for (text, right, cy) in counts {
            let width = self.bar_text.width(&text) * s * COUNT_SCALE;
            self.bar_text.draw(ui, &text, right - width, cy - BAR_TEXT_HEIGHT * s * COUNT_SCALE * 0.62, s * COUNT_SCALE, [1.0; 4]);
        }
        for (text, cx, cy) in bar_texts {
            let width = self.bar_text.width(&text) * s;
            self.bar_text.draw(ui, &text, cx - width / 2.0, cy - BAR_TEXT_HEIGHT * s * 0.62, s, [1.0; 4]);
        }
    }

    /// The action carried from the spellbook, at the pointer (drawn last, over the windows).
    pub fn draw_drag(&self, ui: &mut Ui, size: (u32, u32)) {
        let icon = self.dragging.and_then(|a| self.icon(a));
        if let (Some(texture), Some((px, py))) = (icon, self.cursor) {
            let side = BUTTON * size.1 as f32 / crate::uixml::ui_height();
            ui.quad(Quad { texture, rect: [px - side / 2.0, py - side / 2.0, side, side], uv: FULL_UV, color: [1.0, 1.0, 1.0, 0.8], additive: false });
        }
    }
}

/// White text on a black one-pixel outline, as RGBA: the outline is black, the glyph white, so a tint
/// colours the glyph only.
fn render_text(font: &fontdue::Font, text: &str, px: f32) -> (u32, u32, Vec<u8>) {
    let pad = (px / 12.0).ceil() as usize;
    let line = font.horizontal_line_metrics(px).expect("font has line metrics");
    let glyphs: Vec<_> = text.chars().map(|c| font.rasterize(c, px)).collect();
    let width = glyphs.iter().map(|(m, _)| m.advance_width.ceil() as usize).sum::<usize>() + pad * 2;
    let height = (line.ascent - line.descent).ceil() as usize + pad * 2;
    let mut cover = vec![0u8; width * height];
    let (mut pen, baseline) = (pad as f32, pad as f32 + line.ascent);
    for (m, bitmap) in &glyphs {
        let top = (baseline - m.ymin as f32 - m.height as f32).round() as i32;
        for gy in 0..m.height {
            for gx in 0..m.width {
                let (x, y) = (pen as i32 + m.xmin + gx as i32, top + gy as i32);
                if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                    let c = &mut cover[y as usize * width + x as usize];
                    *c = (*c).max(bitmap[gy * m.width + gx]);
                }
            }
        }
        pen += m.advance_width;
    }
    let mut rgba = vec![0u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let mut outline = 0u8;
            for (dx, dy) in (-(pad as i32)..=pad as i32).flat_map(|dx| (-(pad as i32)..=pad as i32).map(move |dy| (dx, dy))) {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < width && (ny as usize) < height {
                    outline = outline.max(cover[ny as usize * width + nx as usize]);
                }
            }
            let fill = cover[y * width + x];
            let alpha = outline.max(fill);
            let white = if alpha == 0 { 0 } else { (fill as u32 * 255 / alpha as u32) as u8 };
            rgba[(y * width + x) * 4..][..4].copy_from_slice(&[white, white, white, alpha]);
        }
    }
    (width as u32, height as u32, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_saved_by_id_and_every_basic_one_has_an_icon_and_a_description() {
        let spell = Action::Spell(0);
        let potion = Action::Item(crate::items::find("i118"));
        for a in BASIC.into_iter().chain([spell, potion]) {
            assert_eq!(Action::from_id(&a.id()), Some(a));
        }
        assert!(BASIC.iter().all(|a| a.basic_icon().is_some() && a.basic_info().is_some()));
        assert_eq!(Action::from_id(""), None);
    }

    #[test]
    fn every_usable_item_has_an_icon_in_the_game_files() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let usable: Vec<_> = (0..crate::items::all().len()).filter(|&i| crate::consume::Consumed::can_use(i)).collect();
        assert!(!usable.is_empty());
        for i in usable {
            let path = crate::items::get(i).icon;
            assert!(mount.read(&format!("{path}.blp")).is_ok(), "{path} is missing");
        }
    }

    #[test]
    fn pages_turn_round_the_ends() {
        assert_eq!(turned(0, true), 1);
        assert_eq!(turned(PAGES - 1, true), 0);
        assert_eq!(turned(0, false), PAGES - 1);
    }

    #[test]
    fn clicks_find_the_button_under_the_pointer() {
        // At 1024x768 one unit is one pixel: the bar is centred, so button 1 spans x 264..300, 4..40 above the bottom.
        let size = (1024, 768);
        assert_eq!(ActionBar::slot_at((282.0, 748.0), size), Some(0));
        assert_eq!(ActionBar::slot_at((264.0 + 42.0 * 11.0 + 30.0, 748.0), size), Some(11));
        // The gap between buttons, above the bar, and outside it hit nothing.
        assert_eq!(ActionBar::slot_at((264.0 + 39.0, 748.0), size), None);
        assert_eq!(ActionBar::slot_at((282.0, 700.0), size), None);
        assert_eq!(ActionBar::slot_at((100.0, 748.0), size), None);
        assert_eq!(ActionBar::slot_at((900.0, 748.0), size), None);
        // Scaled up, the same button is found at the scaled position.
        assert_eq!(ActionBar::slot_at((1024.0 + (-248.0 + 18.0) * 2.0, 1496.0), (2048, 1536)), Some(0));
    }

    #[test]
    fn text_is_a_white_glyph_on_a_black_outline() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let font = fontdue::Font::from_bytes(mount.read(FONT).unwrap(), fontdue::FontSettings::default()).unwrap();
        let (w, h, rgba) = render_text(&font, "5", 24.0);
        assert!(w > 6 && h > 15 && rgba.len() == (w * h * 4) as usize);
        let opaque = rgba.chunks(4).filter(|p| p[3] == 255).count();
        let white = rgba.chunks(4).filter(|p| p[3] == 255 && p[0] > 200).count();
        let black = rgba.chunks(4).filter(|p| p[3] > 200 && p[0] < 50).count();
        assert!(opaque > 20 && white > 10 && black > 10, "{opaque} opaque, {white} white, {black} black");
    }
}
