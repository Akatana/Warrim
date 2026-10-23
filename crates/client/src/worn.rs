//! What a person wears, drawn: the body dressed in its armor (textures painted on the skin, mesh parts of sleeves, gloves, boots, a robe, a cape) and the
//! models fixed to it: the weapon and the shield (or what else is held) in the hands, a helmet on the head, the shoulder pads, a bow on the back.
//! The player and the people of the world are both dressed by it.

use crate::character::{to_render, Appearance, Character, Gear, Race, Sex};
use crate::effects::Effects;
use crate::items::{self, ItemDef, Look, Wear, SLOTS};
use crate::models::ModelCache;
use crate::renderer::{Gpu, Renderer};
use anyhow::Result;
use glam::{Mat4, Vec3};
use mpq::Mount;
use wowfmt::dbc::Dbc;
use wowfmt::m2::M2;

/// The attachment points of the body that things hang on.
const SHIELD: u32 = 0;
const RIGHT_HAND: u32 = 1;
const LEFT_HAND: u32 = 2;
const RIGHT_SHOULDER: u32 = 5;
const LEFT_SHOULDER: u32 = 6;
const HELM: u32 = 11;
/// Where what is held rests while the hands are not free, by the sheath type the original client reads from the item (`FUN_0047a070`: type 1 is a
/// two-hander at 26 (main hand) / 27, type 2 a staff, polearm or ranged weapon at 30 (main hand, guns) / 31 (bows), type 3 a one-hander at 32 (main
/// hand) / 33, type 4 the shield at 28).
const SHEATH_SHIELD: u32 = 28;
const SHEATH_TWO_HAND: u32 = 26;
const LARGE_LEFT: u32 = 30;
const LARGE_RIGHT: u32 = 31;
const HIP_MAIN: u32 = 32;
const HIP_OFF: u32 = 33;

/// What a part does while a bow is drawn: stays, goes to another attachment (the bow to the hand), or is put away (the sword).
enum Drawn {
    Same,
    To(u32, Mat4),
    Away,
}

/// A model fixed to an attachment of the body.
struct Part {
    /// The tracks that animate how it looks, and what each batch follows.
    looks: Option<(std::sync::Arc<crate::anim::LookTracks>, Vec<crate::anim::BatchRefs>)>,
    drawn: Drawn,
    /// The model, and the row of `ItemVisuals.dbc` of the glows on it.
    model: String,
    enchant: u16,
    entity: usize,
    attach: u32,
    /// How it is turned and moved from the attachment, and whether it is on the head (not drawn when the camera is inside it).
    fix: Mat4,
    on_head: bool,
    /// Where it rests (the attachment, and how it is turned) while `Worn::stowed`; what has none stays where it is.
    sheath: Option<(u32, Mat4)>,
}

/// An effect that goes with a part: where it is on it.
struct Flare {
    effect: usize,
    part: usize,
    local: Mat4,
}

pub struct Worn {
    /// The entity of the body, made again when the clothes change (the skin is part of what was uploaded).
    pub body: usize,
    parts: Vec<Part>,
    /// The glows of enchanted weapons and the emitters of the models (a torch, a burning sword) that burn on the parts.
    fx: Effects,
    flares: Vec<Flare>,
    /// Milliseconds the parts have been worn, for their animated looks.
    age: f32,
    /// What was put on last (where the definitions of the items are), to do nothing when it is the same.
    dressed: Option<[usize; SLOTS]>,
    /// What is held is sheathed (a rider has no hands for it): see `Part::sheath`.
    pub stowed: bool,
}

/// How the models of helmets are named for a race and sex (`Helm_Plate_D_02_HuM.m2`).
fn race_code(look: &Appearance) -> String {
    race_code_of(look.race, look.sex)
}

pub fn race_code_of(race: Race, sex: Sex) -> String {
    let race = match race {
        Race::Human => "Hu",
        Race::Orc => "Or",
        Race::Dwarf => "Dw",
        Race::NightElf => "Ni",
        Race::Undead => "Sc",
        Race::Tauren => "Ta",
        Race::Gnome => "Gn",
        Race::Troll => "Tr",
    };
    format!("{race}{}", if sex == Sex::Male { "M" } else { "F" })
}

