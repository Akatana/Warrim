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
    /// Damage to every creature round the character.
    Area { amount: f32, radius: f32 },
    /// Something put on every creature round the character.
    PutArea { status: Status, radius: f32 },
    /// Items (entries of the original's items) made in the pack.
    Item { entry: u32, count: u32 },
    /// What a dispel takes away: the kinds (bit `n` for kind `n`: 1 magic, 2 curse, 3 disease, 4 poison) from the character, or from the creature.
    Dispel { mask: u32, enemy: bool },
    /// The spell the creature is casting is broken off.
    Interrupt,
    /// The character hides (sneaks), and with `vanish` the creatures that fight him lose him.
    Stealth { vanish: bool },
    /// The character plays dead: the creatures that fight him lose him.
    Feign,
    /// Rage or energy given to the character (stamina).
    Stamina(f32),
    /// The character blinks forward this many yards.
    Blink(f32),
    /// The character leaps at the creature.
    Charge,
    /// Coins taken from the creature's pockets.
    Pickpocket,
    /// An enchantment on the weapon in the hand for some seconds.
    Coat { enchant: u32, seconds: f32 },
    /// Mana taken from the creature.
    Burn(f32),
    /// Health given for mana (a life tap).
    LifeTap(f32),
    /// The creature fights for the character for some seconds (mind control, enslave): `only` is the type of creature it works on (0 for any).
    Charm { seconds: f32, only: u32 },
    /// A fallen creature near the character fights for him; or a pet that is dead gets up.
    Resurrect,
    /// The character takes the shape of a beast: the display row of its model and the percent it adds to his running speed.
    Form { display: u32, percent: i32 },
    /// The creatures of a type (1 beast, 2 dragonkin, 3 demon, 4 elemental, 5 giant, 6 undead, 7 humanoid) show on the map; the same again puts it out. (The game has no creatures that hide, so tracking them shows none.)
    Track(u32),
    /// The character's eyes leave him for some seconds, and go as far from him as this many yards.
    Scout { seconds: f32, reach: f32 },
}

/// The shapes the shapeshifting spells take (the ids of `ShapeshiftForm.dbc`): the display row of the model and the speed it adds in percent. The stances of a warrior have no
/// beast to be.
pub const FORM_DISPLAYS: [u32; 5] = [892, 632, 2428, 2281, 4613];

fn form_look(form: i32) -> Option<(u32, i32)> {
    match form {
        // Cat, travel (a cheetah), aquatic (a sea lion), bear and dire bear, ghost wolf.
        1 => Some((892, 0)),
        3 => Some((632, 40)),
        4 => Some((2428, 0)),
        5 | 8 => Some((2281, 0)),
        16 => Some((4613, 40)),
        _ => None,
    }
}

/// A chance in percent met by the dice.
fn rolled(dice: &mut dyn FnMut() -> u32, percent: f32) -> bool {
    ((dice() % 10_000) as f32 / 100.0) < percent
}

/// How far the eyes of a scouting spell go from the character, yards.
const SCOUT_REACH: f32 = 100.0;

/// How far from the character a fallen creature is raised from, yards, and for how long it fights, seconds.
const RAISE_REACH: f32 = 30.0;
const RAISE_SECONDS: f32 = 180.0;

/// How much more damage a creature takes from what weakens its armor or resistance, and how much less it deals when weakened in attack.
const WEAKEN_TAKEN: f32 = 1.1;
const WEAKEN_DEALT: f32 = 0.85;

/// Who casts a spell, for the effects whose target depends on it.
#[derive(Clone, Copy, PartialEq)]
pub enum Cast {
    /// The spell of a weapon, a potion, a piece of gear.
    Gear,
    /// A spell of a class, with a creature selected.
    Foe,
    /// A spell of a class, with none.
    Own,
}

