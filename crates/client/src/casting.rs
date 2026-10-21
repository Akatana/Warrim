//! Casting a spell, as the original does it: it needs its target (a creature selected within range, for a spell that hurts one), its
//! cost and to be ready; a spell with a cast time (`visuals.rs`) holds the character in the ready animation with glows on its
//! hands while the cast bar fills, and fails ("Interrupted") if the character moves or attacks; when it goes off the cost is paid, the
//! cast animation, effects and sounds play, and a missile, if it has one, flies to the target and what the spell does (rolled between
//! its numbers and grown with the rank of the class skill, which casting trains) happens where it lands, with the burst of its impact.
//! Casting breaks the stealth.

use crate::audio::Sfx;
use crate::combattext::CombatText;
use crate::content::{self, Cost, Effect};
use crate::creature::{Creatures, ARROW_RANGE};
use crate::effects::Effects;
use crate::panels::Panels;
use crate::player::{Player, Shot};
use crate::renderer::{Gpu, Renderer};
use crate::visuals::{Kit, Slot, Visual};
use glam::{Mat4, Vec3};

/// A number from `min` to `max` (whole numbers) by the dice `rng`, times `power`, rounded.
pub fn roll(rng: &mut u32, min: f32, max: f32, power: f32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    ((min + (*rng % ((max - min).max(0.0) as u32 + 1)) as f32) * power).round()
}

/// What a spell does: `roll` and what the gear adds, half as much again with a chance of `crit` percent (a critical spell).
fn spell_roll(rng: &mut u32, min: f32, max: f32, power: f32, bonus: f32, crit: f32) -> f32 {
    let amount = (roll(rng, min, max, power) + bonus).round().max(0.0);
    if dice(rng) < crit { (amount * 1.5).round() } else { amount }
}

/// A number from 0 up to 100 by the dice `rng` (a chance in percent is met when this is below it).
fn dice(rng: &mut u32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    (*rng % 10_000) as f32 / 100.0
}

/// What casting works on, borrowed from the game.
pub struct Ctx<'a> {
    pub player: &'a mut Player,
    pub panels: &'a mut Panels,
    pub wolves: Option<&'a mut Creatures>,
    pub text: Option<&'a mut CombatText>,
    pub effects: &'a mut Effects,
    pub renderer: &'a mut Renderer,
    pub gpu: &'a Gpu,
}

/// A spell being cast.
struct Casting {
    spell: usize,
    elapsed: f32,
    total: f32,
    /// The glows held in the hands and the attachment of the character each follows (0 for none).
    glows: Vec<(usize, u32)>,
}

/// A missile on its way, and what it does where it lands.
struct Flight {
    /// What it shows where it lands.
    vis: Visual,
    /// An arrow tilts to follow its way up and down.
    pitched: bool,
    /// The creature it is meant for (`usize::MAX` for an arrow that flies into the distance and hits nothing).
    target: usize,
    last_seen: Vec3,
    from: Vec3,
    age: f32,
    duration: f32,
    effect: Option<usize>,
    damage: f32,
}

pub struct Spellcaster {
    visuals: Vec<Visual>,
    casting: Option<Casting>,
    flights: Vec<Flight>,
    rng: u32,
    /// A spell has hold of the character (a silence, a stun, a fear): nothing can be cast. And a creature's blow broke the cast off.
    blocked: bool,
    interrupted: bool,
}

/// How high above the feet a creature's chest is.
const CHEST: f32 = 0.9;

/// Where a model put on `slot` of someone standing at `feet` facing `yaw` goes (render space).
pub(crate) fn placed(slot: Slot, feet: Vec3, yaw: f32, height: f32, hand: impl Fn(u32) -> Option<Mat4>) -> Mat4 {
    let facing = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - yaw);
    let at = |up: f32| Mat4::from_translation(feet + Vec3::Y * up) * facing;
    match slot {
        Slot::Base => at(0.0),
        Slot::Head => at(height + 0.1),
        Slot::LeftHand => hand(2).unwrap_or_else(|| at(height * 0.7)),
        Slot::RightHand => hand(1).unwrap_or_else(|| at(height * 0.7)),
        Slot::Chest | Slot::Other => at(height * 0.7),
    }
}

