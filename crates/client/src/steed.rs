//! The steed a mount item calls: a horse, wolf, ram, raptor... The model and skins come from a row of `CreatureDisplayInfo.dbc` (what the original's mount
//! spell names), it is animated by the same `Animator` as the character (standing, running, backing, jumping, shuffling when turned on the spot), and
//! the rider sits where its attachment 0 is.

use crate::animator::{Animator, Motion};
use crate::anim;
use crate::character::MODEL_TO_RENDER;
use crate::models::{ModelCache, ModelCpu, ModelVertex};
use crate::renderer::{Gpu, Renderer};
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mpq::Mount;
use std::sync::Arc;
use wowfmt::dbc::Dbc;
use wowfmt::m2::M2;

/// The attachment of a steed model that the rider sits on.
const SEAT: u32 = 0;
/// The texture slot of the first skin of a creature model; the second and third follow.
const FIRST_SKIN: u32 = 11;

pub struct Steed {
    m2: M2,
    template: Arc<ModelCpu>,
    entity: usize,
    animator: Animator,
    scale: f32,
    clock_ms: f32,
    vertices: Vec<ModelVertex>,
    /// The percent of the running speed it adds.
    pub percent: i32,
    /// The row of `CreatureDisplayInfo.dbc` it was made from.
    pub display: u32,
    /// The footstep set its hooves sound from (`CreatureSoundData`).
    pub foot_set: u32,
    /// It is the shape the character himself has taken (a druid's bear): nobody rides it, and it comes with sneaking.
    pub form: bool,
}

