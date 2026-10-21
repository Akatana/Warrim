//! What the spells of weapons and gear do when they go off: the chance of a weapon's spell on a hit, what the gear does to whoever strikes the character (a spell
//! with a chance when struck, a damage shield), and what these spells put on the character or the creature for a while. The effects are those of the original's spell
//! tables (`classic_spells.ron`) as far as the game models them: damage, a leech, heals, mana, extra swings, damage over time, a stun, a slow, stats and the other
//! effects of `Gear`, health regained every second, and shields.

use crate::combattext::CombatText;
use crate::consume::Timed;
use crate::content::{self, ClassicSpell};
use crate::creature::{Creatures, Status};
use crate::items::{self, Stack, UseEffect};
use crate::panels::Panels;
use crate::player::Player;

/// What a spell that went off does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// Damage to the creature (the school is told for the day creatures resist).
    Hurt { school: u32, amount: f32 },
    /// Damage to the creature that the character regains as health.
    Leech(f32),
    /// Something put on the creature.
    Put(Status),
    Heal(f32),
    Mana(f32),
    /// Something put on the character for some seconds.
    Timed(Timed, f32),
    /// Swings that come for nothing.
    Extra(u32),
}

/// A chance in percent met by the dice.
fn rolled(dice: &mut dyn FnMut() -> u32, percent: f32) -> bool {
    ((dice() % 10_000) as f32 / 100.0) < percent
}

/// What a spell does, added to `out`: each of its effects, and the spell a trigger effect names (to a depth of three).
fn run(spell: &ClassicSpell, level: u32, dice: &mut dyn FnMut() -> u32, out: &mut Vec<Outcome>, depth: u32) {
    let seconds = if spell.duration > 0 { spell.duration as f32 / 1000.0 } else { 10.0 };
    for (n, e) in spell.effects.iter().enumerate() {
        let points = spell.points(n, level, dice()).max(0.0);
        let period = if e.period > 0 { e.period as f32 / 1000.0 } else { 3.0 };
        match e.kind {
            2 => out.push(Outcome::Hurt { school: spell.school, amount: points }),
            9 => out.push(Outcome::Leech(points)),
            10 => out.push(Outcome::Heal(points)),
            30 if e.misc == 0 => out.push(Outcome::Mana(points)),
            19 => out.push(Outcome::Extra(points as u32)),
            64 if depth < 3 => {
                if let Some(t) = content::classic_spell(e.trigger) {
                    run(t, level, dice, out, depth + 1);
                }
            }
            6 if e.enemy() => match e.aura {
                3 => out.push(Outcome::Put(Status::Over { amount: points, period, seconds })),
                12 => out.push(Outcome::Put(Status::Stun(seconds))),
                // A slow's points are negative percents.
                33 => out.push(Outcome::Put(Status::Slow((1.0 + spell.points(n, level, 0) / 100.0).clamp(0.1, 1.0), seconds))),
                _ => {}
            },
            6 => {
                let amount = points as i32;
                let timed = match e.aura {
                    15 => Some(vec![Timed::Shield { amount: points, school: spell.school }]),
                    8 => Some(vec![Timed::Regen(points / period)]),
                    29 => Some((0..5).filter(|s| e.misc == -1 || e.misc == *s).map(|s| Timed::Gear(UseEffect::Boost(s as u8, amount, 0.0))).collect()),
                    22 if e.misc == 1 => Some(vec![Timed::Gear(UseEffect::Boost(5, amount, 0.0))]),
                    13 | 22 | 31 | 47 | 49 | 51 | 52 | 54 | 55 | 71 | 85 | 99 | 102 | 124 | 131 | 135 | 138 | 141 | 158 | 161 | 180 => Some(vec![Timed::Gear(UseEffect::Equip(e.aura as u16, e.misc, amount))]),
                    _ => None,
                };
                out.extend(timed.into_iter().flatten().map(|t| Outcome::Timed(t, seconds)));
            }
            _ => {}
        }
    }
}

/// The spells of a weapon that go off when it lands a blow (`speed` seconds between its swings): the chance is the spell's per-minute rate over the swings, else its own
/// chance, else (for the many with a chance over 100) a chance that grows with the weapon's slowness (CMaNGOS `GetWeaponProcChance`).
pub fn on_hit(item: usize, speed: f32, level: u32, dice: &mut dyn FnMut() -> u32, out: &mut Vec<Outcome>) {
    for s in items::get(item).spells.iter().filter(|s| s.trigger == 2) {
        let Some(spell) = content::classic_spell(s.spell) else { continue };
        let chance = if s.ppm > 0.0 {
            speed * s.ppm / 0.6
        } else if spell.proc.1 == 0 || spell.proc.1 > 100 {
            speed * 1.8
        } else {
            spell.proc.1 as f32
        };
        if rolled(dice, chance) {
            run(spell, level, dice, out, 0);
        }
    }
}