/// The attachment of the character a hand slot follows.
pub(crate) fn attachment(slot: Slot) -> u32 {
    match slot {
        Slot::LeftHand => 2,
        Slot::RightHand => 1,
        _ => 0,
    }
}

impl Spellcaster {
    pub fn new(visuals: Vec<Visual>) -> Spellcaster {
        Spellcaster { visuals, casting: None, flights: Vec::new(), rng: 0xBEEF_1234, blocked: false, interrupted: false }
    }

    pub fn casting(&self) -> bool {
        self.casting.is_some()
    }

    /// Say whether casting is taken away from the character (every frame).
    pub fn set_blocked(&mut self, blocked: bool) {
        self.blocked = blocked;
    }

    /// A creature interrupts the spell being cast, if there is one.
    pub fn interrupt(&mut self) {
        self.interrupted = self.casting.is_some();
    }

    /// Why `spell` cannot be cast on what is selected now, or whom it is cast on (its target and where it stands).
    fn check(&self, ctx: &Ctx, spell: usize, feet: Vec3) -> Result<Option<(usize, Vec3)>, &'static str> {
        let def = content::spell(spell);
        let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z).length();
        let mut victim = None;
        if let Effect::Damage { range, .. } = def.effect {
            let target = ctx.wolves.as_ref().and_then(|w| w.target()).ok_or("You have no target.")?;
            let at = ctx.wolves.as_ref().and_then(|w| w.enemy_at(target)).ok_or("Invalid target.")?;
            if flat(at - feet) > range {
                return Err("Out of range.");
            }
            victim = Some((target, at));
        }
        let cost = ctx.panels.spell_cost(def.cost);
        let enough = match cost {
            Cost::Mana(n) => ctx.player.vitals.mana.current >= n,
            Cost::Stamina(n) => ctx.player.vitals.stamina.current >= n,
        };
        if !enough {
            return Err(match cost {
                Cost::Mana(_) => "Not enough mana.",
                Cost::Stamina(_) => "Not enough stamina.",
            });
        }
        Ok(victim)
    }

    fn sound(ctx: &mut Ctx, sound: Option<u32>) {
        if let Some(entry) = sound {
            ctx.panels.emit(Sfx::Entry(entry));
        }
    }

    /// Put a kit's models on someone (the caster, or whatever stands at `feet` facing `yaw`): returns what it started, with the
    /// attachment each follows if it is a glow on a hand of the character.
    fn show(ctx: &mut Ctx, kit: &Kit, on_caster: bool, feet: Vec3, yaw: f32, height: f32, looping: bool) -> Vec<(usize, u32)> {
        let mut started = Vec::new();
        for e in &kit.effects {
            let world = placed(e.slot, feet, yaw, height, |id| if on_caster { ctx.player.attachment_world(id) } else { None });
            if let Some(id) = ctx.effects.spawn(ctx.renderer, ctx.gpu, &e.model, looping, world) {
                started.push((id, if on_caster { attachment(e.slot) } else { 0 }));
            }
        }
        started
    }

    /// The key of a spell went down. A spell with a cast time starts its cast; an instant one goes off.
    pub fn begin(&mut self, ctx: &mut Ctx, spell: usize, feet: Vec3) {
        let def = content::spell(spell);
        if ctx.player.vitals.health.current <= 0.0 {
            return;
        }
        if self.blocked {
            return ctx.panels.error("You can't do that right now.");
        }
        if self.casting.is_some() {
            return ctx.panels.error("Another action is in progress.");
        }
        if ctx.panels.cooldowns.get(spell).is_some_and(|&c| c > 0.0) {
            return ctx.panels.error("Spell is not ready yet.");
        }
        if let Err(why) = self.check(ctx, spell, feet) {
            ctx.panels.sound("SpellFizzleHoly");
            return ctx.panels.error(why);
        }
        // A spell is cast on foot: the caster gets down from the steed.
        ctx.player.dismount();
        let vis = self.visuals.get(spell).cloned().unwrap_or_default();
        if vis.cast_ms <= 0.0 {
            return self.go(ctx, spell, feet);
        }
        // The cast: the ready animation and the glows in the hands for as long as it takes.
        let yaw = ctx.player.facing();
        let glows = Self::show(ctx, &vis.precast, true, feet, yaw, 1.8, true);
        if let Some(entry) = vis.precast.sound {
            ctx.panels.emit(Sfx::Hold(entry));
        }
        ctx.player.set_precast(vis.precast.anim);
        ctx.panels.cast_start(&def.name, vis.cast_ms / 1000.0);
        self.casting = Some(Casting { spell, elapsed: 0.0, total: vis.cast_ms / 1000.0, glows });
    }

    fn stop_glows(ctx: &mut Ctx, glows: &[(usize, u32)]) {
        glows.iter().for_each(|&(id, _)| ctx.effects.stop(id));
    }

    /// The cast fails: the glows die away, and the bar says why.
    fn fail(&mut self, ctx: &mut Ctx, why: &str) {
        if let Some(c) = self.casting.take() {
            Self::stop_glows(ctx, &c.glows);
            ctx.player.set_precast(None);
            ctx.panels.emit(Sfx::Release);
            ctx.panels.cast_failed(why);
            ctx.panels.sound("SpellFizzleHoly");
        }
    }

    /// Advance `dt` seconds: the cast fills, and fails if the character moves (`moving`), attacks or dies; the glows follow the hands; the
    /// missiles fly and land.
    pub fn update(&mut self, ctx: &mut Ctx, dt: f32, feet: Vec3, moving: bool) {
        if std::mem::take(&mut self.interrupted) || (self.blocked && self.casting.is_some()) {
            self.fail(ctx, "Interrupted");
        }
        if let Some(c) = &mut self.casting {
            c.elapsed += dt;
            for &(id, attachment) in &c.glows {
                if attachment != 0 {
                    if let Some(world) = ctx.player.attachment_world(attachment) {
                        ctx.effects.set_world(id, world);
                    }
                }
            }
            let (done, spell) = (c.elapsed >= c.total, c.spell);
            if moving || ctx.player.is_striking() || ctx.player.vitals.health.current <= 0.0 {
                self.fail(ctx, "Interrupted");
            } else if done {
                let glows = self.casting.take().map(|c| c.glows).unwrap_or_default();
                Self::stop_glows(ctx, &glows);
                ctx.player.set_precast(None);
                ctx.panels.emit(Sfx::Release);
                match self.check(ctx, spell, feet) {
                    Ok(_) => {
                        ctx.panels.cast_stop();
                        self.go(ctx, spell, feet);
                    }
                    Err(why) => {
                        ctx.panels.cast_failed(why);
                        ctx.panels.sound("SpellFizzleHoly");
                    }
                }
            }
        }
        let mut landed = Vec::new();
        for (i, f) in self.flights.iter_mut().enumerate() {
            f.age += dt;
            if let Some(at) = ctx.wolves.as_ref().and_then(|w| w.enemy_at(f.target)) {
                f.last_seen = at;
            }
            let to = f.last_seen + Vec3::Y * CHEST;
            let t = (f.age / f.duration).min(1.0);
            let here = f.from.lerp(to, t);
            if let Some(effect) = f.effect {
                let d = to - f.from;
                let heading = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - d.x.atan2(-d.z));
                let pitch = if f.pitched { Mat4::from_rotation_z(d.y.atan2(d.x.hypot(d.z))) } else { Mat4::IDENTITY };
                ctx.effects.set_world(effect, Mat4::from_translation(here) * heading * pitch);
            }
            if f.age >= f.duration {
                landed.push(i);
            }
        }
        let landed_any = !landed.is_empty();
        for i in landed.into_iter().rev() {
            let f = self.flights.remove(i);
            if let Some(effect) = f.effect {
                ctx.effects.stop(effect);
            }
            self.land(ctx, f);
        }
        // The sound of the flight ends where the missile does (a later missile in the air keeps its own).
        if landed_any && self.flights.is_empty() {
            ctx.panels.emit(Sfx::Release);
        }
    }

    /// A shot of a ranged weapon is let go (it was drawn for `shot.power` of the full draw): a piece of the ammo is used up, and the arrow (or bullet) flies from
    /// the hand that holds the weapon to what is in line ahead, where it does its share of the damage; it flies on into the distance if there is nothing.
    pub fn shoot(&mut self, ctx: &mut Ctx, shot: Shot, feet: Vec3, yaw: f32) {
        let skill = ctx.panels.ranged_skill();
        let damage = (ctx.player.roll_shot() * ctx.panels.skills.damage_factor(skill) * shot.power).round().max(1.0);
        let ammo = &mut ctx.panels.inv.equipment[0];
        *ammo = ammo.and_then(|s| (s.count > 1).then_some(crate::items::Stack { count: s.count - 1, ..s }));
        ctx.player.use_ammo();
        let from = ctx.player.attachment_world(2).map_or(feet + Vec3::Y * 1.3, |m| m.w_axis.truncate());
        let aimed = ctx.wolves.as_ref().and_then(|w| w.aim(feet, yaw, ARROW_RANGE));
        // An arrow may miss, as a blow of a weapon does.
        let (mob, kind) = aimed.and_then(|(t, _)| ctx.wolves.as_ref()?.enemy_info(t)).unwrap_or((ctx.panels.progress.level, 0));
        let speed = ctx.panels.stats.ranged.map_or(2.0, |r| r.2);
        let damage = damage + ctx.panels.stats.ranged_ap_versus[kind as usize % 11] as f32 / crate::items::ATTACK_POWER_PER_DPS * speed;
        let missed = dice(&mut self.rng) < crate::levels::melee_miss(ctx.panels.progress.level, mob, ctx.panels.stats.hit);
        let damage = if missed { 0.0 } else { damage };
        let (target, last_seen) = aimed.map_or((usize::MAX, Vec3::new(from.x + yaw.sin() * ARROW_RANGE, feet.y, from.z - yaw.cos() * ARROW_RANGE)), |(t, at)| (t, at));
        if aimed.is_some() {
            ctx.panels.train(skill, shot.power);
        }
        let to = last_seen + Vec3::Y * CHEST;
        let effect = shot.projectile.as_ref().and_then(|(model, skin)| ctx.effects.spawn_skinned(ctx.renderer, ctx.gpu, model, skin, true, Mat4::from_translation(from)));
        if let Some(entry) = shot.visual.missile_sound {
            ctx.panels.emit(Sfx::Hold(entry));
        }
        let duration = (from.distance(to) / shot.visual.speed.max(1.0)).max(0.05);
        self.flights.push(Flight { vis: shot.visual, pitched: true, target, last_seen, from, age: 0.0, duration, effect, damage });
    }

    /// A missile arrives: the burst where it hits, its sound and what it does.
    fn land(&mut self, ctx: &mut Ctx, f: Flight) {
        if f.target == usize::MAX {
            return;
        }
        let vis = f.vis;
        let alive = ctx.wolves.as_ref().and_then(|w| w.enemy_at(f.target));
        let at = alive.unwrap_or(f.last_seen);
        Self::show(ctx, &vis.impact, false, at, 0.0, CHEST / 0.7, false);
        Self::sound(ctx, vis.impact.sound);
        if alive.is_some() {
            if f.damage <= 0.0 {
                if let Some(text) = ctx.text.as_mut() {
                    text.message("Miss");
                }
                return;
            }
            let hit = ctx.wolves.as_mut().and_then(|w| w.spell_hit(f.target, f.damage));
            if let (Some(hit), Some(text)) = (hit, ctx.text.as_mut()) {
                text.damage_dealt(hit, f.damage as u32);
            }
            // The spells of the ranged weapon that go off on a shot that lands.
            if let (true, Some(weapon), Some(wolves)) = (f.pitched, ctx.panels.inv.equipment[crate::items::RANGED], ctx.wolves.as_mut()) {
                let (level, speed) = (ctx.panels.progress.level, ctx.panels.stats.ranged.map_or(2.0, |r| r.2));
                let mut out = Vec::new();
                crate::procs::on_hit(weapon.item, speed, level, &mut || ctx.panels.consumed.dice(), &mut out);
                crate::procs::apply(out, Some(f.target), ctx.player, ctx.panels, wolves, ctx.text.as_deref_mut());
            }
        }
    }

    /// The spell goes off: the cost is paid, the cast animation, effects and sounds play and it does what it does (a missile does it
    /// where it lands).
    fn go(&mut self, ctx: &mut Ctx, spell: usize, feet: Vec3) {
        let def = content::spell(spell);
        let Ok(victim) = self.check(ctx, spell, feet) else { return };
        if !ctx.player.spend(ctx.panels.spell_cost(def.cost)) {
            return;
        }
        let vis = self.visuals.get(spell).cloned().unwrap_or_default();
        let yaw = ctx.player.facing();
        let power = ctx.panels.spell_power(def.class);
        let crit = ctx.panels.spell_crit(spell);
        let versus = ctx.panels.stats.spell_versus;
        let bonus = ctx.panels.spell_bonus(spell);
        let (level, spell_hit) = (ctx.panels.progress.level, ctx.panels.stats.spell_hit);
        // What it does to the character itself or to all round, now.
        match def.effect {
            Effect::Area { min, max, radius } => {
                let mut rng = self.rng;
                if let Some(wolves) = ctx.wolves.as_mut() {
                    let hits = wolves.spell_area(feet, radius, |mob, kind| if dice(&mut rng) < crate::levels::spell_miss(level, mob, spell_hit) { 0.0 } else { spell_roll(&mut rng, min, max, power, bonus + versus[kind as usize % 11] as f32, crit) });
                    for (hit, damage) in hits {
                        if let Some(text) = ctx.text.as_mut() {
                            text.damage_dealt(hit, damage as u32);
                        }
                        Self::show(ctx, &vis.impact, false, hit - Vec3::Y * CHEST, 0.0, CHEST / 0.7, false);
                    }
                }
                self.rng = rng;
            }
            Effect::Heal { min, max } => {
                let healed = ctx.player.heal(spell_roll(&mut self.rng, min, max, power, bonus, crit));
                if let Some(text) = ctx.text.as_mut() {
                    text.healed(healed as u32);
                }
                Self::show(ctx, &vis.impact, true, feet, yaw, 1.8, false);
                Self::sound(ctx, vis.impact.sound);
            }
            Effect::Damage { .. } => {}
        }
        // The cast itself: the animation, the hands, the sound.
        let directed = matches!(def.effect, Effect::Damage { .. });
        ctx.player.cast_anim(vis.cast.anim.unwrap_or(if directed { 53 } else { 54 }) as u16);
        Self::show(ctx, &vis.cast, true, feet, yaw, 1.8, false);
        Self::sound(ctx, vis.cast.sound);
        if matches!(def.effect, Effect::Area { .. }) {
            Self::sound(ctx, vis.impact.sound);
        }
        if let (Effect::Damage { min, max, .. }, Some((target, at))) = (def.effect, victim) {
            let (mob, kind) = ctx.wolves.as_ref().and_then(|w| w.enemy_info(target)).unwrap_or((level, 0));
            let damage = if dice(&mut self.rng) < crate::levels::spell_miss(level, mob, spell_hit) { 0.0 } else { spell_roll(&mut self.rng, min, max, power, bonus + versus[kind as usize % 11] as f32, crit) };
            let from = ctx.player.attachment_world(1).map_or(feet + Vec3::Y * 1.3, |m| m.w_axis.truncate());
            match vis.missile.as_deref() {
                Some(model) => {
                    let to = at + Vec3::Y * CHEST;
                    let effect = ctx.effects.spawn(ctx.renderer, ctx.gpu, model, true, Mat4::from_translation(from));
                    if let Some(entry) = vis.missile_sound {
                        ctx.panels.emit(Sfx::Hold(entry));
                    }
                    let duration = (from.distance(to) / vis.speed.max(1.0)).max(0.1);
                    self.flights.push(Flight { vis: vis.clone(), pitched: false, target, last_seen: at, from, age: 0.0, duration, effect, damage });
                }
                None => self.land(ctx, Flight { vis: vis.clone(), pitched: false, target, last_seen: at, from, age: 0.0, duration: 0.0, effect: None, damage }),
            }
        }
        ctx.panels.cooldowns[spell] = def.cooldown;
        ctx.panels.train(def.class, 1.0);
        ctx.player.set_sneaking(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_roll_is_a_whole_number_between_the_numbers_times_the_skill() {
        let mut rng = 0xBEEF_1234;
        let rolls: Vec<f32> = (0..500).map(|_| roll(&mut rng, 10.0, 15.0, 1.0)).collect();
        assert!(rolls.iter().all(|r| (10.0..=15.0).contains(r) && r.fract() == 0.0));
        assert!((10..=15).all(|n| rolls.contains(&(n as f32))), "every number comes up");
        assert!((0..100).all(|_| (20.0..=30.0).contains(&roll(&mut rng, 10.0, 15.0, 2.0))));
        assert_eq!(roll(&mut rng, 7.0, 7.0, 1.0), 7.0);
    }

    #[test]
    fn a_model_is_put_where_its_slot_says() {
        let feet = Vec3::new(10.0, 2.0, -5.0);
        let at = |slot| placed(slot, feet, 0.0, 1.8, |_| None).w_axis.truncate();
        assert_eq!(at(Slot::Base), feet);
        assert!(at(Slot::Head).y > at(Slot::Chest).y && at(Slot::Chest).y > feet.y);
        // A hand is where the character's hand is, if it has one.
        let hand = Mat4::from_translation(Vec3::new(1.0, 1.0, 1.0));
        assert_eq!(placed(Slot::RightHand, feet, 0.0, 1.8, |id| (id == 1).then_some(hand)).w_axis.truncate(), Vec3::ONE);
    }

    #[test]
    fn every_spell_has_its_tome_somewhere_to_be_found_and_a_basic_one_asks_for_nothing() {
        let c = content::get();
        for (i, spell) in c.spells.iter().enumerate() {
            let tome = c.items.iter().position(|it| it.tome == Some(i));
            let tome = tome.unwrap_or_else(|| panic!("{} has no tome", spell.name));
            // Found: dropped by a creature that stands in the world or in a chest, or known from the start.
            let dropped = c.loot.values().any(|t| t.items.iter().any(|d| d.0 == tome));
            assert!(dropped || c.start.spells.contains(&i), "{} cannot be found", spell.name);
            // A basic spell's tome asks for nothing; a greater one asks for a rank that the skill can reach.
            assert!(spell.level == 1 || (2..=crate::skills::MAX).contains(&spell.level));
        }
        // Each class has a basic and a greater spell.
        for class in crate::skills::ALL.iter().filter(|s| s.is_class()) {
            let of = |basic: bool| c.spells.iter().filter(|s| s.class == *class && (s.level == 1) == basic).count();
            assert!(of(true) >= 1 && of(false) >= 1, "{}", class.name());
        }
    }
}
