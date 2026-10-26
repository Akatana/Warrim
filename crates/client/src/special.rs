//! The spells of the classes that do what the original's spell does (`Effect::Classic` in spells.ron): the effects of the spell in `classic_spells.ron` are run by `procs`
//! (hurts, heals, auras on the character, wounds and other statuses on creatures, conjuring, dispelling, hiding) and carried out here, with their numbers brought
//! to the scale of this game's spells: those of the original are written for a character of the level the spell is taught at.

use crate::casting::Ctx;
use crate::consume::Timed;
use crate::content::{self, Effect, PlaceKind, Rider};
use crate::creature::Status;
use crate::items::UseEffect;
use crate::levels;
use crate::procs::{self, Outcome};
use crate::skills::Skill;
use glam::{Mat4, Vec3};

/// Seconds between the pulses of a totem, and between the gifts of a well.
const TOTEM_EVERY: f32 = 2.0;
const WELL_EVERY: f32 = 3.0;
/// How far from the player a pet appears, yards.
const PET_AT: f32 = 2.5;
/// The levels above the player's that a beast may be tamed at.
const TAME_LEVELS: u32 = 3;
/// What a trap and a well look like (models of the game's spell objects).
const TRAP_MODEL: &str = r"Creature\Spells\LandMine01.m2";
const WELL_MODEL: &str = r"Creature\Spells\HealingTotem.m2";

/// What an outcome is worth at the scale of this game's spells (`k` times the original's numbers).
fn scaled(o: Outcome, k: f32) -> Outcome {
    match o {
        Outcome::Hurt { school, amount } => Outcome::Hurt { school, amount: amount * k },
        Outcome::Leech(a) => Outcome::Leech(a * k),
        Outcome::Heal(a) => Outcome::Heal(a * k),
        Outcome::Mana(a) => Outcome::Mana(a * k),
        Outcome::Area { amount, radius } => Outcome::Area { amount: amount * k, radius },
        Outcome::Burn(a) => Outcome::Burn(a * k),
        Outcome::LifeTap(a) => Outcome::LifeTap(a * k),
        Outcome::Put(Status::Over { amount, period, seconds }) => Outcome::Put(Status::Over { amount: amount * k, period, seconds }),
        Outcome::Timed(Timed::Shield { amount, school }, s) => Outcome::Timed(Timed::Shield { amount: amount * k, school }, s),
        Outcome::Timed(Timed::Regen(a), s) => Outcome::Timed(Timed::Regen(a * k), s),
        // Immunity is all or nothing; a ward is worth what the spell was worth.
        Outcome::Timed(Timed::Absorb(a), s) if a < 1e30 => Outcome::Timed(Timed::Absorb(a * k), s),
        Outcome::Timed(Timed::Gear(UseEffect::Boost(stat, amount, secs)), s) => Outcome::Timed(Timed::Gear(UseEffect::Boost(stat, (amount as f32 * k).round() as i32, secs)), s),
        // Attack power, armor of schools and the like grow too; percents and speeds do not.
        Outcome::Timed(Timed::Gear(UseEffect::Equip(aura @ (22 | 99 | 124 | 135), misc, amount)), s) => Outcome::Timed(Timed::Gear(UseEffect::Equip(aura, misc, (amount as f32 * k).round() as i32)), s),
        other => other,
    }
}

fn flat(v: Vec3) -> f32 {
    Vec3::new(v.x, 0.0, v.z).length()
}

/// One thing a spell set down in the world.
#[derive(Clone)]
struct Standing {
    kind: PlaceKind,
    spell: u32,
    class: Skill,
    icon: String,
    at: Vec3,
    left: f32,
    radius: f32,
    slot: u8,
    uses: u8,
    /// Seconds to its next pulse.
    next: f32,
    effect: Option<usize>,
    /// A trap that has gone off and whose spell goes on (the cold of a frost trap).
    zone: bool,
}

/// The totems, traps and wells that stand in the world.
#[derive(Default)]
pub struct Placed {
    list: Vec<Standing>,
}

