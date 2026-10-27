//! What the spells of creatures do to the player, and how they look. A creature's script (`brain.rs`) casts a spell of the original (`content::classic_spell`):
//! the creature shows the stance of the cast while it takes (the precast of the spell's visual kit), the cast goes off with its own kit, and a missile, if the spell
//! has one, flies to the player and lands with the burst of its impact kit. What it does is read off the spell's effects as MaNGOS has them: school damage and
//! weapon damage (the damage of the creature's weapon, or of its ranged weapon for Shoot and Throw, plus the points), auras (a damage over time, a slow, a root, a stun,
//! a fear, a silence, a pacify, a disarm, any other kept as a mark that the creatures' scripts can look for), mana drained or burned, an interrupted spell, a knock back,
//! and the spells that an effect triggers. Effects for friends (heals, buffs) were done by the creatures themselves. Auras on the player are `Afflictions`.

use crate::audio::Sfx;
use crate::brain::Aim;
use crate::casting::{attachment, placed};
use crate::content::{self, ClassicSpell};
use crate::creature::{Cast, Creatures, Foe, SpellAnims};
use crate::effects::Effects;
use crate::renderer::{Gpu, Renderer};
use crate::visuals::{Kit, Visual};
use glam::{Mat4, Vec3};
use std::collections::HashMap;

/// How high above his feet the player's chest is, where missiles land.
const CHEST: f32 = 1.2;
/// How tall the player is for the placing of a burst on him.
const HEIGHT: f32 = 1.8;
/// Seconds between the ticks of a damage over time that does not say.
const PERIOD: f32 = 3.0;
/// The effect of the original that triggers another spell, and how deep such triggers are followed.
const TRIGGER_SPELL: u32 = 64;
const DEEPEST: u32 = 2;

/// What a creature's spell does to the player at once.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Effect {
    /// Damage: a physical one is cut by the armor and can be blocked with the shield; a magic one is neither.
    Hurt { amount: f32, physical: bool, school: u32 },
    /// A blow of a creature of this level: a physical `Hurt` that can also be a critical or a crushing blow.
    Swing { amount: f32, level: u32 },
    /// Mana taken; a burn hurts for as much as it took.
    Drain { amount: f32, burn: bool },
    /// The spell the player is casting is broken off.
    Interrupt,
    /// Thrown back from `from`: horizontal and upward speed, yards a second.
    Knock { from: Vec3, speed: f32, up: f32 },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Damage every few seconds.
    Dot,
    /// Slower by a share.
    Slow,
    Root,
    Stun,
    /// Runs from where it came from, out of control.
    Fear,
    /// Cannot cast.
    Silence,
    /// Cannot attack.
    Pacify,
    /// Cannot swing the weapons.
    Disarm,
    /// Does nothing by itself; creatures' scripts look for it.
    Mark,
}

/// A spell that has hold of the player.
#[derive(Clone, Debug)]
pub struct Affliction {
    pub spell: u32,
    /// The icon in `Interface\Icons`.
    pub icon: String,
    pub kind: Kind,
    /// Seconds left.
    pub left: f32,
    /// The damage of a tick, or the share of speed taken.
    pub amount: f32,
    period: f32,
    next: f32,
    physical: bool,
    school: u32,
    /// Where it came from (what a fear runs from).
    pub from: Vec3,
}

/// Everything that has hold of the player.
#[derive(Default)]
pub struct Afflictions {
    list: Vec<Affliction>,
}

impl Afflictions {
    pub fn all(&self) -> &[Affliction] {
        &self.list
    }

    /// Put an affliction on: one of the same spell and kind is renewed.
    pub fn add(&mut self, a: Affliction) {
        self.list.retain(|o| !(o.spell == a.spell && o.kind == a.kind));
        self.list.push(a);
    }

    /// Take off what is of these kinds (bit `n` for the dispel kind `n`).
    pub fn cleanse(&mut self, mask: u32) {
        self.list.retain(|a| !crate::content::classic_spell(a.spell).is_some_and(|s| mask >> s.dispel.min(31) & 1 == 1));
    }