/// What a spell puts on a creature by its aura `e` (and its mechanic), for `seconds`.
fn status_of(spell: &ClassicSpell, e: &content::ClassicEffect, points: f32, seconds: f32) -> Option<Status> {
    Some(match e.aura {
        3 | 53 => Status::Over { amount: points, period: if e.period > 0 { e.period as f32 / 1000.0 } else { 3.0 }, seconds },
        12 => match spell.mechanic {
            10 | 14 | 20 | 30 => Status::Sleep(seconds),
            18 => Status::Banish(seconds),
            _ => Status::Stun(seconds),
        },
        // Fear, and what leaves it confused or horrified.
        5 | 7 => Status::Fear(seconds),
        26 => Status::Root(seconds),
        56 => Status::Sheep(seconds),
        // A slow's points are negative percents.
        33 => Status::Slow((1.0 + e.low.min(e.high) as f32 / 100.0).clamp(0.1, 1.0), seconds),
        22 | 68 | 127 => Status::Weaken { dealt: 1.0, taken: WEAKEN_TAKEN, seconds },
        // Disarmed: it hits half as hard.
        67 => Status::Weaken { dealt: 0.5, taken: 1.0, seconds },
        13 | 29 | 79 | 99 if e.low < 0 => Status::Weaken { dealt: WEAKEN_DEALT, taken: 1.0, seconds },
        _ => return None,
    })
}

/// What a spell does, added to `out`: each of its effects, and the spell a trigger effect names (to a depth of three).
fn run(spell: &ClassicSpell, level: u32, dice: &mut dyn FnMut() -> u32, out: &mut Vec<Outcome>, depth: u32) {
    run_with(spell, level, dice, out, depth, Cast::Gear)
}

/// What a spell of the original does, for a creature that casts it (a pet): its outcomes.
pub fn effects_of(spell: &ClassicSpell, level: u32, dice: &mut dyn FnMut() -> u32) -> Vec<Outcome> {
    let mut out = Vec::new();
    run(spell, level, dice, &mut out, 0);
    out
}

/// The same for a spell a class casts: it also reaches everything round the character, and does what only such a spell does (conjuring, dispelling, hiding).
/// An effect for "any unit" is for the creature when there is one selected.
pub fn run_cast(spell: &ClassicSpell, level: u32, dice: &mut dyn FnMut() -> u32, out: &mut Vec<Outcome>, foe: bool) {
    run_with(spell, level, dice, out, 0, if foe { Cast::Foe } else { Cast::Own })
}