impl Placed {
    /// A spell sets something down at `at`: a totem takes the place of the one of its element that stood, and shows the model of its creature.
    pub fn place(&mut self, ctx: &mut Ctx, class: Skill, icon: &str, rider: Rider, at: Vec3) {
        let Rider::Place { entry, spell, kind, seconds, radius, slot, uses } = rider else { return };
        if kind == PlaceKind::Totem {
            if let Some(i) = self.list.iter().position(|s| s.kind == kind && s.slot == slot) {
                let old = self.list.remove(i);
                old.effect.into_iter().for_each(|e| ctx.effects.stop(e));
            }
        }
        // The model: the creature's, or what the kind has.
        let creature = content::get().creature_numbers.get(&format!("c{entry}")).and_then(|&k| match &content::get().creatures[k].body {
            content::Body::Beast { model, .. } => Some(model.clone()),
            _ => None,
        });
        let model = creature.unwrap_or_else(|| if kind == PlaceKind::Well { WELL_MODEL } else { TRAP_MODEL }.to_string());
        let effect = ctx.effects.spawn(ctx.renderer, ctx.gpu, &model, true, Mat4::from_translation(at));
        let first = if kind == PlaceKind::Totem { 0.5 } else { WELL_EVERY };
        self.list.push(Standing { kind, spell, class, icon: icon.to_string(), at, left: seconds.max(5.0), radius: radius.max(3.0), slot, uses, next: first, effect, zone: false });
        ctx.panels.notice(match kind {
            PlaceKind::Totem => "A totem rises.",
            PlaceKind::Trap => "You set a trap.",
            PlaceKind::Well => "A well of light rises.",
        });
    }

    /// Time goes by: what stands pulses, waits for an enemy, gives; what is over is taken away.
    pub fn update(&mut self, ctx: &mut Ctx, dt: f32) {
        for i in (0..self.list.len()).rev() {
            let mut s = self.list[i].clone();
            s.left -= dt;
            s.next -= dt;
            let mut gone = s.left <= 0.0;
            match s.kind {
                PlaceKind::Totem if s.next <= 0.0 => {
                    s.next = TOTEM_EVERY;
                    pulse(ctx, &s, None);
                }
                PlaceKind::Trap if !s.zone => {
                    if let Some(victim) = ctx.wolves.as_ref().and_then(|w| w.foe_near(s.at, s.radius)) {
                        pulse(ctx, &s, Some(victim));
                        // Its spell goes on if it lasts (a frost trap's cold lies there for a while); else it is spent.
                        let lasts = content::classic_spell(s.spell).map_or(0.0, |c| c.duration as f32 / 1000.0);
                        if lasts > 10.0 {
                            (s.zone, s.left, s.next) = (true, lasts, 1.0);
                        } else {
                            gone = true;
                        }
                    }
                }
                PlaceKind::Trap if s.next <= 0.0 => {
                    s.next = TOTEM_EVERY;
                    pulse(ctx, &s, None);
                }
                PlaceKind::Well if s.next <= 0.0 => {
                    let hurt = ctx.player.vitals.health.fraction() < 0.99;
                    if hurt && flat(ctx.player.feet() - s.at) <= s.radius && s.uses > 0 {
                        s.uses -= 1;
                        s.next = WELL_EVERY;
                        pulse(ctx, &s, None);
                    }
                    gone |= s.uses == 0;
                }
                _ => {}
            }
            if gone {
                s.effect.into_iter().for_each(|e| ctx.effects.stop(e));
                self.list.remove(i);
            } else {
                self.list[i] = s;
            }
        }
    }

    /// Everything that stands is taken away (the map was left).
    pub fn clear(&mut self, effects: &mut crate::effects::Effects) {
        for s in self.list.drain(..) {
            s.effect.into_iter().for_each(|e| effects.stop(e));
        }
    }
}