/// The model file, the skins (texture slot, file), the size and the footstep set of the display row `display` of `CreatureDisplayInfo.dbc`.
fn look_of(mount: &Mount, display: u32) -> Result<(String, Vec<(u32, String)>, f32, u32)> {
        let dbc = |name: &str| -> Result<Dbc> { Ok(Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc"))?)?) };
        let (displays, models) = (dbc("CreatureDisplayInfo")?, dbc("CreatureModelData")?);
        let row = displays.find_by_id(display).with_context(|| format!("no creature display {display}"))?;
        let data = models.find_by_id(row.u32(1)).with_context(|| format!("no creature model {}", row.u32(1)))?;
        let (stem, _) = data.string(2).rsplit_once('.').context("a model without a file")?;
        let model = format!("{stem}.m2");
        let folder = model.rsplit_once('\\').map_or("", |(f, _)| f);
        let skins: Vec<(u32, String)> = (0..3).map(|i| (FIRST_SKIN + i as u32, row.string(6 + i))).filter(|(_, n)| !n.is_empty()).map(|(slot, n)| (slot, format!(r"{folder}\{n}.blp"))).collect();
        // The feet: the model's sound data (field 13 of the model) names a footstep set (field 9 of that).
        let sounds = dbc("CreatureSoundData")?;
        let set_of = |model: &wowfmt::dbc::Record| sounds.find_by_id(model.u32(13)).map_or(0, |r| r.u32(9));
        let dir_of = |model: &wowfmt::dbc::Record| model.string(2).rsplit_once('\\').map_or(String::new(), |(f, _)| f.to_ascii_lowercase());
        // A model whose sound data has no feet (the war kodos) steps like the others of its folder.
        let foot_set = match set_of(&data) {
            0 => models.iter().filter(|m| dir_of(m) == dir_of(&data)).map(|m| set_of(&m)).find(|&set| set != 0).unwrap_or(0),
            set => set,
        };
        // The size of the display times that of the model.
        Ok((model, skins, row.f32(4).max(0.01) * data.f32(4).max(0.01), foot_set))
}

impl Steed {
    /// The steed of display row `display` of `CreatureDisplayInfo.dbc`, which adds `percent` to the running speed.
    pub fn load(mount: &Mount, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu, display: u32, percent: i32, form: bool) -> Result<Steed> {
        let (model, skins, scale, foot_set) = look_of(mount, display)?;
        let skins: Vec<(u32, &str)> = skins.iter().map(|(slot, file)| (*slot, file.as_str())).collect();
        let m2 = M2::parse(&mount.read(&model)?).with_context(|| model.clone())?;
        let template = cache.creature(mount, &model, &skins)?;
        let entity = renderer.add_room_lit_entity(gpu, &template);
        Ok(Steed { animator: Animator::new(&m2, 0x5EED), m2, template, entity, scale, clock_ms: 0.0, vertices: Vec::new(), percent, display, foot_set, form })
    }

    /// Advance the steed `dt` seconds with the player's `motion` and put it on the ground at `feet`, facing `yaw`. Returns where the rider sits.
    pub fn update(&mut self, dt: f32, motion: &Motion, feet: Vec3, yaw: f32, renderer: &mut Renderer, gpu: &Gpu) -> Vec3 {
        self.animator.update(&self.m2, dt, &Motion { sneaking: false, mounted: false, ..*motion }, true);
        self.clock_ms += dt * 1000.0;
        let (body, _) = self.animator.mixes(&self.m2);
        let bones = anim::bone_matrices_with(&self.m2, body, self.clock_ms, &[], &[]);
        anim::skin(&self.m2, &bones, &self.template.vertices, &mut self.vertices);
        renderer.update_entity_vertices(gpu, self.entity, &self.vertices);
        let facing = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - yaw);
        renderer.set_entity(self.entity, feet, facing * Mat4::from_scale(Vec3::splat(self.scale)), true);
        // A model without a seat carries the rider at its middle, as high as the model is.
        let seat = self.m2.attachment_lookup.get(SEAT as usize).and_then(|&i| self.m2.attachments.get(i as usize)).and_then(|a| Some(bones.get(a.bone as usize)?.transform_point3(Vec3::from(a.position))));
        let seat = seat.unwrap_or_else(|| Vec3::new(0.0, 0.0, self.m2.bounds_max[2] * 0.8));
        feet + facing.transform_point3(MODEL_TO_RENDER.transform_point3(seat)) * Vec3::splat(self.scale)
    }

    /// Give the entity back to the renderer.
    pub fn retire(self, renderer: &mut Renderer) {
        renderer.remove_entity(self.entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every steed that a mount item of the content names has a model and skins in the game's files, and a seat for the rider.
    #[test]
    fn every_mount_item_names_a_steed_the_game_can_draw() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let mut displays: Vec<u32> = crate::items::all().iter().flat_map(|i| i.spells.iter().flat_map(|s| s.effects.iter())).filter_map(|e| if let crate::items::UseEffect::Mount(d, _) = e { Some(*d) } else { None }).collect();
        displays.sort_unstable();
        displays.dedup();
        assert!(displays.len() > 30, "only {} steeds", displays.len());
        let mut missing = Vec::new();
        for d in displays {
            let drawn = look_of(&mount, d).ok().filter(|(model, skins, ..)| mount.exists(model) && skins.iter().all(|(_, f)| mount.exists(f)));
            let seat = drawn.and_then(|(model, ..)| M2::parse(&mount.read(&model).ok()?).ok()).is_some_and(|m2| m2.attachment_lookup.first().is_some_and(|&i| i != 0xFFFF));
            if !seat {
                missing.push(d);
            }
        }
        assert!(missing.is_empty(), "steeds that cannot be drawn or ridden: {missing:?}");
    }

    #[test]
    fn the_brown_horse_is_the_riding_horse_in_its_brown_skin() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let (model, skins, scale, _) = look_of(&mount, 2404).unwrap();
        assert_eq!((model.to_ascii_lowercase().as_str(), scale), (r"creature\ridinghorse\ridinghorse.m2", 1.0));
        assert_eq!(skins, vec![(FIRST_SKIN, r"Creature\RidingHorse\RidingHorseSkinBrown.blp".to_string())]);
    }

    /// Every steed's hooves make a sound on the ground: its footstep set has an entry on some kind of terrain.
    #[test]
    fn every_steed_has_hoofbeats_on_some_ground() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let bank = crate::audio::SoundBank::load(&mount).unwrap();
        let mut displays: Vec<u32> = crate::items::all().iter().flat_map(|i| i.spells.iter().flat_map(|s| s.effects.iter())).filter_map(|e| if let crate::items::UseEffect::Mount(d, _) = e { Some(*d) } else { None }).collect();
        displays.sort_unstable();
        displays.dedup();
        // (The drake and the riding turtle have no feet in the original's sound data and no sibling model to borrow them from: they are silent there too.)
        let silent: Vec<u32> = displays.into_iter().filter(|d| ![15293, 17158].contains(d)).filter(|&d| { let set = look_of(&mount, d).unwrap().3; !(0..40).any(|terrain| bank.footstep_in(set, terrain).is_some()) }).collect();
        assert!(silent.is_empty(), "steeds with no hoofbeats: {silent:?}");
    }
}

#[cfg(test)]
mod form_tests {
    use super::*;

    /// The beasts a druid or a shaman can turn into are creatures of the game's files, with their skins.
    #[test]
    fn every_shape_a_spell_takes_can_be_drawn() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let cache = ModelCache::default();
        for display in crate::procs::FORM_DISPLAYS {
            let (model, skins, ..) = look_of(&mount, display).unwrap_or_else(|e| panic!("form {display}: {e:#}"));
            assert!(mount.exists(&model) && skins.iter().all(|(_, f)| mount.exists(f)), "form {display}: {model}");
            let skins: Vec<(u32, &str)> = skins.iter().map(|(slot, file)| (*slot, file.as_str())).collect();
            cache.creature(&mount, &model, &skins).unwrap_or_else(|e| panic!("form {display}: {e:#}"));
        }
    }
}
