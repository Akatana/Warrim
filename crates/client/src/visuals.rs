//! How the original shows a spell, read from its own tables: `Spell.dbc` (its cast time and missile speed, and its row of
//! `SpellVisual.dbc`) gives three kits, one for the start of the cast (the precast: glows on the hands, the ready animation of the
//! character, a sound), one for when it goes off and one for where it hits, and a missile model that flies between; each kit
//! (`SpellVisualKit.dbc`) names an animation, models to put on the head, chest, feet, hands and so on
//! (`SpellVisualEffectName.dbc`) and a sound (`SoundEntries.dbc`).

use crate::content;
use anyhow::{Context, Result};
use mpq::Mount;
use wowfmt::dbc::Dbc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Head,
    Chest,
    Base,
    LeftHand,
    RightHand,
    Other,
}

/// A model of effects to put on a part of whoever the kit is for.
#[derive(Clone, Debug)]
pub struct Placed {
    pub slot: Slot,
    /// The model's file (`Spells\...m2`).
    pub model: String,
}

#[derive(Clone, Debug, Default)]
pub struct Kit {
    /// The animation the caster plays (an animation id of the character model, such as 51 for the ready stance of a spell cast at a
    /// target).
    pub anim: Option<u16>,
    pub effects: Vec<Placed>,
    /// A sound entry.
    pub sound: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Visual {
    /// Milliseconds the cast takes (0 for an instant one).
    pub cast_ms: f32,
    /// How fast the missile flies, yards a second.
    pub speed: f32,
    pub precast: Kit,
    pub cast: Kit,
    pub impact: Kit,
    pub missile: Option<String>,
    pub missile_sound: Option<u32>,
}

/// The file of a model of the tables (`...mdx`) as the game has it (`...m2`).
fn model_file(name: &str) -> String {
    name.rsplit_once('.').map_or(name.to_string(), |(stem, _)| format!("{stem}.m2"))
}

/// The tables the pictures of a spell or of a shot are made from.
struct Tables {
    visuals: Dbc,
    kits: Dbc,
    effects: Dbc,
}

impl Tables {
    fn new(mount: &Mount) -> Result<Tables> {
        let dbc = |name: &str| -> Result<Dbc> { Ok(Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc"))?)?) };
        Ok(Tables { visuals: dbc("SpellVisual")?, kits: dbc("SpellVisualKit")?, effects: dbc("SpellVisualEffectName")? })
    }

    fn effect(&self, id: u32) -> Option<String> {
        self.effects.find_by_id(id).map(|r| model_file(r.string(2))).filter(|f| !f.is_empty() && id != u32::MAX)
    }

    fn kit(&self, id: u32) -> Kit {
        let Some(k) = (id != 0 && id != u32::MAX).then(|| self.kits.find_by_id(id)).flatten() else { return Kit::default() };
        let slots = [(3, Slot::Head), (4, Slot::Chest), (5, Slot::Base), (6, Slot::LeftHand), (7, Slot::RightHand), (11, Slot::Other), (12, Slot::Other)];
        Kit {
            anim: Some(k.u32(2)).filter(|&a| a != 0 && a != u32::MAX).map(|a| a as u16),
            effects: slots.into_iter().filter_map(|(field, slot)| Some(Placed { slot, model: self.effect(k.u32(field))? })).collect(),
            sound: Some(k.u32(13)).filter(|&s| s != 0 && s != u32::MAX),
        }
    }

    /// What a row of `SpellVisual.dbc` shows.
    fn visual(&self, v: wowfmt::dbc::Record, cast_ms: f32, speed: f32) -> Visual {
        Visual {
            cast_ms,
            speed,
            precast: self.kit(v.u32(1)),
            cast: self.kit(v.u32(2)),
            impact: self.kit(v.u32(3)),
            missile: (v.u32(6) != 0).then(|| self.effect(v.u32(7))).flatten(),
            missile_sound: Some(v.u32(10)).filter(|&s| s != 0),
        }
    }
}

/// How fast the shot of a ranged weapon flies, yards a second (`Spell.dbc` gives 40 to Shoot Bow, Shoot Gun and Shoot Crossbow).
pub const SHOT_SPEED: f32 = 40.0;

/// What the shot of a ranged weapon shows: its row (the `ItemDisplayInfo.dbc` field of the weapon) of `SpellVisual.dbc`. A bow is drawn with the
/// kit of the precast and let go with the kit of the cast (animation, sound), the missile sound goes with the arrow in flight, and the impact kit
/// where it hits.
pub fn of_shot(mount: &Mount, id: u32) -> Visual {
    Tables::new(mount).ok().and_then(|t| t.visuals.find_by_id(id).map(|v| t.visual(v, 0.0, SHOT_SPEED))).unwrap_or_default()
}

/// What the original shows for a spell by its row of `Spell.dbc` (the gathering spells: Mining 2575, Herb Gathering 2366): the kits of its visual row.
pub fn of_spell(mount: &Mount, spell: u32) -> Visual {
    let dbc = |name: &str| Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc")).ok()?).ok();
    let (Some(spells), Some(tables)) = (dbc("Spell"), Tables::new(mount).ok()) else { return Visual::default() };
    spells.find_by_id(spell).and_then(|s| tables.visuals.find_by_id(s.u32(115)).map(|v| tables.visual(v, 0.0, 0.0))).unwrap_or_default()
}

/// What the original shows for a spell, read once for each: the work of a trade (a hammer on the anvil, a pot on the fire) is its spell's precast kit.
pub fn spell(mount: &Mount, id: u32) -> Visual {
    static CACHE: std::sync::Mutex<std::collections::BTreeMap<u32, Visual>> = std::sync::Mutex::new(std::collections::BTreeMap::new());
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.entry(id).or_insert_with(|| of_spell(mount, id)).clone()
}

/// The visuals of Mining (spell 2575) and of Herb Gathering (2366), read once: the stance and the model in the hand, and the sounds.
pub fn gathering(mount: &Mount) -> &'static [Visual; 2] {
    static CACHE: std::sync::OnceLock<[Visual; 2]> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| [of_spell(mount, 2575), of_spell(mount, 2366)])
}

/// The Stealth spell of the original, whose pictures and sound a sneaking player has.
pub const STEALTH: u32 = 1784;

/// The sound a spell keeps up while its aura lasts: the "state" kit (`SpellVisual.dbc` field 4) of its visual. Stealth (spell 1784) has no
/// models there, only the sound `Stealth.wav`.
pub fn state_sound(mount: &Mount, spell: u32) -> Option<u32> {
    let dbc = |name: &str| Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc")).ok()?).ok();
    let (spells, tables) = (dbc("Spell")?, Tables::new(mount).ok()?);
    let visual = tables.visuals.find_by_id(spells.find_by_id(spell)?.u32(115))?;
    tables.kit(visual.u32(4)).sound
}

/// The pictures and sounds of spells of the original by their row of `Spell.dbc`: the ones that the scripts of creatures cast, and what those trigger.
pub fn load_classic(mount: &Mount) -> Result<std::collections::HashMap<u32, Visual>> {
    use std::collections::HashSet;
    let c = content::get();
    let mut wanted: HashSet<u32> = HashSet::new();
    for act in c.creatures.iter().flat_map(|d| &d.brain.events).flat_map(|e| &e.acts).filter(|a| a.0 == crate::brain::act::CAST) {
        wanted.insert(act.1 as u32);
        if let Some(s) = content::classic_spell(act.1 as u32) {
            wanted.extend(s.effects.iter().map(|e| e.trigger).filter(|&t| t != 0));
        }
    }
    let dbc = |name: &str| -> Result<Dbc> { Ok(Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc"))?)?) };
    let (spells, times) = (dbc("Spell")?, dbc("SpellCastTimes")?);
    let tables = Tables::new(mount)?;
    Ok(spells
        .iter()
        .filter(|s| wanted.contains(&s.u32(0)))
        .map(|spell| {
            let cast_ms = times.find_by_id(spell.u32(18)).map_or(0.0, |r| r.i32(1).max(0) as f32);
            let speed = spell.f32(37);
            let visual = match tables.visuals.find_by_id(spell.u32(115)) {
                None => Visual { cast_ms, speed, ..Visual::default() },
                Some(v) => tables.visual(v, cast_ms, if speed > 1.0 { speed } else { 20.0 }),
            };
            (spell.u32(0), visual)
        })
        .collect())
}

/// The visuals of every spell of the content, in the order of the content.
pub fn load(mount: &Mount) -> Result<Vec<Visual>> {
    let dbc = |name: &str| -> Result<Dbc> { Ok(Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc"))?)?) };
    let (spells, times) = (dbc("Spell")?, dbc("SpellCastTimes")?);
    let tables = Tables::new(mount)?;
    content::get()
        .spells
        .iter()
        .map(|def| {
            let spell = spells.find_by_id(def.spell).with_context(|| format!("{}: no spell {} in Spell.dbc", def.name, def.spell))?;
            let cast_ms = times.find_by_id(spell.u32(18)).map_or(0.0, |r| r.i32(1).max(0) as f32);
            let speed = spell.f32(37);
            Ok(match tables.visuals.find_by_id(spell.u32(115)) {
                None => Visual { cast_ms, speed, ..Visual::default() },
                Some(v) => tables.visual(v, cast_ms, if speed > 1.0 { speed } else { 20.0 }),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spells_of_the_game_have_the_pictures_sounds_and_cast_times_of_the_original() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let v = load(&mount).unwrap();
        let of = |id: &str| &v[content::spell_index(id)];
        // Fireball: 1.5 seconds of fire in the hands, a missile, a burst where it hits.
        let fireball = of("fireball");
        assert_eq!(fireball.cast_ms, 1500.0);
        assert!(fireball.precast.effects.len() == 2 && fireball.precast.anim == Some(51) && fireball.precast.sound.is_some());
        assert!(fireball.missile.as_deref().is_some_and(|m| m.ends_with("Fireball_Missile_Low.m2")));
        assert!(fireball.impact.effects.iter().any(|e| e.slot == Slot::Chest));
        // Instant spells have no cast time; a heal lands on the base of the caster.
        assert_eq!(of("heroic_strike").cast_ms, 0.0);
        assert!(of("heal").impact.effects.iter().any(|e| e.slot == Slot::Base));
        // Every model they name is a file of the game, and so is every sound.
        for (spell, vis) in content::get().spells.iter().zip(&v) {
            let all = [&vis.precast, &vis.cast, &vis.impact];
            for placed in all.iter().flat_map(|k| &k.effects) {
                assert!(mount.read(&placed.model).is_ok(), "{}: {}", spell.name, placed.model);
            }
            if let Some(m) = &vis.missile {
                assert!(mount.read(m).is_ok(), "{}: {m}", spell.name);
            }
            assert!(vis.cast_ms >= 0.0 && vis.cast_ms < 10_000.0, "{}: {}", spell.name, vis.cast_ms);
        }
    }

    #[test]
    fn stealth_has_the_sound_of_the_original() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        assert_eq!(state_sound(&mount, STEALTH), Some(3325));
    }

    #[test]
    fn the_shots_of_bows_and_guns_have_the_draw_and_release_of_the_original() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        // The bow of ItemDisplayInfo (5): the pull-back and the release with their animations and sounds, the arrow's flight sound, the hit.
        let bow = of_shot(&mount, 5);
        assert_eq!((bow.cast.anim, bow.precast.sound, bow.cast.sound, bow.impact.sound, bow.missile_sound), (Some(46), Some(1144), Some(1146), Some(4296), Some(4222)));
        // A gun and a crossbow are let go with the rifle's animation.
        assert_eq!(of_shot(&mount, 224).cast.anim, Some(49));
        assert_eq!(of_shot(&mount, 743).cast.anim, Some(49));
        assert!(of_shot(&mount, 0).cast.anim.is_none());
    }

    #[test]
    fn the_spells_that_creatures_cast_have_their_pictures_and_sounds_and_every_model_exists() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let v = load_classic(&mount).unwrap();
        assert!(v.len() > 1000, "{} spells", v.len());
        // A frost bolt of a creature: three seconds of cast, a missile, a burst.
        let bolt = &v[&13322];
        assert_eq!(bolt.cast_ms, 3000.0);
        assert!(bolt.missile.is_some() && !bolt.impact.effects.is_empty(), "{bolt:?}");
        // Shoot and Throw have no kit of their own: the original takes the picture of the shot from the weapon.
        assert!(v[&6660].cast.anim.is_none() && v[&6660].missile.is_none());
        let mut missing = Vec::new();
        for (spell, vis) in &v {
            let kits = [&vis.precast, &vis.cast, &vis.impact];
            for model in kits.iter().flat_map(|k| &k.effects).map(|e| &e.model).chain(vis.missile.iter()) {
                if mount.read(model).is_err() {
                    missing.push((*spell, model.clone()));
                }
            }
        }
        assert!(missing.len() * 50 < v.len(), "{} of {} spells name a model that is not there: {:?}", missing.len(), v.len(), &missing[..missing.len().min(8)]);
    }
}

#[cfg(test)]
mod gather_tests {
    #[test]
    fn mining_swings_a_pick_and_herb_gathering_has_its_own_stance_and_sound() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let [mining, herbs] = super::gathering(&mount);
        assert!(mining.precast.anim.is_some() && mining.precast.effects.iter().any(|e| e.model.contains("MiningPick")) && mining.missile_sound.is_some());
        assert!(herbs.precast.anim.is_some() && herbs.precast.sound.is_some());
    }
}