/// What a standing thing does now: its spell for the character if he is within its reach, against the enemy `victim` (or the nearest one near it); what hurts or slows
/// all round goes on the creatures round the thing, not round the character.
fn pulse(ctx: &mut Ctx, s: &Standing, victim: Option<usize>) {
    let Some(classic) = content::classic_spell(s.spell) else { return };
    let level = ctx.panels.progress.level;
    let victim = victim.or_else(|| ctx.wolves.as_ref().and_then(|w| w.foe_near(s.at, s.radius.max(20.0))));
    let mut out = Vec::new();
    {
        let panels = &mut *ctx.panels;
        procs::run_cast(classic, level, &mut || panels.consumed.dice(), &mut out, victim.is_some());
    }
    let k = ctx.panels.spell_power(s.class) / levels::spell_scale(classic.level.max(1));
    let near = flat(ctx.player.feet() - s.at) <= s.radius;
    let lasts = out.iter().filter_map(|o| if let Outcome::Timed(_, secs) = o { Some(*secs) } else { None }).fold(0.0, f32::max);
    let mut rest = Vec::new();
    if let Some(wolves) = ctx.wolves.as_mut() {
        wolves.set_source(&classic.name, &s.icon, classic.dispel);
    }
    for o in out.into_iter().map(|o| scaled(o, k)) {
        match o {
            Outcome::Area { amount, radius } => {
                if let Some(wolves) = ctx.wolves.as_mut() {
                    for (at, dealt) in wolves.spell_area(s.at, radius, |_, _| amount) {
                        if let Some(text) = ctx.text.as_mut() {
                            text.damage_dealt(at, dealt as u32);
                        }
                    }
                }
            }
            Outcome::PutArea { status, radius } => {
                if let Some(wolves) = ctx.wolves.as_mut() {
                    wolves.afflict_area(s.at, radius, status);
                }
            }
            Outcome::Hurt { .. } | Outcome::Leech(_) | Outcome::Put(_) | Outcome::Interrupt | Outcome::Burn(_) | Outcome::Dispel { enemy: true, .. } | Outcome::Charm { .. } => {
                if victim.is_some() {
                    rest.push(o);
                }
            }
            _ => {
                if near {
                    rest.push(o);
                }
            }
        }
    }
    if near && lasts > 0.0 {
        ctx.panels.consumed.show(&s.icon, lasts);
    }
    if let Some(wolves) = ctx.wolves.as_mut() {
        procs::apply(rest, victim, ctx.player, ctx.panels, wolves, ctx.text.as_deref_mut());
    }
}

/// What the riders of a spell do about pets (see `Rider`): called, brought, sent away, revived, healed or tamed.
pub fn pet_rider(ctx: &mut Ctx, rider: Rider, victim: Option<usize>, feet: Vec3, yaw: f32) {
    let level = ctx.panels.progress.level;
    let at = feet + Vec3::new(yaw.sin(), 0.0, -yaw.cos()) * PET_AT;
    let Some(wolves) = ctx.wolves.as_mut() else { return };
    match rider {
        Rider::Summon { entry, spell, seconds } => {
            wolves.dismiss_pet();
            wolves.call_ally(entry, at, yaw, level, seconds, spell, true);
            // What stays is the pet that is called again by a spell of the hunter.
            if seconds == 0.0 {
                ctx.panels.pet = Some(format!("c{entry}"));
            }
        }
        Rider::CallPet => {
            if wolves.pet().is_some() {
                return ctx.panels.error("You already have a pet.");
            }
            let entry = ctx.panels.pet.as_ref().and_then(|id| id.strip_prefix('c')).and_then(|n| n.parse().ok());
            match entry {
                Some(entry) => drop(wolves.call_ally(entry, at, yaw, level, 0.0, 0, true)),
                None => ctx.panels.error("You have no pet. Tame a beast first."),
            }
        }
        Rider::DismissPet => wolves.dismiss_pet(),
        Rider::RevivePet => {
            if !wolves.revive_pet() {
                ctx.panels.error("Your pet is not dead.");
            }
        }
        Rider::MendPet(percent) => wolves.mend_pet(percent),
        Rider::Tame => {
            let Some(id) = victim.and_then(|v| wolves.tame(v, level + TAME_LEVELS)) else { return ctx.panels.error("You cannot tame that.") };
            wolves.dismiss_pet();
            ctx.panels.pet = Some(id.clone());
            if let Some(entry) = id.strip_prefix('c').and_then(|n| n.parse().ok()) {
                wolves.call_ally(entry, at, yaw, level, 0.0, 0, true);
            }
            ctx.panels.notice("You have a new pet.");
        }
        _ => {}
    }
}