    pub fn clear(&mut self) {
        self.list.clear();
    }

    /// The spells whose auras are on the player.
    pub fn spells(&self) -> Vec<u32> {
        let mut spells: Vec<u32> = self.list.iter().map(|a| a.spell).collect();
        spells.dedup();
        spells
    }

    fn has(&self, kind: Kind) -> bool {
        self.list.iter().any(|a| a.kind == kind)
    }

    pub fn stunned(&self) -> bool {
        self.has(Kind::Stun)
    }

    pub fn rooted(&self) -> bool {
        self.has(Kind::Root)
    }

    /// Where the player runs from, while a fear has him.
    pub fn feared(&self) -> Option<Vec3> {
        self.list.iter().find(|a| a.kind == Kind::Fear).map(|a| a.from)
    }

    /// Whether he cannot cast, swing the weapon in the hand, or shoot.
    pub fn cast_blocked(&self) -> bool {
        self.has(Kind::Silence) || self.stunned() || self.feared().is_some()
    }

    pub fn swing_blocked(&self) -> bool {
        self.has(Kind::Pacify) || self.has(Kind::Disarm) || self.stunned() || self.feared().is_some()
    }

    pub fn shot_blocked(&self) -> bool {
        self.has(Kind::Pacify) || self.stunned() || self.feared().is_some()
    }

    /// The share of his speed that slows take (the worst of them).
    pub fn slow(&self) -> f32 {
        self.list.iter().filter(|a| a.kind == Kind::Slow).map(|a| a.amount).fold(0.0, f32::max)
    }

    /// Let `dt` seconds go by: the damage of the ticks, and what has run out is gone.
    pub fn tick(&mut self, dt: f32) -> Vec<Effect> {
        let mut out = Vec::new();
        for a in &mut self.list {
            a.left -= dt;
            if a.kind == Kind::Dot {
                a.next -= dt;
                while a.next <= 0.0 && a.period > 0.0 {
                    a.next += a.period;
                    out.push(Effect::Hurt { amount: a.amount, physical: a.physical, school: a.school });
                }
            }
        }
        self.list.retain(|a| a.left > 0.0);
        out
    }
}

/// A missile on its way to the player.
struct Missile {
    cast: Cast,
    vis: Visual,
    from: Vec3,
    age: f32,
    duration: f32,
    effect: Option<usize>,
}

pub struct Foes {
    /// The pictures and sounds of the spells that creatures cast, by their row of `Spell.dbc`.
    visuals: HashMap<u32, Visual>,
    missiles: Vec<Missile>,
    /// The glows of the casts that are going on, by the creature that casts, with the attachment of the creature each follows (0 for none).
    glows: HashMap<usize, Vec<(usize, u32)>>,
    pub afflictions: Afflictions,
    sounds: Vec<(Sfx, Vec3)>,
    rng: u32,
}

impl Foes {
    pub fn new(visuals: HashMap<u32, Visual>) -> Foes {
        Foes { visuals, missiles: Vec::new(), glows: HashMap::new(), afflictions: Afflictions::default(), sounds: Vec::new(), rng: 0x2545_F491 }
    }

    /// The animations that the spells ask for: the stance of the cast and the cast.
    pub fn anims(&self) -> SpellAnims {
        self.visuals.iter().map(|(&spell, v)| (spell, (v.precast.anim, v.cast.anim))).collect()
    }

    /// The sounds made since the last call, with where they came from.
    pub fn take_sounds(&mut self) -> Vec<(Sfx, Vec3)> {
        std::mem::take(&mut self.sounds)
    }