/// Where a part is, relative to what the body places it by, and whether it is shown (`drawing`: a bow is drawn).
fn part_matrix(part: &Part, character: &Character, drawing: bool, stowed: bool) -> Option<(Mat4, bool)> {
    if let (true, Some((attach, fix))) = (stowed, part.sheath) {
        return Some((to_render(character.attachment(attach)?) * fix, true));
    }
    let (attach, fix, shown) = match (&part.drawn, drawing) {
        (Drawn::To(attach, fix), true) => (*attach, *fix, true),
        (Drawn::Away, true) => (part.attach, part.fix, false),
        _ => (part.attach, part.fix, true),
    };
    Some((to_render(character.attachment(attach)?) * fix, shown))
}

/// What burns on a model: the emitters it has of its own (shown without its mesh), then the glow of each slot of the enchantment at the point of
/// the model that the slot stands for (the points along a blade). Each is the model of the effect, whether it has its mesh drawn, and where it is.
fn flares_of(mount: &Mount, model: &str, enchant: u16) -> Vec<(String, bool, Mat4)> {
    let Some(m2) = mount.read(model).ok().and_then(|bytes| M2::parse(&bytes).ok()) else { return Vec::new() };
    let mut flares = Vec::new();
    if !m2.particles.is_empty() {
        flares.push((model.to_string(), false, Mat4::IDENTITY));
    }
    for (point, glow) in enchant_models(mount, enchant).into_iter().enumerate() {
        let Some(glow) = glow else { continue };
        let Some(at) = m2.attachment_lookup.get(point).and_then(|&a| m2.attachments.get(a as usize)) else { continue };
        flares.push((glow, true, to_render(Mat4::from_translation(Vec3::from(at.position)))));
    }
    flares
}

/// The models of the glows of an enchantment (a row of `ItemVisuals.dbc`), one for each of the five points along a weapon.
fn enchant_models(mount: &Mount, visual: u16) -> [Option<String>; 5] {
    let mut models: [Option<String>; 5] = Default::default();
    if visual == 0 {
        return models;
    }
    let read = |path: &str| mount.read(path).ok().and_then(|b| Dbc::parse(&b).ok());
    let (Some(visuals), Some(effects)) = (read(r"DBFilesClient\ItemVisuals.dbc"), read(r"DBFilesClient\ItemVisualEffects.dbc")) else { return models };
    let Some(row) = visuals.find_by_id(visual as u32) else { return models };
    for (point, model) in models.iter_mut().enumerate() {
        let path = effects.find_by_id(row.u32(1 + point)).map(|e| e.string(1)).filter(|p| !p.is_empty());
        *model = path.map(|p| format!("{}.m2", p.rsplit_once('.').map_or(p, |(stem, _)| stem)));
    }
    models
}

/// The armor class that decides how a glove or a boot is shaped (cloth and leather, mail, plate).
fn shape(item: &ItemDef) -> u16 {
    match item.subclass {
        3 => 1,
        4 => 2,
        _ => 0,
    }
}