/// The spell goes off: the effects of the original's spell are carried out for the creature `victim` (if there is one selected within reach).
pub fn cast(ctx: &mut Ctx, spell: usize, victim: Option<usize>) {
    let def = content::spell(spell);
    if !matches!(def.effect, Effect::Classic { .. }) {
        return;
    }
    let Some(classic) = content::classic_spell(def.spell) else { return };
    let level = ctx.panels.progress.level;
    let mut out = Vec::new();
    {
        let panels = &mut *ctx.panels;
        procs::run_cast(classic, level, &mut || panels.consumed.dice(), &mut out, victim.is_some());
    }
    let k = ctx.panels.spell_power(def.class) / levels::spell_scale(classic.level.max(1));
    let out: Vec<Outcome> = out.into_iter().map(|o| scaled(o, k)).collect();
    // What lasts is shown among the buffs, with the seconds that are left.
    let lasts = out.iter().filter_map(|o| if let Outcome::Timed(_, s) = o { Some(*s) } else { None }).fold(0.0, f32::max);
    if lasts > 0.0 {
        ctx.panels.consumed.show(&def.icon, lasts);
    }
    if let Some(wolves) = ctx.wolves.as_mut() {
        wolves.set_source(&classic.name, &def.icon, classic.dispel);
        procs::apply(out, victim, ctx.player, ctx.panels, wolves, ctx.text.as_deref_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a spell sets down or calls is in the game: the creature (with a model) of a totem or a pet, and the spells they cast.
    #[test]
    fn what_a_spell_sets_down_or_calls_exists() {
        let c = content::get();
        for def in &c.spells {
            for rider in &def.then {
                let (entry, spell, totem) = match *rider {
                    Rider::Place { entry, spell, kind, .. } => (entry, spell, kind == PlaceKind::Totem),
                    Rider::Summon { entry, spell, .. } => (entry, spell, true),
                    _ => continue,
                };
                if totem {
                    let kind = c.creature_numbers.get(&format!("c{entry}")).unwrap_or_else(|| panic!("{}: no creature c{entry}", def.name));
                    assert!(matches!(c.creatures[*kind].body, content::Body::Beast { .. } | content::Body::Humanoid { .. }), "{}: c{entry} has no body", def.name);
                }
                assert!(spell == 0 || content::classic_spell(spell).is_some(), "{}: no spell {spell}", def.name);
            }
        }
    }

    /// A spell that runs from the original's does something: an outcome, or a rider that the spell carries.
    #[test]
    fn every_spell_run_from_the_originals_does_something() {
        let nothing: Vec<&str> = content::get()
            .spells
            .iter()
            .filter(|def| matches!(def.effect, Effect::Classic { .. }))
            .filter(|def| {
                let classic = content::classic_spell(def.spell).unwrap_or_else(|| panic!("{} has no spell of the original", def.name));
                let mut out = Vec::new();
                procs::run_cast(classic, 30, &mut || 0, &mut out, true);
                out.is_empty() && def.then.is_empty()
            })
            .map(|def| def.name.as_str())
            .collect();
        assert!(nothing.is_empty(), "do nothing: {nothing:?}");
    }

    /// What a spell does is read from the original's tables: a conjuring makes items, a fear frightens, a seal goes off on a blow, a dispel takes auras off.
    #[test]
    fn the_effects_of_the_originals_spells_are_what_they_are() {
        let run = |id: u32, foe: bool| {
            let spell = content::classic_spell(id).unwrap_or_else(|| panic!("no spell {id}"));
            let mut out = Vec::new();
            procs::run_cast(spell, 20, &mut || 0, &mut out, foe);
            out
        };
        assert!(run(587, false).iter().any(|o| matches!(o, Outcome::Item { .. })), "Conjure Food");
        assert!(run(5782, true).iter().any(|o| matches!(o, Outcome::Put(Status::Fear(_)))), "Fear");
        assert!(run(339, true).iter().any(|o| matches!(o, Outcome::Put(Status::Root(_)))), "Entangling Roots");
        assert!(run(118, true).iter().any(|o| matches!(o, Outcome::Put(Status::Sheep(_)))), "Polymorph");
        assert!(run(2139, true).contains(&Outcome::Interrupt), "Counterspell");
        assert!(run(20154, false).iter().any(|o| matches!(o, Outcome::Timed(Timed::Seal(_), _))), "Seal of Righteousness");
        assert!(run(528, false).iter().any(|o| matches!(o, Outcome::Dispel { enemy: false, .. })), "Cure Disease");
        assert!(run(1856, false).contains(&Outcome::Stealth { vanish: true }), "Vanish");
    }
}