    fn next(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// Put a kit of models on someone standing at `feet` facing `yaw`; what belongs in a hand goes there if `hand` finds it. Returns what it started and the
    /// attachment each follows.
    #[allow(clippy::too_many_arguments)]
    fn show(fx: &mut Effects, renderer: &mut Renderer, gpu: &Gpu, kit: &Kit, feet: Vec3, yaw: f32, height: f32, looping: bool, hand: &dyn Fn(u32) -> Option<Mat4>) -> Vec<(usize, u32)> {
        kit.effects
            .iter()
            .filter_map(|e| {
                let id = fx.spawn(renderer, gpu, &e.model, looping, placed(e.slot, feet, yaw, height, hand))?;
                Some((id, if hand(attachment(e.slot)).is_some() { attachment(e.slot) } else { 0 }))
            })
            .collect()
    }

    fn stop_glows(&mut self, who: usize, fx: &mut Effects) {
        for (id, _) in self.glows.remove(&who).unwrap_or_default() {
            fx.stop(id);
        }
    }

    /// Everything is over: no missile in the air, no cast going on, nothing has hold of the player (he died).
    pub fn clear(&mut self, fx: &mut Effects) {
        for m in self.missiles.drain(..) {
            m.effect.into_iter().for_each(|e| fx.stop(e));
        }
        let who: Vec<usize> = self.glows.keys().copied().collect();
        who.into_iter().for_each(|w| self.stop_glows(w, fx));
        self.afflictions.clear();
    }

    /// What a creature did: shown, and what it does to the player at once.
    pub fn handle(&mut self, foe: Foe, feet: Vec3, wolves: &Creatures, fx: &mut Effects, renderer: &mut Renderer, gpu: &Gpu) -> Vec<Effect> {
        match foe {
            Foe::Melee(amount, level) => vec![Effect::Swing { amount, level }],
            Foe::Begin { who, spell, at, yaw, height } => {
                self.stop_glows(who, fx);
                if let Some(vis) = self.visuals.get(&spell) {
                    let glows = Self::show(fx, renderer, gpu, &vis.precast, at, yaw, height, true, &|id| wolves.hand(who, id));
                    self.glows.insert(who, glows);
                    self.sounds.extend(vis.precast.sound.map(|s| (Sfx::Entry(s), at)));
                }
                vec![]
            }
            Foe::Stop { who } => {
                self.stop_glows(who, fx);
                vec![]
            }
            Foe::Cast { who, cast, height, allies } => {
                self.stop_glows(who, fx);
                let vis = self.visuals.get(&cast.spell).cloned().unwrap_or_default();
                Self::show(fx, renderer, gpu, &vis.cast, cast.from, cast.yaw, height, false, &|id| wolves.hand(who, id));
                self.sounds.extend(vis.cast.sound.map(|s| (Sfx::Entry(s), cast.from)));
                for at in allies {
                    Self::show(fx, renderer, gpu, &vis.impact, at, 0.0, height, false, &|_| None);
                    self.sounds.extend(vis.impact.sound.map(|s| (Sfx::Entry(s), at)));
                }
                let Some(sp) = content::classic_spell(cast.spell) else { return vec![] };
                if Self::reaches(sp, &cast, feet).is_empty() {
                    return vec![];
                }
                // A missile flies; anything else lands now.
                match vis.missile.as_deref() {
                    Some(model) if cast.aim == Aim::Victim => {
                        let from = wolves.hand(who, 1).map_or(cast.from + Vec3::Y * height * 0.7, |m| m.w_axis.truncate());
                        let to = feet + Vec3::Y * CHEST;
                        let effect = fx.spawn(renderer, gpu, model, true, Mat4::from_translation(from));
                        self.sounds.extend(vis.missile_sound.map(|s| (Sfx::Entry(s), from)));
                        let duration = (from.distance(to) / vis.speed.max(1.0)).max(0.1);
                        self.missiles.push(Missile { cast, vis, from, age: 0.0, duration, effect });
                        vec![]
                    }
                    _ => self.land(&cast, &vis, feet, 0, fx, renderer, gpu),
                }
            }
        }
    }

    /// Let `dt` seconds go by: the missiles fly and land, the auras on the player run and tick. `feet` is where the player is.
    pub fn update(&mut self, dt: f32, feet: Vec3, wolves: &Creatures, fx: &mut Effects, renderer: &mut Renderer, gpu: &Gpu) -> Vec<Effect> {
        let mut out = self.afflictions.tick(dt);
        // The glows in the hands of a creature that casts follow the hands.
        for (&who, glows) in &self.glows {
            for &(id, att) in glows.iter().filter(|g| g.1 != 0) {
                if let Some(world) = wolves.hand(who, att) {
                    fx.set_world(id, world);
                }
            }
        }
        let to = feet + Vec3::Y * CHEST;
        let mut landed = Vec::new();
        for (i, m) in self.missiles.iter_mut().enumerate() {
            m.age += dt;
            if let Some(e) = m.effect {
                let here = m.from.lerp(to, (m.age / m.duration).min(1.0));
                let d = to - m.from;
                fx.set_world(e, Mat4::from_translation(here) * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - d.x.atan2(-d.z)));
            }
            if m.age >= m.duration {
                landed.push(i);
            }
        }
        for i in landed.into_iter().rev() {
            let m = self.missiles.remove(i);
            m.effect.into_iter().for_each(|e| fx.stop(e));
            out.extend(self.land(&m.cast, &m.vis, feet, 0, fx, renderer, gpu));
        }
        out
    }