/// What hits back at whoever strikes the character in melee: the damage shields of the worn gear and the timed `shields` (damage and school), and the worn
/// spells that go off when struck (by their chance).
pub fn struck(equipment: &[Option<Stack>], shields: &[(f32, u32)], level: u32, dice: &mut dyn FnMut() -> u32, out: &mut Vec<Outcome>) {
    out.extend(shields.iter().map(|&(amount, school)| Outcome::Hurt { school, amount }));
    for stack in equipment.iter().flatten() {
        for s in items::get(stack.item).spells.iter().filter(|s| s.trigger == 1) {
            let Some(spell) = content::classic_spell(s.spell) else { continue };
            for (n, e) in spell.effects.iter().enumerate() {
                match e.aura {
                    15 => out.push(Outcome::Hurt { school: spell.school, amount: spell.points(n, level, dice()).max(0.0) }),
                    // Taken a blow (0x8) or the blow of a spell (0x20) in melee.
                    42 if spell.proc.0 & 0x28 != 0 => {
                        let chance = if spell.proc.1 == 0 || spell.proc.1 > 100 { 100.0 } else { spell.proc.1 as f32 };
                        if let (true, Some(trigger)) = (rolled(dice, chance), content::classic_spell(e.trigger)) {
                            run(trigger, level, dice, out, 1);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Carry out what went off: on the creature `target` if there is one, and on the character.
pub fn apply(out: Vec<Outcome>, target: Option<usize>, player: &mut Player, panels: &mut Panels, wolves: &mut Creatures, mut text: Option<&mut CombatText>) {
    for o in out {
        match o {
            Outcome::Hurt { amount, .. } | Outcome::Leech(amount) => {
                let hit = target.and_then(|i| wolves.spell_hit(i, amount));
                if let (Some(at), Some(text)) = (hit, text.as_deref_mut()) {
                    text.damage_dealt(at, amount as u32);
                }
                if let (Outcome::Leech(_), Some(_)) = (o, hit) {
                    let healed = player.heal(amount);
                    if let Some(text) = text.as_deref_mut() {
                        text.healed(healed as u32);
                    }
                }
            }
            Outcome::Put(status) => {
                if let Some(i) = target {
                    wolves.afflict(i, status);
                }
            }
            Outcome::Heal(amount) => {
                let healed = player.heal(amount);
                if let Some(text) = text.as_deref_mut() {
                    text.healed(healed as u32);
                }
            }
            Outcome::Mana(amount) => player.vitals.mana.current = (player.vitals.mana.current + amount).min(player.vitals.mana.max),
            Outcome::Timed(what, seconds) => panels.proc_timed(what, seconds),
            Outcome::Extra(n) => player.extra_swings(n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dice(values: &'static [u32]) -> impl FnMut() -> u32 {
        let mut i = 0;
        move || {
            i += 1;
            values[(i - 1) % values.len()]
        }
    }

    /// "Smites an enemy for 30 Holy damage" (i13519 of Shortsword of Vengeance) and a bleed that hurts over time.
    #[test]
    fn a_spell_hurts_or_puts_a_wound_on_the_creature() {
        let mut out = Vec::new();
        run(content::classic_spell(13519).unwrap(), 20, &mut dice(&[0]), &mut out, 0);
        assert_eq!(out, [Outcome::Hurt { school: 1, amount: 30.0 }]);
        out.clear();
        run(content::classic_spell(17504).unwrap(), 20, &mut dice(&[0]), &mut out, 0);
        assert_eq!(out, [Outcome::Put(Status::Over { amount: 12.0, period: 3.0, seconds: 30.0 })]);
    }

    /// A weapon with a spell that goes off on a hit, by its rate per minute: always with a roll of 0, never with a roll of 99.99.
    #[test]
    fn a_weapon_spell_goes_off_by_its_chance() {
        let item = items::all().iter().position(|i| i.spells.iter().any(|s| s.trigger == 2 && s.ppm > 0.0 && content::classic_spell(s.spell).is_some_and(|c| c.effects.iter().any(|e| e.kind == 2)))).expect("a weapon with a ppm");
        let (mut hit, mut none) = (Vec::new(), Vec::new());
        on_hit(item, 2.0, 30, &mut dice(&[0]), &mut hit);
        on_hit(item, 2.0, 30, &mut dice(&[9999]), &mut none);
        assert!(hit.iter().any(|o| matches!(o, Outcome::Hurt { .. })) && none.is_empty(), "{hit:?}");
    }

    /// A damage shield and a spell that hits back when the character is struck are in the gear of the original; a roll of 0 always meets a chance.
    #[test]
    fn what_strikes_the_character_is_hit_back() {
        let mut out = Vec::new();
        struck(&[], &[(5.0, 2)], 20, &mut dice(&[0]), &mut out);
        assert_eq!(out, [Outcome::Hurt { school: 2, amount: 5.0 }]);
    }
}