/// The gear that what is worn gives: what is painted on the skin and which mesh parts of the body show.
pub fn gear_of(mount: &Mount, equipment: &[Option<&'static ItemDef>; SLOTS], look: &Appearance) -> Gear {
    let mut gear = Gear::default();
    let worn = |slot: usize| equipment[slot];
    for item in (0..SLOTS).filter_map(worn) {
        if item.look != Look::NONE {
            gear.looks.push(item.look);
        }
    }
    let geo = |slot: usize| worn(slot).map_or([0; 3], |i| i.visual.geo);
    // Sleeves; a robe (and a kilt) lets down its skirt and takes the trousers off.
    for slot in [items::SHIRT, items::CHEST, items::TABARD] {
        let g = geo(slot);
        if g[0] > 0 {
            gear.show.push(801 + g[0] as u16);
        }
        if g[2] > 0 {
            gear.show.push(1001 + g[2] as u16);
        }
        if g[1] == 1 {
            gear.show.push(1302);
            gear.hide.push(1301);
        }
    }
    if geo(items::LEGS)[1] == 1 {
        gear.show.push(1302);
        gear.hide.push(1301);
    }
    if worn(items::TABARD).is_some() {
        gear.show.push(1202);
    }
    // Gloves and boots take the shape of their armor.
    if let Some(item) = worn(items::HANDS) {
        gear.hide.push(401);
        gear.show.push(402 + shape(item));
    }
    if let Some(item) = worn(items::FEET) {
        gear.hide.push(501);
        gear.show.push(502 + shape(item));
    }
    // A cape.
    if let Some(texture) = worn(items::BACK).and_then(|i| i.visual.textures[0]) {
        gear.cape = Some(format!(r"Item\ObjectComponents\Cape\{texture}.blp"));
        gear.hide.push(1501);
        gear.show.extend(1502..=1506);
    }
    // A helmet hides some of the hair and face.
    if let Some(item) = worn(items::HEAD) {
        let id = item.visual.helm[usize::from(look.sex == Sex::Female)] as u32;
        if let Some(flags) = Dbc::parse(&mount.read(r"DBFilesClient\HelmetGeosetVisData.dbc").unwrap_or_default()).ok().and_then(|d| d.find_by_id(id).map(|r| [r.u32(1), r.u32(2), r.u32(3), r.u32(4), r.u32(5)])) {
            gear.helm = Some(flags);
        }
    }
    gear
}

impl Worn {
    #[cfg(test)]
    pub fn none() -> Worn {
        Worn { body: 0, parts: Vec::new(), fx: Effects::unmounted(), flares: Vec::new(), age: 0.0, dressed: None, stowed: false }
    }

    /// The body of `character` as an entity, undressed.
    pub fn new(character: &Character, renderer: &mut Renderer, gpu: &Gpu) -> Worn {
        Worn { body: renderer.add_room_lit_entity(gpu, &character.model), parts: Vec::new(), fx: Effects::unmounted(), flares: Vec::new(), age: 0.0, dressed: None, stowed: false }
    }

    /// Dress `character` in what the equipment holds and make the entities of the models that hang on it, giving the earlier ones back. Nothing is
    /// done when it is what was put on last.
    #[allow(clippy::too_many_arguments)]
    pub fn dress(&mut self, character: &mut Character, look: &Appearance, mount: &Mount, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu, equipment: &[Option<&'static ItemDef>; SLOTS]) -> Result<()> {
        let here: [usize; SLOTS] = equipment.map(|i| i.map_or(0, |i| i as *const ItemDef as usize));
        if self.dressed == Some(here) {
            return Ok(());
        }
        character.dress(mount, cache, &gear_of(mount, equipment, look))?;
        renderer.remove_entity(self.body);
        self.body = renderer.add_room_lit_entity(gpu, &character.model);
        for part in self.parts.drain(..) {
            renderer.remove_entity(part.entity);
        }
        let worn = |slot: usize| equipment[slot];
        let mut add = |model: &str, skin: &str, attach: u32, fix: Mat4, on_head: bool, parts: &mut Vec<Part>| {
            if !mount.exists(model) {
                log::warn!("worn model {model} is not in the game files");
                return;
            }
            match cache.item(mount, model, skin) {
                Ok(m) => parts.push(Part { looks: m.tracks.clone().filter(|t| t.animated).map(|t| (t, m.batches.iter().map(|b| b.refs).collect())), drawn: Drawn::Same, model: model.to_string(), enchant: 0, entity: renderer.add_room_lit_entity(gpu, &m), attach, fix, on_head, sheath: None }),
                Err(e) => log::warn!("worn model {model}: {e:#}"),
            }
        };
        let mut parts = Vec::new();
        // What is held.
        if let Some((model, skin)) = worn(items::MAIN_HAND).and_then(|i| i.held) {
            add(model, skin, RIGHT_HAND, Mat4::IDENTITY, false, &mut parts);
            if let Some(p) = parts.last_mut() {
                let main = worn(items::MAIN_HAND);
                p.sheath = Some((
                    match main {
                        Some(i) if i.wear == Wear::TwoHand && i.class == 2 && matches!(i.subclass, 6 | 10) => LARGE_LEFT,
                        Some(i) if i.wear == Wear::TwoHand => SHEATH_TWO_HAND,
                        _ => HIP_MAIN,
                    },
                    Mat4::IDENTITY,
                ));
                p.drawn = Drawn::Away;
                p.enchant = worn(items::MAIN_HAND).map_or(0, |i| i.visual.enchant);
            }
        }
        if let Some(item) = worn(items::OFF_HAND) {
            if let Some((model, skin)) = item.held {
                add(model, skin, if item.wear == Wear::Shield { SHIELD } else { LEFT_HAND }, Mat4::IDENTITY, false, &mut parts);
                // The bow takes the left hand while it is drawn.
                if let Some(p) = parts.last_mut() {
                    p.sheath = Some(if item.wear == Wear::Shield { (SHEATH_SHIELD, Mat4::IDENTITY) } else { (HIP_OFF, Mat4::IDENTITY) });
                    p.drawn = Drawn::Away;
                    p.enchant = item.visual.enchant;
                }
            }
        }
        // A bow, a gun or a crossbow rests on the back; a bow is drawn in the left hand, the others in the right (as the original does).
        if let Some(item) = worn(items::RANGED).filter(|i| i.class == 2 && matches!(i.subclass, 2 | 3 | 18)) {
            if let Some((model, skin)) = item.held {
                let before = parts.len();
                let (rest, hand) = if item.subclass == 2 { (LARGE_RIGHT, LEFT_HAND) } else { (LARGE_LEFT, RIGHT_HAND) };
                add(model, skin, rest, Mat4::IDENTITY, false, &mut parts);
                if parts.len() > before {
                    parts[before].drawn = Drawn::To(hand, Mat4::IDENTITY);
                    parts[before].enchant = item.visual.enchant;
                }
            }
        }
        // The helmet, and the shoulder pads.
        if let Some(item) = worn(items::HEAD) {
            if let (Some(model), Some(texture)) = (item.visual.models[0], item.visual.textures[0]) {
                add(&format!(r"Item\ObjectComponents\Head\{model}_{}.m2", race_code(look)), &format!(r"Item\ObjectComponents\Head\{texture}.blp"), HELM, Mat4::IDENTITY, true, &mut parts);
            }
        }
        if let Some(item) = worn(items::SHOULDER) {
            for (side, attach) in [(0, LEFT_SHOULDER), (1, RIGHT_SHOULDER)] {
                if let (Some(model), Some(texture)) = (item.visual.models[side], item.visual.textures[side].or(item.visual.textures[0])) {
                    add(&format!(r"Item\ObjectComponents\Shoulder\{model}.m2"), &format!(r"Item\ObjectComponents\Shoulder\{texture}.blp"), attach, Mat4::IDENTITY, false, &mut parts);
                }
            }
        }
        self.kindle(mount, renderer, gpu, &parts);
        self.parts = parts;
        self.dressed = Some(here);
        Ok(())
    }

    /// Light what burns on the parts: the particles of a model that has emitters of its own, and the glows of the enchantment of a weapon or a shield,
    /// which stand along it. The earlier ones burn out.
    fn kindle(&mut self, mount: &Mount, renderer: &mut Renderer, gpu: &Gpu, parts: &[Part]) {
        for flare in self.flares.drain(..) {
            self.fx.stop(flare.effect);
        }
        let mut lit = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            lit.extend(flares_of(mount, &part.model, part.enchant).into_iter().map(|(model, mesh, local)| (model, mesh, i, local)));
        }
        for (model, mesh, part, local) in lit {
            if let Some(effect) = self.fx.spawn_in(mount, renderer, gpu, &model, None, true, mesh, Mat4::IDENTITY) {
                self.flares.push(Flare { effect, part, local });
            }
        }
    }

    /// Forget what was put on, so that the next `dress` does it again (the content changed under the same numbers).
    pub fn forget(&mut self) {
        self.dressed = None;
    }

    /// Put the body and what hangs on it at `pos`, turned by `orient`; `first_person`: the camera is inside the head, so a helmet is not drawn.
    pub fn place(&self, renderer: &mut Renderer, character: &Character, pos: Vec3, orient: Mat4, first_person: bool, drawing: bool) {
        renderer.set_entity(self.body, pos, orient, true);
        for part in &self.parts {
            if let Some((m, shown)) = part_matrix(part, character, drawing, self.stowed) {
                renderer.set_entity(part.entity, pos, orient * m, shown && !(first_person && part.on_head));
            }
        }
    }

    /// Move the effects of the parts along with them (`place` was called with the same arguments), and advance them `dt` seconds, drawn to face
    /// `forward`, the way the viewer looks, from `viewer`.
    #[allow(clippy::too_many_arguments)]
    pub fn animate(&mut self, dt: f32, forward: Vec3, viewer: Vec3, renderer: &mut Renderer, gpu: &Gpu, character: &Character, pos: Vec3, orient: Mat4, first_person: bool, drawing: bool) {
        self.age += dt * 1000.0;
        for part in &self.parts {
            if let Some((tracks, refs)) = &part.looks {
                let looks: Vec<_> = refs.iter().map(|&r| tracks.look(r, self.age)).collect();
                renderer.set_entity_looks(gpu, part.entity, &looks);
            }
        }
        for flare in &self.flares {
            let Some(part) = self.parts.get(flare.part) else { continue };
            if let Some((m, shown)) = part_matrix(part, character, drawing, self.stowed) {
                self.fx.set_world(flare.effect, Mat4::from_translation(pos) * orient * m * flare.local);
                self.fx.set_hidden(flare.effect, !shown || (first_person && part.on_head));
            }
        }
        self.fx.update_facing(dt, forward, viewer, renderer, gpu);
    }

    /// Make the body and what hangs on it see-through (`fade` 1 is solid); call after `animate`, which gives the parts their own looks.
    pub fn fade(&self, renderer: &mut Renderer, gpu: &Gpu, fade: f32) {
        for e in std::iter::once(self.body).chain(self.parts.iter().map(|p| p.entity)) {
            renderer.set_entity_fade(gpu, e, fade);
        }
    }

    /// Every entity: the body and the models on it.
    pub fn entities(&self) -> Vec<usize> {
        std::iter::once(self.body).chain(self.parts.iter().map(|p| p.entity)).chain(self.fx.entities()).collect()
    }

    /// Out of the picture.
    pub fn hide(&self, renderer: &mut Renderer, pos: Vec3) {
        for e in self.entities() {
            renderer.set_entity(e, pos, Mat4::IDENTITY, false);
        }
    }

    /// Give every entity back to the renderer.
    pub fn retire(self, renderer: &mut Renderer) {
        self.entities().into_iter().for_each(|e| renderer.remove_entity(e));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_enchantment_has_the_models_of_its_glows_and_no_enchantment_has_none() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        assert!(enchant_models(&mount, 0).iter().all(Option::is_none));
        // Every row of the table names models that are in the game files, in the model format.
        let visuals = Dbc::parse(&mount.read(r"DBFilesClient\ItemVisuals.dbc").unwrap()).unwrap();
        for row in visuals.iter() {
            for model in enchant_models(&mount, row.u32(0) as u16).into_iter().flatten() {
                assert!(model.ends_with(".m2") && mount.exists(&model), "{model}");
            }
        }
        assert!(enchant_models(&mount, 25).iter().any(Option::is_some));
    }

    #[test]
    fn what_burns_on_a_model_is_its_own_emitters_and_the_glows_along_the_blade() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let weapon = |name: &str| format!(r"Item\ObjectComponents\Weapon\{name}.m2");
        // A plain sword burns nothing; a torch has its own flame (no second mesh); an enchanted one glows at each of the five points of the blade.
        assert!(flares_of(&mount, &weapon("Sword_1H_Long_A_01"), 0).is_empty());
        let torch = flares_of(&mount, &weapon("Club_1H_Torch_A_01"), 0);
        assert_eq!(torch.len(), 1);
        assert!(!torch[0].1);
        let glows = flares_of(&mount, &weapon("Sword_1H_Long_A_01"), 25);
        assert_eq!(glows.len(), 5);
        let along: Vec<f32> = glows.iter().map(|g| g.2.w_axis.truncate().length()).collect();
        assert!(glows.iter().all(|g| g.1 && g.0.ends_with("RedFlame_Low.m2")) && along.windows(2).all(|w| w[0] < w[1]), "{along:?}");
    }
}