fn run_with(spell: &ClassicSpell, level: u32, dice: &mut dyn FnMut() -> u32, out: &mut Vec<Outcome>, depth: u32, cast: Cast) {
    let class = cast != Cast::Gear;
    // An aura of a class that has no time holds until it is given up (a shape, an aura of a paladin): an hour is as good.
    let seconds = if spell.duration > 0 { spell.duration as f32 / 1000.0 } else if class { 3600.0 } else { 10.0 };
    for (n, e) in spell.effects.iter().enumerate() {
        let raw = spell.points(n, level, dice());
        let points = raw.max(0.0);
        let period = if e.period > 0 { e.period as f32 / 1000.0 } else { 3.0 };
        // An effect for any unit is for the creature when there is one selected.
        let enemy = e.enemy() || (cast == Cast::Foe && e.targets.0 == 25);
        let round = class && enemy && e.area();
        match e.kind {
            2 if round => out.push(Outcome::Area { amount: points, radius: e.radius.max(5.0) }),
            2 => out.push(Outcome::Hurt { school: spell.school, amount: points }),
            9 => out.push(Outcome::Leech(points)),
            10 => out.push(Outcome::Heal(points)),
            30 if e.misc == 0 => out.push(Outcome::Mana(points)),
            30 if class => out.push(Outcome::Stamina(if e.misc == 1 { points / 10.0 } else { points / 3.0 })),
            19 => out.push(Outcome::Extra(points as u32)),
            64 if depth < 3 => {
                if let Some(t) = content::classic_spell(e.trigger) {
                    run_with(t, level, dice, out, depth + 1, cast);
                }
            }
            24 if class && e.item != 0 => out.push(Outcome::Item { entry: e.item, count: points.max(1.0) as u32 }),
            38 if class => out.push(Outcome::Dispel { mask: 1 << e.misc.clamp(0, 31), enemy }),
            68 if class => out.push(Outcome::Interrupt),
            62 if class => out.extend([Outcome::Hurt { school: spell.school, amount: points }, Outcome::Burn(points)]),
            8 if class => out.extend([Outcome::Burn(points), Outcome::Mana(points)]),
            71 if class => out.push(Outcome::Pickpocket),
            // Everything the health holds, at once (a lay on hands).
            67 if class => out.push(Outcome::Heal(f32::MAX)),
            29 if class => out.push(Outcome::Blink(e.radius.max(points).max(10.0))),
            96 if class => out.push(Outcome::Charge),
            54 if class && e.misc > 0 => out.push(Outcome::Coat { enchant: e.misc as u32, seconds }),
            // A spell that casts another every moment (a channel): the blows it makes in the time, all at once.
            6 if class && e.aura == 23 && e.trigger != 0 => {
                if let Some(t) = content::classic_spell(e.trigger) {
                    let mut each = Vec::new();
                    run_with(t, level, dice, &mut each, depth + 1, cast);
                    let ticks = (seconds / period).round().max(1.0);
                    out.extend(each.into_iter().map(|o| match o {
                        Outcome::Hurt { school, amount } => Outcome::Hurt { school, amount: amount * ticks },
                        Outcome::Heal(amount) => Outcome::Heal(amount * ticks),
                        other => other,
                    }));
                }
            }
            // Mana drained every moment.
            6 if class && enemy && e.aura == 64 => {
                let total = points * (seconds / period).round().max(1.0);
                out.extend([Outcome::Burn(total), Outcome::Mana(total)]);
            }
            // What the tables leave to a script.
            3 | 77 if class => match spell.name.as_str() {
                "Life Tap" => out.push(Outcome::LifeTap(points.max(30.0))),
                "Execute" => out.push(Outcome::Hurt { school: 0, amount: points.max(125.0) }),
                "Judgement" => out.push(Outcome::Hurt { school: 1, amount: 6.0 }),
                "Holy Shock" if cast == Cast::Foe => out.push(Outcome::Hurt { school: 1, amount: 8.0 }),
                "Holy Shock" => out.push(Outcome::Heal(8.0)),

                _ => {}
            },
            6 if class && spell.name == "Frenzied Regeneration" => out.push(Outcome::Timed(Timed::Regen(6.0), seconds)),
            6 if class && spell.name == "Barkskin" => out.push(Outcome::Timed(Timed::Gear(UseEffect::Equip(87, 127, -20)), seconds)),
            // Rage given over the time (an enrage).
            6 if class && e.aura == 24 && e.misc == 1 => out.push(Outcome::Stamina(points * (seconds / period).round().max(1.0) / 10.0)),
            // A creature taken over (a mind control works on people, an enslave on demons).
            6 | 35 if class && enemy && matches!(e.aura, 2 | 6) => {
                out.push(Outcome::Charm { seconds: seconds.min(300.0), only: if spell.name == "Enslave Demon" { 3 } else { 7 } });
            }
            113 if class => out.push(Outcome::Resurrect),
            72 | 73 if class => out.push(Outcome::Scout { seconds: seconds.min(120.0), reach: SCOUT_REACH }),
            6 if class && matches!(e.aura, 1 | 128) => out.push(Outcome::Scout { seconds: seconds.min(120.0), reach: SCOUT_REACH }),
            6 if class && e.aura == 44 => out.push(Outcome::Track(e.misc.max(0) as u32)),
            6 if class && e.aura == 151 => out.push(Outcome::Track(255)),
            6 | 35 if enemy && round => {
                if let Some(status) = status_of(spell, e, points, seconds) {
                    out.push(Outcome::PutArea { status, radius: e.radius.max(5.0) });
                }
            }
            6 | 35 if enemy => {
                if let Some(status) = status_of(spell, e, points, seconds) {
                    out.push(Outcome::Put(status));
                    // A drain gives back what it takes.
                    if e.aura == 53 {
                        out.push(Outcome::Heal(points * (seconds / period).max(1.0)));
                    }
                }
            }
            6 | 35 => {
                let amount = points as i32;
                let timed = match e.aura {
                    15 => Some(vec![Timed::Shield { amount: points, school: spell.school }]),
                    // What absorbs the damage of blows and spells, and a bubble that nothing gets through.
                    69 | 97 => Some(vec![Timed::Absorb(points)]),
                    39 | 40 if class && points == 0.0 => Some(vec![Timed::Absorb(f32::MAX)]),
                    8 => Some(vec![Timed::Regen(points / period)]),
                    29 => Some((0..5).filter(|s| e.misc == -1 || e.misc == *s).map(|s| Timed::Gear(UseEffect::Boost(s as u8, amount, 0.0))).collect()),
                    22 if e.misc == 1 => Some(vec![Timed::Gear(UseEffect::Boost(5, amount, 0.0))]),
                    // A spell that goes off when the character lands a blow (a seal), or on whoever strikes him.
                    42 if class && e.trigger != 0 && spell.proc.0 & 0x14 != 0 => Some(vec![Timed::Seal(e.trigger)]),
                    42 if class && e.trigger != 0 => Some(vec![Timed::Struck(e.trigger)]),
                    // The seals the tables leave to a script: each blow does a little holy damage.
                    4 if class && spell.name.starts_with("Seal of") => Some(vec![Timed::Seal(spell.spell)]),
                    87 | 14 | 149 if class && raw < 0.0 => Some(vec![Timed::Gear(UseEffect::Equip(87, e.misc, raw as i32))]),
                    // The resistance of a school that does not stack (an aura of a paladin, an aspect).
                    143 if class => Some(vec![Timed::Gear(UseEffect::Equip(22, e.misc, amount))]),
                    // Mana given every moment, and more of it regained (a blessing of wisdom, an innervate).
                    24 if class && e.misc == 0 => Some(vec![Timed::Gear(UseEffect::Equip(85, 0, (points * 5.0 / period) as i32))]),
                    110 | 134 if class => Some(vec![Timed::Gear(UseEffect::Equip(85, 0, 20 + level as i32 * 3))]),
                    // All the stats a tenth more (a blessing of kings).
                    137 if class => Some((0..5).map(|s| Timed::Gear(UseEffect::Boost(s, 2 + level as i32 / 5, 0.0))).collect()),
                    // What makes spells heal for more (a blessing of light), the shot that comes faster (a rapid fire), a soft fall and breath under water.
                    4 if class && spell.name.contains("Blessing of Light") => Some(vec![Timed::Gear(UseEffect::Equip(135, 0, 20 + level as i32 * 2))]),
                    140 if class => Some(vec![Timed::Gear(UseEffect::Equip(141, 0, raw.abs() as i32))]),
                    105 | 82 if class => Some(vec![Timed::Gear(UseEffect::Equip(e.aura as u16, 0, 1))]),
                    // What the game has no use for (threat, immunities, the pushback of casts): the spell still lasts, and is shown among the buffs.
                    10 | 17 | 18 | 19 | 77 | 94 | 100 | 103 | 107 | 115 | 117 | 149 | 153 | 14 if class => Some(vec![Timed::Gear(UseEffect::Equip(10, 0, 0))]),
                    87 if class => Some(vec![Timed::Gear(UseEffect::Equip(87, e.misc, raw as i32))]),
                    // A stance of the warrior has no beast to be: it shows among the buffs.
                    36 if class && form_look(e.misc).is_none() => Some(vec![Timed::Gear(UseEffect::Equip(10, 0, 0))]),
                    13 | 22 | 31 | 47 | 49 | 51 | 52 | 54 | 55 | 71 | 85 | 99 | 102 | 124 | 131 | 135 | 138 | 141 | 158 | 161 | 180 => Some(vec![Timed::Gear(UseEffect::Equip(e.aura as u16, e.misc, amount))]),
                    _ => None,
                };
                out.extend(timed.into_iter().flatten().map(|t| Outcome::Timed(t, seconds)));
                match e.aura {
                    36 if class => out.extend(form_look(e.misc).map(|(display, percent)| Outcome::Form { display, percent })),
                    16 if class => out.push(Outcome::Stealth { vanish: spell.name == "Vanish" }),
                    66 if class => out.push(Outcome::Feign),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// What an enchantment of the weapon casts: by blows a minute, else by its own percent (CMaNGOS `CastItemCombatSpell`).
fn enchant_procs(enchant: u32, speed: f32, level: u32, dice: &mut dyn FnMut() -> u32, out: &mut Vec<Outcome>) {
    if let Some(e) = content::get().enchants.get(&enchant) {
        for &(id, ppm, percent) in &e.procs {
            let Some(spell) = content::classic_spell(id) else { continue };
            let chance = if ppm > 0.0 { speed * ppm / 0.6 } else if percent > 0.0 { percent } else { speed * 1.8 };
            if rolled(dice, chance) {
                run(spell, level, dice, out, 0);
            }
        }
    }
}

/// What the spells on the character (`Timed`) cast when a blow of the weapon in the hand lands: each seal goes off, and each coat (a poison, a weapon's blessing)
/// casts as an enchantment does.
pub fn on_timed(timed: &[Timed], speed: f32, level: u32, dice: &mut dyn FnMut() -> u32, out: &mut Vec<Outcome>) {
    for t in timed {
        match *t {
            Timed::Seal(id) => {
                if let Some(spell) = content::classic_spell(id) {
                    let before = out.len();
                    run(spell, level, dice, out, 1);
                    // A seal whose effect the tables do not give: a few points of holy damage that grow with the level.
                    if out.len() == before && spell.name.starts_with("Seal of") {
                        out.push(Outcome::Hurt { school: 1, amount: 2.0 + level as f32 * 0.5 });
                    }
                }
            }
            Timed::Coat(enchant) => enchant_procs(enchant, speed, level, dice, out),
            _ => {}
        }
    }
}

/// The spells of a weapon that go off when it lands a blow (`speed` seconds between its swings): the chance is the spell's per-minute rate over the swings, else its own
/// chance, else (for the many with a chance over 100) a chance that grows with the weapon's slowness (CMaNGOS `GetWeaponProcChance`).
pub fn on_hit(weapon: Stack, speed: f32, level: u32, dice: &mut dyn FnMut() -> u32, out: &mut Vec<Outcome>) {
    enchant_procs(weapon.enchant, speed, level, dice, out);
    for s in items::get(weapon.item).spells.iter().filter(|s| s.trigger == 2) {
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
pub fn struck(equipment: &[Option<Stack>], shields: &[(f32, u32)], reactions: &[u32], level: u32, dice: &mut dyn FnMut() -> u32, out: &mut Vec<Outcome>) {
    out.extend(shields.iter().map(|&(amount, school)| Outcome::Hurt { school, amount }));
    for spell in reactions.iter().filter_map(|&id| content::classic_spell(id)) {
        run(spell, level, dice, out, 1);
    }
    for stack in equipment.iter().flatten() {
        // The worn spells of the item, and those of its enchantment.
        let enchanted = content::get().enchants.get(&stack.enchant).into_iter().flat_map(|e| e.spells.iter().copied());
        for id in items::get(stack.item).spells.iter().filter(|s| s.trigger == 1).map(|s| s.spell).chain(enchanted) {
            let Some(spell) = content::classic_spell(id) else { continue };
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
            Outcome::Area { amount, radius } => {
                for (at, dealt) in wolves.spell_area(player.feet(), radius, |_, _| amount) {
                    if let Some(text) = text.as_deref_mut() {
                        text.damage_dealt(at, dealt as u32);
                    }
                }
            }
            Outcome::PutArea { status, radius } => wolves.afflict_area(player.feet(), radius, status),
            Outcome::Item { entry, count } => {
                if let Some(item) = items::all().iter().position(|i| i.id == format!("i{entry}")) {
                    panels.inv.add(Stack { item, count: count.min(items::get(item).stack), enchant: 0 });
                    panels.notice(&format!("You create: {}.", items::get(item).name));
                }
            }
            Outcome::Dispel { mask, enemy: false } => panels.requests.push(crate::panels::Request::Cleanse(mask)),
            Outcome::Dispel { mask, enemy: true } => {
                if let Some(i) = target {
                    wolves.purge(i, mask);
                }
            }
            Outcome::Interrupt => {
                if let Some(i) = target {
                    wolves.interrupt(i);
                }
            }
            Outcome::Stealth { vanish } => {
                player.set_sneaking(true);
                if vanish {
                    panels.requests.push(crate::panels::Request::Vanish);
                }
            }
            Outcome::Feign => panels.requests.push(crate::panels::Request::Vanish),
            Outcome::Stamina(amount) => player.vitals.stamina.current = (player.vitals.stamina.current + amount).min(player.vitals.stamina.max),
            Outcome::Blink(yards) => panels.requests.push(crate::panels::Request::Blink(yards)),
            Outcome::Charge => {
                if let Some(at) = target.and_then(|i| wolves.enemy_at(i)) {
                    panels.requests.push(crate::panels::Request::Leap(at));
                }
            }
            Outcome::Pickpocket => {
                match target.and_then(|i| wolves.pickpocket(i)) {
                    Some(copper) => {
                        panels.inv.money += copper;
                        panels.notice(&format!("You steal {copper} copper."));
                    }
                    None => panels.error("You find nothing to steal."),
                }
            }
            Outcome::Coat { enchant, seconds } => panels.coat(enchant, seconds),
            Outcome::Burn(amount) => {
                if let Some(i) = target {
                    wolves.drain_mana(i, amount);
                }
            }
            Outcome::Charm { seconds, only } => {
                let level = panels.progress.level;
                let Some(i) = target else { return panels.error("You have no target.") };
                if wolves.enemy_info(i).is_some_and(|(_, kind)| only == 0 || kind == only) && wolves.charm(i, seconds, level) {
                    panels.notice("It fights for you.");
                } else {
                    panels.error("It cannot be controlled.");
                }
            }
            Outcome::Track(kind) => panels.track(kind),
            Outcome::Scout { seconds, reach } => panels.requests.push(crate::panels::Request::Scout { seconds, reach }),
            Outcome::Form { display, percent } => panels.requests.push(crate::panels::Request::Form { display, percent }),
            Outcome::Resurrect => {
                let level = panels.progress.level;
                if !wolves.revive_pet() && !wolves.raise(player.feet(), RAISE_REACH, level, RAISE_SECONDS) {
                    panels.error("There is nothing to raise.");
                }
            }
            Outcome::LifeTap(amount) => {
                let health = &mut player.vitals.health;
                let given = amount.min(health.current - 1.0).max(0.0);
                health.current -= given;
                player.vitals.mana.current = (player.vitals.mana.current + given).min(player.vitals.mana.max);
            }
        }
    }
    wolves.set_source("", "", 0);
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
        let weapon = Stack { item, count: 1, enchant: 0 };
        on_hit(weapon, 2.0, 30, &mut dice(&[0]), &mut hit);
        on_hit(weapon, 2.0, 30, &mut dice(&[9999]), &mut none);
        assert!(hit.iter().any(|o| matches!(o, Outcome::Hurt { .. })) && none.is_empty(), "{hit:?}");
    }

    /// An enchantment that casts on a blow: Fiery Weapon burns by blows a minute, and no enchantment means no spell.
    #[test]
    fn an_enchantment_casts_on_a_blow() {
        let c = content::get();
        let (&fiery, _) = c.enchants.iter().find(|(_, e)| e.name == "Fiery Weapon").expect("Fiery Weapon");
        let sword = items::find("longsword");
        let (mut hit, mut none, mut plain) = (Vec::new(), Vec::new(), Vec::new());
        on_hit(Stack { item: sword, count: 1, enchant: fiery }, 2.0, 30, &mut dice(&[0]), &mut hit);
        on_hit(Stack { item: sword, count: 1, enchant: fiery }, 2.0, 30, &mut dice(&[9999]), &mut none);
        on_hit(Stack { item: sword, count: 1, enchant: 0 }, 2.0, 30, &mut dice(&[0]), &mut plain);
        assert!(hit.iter().any(|o| matches!(o, Outcome::Hurt { school: 2, .. })), "{hit:?}");
        assert!(none.is_empty() && plain.is_empty());
        // Lifestealing leeches and Crusader heals.
        let named = |name: &str| *c.enchants.iter().find(|(_, e)| e.name == name).map(|(id, _)| id).unwrap();
        let (mut leech, mut crusader) = (Vec::new(), Vec::new());
        on_hit(Stack { item: sword, count: 1, enchant: named("Lifestealing") }, 2.0, 30, &mut dice(&[0]), &mut leech);
        on_hit(Stack { item: sword, count: 1, enchant: named("Crusader") }, 2.0, 30, &mut dice(&[0]), &mut crusader);
        assert!(leech.iter().any(|o| matches!(o, Outcome::Leech(_))), "{leech:?}");
        assert!(crusader.iter().any(|o| matches!(o, Outcome::Heal(_))), "{crusader:?}");
    }

    /// A damage shield and a spell that hits back when the character is struck are in the gear of the original; a roll of 0 always meets a chance.
    #[test]
    fn what_strikes_the_character_is_hit_back() {
        let mut out = Vec::new();
        struck(&[], &[(5.0, 2)], &[], 20, &mut dice(&[0]), &mut out);
        assert_eq!(out, [Outcome::Hurt { school: 2, amount: 5.0 }]);
    }
}