    /// Which effects of the spell reach the player at `feet`: the ones for enemies that are in their area (a cone is in front of the caster), and those of
    /// a single enemy when the spell is aimed at him.
    pub fn reaches(sp: &ClassicSpell, cast: &Cast, feet: Vec3) -> Vec<usize> {
        let to = Vec3::new(feet.x - cast.from.x, 0.0, feet.z - cast.from.z);
        let dist = to.length();
        let in_front = dist < 0.5 || (to / dist).dot(Vec3::new(cast.yaw.sin(), 0.0, -cast.yaw.cos())) > 0.5;
        sp.effects
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.kind != 0
                    && !e.friendly()
                    && if e.area() { dist <= (if e.radius > 0.0 { e.radius } else { 10.0 }) + 1.0 && (!e.cone() || in_front) } else { cast.aim == Aim::Victim }
            })
            .map(|(n, _)| n)
            .collect()
    }

    /// The spell lands on the player: the burst where it hits, and what it does.
    #[allow(clippy::too_many_arguments)]
    fn land(&mut self, cast: &Cast, vis: &Visual, feet: Vec3, depth: u32, fx: &mut Effects, renderer: &mut Renderer, gpu: &Gpu) -> Vec<Effect> {
        let Some(sp) = content::classic_spell(cast.spell) else { return vec![] };
        let hits = Self::reaches(sp, cast, feet);
        if hits.is_empty() {
            return vec![];
        }
        Self::show(fx, renderer, gpu, &vis.impact, feet, 0.0, HEIGHT, false, &|_| None);
        self.sounds.extend(vis.impact.sound.map(|s| (Sfx::Entry(s), feet)));
        let mut out = Vec::new();
        for (effect, trigger) in self.resolve(sp, cast, &hits) {
            out.extend(effect);
            // A spell that triggers another (a charge that stuns): the other goes off from the same creature.
            if let (Some(t), true) = (trigger, depth < DEEPEST) {
                let triggered = Cast { spell: t, aim: Aim::Victim, friend: None, ..cast.clone() };
                let vis = self.visuals.get(&t).cloned().unwrap_or_default();
                out.extend(self.land(&triggered, &vis, feet, depth + 1, fx, renderer, gpu));
            }
        }
        out
    }

    /// What the effects `hits` of the spell do to the player: each gives what happens at once and the spell it triggers, if it does.
    pub fn resolve(&mut self, sp: &ClassicSpell, cast: &Cast, hits: &[usize]) -> Vec<(Option<Effect>, Option<u32>)> {
        let mut out = Vec::new();
        for &n in hits {
            let e = &sp.effects[n];
            let points = sp.points(n, cast.level, self.next());
            let weapon = if sp.is_ranged_attack() { cast.shot } else { cast.melee };
            let now = match e.kind {
                2 => Some(Effect::Hurt { amount: points.max(0.0), physical: sp.school == 0, school: sp.school }),
                17 | 58 | 121 => Some(Effect::Hurt { amount: (weapon + points).max(1.0), physical: true, school: 0 }),
                31 => Some(Effect::Hurt { amount: (weapon * points / 100.0).max(1.0), physical: true, school: 0 }),
                8 => Some(Effect::Drain { amount: points.max(0.0), burn: false }),
                62 => Some(Effect::Drain { amount: points.max(0.0), burn: true }),
                68 => Some(Effect::Interrupt),
                98 => Some(Effect::Knock { from: cast.from, speed: e.misc as f32 / 10.0, up: points / 10.0 }),
                6 => {
                    self.afflict(sp, n, cast, points);
                    None
                }
                _ => None,
            };
            let trigger = (e.kind == TRIGGER_SPELL && e.trigger != 0).then_some(e.trigger);
            if now.is_some() || trigger.is_some() {
                out.push((now, trigger));
            }
        }
        out
    }

    /// The aura of effect `n` of the spell is put on the player.
    fn afflict(&mut self, sp: &ClassicSpell, n: usize, cast: &Cast, points: f32) {
        let e = &sp.effects[n];
        let seconds = sp.duration as f32 / 1000.0;
        // An aura that does not end (a stance, a stat that stays) is not one that creatures put on a player.
        if seconds <= 0.0 {
            return;
        }
        let mut kinds: Vec<Kind> = match e.aura {
            3 => vec![Kind::Dot],
            33 => vec![Kind::Slow],
            26 => vec![Kind::Root],
            12 => vec![Kind::Stun],
            7 | 5 => vec![Kind::Fear],
            27 => vec![Kind::Silence],
            25 => vec![Kind::Pacify],
            60 => vec![Kind::Pacify, Kind::Silence],
            67 => vec![Kind::Disarm],
            // Stats, armor, damage, speed of attacks: shown, and found by the scripts that look for them.
            22 | 29 | 9 | 13 | 14 | 79 | 99 | 138 | 23 | 101 | 83 | 142 | 87 | 126 | 124 => vec![Kind::Mark],
            _ => vec![],
        };
        kinds.dedup();
        for kind in kinds {
            let period = if e.period > 0 { e.period as f32 / 1000.0 } else { PERIOD };
            let amount = match kind {
                Kind::Dot => points.max(0.0),
                // A slow's points are negative percents.
                Kind::Slow => (-points / 100.0).clamp(0.0, 0.95),
                _ => 0.0,
            };
            // A mark is only worth showing when something else of the spell is not already.
            self.afflictions.add(Affliction { spell: sp.spell, icon: sp.icon.clone(), kind, left: seconds, amount, period, next: period, physical: sp.school == 0, school: sp.school, from: cast.from });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cast(spell: u32, aim: Aim) -> Cast {
        Cast { spell, aim, friend: None, from: Vec3::ZERO, yaw: 0.0, level: 10, melee: 12.0, shot: 20.0 }
    }

    fn foes() -> Foes {
        Foes::new(HashMap::new())
    }

    fn spell(id: u32) -> &'static ClassicSpell {
        content::classic_spell(id).unwrap_or_else(|| panic!("no spell {id}"))
    }

    #[test]
    fn a_bolt_hurts_the_one_it_is_aimed_at_with_magic_and_a_buff_on_a_friend_does_not_reach_him() {
        let fireball = spell(133);
        let c = cast(133, Aim::Victim);
        // Its damage and the burning that follows.
        assert_eq!(Foes::reaches(fireball, &c, Vec3::new(0.0, 0.0, -20.0)), [0, 1]);
        let out = foes().resolve(fireball, &c, &[0]);
        let Some((Some(Effect::Hurt { amount, physical: false, .. }), None)) = out.first().copied() else { panic!("{out:?}") };
        assert!((fireball.effects[0].low as f32..=fireball.effects[0].high as f32 + 20.0).contains(&amount), "{amount}");
        // Frost Armor is for its caster.
        let armor = cast(168, Aim::Itself);
        assert!(Foes::reaches(spell(168), &armor, Vec3::new(0.0, 0.0, -2.0)).is_empty());
    }

    #[test]
    fn an_area_spell_reaches_those_within_its_radius_round_the_caster_only() {
        // Frost Nova, rank 1: ten yards, a root.
        let nova = spell(122);
        let c = cast(122, Aim::Itself);
        assert!(!Foes::reaches(nova, &c, Vec3::new(5.0, 0.0, 0.0)).is_empty(), "five yards away is in it");
        assert!(Foes::reaches(nova, &c, Vec3::new(30.0, 0.0, 0.0)).is_empty(), "thirty is not");
    }

    #[test]
    fn shoot_does_the_damage_of_the_ranged_weapon_and_a_strike_the_damage_of_the_melee_one() {
        let shoot = spell(6660);
        assert!(shoot.is_ranged_attack(), "Shoot is a ranged attack");
        let c = cast(6660, Aim::Victim);
        let hits = Foes::reaches(shoot, &c, Vec3::new(0.0, 0.0, -20.0));
        let out = foes().resolve(shoot, &c, &hits);
        let Some((Some(Effect::Hurt { amount, physical: true, .. }), _)) = out.first().copied() else { panic!("{out:?}") };
        assert!(amount >= c.shot, "its weapon and the points: {amount}");
        // Backstab (rank 1) hits with the melee weapon.
        let stab = spell(53);
        let c = cast(53, Aim::Victim);
        let hits = Foes::reaches(stab, &c, Vec3::new(0.0, 0.0, -2.0));
        let out = foes().resolve(stab, &c, &hits);
        assert!(out.iter().any(|o| matches!(o.0, Some(Effect::Hurt { amount, physical: true, .. }) if amount >= c.melee)), "{out:?}");
    }

    #[test]
    fn auras_hold_the_player_slow_him_and_wear_off() {
        let mut f = foes();
        // Net (6533): a root. Hamstring (1715): a slow.
        for (id, aim) in [(6533, Aim::Victim), (1715, Aim::Victim)] {
            let c = cast(id, aim);
            let sp = spell(id);
            let hits = Foes::reaches(sp, &c, Vec3::new(0.0, 0.0, -5.0));
            f.resolve(sp, &c, &hits);
        }
        assert!(f.afflictions.rooted() && f.afflictions.slow() > 0.2 && f.afflictions.slow() < 1.0, "{:?}", f.afflictions.all());
        assert!(f.afflictions.spells().contains(&6533));
        f.afflictions.tick(100.0);
        assert!(!f.afflictions.rooted() && f.afflictions.slow() == 0.0 && f.afflictions.all().is_empty());
    }

    #[test]
    fn a_damage_over_time_ticks_until_it_is_over() {
        let mut f = foes();
        let rend = spell(772);
        let c = cast(772, Aim::Victim);
        let hits = Foes::reaches(rend, &c, Vec3::new(0.0, 0.0, -2.0));
        f.resolve(rend, &c, &hits);
        let total: f32 = (0..400).flat_map(|_| f.afflictions.tick(0.1)).map(|e| if let Effect::Hurt { amount, .. } = e { amount } else { 0.0 }).sum();
        assert!(total > 0.0 && f.afflictions.all().is_empty(), "it hurt for {total} and ended");
    }

    #[test]
    fn the_control_auras_take_away_what_they_take_away() {
        let mut a = Afflictions::default();
        let on = |kind| Affliction { spell: 1, icon: String::new(), kind, left: 5.0, amount: 0.0, period: 3.0, next: 3.0, physical: false, school: 2, from: Vec3::X };
        a.add(on(Kind::Silence));
        assert!(a.cast_blocked() && !a.swing_blocked() && !a.shot_blocked());
        a.add(on(Kind::Disarm));
        assert!(a.swing_blocked() && !a.shot_blocked());
        a.add(on(Kind::Fear));
        assert!(a.feared() == Some(Vec3::X) && a.shot_blocked());
        a.clear();
        a.add(on(Kind::Stun));
        assert!(a.stunned() && a.cast_blocked() && a.swing_blocked() && a.shot_blocked());
    }
}
