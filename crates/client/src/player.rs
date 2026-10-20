//! The player: the animated character, its equipment and the first / third person view. Both views show
//! the same body with the same animations; in first person the camera rides on the head, just in front of the
//! eyes, so looking down shows the chest, arms and legs, and the head is behind the camera.

use crate::audio::Sfx;
use crate::camera::Camera;
use crate::character::{to_render, Character, MODEL_TO_RENDER};
use crate::steed::Steed;
use crate::worn::Worn;
use crate::items::{self, Stack, Stats, SLOTS};
use crate::models::ModelCache;
use crate::renderer::{Gpu, Renderer};
use anyhow::Result;
use glam::{Mat4, Vec3};
use mpq::Mount;
use std::sync::Arc;

/// Third-person camera distance behind the head, yards.
const THIRD_PERSON_DISTANCE: f32 = 4.5;
/// How fast the camera moves back out after an obstacle lets go of it, yards per second.
const CAMERA_RECOVERY: f32 = 8.0;
/// Near plane in first person, yards: closer than the shoulders and chest below the eye.
const FIRST_PERSON_NEAR: f32 = 0.05;
/// Stamina: sprinting drains this much per second while moving, it comes back at `STAMINA_REGEN` per second
/// once `REGEN_DELAY` seconds have passed without spending any, a worn-out character may sprint again at
/// `SPRINT_RESUME`, a blocked hit costs `BLOCK_COST` and a jump `JUMP_COST`.
const SPRINT_DRAIN: f32 = 20.0;
const STAMINA_REGEN: f32 = 15.0;
const REGEN_DELAY: f32 = 1.0;
const SPRINT_RESUME: f32 = 25.0;
/// Swimming drains this much stamina a second on the move, and treading water a quarter of it (before the skill in swimming).
const SWIM_DRAIN: f32 = 6.0;
const SWIM_REST_DRAIN: f32 = 1.5;
const BLOCK_COST: f32 = 20.0;
/// Mana comes back at this many points a second.
const MANA_REGEN: f32 = 4.0;
const JUMP_COST: f32 = 10.0;
/// Seconds a light swing takes at the most (a fast weapon swings faster), and a heavy one.
const ATTACK_SECONDS: f32 = 0.65;
const HEAVY_SECONDS: f32 = 1.1;
/// How long the button is held for a swing to be a heavy one.
const HEAVY_HOLD: f32 = 0.4;
/// Seconds the hand takes to put the weapons away or draw them.
const SHEATH_SECONDS: f32 = 1.0;
/// Stamina a swing costs (of a weapon of speed 2; a slower weapon is heavier to swing and a faster one lighter, by `swing_cost`).
const LIGHT_COST: f32 = 8.0;
const HEAVY_COST: f32 = 25.0;
/// A heavy swing does this much more damage.
pub const HEAVY_DAMAGE: f32 = 2.0;
/// The least share of its damage a swing does, however soon after the last one it comes (see `Player::begin_swing`).
const MIN_POWER: f32 = 0.15;
/// How fast the body turns into the archer's stance (per second, as the rate of an exponential approach).
const AIM_TURN_RATE: f32 = 20.0;

/// The sound of the weapons going into the sheath or coming out (the original's `SHEATHING...SHEATHE` entries), when there is a weapon to make it.
fn sheath_sound(kind: &'static str, on: bool) -> Option<Sfx> {
    Some(Sfx::Named(match (kind, on) {
        ("METALWEAPON", true) => "SHEATHINGMETALWEAPONSHEATHE",
        ("METALWEAPON", false) => "SHEATHINGMETALWEAPONUNSHEATHE",
        ("WOODWEAPON", true) => "SHEATHINGWOODWEAPONSHEATHE",
        ("WOODWEAPON", false) => "SHEATHINGWOODWEAPONUNSHEATHE",
        ("SHIELD", true) => "SHEATHINGSHIELDSHEATHE",
        ("SHIELD", false) => "SHEATHINGSHIELDUNSHEATHE",
        _ => return None,
    }))
}

/// How a sword swing is made: a tap is a light one, the button held (or the heavy attack's own button) a heavy one: slower, harder, and it costs more
/// stamina.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Swing {
    Light,
    Heavy,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    First,
    Third,
}

pub use crate::animator::Motion;

/// A resource with a current and a maximum value.
#[derive(Clone, Copy, Debug)]
pub struct Resource {
    pub current: f32,
    pub max: f32,
}

impl Resource {
    pub fn full(max: f32) -> Resource {
        Resource { current: max, max }
    }

    /// How full it is, 0..1.
    pub fn fraction(&self) -> f32 {
        if self.max > 0.0 { (self.current / self.max).clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// The three bars above the action bar.
#[derive(Clone, Copy, Debug)]
pub struct Vitals {
    pub health: Resource,
    pub mana: Resource,
    pub stamina: Resource,
}

impl Vitals {
    pub fn full() -> Vitals {
        Vitals { health: Resource::full(100.0), mana: Resource::full(100.0), stamina: Resource::full(100.0) }
    }
}

pub struct Player {
    character: Character,
    /// The body, dressed, and what hangs on it: what is held, the helmet, the shoulders.
    worn: Worn,
    cache: ModelCache,
    pub view: View,
    /// Extra body rotation in radians (previews only; normally the body faces where the camera looks).
    pub body_turn: f32,
    /// The swing that is on: seconds into it, how long it takes, its kind, and the share of the damage it does.
    attack: Option<(f32, f32, Swing, f32)>,
    /// The speed of the weapon in the main hand (the seconds that a swing is worth), and the seconds since the last swing began.
    weapon_speed: f32,
    since_swing: f32,
    /// How long the attack button has been held since the swing before was over (`None` when it is not down).
    charge: Option<f32>,
    /// A swing was refused for want of stamina (see `take_refusal`).
    refused: Option<&'static str>,
    ranged: Ranged,
    /// The shield is up (the block button is down).
    blocking: bool,
    pub vitals: Vitals,
    /// What the skill in blocking makes of the stamina a block costs (1 for none).
    block_cost: f32,
    /// In the water, and what the skill in swimming makes of the stamina swimming costs (1 for none).
    swimming: bool,
    swim_cost: f32,
    /// How big it is (the hull it walks with, and its scale).
    size: crate::character::Size,
    mount: Arc<Mount>,
    /// Where the feet are and the heading the body faces (0 = north), as of the last update.
    feet: Vec3,
    facing: f32,
    /// What the worn items make of the character: the share of damage armor stops, the sword's damage range, and
    /// whether the sword and the shield are in the hands.
    mitigation: f32,
    damage: (f32, f32),
    rng: u32,
    /// Sprint key held with stamina to spend (set by the game each frame).
    sprinting: bool,
    /// Crouched and quiet: slower, softer footsteps, harder to notice, and the first blow on one who has not noticed is heavier.
    sneaking: bool,
    /// Out of stamina: no sprinting until it has recovered to `SPRINT_RESUME`.
    exhausted: bool,
    /// Seconds since stamina was last spent.
    rested: f32,
    /// Current distance of the third-person camera behind the head, yards.
    camera_distance: f32,
    /// How many times the usual distance the camera stands behind in third person (the interface options).
    pub follow: f32,
    /// The eye's yaw and height of the previous frame, for the turning and vertical speeds.
    last_eye: Option<(f32, f32)>,
    /// Sounds made since the last `take_events`.
    events: Vec<Sfx>,
    steps: Stepper,
    swings: u32,
    swing_pending: bool,
    /// The swing is at its cut and has not struck yet / has just struck (see `take_swing_hit`).
    hit_pending: bool,
    swing_hit: Option<(Swing, f32)>,
    /// The steed the character rides, if any; `dismount_wanted`: it is sent away at the start of the next `update`.
    steed: Option<Steed>,
    dismount_wanted: bool,
    /// The weapons are put away (R); fighting draws them again. `stowed` is how they show: they go to the sheath halfway through the animation (`sheath_wait` seconds to go).
    sheathed: bool,
    stowed: bool,
    sheath_wait: f32,
    /// Seconds of the hand's trip to or from the sheath still to go (no blow can be struck before it is over), and what the weapons are (for the
    /// sound they make): "METALWEAPON", "WOODWEAPON" or "SHIELD".
    busy: f32,
    sheath_kind: &'static str,
    /// Steeds that were replaced or sent away, to be handed back to the renderer at the next `update`.
    retired: Vec<Steed>,
    /// Who the character is: its race, sex and looks.
    look: crate::character::Appearance,
    /// Held back by a spell (a stun, a pacify, a disarm, a fear): no swing, no shot.
    no_swing: bool,
    no_shot: bool,
}

/// The bow, crossbow or gun in the ranged slot: its damage, the seconds it takes to draw (its speed), and the item subclass of the ammo it takes.
#[derive(Clone, Copy)]
struct Launcher {
    damage: (f32, f32),
    draw: f32,
    ammo_kind: u8,
}

/// A shot let go: the share of its damage it does, how it looks (the visual of the weapon: animation, sounds, impact) and the model of the
/// ammo that flies (the model and the texture of the ammo item's second model).
pub struct Shot {
    pub power: f32,
    pub visual: crate::visuals::Visual,
    pub projectile: Option<(String, String)>,
}

/// The ranged weapon, the ammo in the ammo slot, and the shot being drawn.
#[derive(Default)]
struct Ranged {
    launcher: Option<Launcher>,
    /// The pictures and sounds of its shot (`ItemDisplayInfo.dbc` names the row of `SpellVisual.dbc`), and the flying model of the ammo.
    visual: crate::visuals::Visual,
    projectile: Option<(String, String)>,
    ammo: u32,
    /// Seconds the string has been drawn (`None` when it is not).
    drawn: Option<f32>,
    /// The share of its damage the shot let go of does, until `take_shot`.
    shot: Option<Shot>,
    /// How far the body is turned from where the camera looks so that the head, which the archer's stance turns, looks along it.
    turn: f32,
}

/// Footstep timing: one step per stride (the walk and run animations step every 1.25 and 2.3 yards),
/// and a thud on landing.
#[derive(Default)]
struct Stepper {
    /// Distance walked towards the next footstep, yards.
    stride: f32,
    was_airborne: bool,
    /// The footstep set of the steed that is ridden (hoofbeats, four to a gallop's stride) instead of the boots'.
    steed: Option<u32>,
}

impl Stepper {
    fn update(&mut self, dt: f32, motion: &Motion, out: &mut Vec<Sfx>) {
        if motion.airborne {
            self.was_airborne = true;
            return;
        }
        let foot = self.steed.map_or(Sfx::Footstep, Sfx::SteedFoot);
        if std::mem::take(&mut self.was_airborne) {
            out.push(if self.steed.is_some() { foot } else { Sfx::Land });
        }
        if motion.speed > 0.3 {
            let step = match (self.steed, motion.speed > 5.0) {
                (Some(_), gallop) => if gallop { 1.8 } else { 0.6 },
                (None, run) => if run { 2.3 } else { 1.25 },
            };
            self.stride += motion.speed * dt;
            if self.stride >= step {
                self.stride -= step;
                out.push(foot);
            }
        } else {
            // Standing: the first step after setting off comes after half a stride.
            self.stride = 0.6;
        }
    }
}

impl Player {
    pub fn load(mount: &Arc<Mount>, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu, look: &crate::character::Appearance) -> Result<Player> {
        let character = Character::load(mount, cache, "player", look)?;
        let worn = Worn::new(&character, renderer, gpu);
        Ok(Player { character, worn, mount: mount.clone(), cache: ModelCache::default(), view: View::First, body_turn: 0.0, attack: None, weapon_speed: 2.0, since_swing: 99.0, charge: None, refused: None, ranged: Ranged::default(), camera_distance: THIRD_PERSON_DISTANCE, follow: 1.0, last_eye: None, feet: Vec3::ZERO, facing: 0.0, mitigation: 0.0, damage: (12.0, 12.0), rng: 0x1234_5678, blocking: false, block_cost: 1.0, swimming: false, swim_cost: 1.0, size: crate::character::Size::of(mount, look).unwrap_or_else(|e| { log::warn!("no size for the player: {e:#}"); Default::default() }), sprinting: false, sneaking: false, exhausted: false, rested: 0.0, vitals: Vitals::full(), events: Vec::new(), steps: Stepper::default(), swings: 0, swing_pending: false, hit_pending: false, swing_hit: None, steed: None, dismount_wanted: false, sheathed: false, stowed: false, sheath_wait: 0.0, busy: 0.0, sheath_kind: "", retired: Vec::new(), look: *look, no_swing: false, no_shot: false })
    }

    pub fn toggle_view(&mut self) {
        self.view = if self.view == View::First { View::Third } else { View::First };
    }

    /// The stamina a swing costs with the weapon in the main hand: more for a slow weapon, less for a fast one.
    pub fn swing_cost(&self, swing: Swing) -> f32 {
        let base = if swing == Swing::Heavy { HEAVY_COST } else { LIGHT_COST };
        base * (self.weapon_speed / 2.0).clamp(0.6, 1.6)
    }

    /// The share of its damage a swing would do now: the time since the last one began as a share of the weapon's speed, from `MIN_POWER`
    /// to everything. Swinging again sooner is allowed, but does less.
    pub fn swing_power(&self) -> f32 {
        (self.since_swing / self.weapon_speed.max(0.1)).clamp(MIN_POWER, 1.0)
    }

    /// Begin a swing if the last one is over and there is stamina for it: it is spent (and what is left stays spent for a moment). The swing does the
    /// `swing_power` of the moment. False when not.
    fn begin_swing(&mut self, swing: Swing) -> bool {
        if !self.ready_to_fight() {
            return false;
        }
        if self.no_swing {
            self.refused = Some("You can't do that right now.");
            return false;
        }
        if self.attack.is_some() || self.ranged.drawn.is_some() {
            return false;
        }
        let cost = self.swing_cost(swing);
        if self.vitals.stamina.current < cost {
            self.refused = Some("Not enough stamina.");
            return false;
        }
        self.vitals.stamina.current -= cost;
        self.rested = 0.0;
        let length = if swing == Swing::Light { ATTACK_SECONDS.min(self.weapon_speed) } else { HEAVY_SECONDS };
        self.attack = Some((0.0, length, swing, self.swing_power()));
        self.since_swing = 0.0;
        if swing == Swing::Light {
            self.character.attack(length);
        } else {
            self.character.play_once(crate::animator::ATTACK_2H, length);
        }
        self.swing_pending = true;
        self.hit_pending = true;
        // A grunt on every other swing, and on every heavy one.
        self.swings += 1;
        if self.swings % 2 == 1 || swing == Swing::Heavy {
            self.events.push(Sfx::Exertion);
        }
        true
    }

    /// The attack button (or key) went down: a swing is made when it comes up, or a heavy one when it has been held long enough.
    pub fn attack_press(&mut self) {
        if self.attack.is_none() && self.ranged.drawn.is_none() {
            self.charge = Some(0.0);
        }
    }

    /// The attack button came up: a tap makes a light swing.
    pub fn attack_release(&mut self) {
        if self.charge.take().is_some() {
            self.begin_swing(Swing::Light);
        }
    }

    /// A heavy swing at once (the heavy attack's own button).
    pub fn heavy_attack(&mut self) {
        self.begin_swing(Swing::Heavy);
    }

    /// `dt` seconds pass for the weapon: its speed runs down, and a button held long enough makes a heavy swing.
    fn tick_swing(&mut self, dt: f32) {
        self.since_swing += dt;
        self.busy = (self.busy - dt).max(0.0);
        if let (Some(drawn), Some(l)) = (&mut self.ranged.drawn, self.ranged.launcher) {
            *drawn = (*drawn + dt).min(l.draw);
        }
        if let Some(held) = &mut self.charge {
            *held += dt;
            if *held >= HEAVY_HOLD {
                self.charge = None;
                self.begin_swing(Swing::Heavy);
            }
        }
    }

    /// Why a swing or a shot was refused since the last call.
    pub fn take_refusal(&mut self) -> Option<&'static str> {
        self.refused.take()
    }

    /// The shoot button went down: the string is drawn for as long as it is held, and takes the speed of the weapon to be drawn fully.
    pub fn shoot_press(&mut self) {
        if !self.ready_to_fight() {
            return;
        }
        if self.no_shot {
            return self.refused = Some("You can't do that right now.");
        }
        if self.attack.is_some() || self.ranged.drawn.is_some() {
            return;
        }
        let Some(l) = self.ranged.launcher else { return self.refused = Some("You have no ranged weapon equipped.") };
        if self.ranged.ammo == 0 {
            return self.refused = Some("Ammo needs to be put in the paper doll ammo slot before it can be fired.");
        }
        let cost = LIGHT_COST * (l.draw / 2.0).clamp(0.6, 1.6);
        if self.vitals.stamina.current < cost {
            return self.refused = Some("Not enough stamina.");
        }
        self.vitals.stamina.current -= cost;
        self.rested = 0.0;
        self.ranged.drawn = Some(0.0);
        let anim = self.ranged.visual.cast.anim.unwrap_or(if l.ammo_kind == 3 { crate::animator::ATTACK_RIFLE } else { crate::animator::ATTACK_BOW });
        self.character.draw(anim, l.draw);
        self.events.extend(self.ranged.visual.precast.sound.map(Sfx::Hold));
    }

    /// The shoot button came up: the shot flies, doing the share of its damage that the string was drawn of its full draw.
    pub fn shoot_release(&mut self) {
        if let (Some(drawn), Some(l)) = (self.ranged.drawn.take(), self.ranged.launcher) {
            let power = (drawn / l.draw.max(0.1)).clamp(MIN_POWER, 1.0);
            self.ranged.shot = Some(Shot { power, visual: self.ranged.visual.clone(), projectile: self.ranged.projectile.clone() });
            self.character.loose();
            self.events.push(Sfx::Release);
            self.events.extend(self.ranged.visual.cast.sound.map(Sfx::Entry));
        }
    }

    /// The shot let go since the last call.
    pub fn take_shot(&mut self) -> Option<Shot> {
        self.ranged.shot.take()
    }

    /// The damage of a shot at full draw: a whole number between the weapon's lowest and highest.
    pub fn roll_shot(&mut self) -> f32 {
        self.roll(self.ranged.launcher.map_or(self.damage, |l| l.damage))
    }

    /// One piece of ammo was used up.
    pub fn use_ammo(&mut self) {
        self.ranged.ammo = self.ranged.ammo.saturating_sub(1);
    }

    pub fn is_drawing(&self) -> bool {
        self.ranged.drawn.is_some()
    }

    /// How far the weapon is from being ready for a swing that does all its damage, 0 to 1.
    pub fn swing_ready(&self) -> f32 {
        (self.since_swing / self.weapon_speed.max(0.1)).clamp(0.0, 1.0)
    }

    /// Whether the swing that is on is a heavy one.
    pub fn is_heavy_swinging(&self) -> bool {
        self.attack.is_some_and(|(_, _, swing, _)| swing == Swing::Heavy)
    }

    /// The animation of a spell being cast, held (`Some`) or let go of (`None`), and the one that plays when it goes off.
    pub fn set_precast(&mut self, id: Option<u16>) {
        self.character.set_precast(id);
    }

    pub fn cast_anim(&mut self, id: u16) {
        self.character.play_once(id, 0.9);
    }

    /// Where an attachment of the character (1 the right hand, 2 the left) is in the world, if it has it.
    pub fn attachment_world(&self, id: u32) -> Option<Mat4> {
        let body = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - self.facing);
        Some(Mat4::from_translation(self.feet) * body * to_render(self.character.attachment(id)?))
    }

    /// A sound the player's actions make, played with the next `take_events`.
    pub fn emit(&mut self, sfx: Sfx) {
        self.events.push(sfx);
    }

    /// The sounds the player made since the last call.
    pub fn take_events(&mut self) -> Vec<Sfx> {
        std::mem::take(&mut self.events)
    }

    pub fn set_block(&mut self, on: bool) {
        // The shield cannot be raised before it is in hand.
        if on && !self.ready_to_fight() {
            return;
        }
        self.blocking = on;
        self.character.set_block(on);
    }

    /// Pay for a jump: false (and nothing spent) when there is too little stamina.
    pub fn try_jump(&mut self) -> bool {
        let stamina = &mut self.vitals.stamina;
        if stamina.current < JUMP_COST {
            return false;
        }
        stamina.current -= JUMP_COST;
        self.rested = 0.0;
        self.events.push(Sfx::Jump);
        true
    }

    /// Where the feet are and the heading of the body (0 = north), as of the last update.
    pub fn feet(&self) -> Vec3 {
        self.feet
    }

    pub fn facing(&self) -> f32 {
        self.facing
    }

    /// The entities that make up the player: the body and what hangs on it.
    pub fn entities(&self) -> Vec<usize> {
        self.worn.entities()
    }

    /// What is worn may be other things now under the same numbers (the content changed): the next `set_equipment` dresses the body again.
    pub fn forget_items(&mut self) {
        self.worn.forget();
    }

    /// Take on what is worn: the stats (health, armor, the weapon's damage), the textures on the body, and the
    /// weapon and shield in the hands. Call at the start and whenever the equipment changes.
    pub fn set_equipment(&mut self, renderer: &mut Renderer, gpu: &Gpu, equipment: &[Option<Stack>; SLOTS], stats: &Stats) {
        let health = &mut self.vitals.health;
        let share = health.fraction();
        health.max = stats.max_health;
        health.current = (share * health.max).round().max(if health.current > 0.0 { 1.0 } else { 0.0 });
        (self.mitigation, self.damage, self.weapon_speed) = (stats.mitigation(), stats.damage, stats.speed);
        // What the weapons are made of says how they sound in the sheath: the main hand's, else the shield's.
        let handling = |slot: usize| equipment[slot].map(|s| items::get(s.item).handling).unwrap_or("");
        self.sheath_kind = match (handling(items::MAIN_HAND), handling(items::OFF_HAND)) {
            (h, _) if h.starts_with("Metal") => "METALWEAPON",
            (h, _) if h.starts_with("Wood") => "WOODWEAPON",
            (_, h) if !h.is_empty() => "SHIELD",
            _ => "",
        };
        self.ranged.launcher = equipment[items::RANGED].map(|s| items::get(s.item)).filter(|i| i.class == 2 && matches!(i.subclass, 2 | 3 | 18)).and_then(|i| {
            let w = i.weapon?;
            let bonus = stats.attack_power as f32 / items::ATTACK_POWER_PER_DPS * w.speed;
            Some(Launcher { damage: (w.min + bonus, w.max + bonus), draw: w.speed, ammo_kind: if i.subclass == 3 { 3 } else { 2 } })
        });
        self.ranged.visual = self.ranged.launcher.zip(equipment[items::RANGED]).map_or_else(Default::default, |(_, s)| crate::visuals::of_shot(&self.mount, items::get(s.item).visual.shot as u32));
        // What flies is the second model of the ammo's display: ArrowFlight_01 painted with Arrow_A_01Brown, BulletFlight_01 with bullet.
        self.ranged.projectile = equipment[0].map(|s| items::get(s.item).visual).and_then(|v| Some((format!(r"Item\ObjectComponents\Ammo\{}.m2", v.models[1]?), format!(r"Item\ObjectComponents\Ammo\{}.blp", v.textures[1]?))));
        self.ranged.ammo = equipment[0].filter(|s| items::get(s.item).wear == items::Wear::Ammo && self.ranged.launcher.is_some_and(|l| l.ammo_kind == items::get(s.item).subclass)).map_or(0, |s| s.count);
        if self.ranged.launcher.is_none() {
            self.ranged.drawn = None;
        }
        // The body is dressed, and the weapon, the shield, the helmet and the shoulders are put on it.
        let defs = equipment.map(|s| s.map(|s| items::get(s.item)));
        if let Err(e) = self.worn.dress(&mut self.character, &self.look, &self.mount, &self.cache, renderer, gpu, &defs) {
            log::warn!("cannot dress the character: {e:#}");
        }
    }

    /// The damage of a sword swing: a whole number between the weapon's lowest and highest.
    pub fn roll_damage(&mut self) -> f32 {
        self.roll(self.damage)
    }

    fn roll(&mut self, damage: (f32, f32)) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        let (low, high) = (damage.0.floor(), damage.1.ceil());
        (low + (self.rng % ((high - low) as u32 + 1)) as f32).max(1.0)
    }

    pub fn is_sneaking(&self) -> bool {
        self.sneaking
    }

    /// Sneaking (Ctrl) is for feet: in the saddle the key gets down off the steed instead.
    pub fn toggle_sneak(&mut self) {
        if self.steed.is_some() {
            return self.dismount();
        }
        self.sneaking = !self.sneaking;
    }

    pub fn set_sneaking(&mut self, on: bool) {
        self.sneaking = on && self.steed.is_none();
    }

    /// A mount item was used: get on the steed (display row, speed bonus in percent), or down from it when already riding.
    pub fn ride(&mut self, display: u32, percent: i32, renderer: &mut Renderer, gpu: &Gpu) -> Result<()> {
        if self.steed.is_some() {
            return Ok(self.dismount());
        }
        self.mount(Steed::load(&self.mount, &self.cache, renderer, gpu, display, percent)?);
        Ok(())
    }

    /// Get on the steed; the character stands up from a sneak and lowers the shield.
    fn mount(&mut self, steed: Steed) {
        self.sneaking = false;
        self.set_block(false);
        self.dismount_wanted = false;
        if let Some(old) = self.steed.replace(steed) {
            self.retired.push(old);
        }
    }

    /// R: put the weapons away, or draw them. A rider's stay put away.
    pub fn toggle_sheath(&mut self) {
        if self.steed.is_none() && self.busy <= 0.0 && self.attack.is_none() && self.ranged.drawn.is_none() {
            self.start_sheath(!self.sheathed);
        }
    }

    /// The hand goes to the sheath (`on`) or to the weapons: the animation plays, and halfway the weapons change places and make their sound.
    fn start_sheath(&mut self, on: bool) {
        self.sheathed = on;
        self.character.sheath(SHEATH_SECONDS);
        (self.sheath_wait, self.busy) = (SHEATH_SECONDS * 0.45, SHEATH_SECONDS);
    }

    /// Whether a fight can begin: the weapons are in hand. When they are not (put away with R, or a rider's), the hand goes for them, a rider gets down,
    /// and the blow waits (false) until it is done: it must be asked for again.
    fn ready_to_fight(&mut self) -> bool {
        if self.busy > 0.0 {
            return false;
        }
        if self.steed.is_some() {
            // The weapons stay in the sheath until the hand reaches them.
            (self.dismount_wanted, self.stowed) = (true, true);
        } else if !self.sheathed {
            return true;
        }
        self.start_sheath(false);
        false
    }

    pub fn is_sheathed(&self) -> bool {
        self.sheathed
    }

    /// The weapons are away (a loaded save), without the animation.
    pub fn set_sheathed(&mut self, on: bool) {
        (self.sheathed, self.stowed, self.sheath_wait, self.busy) = (on, on, 0.0, 0.0);
    }

    /// Get down at the next `update` (casting, fighting, sneaking and a new life all do).
    pub fn dismount(&mut self) {
        self.dismount_wanted = self.steed.is_some();
    }

    /// The steed being ridden: its display row and speed bonus in percent (what a save keeps).
    pub fn steed_info(&self) -> Option<(u32, i32)> {
        self.steed.as_ref().map(|s| (s.display, s.percent))
    }

    pub fn is_mounted(&self) -> bool {
        self.steed.is_some()
    }

    /// How much faster than running on foot the steed goes, as a share (0.6 for 60 percent; 0 on foot).
    pub fn speed_bonus(&self) -> f32 {
        self.steed.as_ref().map_or(0.0, |s| s.percent as f32 / 100.0)
    }

    /// Whether sprinting is possible now.
    pub fn can_sprint(&self) -> bool {
        !self.exhausted && self.vitals.stamina.current > 0.0
    }

    pub fn size(&self) -> crate::character::Size {
        self.size
    }

    /// Out of breath: a swimmer in this state sinks and is slow, until stamina has recovered to `SPRINT_RESUME`.
    pub fn is_worn_out(&self) -> bool {
        self.exhausted
    }

    /// Tell the player whether the character swims, and what the skill in swimming makes of the stamina it costs.
    pub fn set_swimming(&mut self, on: bool, cost: f32) {
        (self.swimming, self.swim_cost) = (on, cost);
        if on {
            (self.sprinting, self.sneaking) = (false, false);
        }
    }

    /// Tell the player the sprint key is down and was granted this frame.
    pub fn set_sprinting(&mut self, on: bool) {
        self.sprinting = on;
    }

    /// An attack of `damage` lands. With the shield up it is blocked, which costs stamina (`None`); without enough
    /// stamina the block fails (it uses up what is left) and the hit goes through. A hit that lands is cut by the
    /// armor and returns the damage taken.
    pub fn take_hit(&mut self, damage: f32) -> Option<u32> {
        let stamina = &mut self.vitals.stamina;
        self.rested = 0.0;
        let cost = BLOCK_COST * self.block_cost;
        if self.blocking && stamina.current >= cost {
            stamina.current -= cost;
            self.events.push(Sfx::Block);
            return None;
        }
        if self.blocking {
            stamina.current = 0.0;
        }
        let taken = (damage * (1.0 - self.mitigation)).round().max(1.0);
        self.vitals.health.current = (self.vitals.health.current - taken).max(0.0);
        self.events.push(if self.vitals.health.current <= 0.0 { Sfx::Death } else { Sfx::Wound });
        Some(taken as u32)
    }

    /// Damage of magic (or of a damage over time) lands: no shield and no armor stop it. Returns the damage taken.
    pub fn take_magic(&mut self, damage: f32) -> u32 {
        self.rested = 0.0;
        let taken = damage.round().max(1.0);
        self.vitals.health.current = (self.vitals.health.current - taken).max(0.0);
        self.events.push(if self.vitals.health.current <= 0.0 { Sfx::Death } else { Sfx::Wound });
        taken as u32
    }

    /// Spells hold the character back from swinging the weapon in the hand and from shooting (set by the game each frame).
    pub fn set_held_back(&mut self, swing: bool, shot: bool) {
        (self.no_swing, self.no_shot) = (swing, shot);
        if shot {
            self.ranged.drawn = None;
        }
    }

    /// Mana drained away: how much was there to take.
    pub fn drain_mana(&mut self, amount: f32) -> f32 {
        let taken = amount.min(self.vitals.mana.current).max(0.0);
        self.vitals.mana.current -= taken;
        taken
    }

    /// What the armor stops, from the stats with the skills in the kinds of armor worn (set by the game).
    pub fn set_mitigation(&mut self, share: f32) {
        self.mitigation = share;
    }

    /// The skill in blocking says how much of the stamina a block costs is spent (set by the game).
    pub fn set_block_cost(&mut self, factor: f32) {
        self.block_cost = factor;
    }

    /// Pay what a spell costs. False, and nothing spent, when there is not enough of it.
    pub fn spend(&mut self, cost: crate::content::Cost) -> bool {
        let (resource, amount) = match cost {
            crate::content::Cost::Mana(n) => (&mut self.vitals.mana, n),
            crate::content::Cost::Stamina(n) => (&mut self.vitals.stamina, n),
        };
        if resource.current < amount {
            return false;
        }
        resource.current -= amount;
        if matches!(cost, crate::content::Cost::Stamina(_)) {
            self.rested = 0.0;
        }
        true
    }

    /// Heal by `amount` (at most to full); returns how much it was.
    pub fn heal(&mut self, amount: f32) -> f32 {
        let health = &mut self.vitals.health;
        let healed = amount.min(health.max - health.current).max(0.0);
        health.current += healed;
        healed
    }

    /// Back on one's feet after dying: health, mana and stamina full again, the shield down.
    pub fn revive(&mut self) {
        self.vitals = Vitals::full();
        self.exhausted = false;
        self.sneaking = false;
        self.set_block(false);
        self.dismount();
    }

    /// Whether the sword reached the end of its cut since the last call: this is when it hits what is in front.
    /// The swing that reached the end of its cut since the last call, and the share of the damage it does.
    pub fn take_swing_hit(&mut self) -> Option<(Swing, f32)> {
        self.swing_hit.take()
    }

    pub fn is_blocking(&self) -> bool {
        self.blocking
    }

    pub fn is_attacking(&self) -> bool {
        self.attack.is_some()
    }

    /// Whether the character has gone to attack: the button held for a swing, a swing on, or a string drawn.
    pub fn is_striking(&self) -> bool {
        self.charge.is_some() || self.attack.is_some() || self.ranged.drawn.is_some()
    }

    /// Sprinting on the move drains stamina; otherwise it comes back after a rest.
    fn update_stamina(&mut self, dt: f32, motion: &Motion) {
        let mana = &mut self.vitals.mana;
        mana.current = (mana.current + MANA_REGEN * dt).min(mana.max);
        let stamina = &mut self.vitals.stamina;
        if self.swimming && !self.exhausted {
            let moving = motion.speed > 0.3 || motion.vertical_speed.abs() > 0.3;
            stamina.current = (stamina.current - if moving { SWIM_DRAIN } else { SWIM_REST_DRAIN } * self.swim_cost * dt).max(0.0);
            self.rested = 0.0;
            self.exhausted |= stamina.current <= 0.0;
        } else if self.sprinting && motion.speed > 0.3 && !motion.airborne {
            stamina.current = (stamina.current - SPRINT_DRAIN * dt).max(0.0);
            self.rested = 0.0;
            self.exhausted |= stamina.current <= 0.0;
        } else {
            self.rested += dt;
            if self.rested >= REGEN_DELAY {
                stamina.current = (stamina.current + STAMINA_REGEN * dt).min(stamina.max);
            }
        }
        if self.exhausted && stamina.current >= SPRINT_RESUME {
            self.exhausted = false;
        }
    }

    /// Advance the player and place its entities. `eye` is the first-person camera (its position is the
    /// head); returns the camera to render with.
    pub fn update(&mut self, dt: f32, eye: &Camera, mut motion: Motion, clip: impl Fn(Vec3, Vec3) -> f32, renderer: &mut Renderer, gpu: &Gpu) -> Camera {
        let ground = eye.pos - Vec3::Y * crate::camera::EYE_HEIGHT;
        (self.feet, self.facing) = (ground, eye.yaw - self.body_turn);
        if std::mem::take(&mut self.dismount_wanted) {
            self.retired.extend(self.steed.take());
        }
        self.retired.drain(..).for_each(|s| s.retire(renderer));
        motion.mounted = self.steed.is_some();

        self.tick_swing(dt);
        if let Some((t, length, swing, power)) = &mut self.attack {
            *t += dt;
            // The blow comes at the cut (a heavy one later, after the wind-up), and the blade whooshes as the cut starts.
            let (cut, whoosh) = if *swing == Swing::Heavy { (0.55, 0.4) } else { (0.4, 0.28) };
            if self.hit_pending && *t >= cut * *length {
                (self.hit_pending, self.swing_hit) = (false, Some((*swing, *power)));
            }
            if self.swing_pending && *t >= whoosh * *length {
                self.swing_pending = false;
                self.events.push(Sfx::Swing);
            }
            if *t >= *length {
                self.attack = None;
            }
        }
        motion.sneaking = self.sneaking;
        // A sneaking character's feet are not heard.
        self.steps.steed = self.steed.as_ref().map(|s| s.foot_set);
        if !self.sneaking && !motion.swimming {
            self.steps.update(dt, &motion, &mut self.events);
        }
        self.update_stamina(dt, &motion);
        if let Some((yaw, y)) = self.last_eye {
            let turned = (eye.yaw - yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            (motion.yaw_rate, motion.vertical_speed) = (turned / dt.max(1e-3), (eye.pos.y - y) / dt.max(1e-3));
        }
        self.last_eye = Some((eye.yaw, eye.pos.y));

        // On a steed the rider sits where its saddle is.
        let feet = match &mut self.steed {
            Some(steed) => steed.update(dt, &motion, ground, eye.yaw - self.body_turn, renderer, gpu),
            None => ground,
        };
        self.feet = feet;
        // Only the steed side-steps: the rider sits, and a man on foot runs the way he faces.
        if !motion.swimming {
            motion.side_speed = 0.0;
        }
        let third = self.view == View::Third;
        self.character.update(dt, &motion, true);
        renderer.update_entity_vertices(gpu, self.worn.body, &self.character.vertices);
        // While a bow is drawn the animation turns the head away from the body: the body turns the other way, so the head (and the camera on
        // it) look where the camera is aimed.
        let target = if self.character.aiming() { -self.character.head_yaw() } else { 0.0 };
        self.ranged.turn += (target - self.ranged.turn) * (1.0 - (-AIM_TURN_RATE * dt).exp());
        let body = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - eye.yaw + self.body_turn + self.ranged.turn);
        // The weapons reach the sheath (or the hand) when the hand does.
        if self.sheath_wait > 0.0 {
            self.sheath_wait -= dt;
            if self.sheath_wait <= 0.0 {
                self.stowed = self.sheathed;
                self.events.extend(sheath_sound(self.sheath_kind, self.sheathed));
            }
        }
        self.worn.stowed = self.steed.is_some() || self.stowed;
        self.worn.place(renderer, &self.character, feet, body, !third, self.ranged.drawn.is_some());
        self.worn.animate(dt, eye.forward(), eye.pos, renderer, gpu, &self.character, feet, body, !third, self.ranged.drawn.is_some());

        // Camera.
        let mut cam = Camera { pos: eye.pos, ..*eye };
        if third {
            // Behind the head, as far as the terrain and objects allow: it moves in at once when something is
            // in the way and eases back out afterwards.
            let pivot = eye.pos + Vec3::Y * (0.4 + feet.y - ground.y);
            let free = clip(pivot, pivot - eye.forward() * THIRD_PERSON_DISTANCE * self.follow);
            self.camera_distance = if free < self.camera_distance { free } else { (self.camera_distance + CAMERA_RECOVERY * dt).min(free) };
            cam.pos = pivot - eye.forward() * self.camera_distance;
        } else {
            // The camera rides on the head, so it follows the animation.
            cam.pos = feet + body.transform_point3(MODEL_TO_RENDER.transform_point3(self.character.eye()));
            // The shoulders and chest are closer than the usual near plane: it would cut them open.
            cam.near = FIRST_PERSON_NEAR;
        }
        cam
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps_over(seconds: f32, motion: Motion) -> Vec<Sfx> {
        let (mut stepper, mut out) = (Stepper::default(), Vec::new());
        for _ in 0..(seconds * 60.0) as usize {
            stepper.update(1.0 / 60.0, &motion, &mut out);
        }
        out
    }

    #[test]
    fn a_swing_can_come_at_any_time_but_does_the_share_of_its_damage_the_time_since_the_last_one_is_of_the_weapons_speed() {
        let mut p = player_for_tests();
        p.weapon_speed = 2.6;
        let full = p.vitals.stamina.current;
        // The first swing after a rest does all its damage, and costs 8 of a weapon of speed 2, a third more for one of 2.6.
        (p.attack_press(), p.attack_release());
        assert_eq!(p.attack.map(|a| a.3), Some(1.0));
        let light = full - p.vitals.stamina.current;
        assert!((light - 8.0 * 1.3).abs() < 1e-3, "{light}");
        // The next one may come as soon as the animation is over, and does less: 0.65 of 2.6 seconds is a quarter.
        p.attack = None;
        p.tick_swing(0.65);
        (p.attack_press(), p.attack_release());
        assert!((p.attack.unwrap().3 - 0.25).abs() < 1e-3);
        // A swing right on top of the other does the least, and what is waited for is worth the time: half the speed, half the damage.
        p.attack = None;
        p.tick_swing(0.01);
        assert_eq!(p.swing_power(), 0.15);
        p.tick_swing(1.29);
        assert!((p.swing_power() - 0.5).abs() < 1e-3);
        // A heavy one costs more than twice a light one, and is the same share of the damage (doubled by the game).
        p.attack = None;
        p.vitals.stamina.current = full;
        p.heavy_attack();
        assert!(p.is_heavy_swinging());
        let heavy = full - p.vitals.stamina.current;
        assert!(heavy > 2.0 * light && (heavy - 25.0 * 1.3).abs() < 1e-3, "{heavy}");
    }

    #[test]
    fn the_button_held_makes_a_heavy_swing_and_too_little_stamina_refuses_one() {
        let mut p = player_for_tests();
        p.attack_press();
        p.tick_swing(0.2);
        assert!(p.attack.is_none(), "not yet held long enough");
        p.tick_swing(0.3);
        assert!(p.is_heavy_swinging());
        // Letting go afterwards makes no second swing.
        p.attack_release();
        assert!(p.swings == 1);
        // Too tired.
        let mut p = player_for_tests();
        p.vitals.stamina.current = 5.0;
        (p.attack_press(), p.attack_release());
        assert!(p.attack.is_none() && p.take_refusal().is_some() && p.vitals.stamina.current == 5.0);
        p.heavy_attack();
        assert!(p.attack.is_none());
    }

    #[test]
    fn a_bow_takes_its_speed_to_draw_and_letting_go_earlier_does_part_of_the_damage() {
        let mut p = player_for_tests();
        let bow = Launcher { damage: (10.0, 10.0), draw: 2.0, ammo_kind: 2 };
        // Without a bow, or without arrows, it is refused.
        p.shoot_press();
        assert_eq!((p.take_refusal(), p.is_drawing()), (Some("You have no ranged weapon equipped."), false));
        p.ranged.launcher = Some(bow);
        p.shoot_press();
        assert!(p.take_refusal().is_some_and(|why| why.starts_with("Ammo")) && !p.is_drawing());
        p.ranged.ammo = 3;
        // Let go after half the draw: half the damage.
        p.shoot_press();
        assert!(p.is_drawing());
        p.tick_swing(1.0);
        p.shoot_release();
        assert_eq!(p.take_shot().map(|s| s.power), Some(0.5));
        // Drawn longer than it takes, it is held at the full draw.
        p.shoot_press();
        p.tick_swing(5.0);
        p.shoot_release();
        assert_eq!(p.take_shot().map(|s| s.power), Some(1.0));
        // A shot at once does the least.
        p.shoot_press();
        p.shoot_release();
        assert_eq!(p.take_shot().map(|s| s.power), Some(MIN_POWER));
    }

    #[test]
    fn r_puts_the_weapons_away_and_fighting_draws_them_again() {
        let mut p = player_for_tests();
        assert!(!p.sheathed);
        p.toggle_sheath();
        assert!(p.sheathed);
        // Again while the hand is still on its way does nothing.
        p.toggle_sheath();
        assert!(p.sheathed);
        p.tick_swing(1.1);
        p.toggle_sheath();
        assert!(!p.sheathed);
        p.tick_swing(1.1);
        p.toggle_sheath();
        p.tick_swing(1.1);
        // A blow asks the hand to draw the sword, and waits for it: the swing comes only when the press is made again afterwards.
        (p.attack_press(), p.attack_release());
        assert!(!p.sheathed && p.attack.is_none() && p.busy > 0.0, "the sword is being drawn");
        (p.attack_press(), p.attack_release());
        assert!(p.attack.is_none(), "no swing while the hand is on its way");
        p.set_block(true);
        assert!(!p.is_blocking(), "nor a block");
        p.tick_swing(1.1);
        (p.attack_press(), p.attack_release());
        assert!(p.attack.is_some(), "drawn, it strikes");
        p.attack = None;
        p.tick_swing(2.0);
        p.toggle_sheath();
        p.tick_swing(2.0);
        p.set_block(true);
        p.tick_swing(2.0);
        p.set_block(true);
        assert!(p.is_blocking(), "the shield is raised once drawn");
    }

    #[test]
    fn the_sheathing_sounds_are_in_the_games_sound_table() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let bank = crate::audio::SoundBank::load(&mount).unwrap();
        for kind in ["METALWEAPON", "WOODWEAPON", "SHIELD"] {
            for on in [true, false] {
                let Some(Sfx::Named(name)) = sheath_sound(kind, on) else { panic!("no sound for {kind}") };
                assert!(bank.named(name).and_then(|id| bank.entry(id)).is_some(), "{name} has no sound file");
            }
        }
        assert!(sheath_sound("", true).is_none());
    }

    #[test]
    fn resources_report_how_full_they_are() {
        let r = |current, max| Resource { current, max }.fraction();
        assert_eq!((r(50.0, 100.0), r(150.0, 100.0), r(-5.0, 100.0), r(5.0, 0.0)), (0.5, 1.0, 0.0, 0.0));
    }

    fn player_for_tests() -> Player {
        // Only the vitals logic is used; the character and entities are not needed.
        let mount = Arc::new(mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()).expect("game data"));
        Player {
            character: Character::load(&mount, &ModelCache::default(), "player", &crate::character::Appearance::default()).unwrap(),
            mount,
            worn: Worn::none(),
            cache: ModelCache::default(),
            view: View::First,
            body_turn: 0.0,
            attack: None,
            weapon_speed: 2.0,
            since_swing: 99.0,
            charge: None,
            refused: None,
            ranged: Ranged::default(),
            last_eye: None,
            blocking: false,
            sprinting: false,
            block_cost: 1.0,
            swimming: false,
            swim_cost: 1.0,
            size: Default::default(),
            sneaking: false,
            exhausted: false,
            rested: 0.0,
            vitals: Vitals::full(),
            feet: Vec3::ZERO,
            facing: 0.0,
            mitigation: 0.0,
            damage: (12.0, 12.0),
            rng: 1,
            camera_distance: THIRD_PERSON_DISTANCE,
            follow: 1.0,
            events: Vec::new(),
            steps: Stepper::default(),
            swings: 0,
            swing_pending: false,
            hit_pending: false,
            swing_hit: None,
            steed: None,
            dismount_wanted: false,
            sheathed: false,
            stowed: false,
            sheath_wait: 0.0,
            busy: 0.0,
            sheath_kind: "",
            retired: Vec::new(),
            look: Default::default(),
            no_swing: false,
            no_shot: false,
        }
    }

    #[test]
    fn sprinting_drains_stamina_until_exhausted_and_it_recovers_after_a_rest() {
        if mpq::find_wow_dir().is_none() {
            return eprintln!("skipped: no game data");
        }
        let mut p = player_for_tests();
        let running = Motion { speed: 10.0, forward_speed: 10.0, ..Default::default() };
        p.set_sprinting(true);
        // Standing still with the key down costs nothing.
        p.update_stamina(1.0, &Motion::default());
        assert_eq!(p.vitals.stamina.current, 100.0);
        // Four seconds of sprinting: 80 gone.
        for _ in 0..240 {
            p.update_stamina(1.0 / 60.0, &running);
        }
        assert!((p.vitals.stamina.current - 20.0).abs() < 0.5, "{}", p.vitals.stamina.current);
        assert!(p.can_sprint());
        // Out of stamina: no more sprinting until it recovers to the resume level.
        for _ in 0..120 {
            p.update_stamina(1.0 / 60.0, &running);
        }
        assert!(p.vitals.stamina.current == 0.0 && !p.can_sprint());
        p.set_sprinting(false);
        p.update_stamina(0.9, &Motion::default());
        assert_eq!(p.vitals.stamina.current, 0.0, "recovery starts after the rest");
        for _ in 0..60 {
            p.update_stamina(1.0 / 60.0, &Motion::default());
        }
        assert!(p.vitals.stamina.current > 10.0 && !p.can_sprint());
        for _ in 0..120 {
            p.update_stamina(1.0 / 60.0, &Motion::default());
        }
        assert!(p.can_sprint());
    }

    #[test]
    fn swimming_drains_stamina_less_with_skill_and_a_worn_out_swimmer_waits_to_recover() {
        if mpq::find_wow_dir().is_none() {
            return eprintln!("skipped: no game data");
        }
        let mut p = player_for_tests();
        let swimming = Motion { speed: 4.7, forward_speed: 4.7, swimming: true, ..Default::default() };
        p.set_swimming(true, 1.0);
        for _ in 0..60 {
            p.update_stamina(1.0 / 60.0, &swimming);
        }
        let plain = 100.0 - p.vitals.stamina.current;
        assert!((plain - SWIM_DRAIN).abs() < 0.2, "{plain}");
        p.vitals.stamina.current = 100.0;
        p.set_swimming(true, 0.5);
        for _ in 0..60 {
            p.update_stamina(1.0 / 60.0, &swimming);
        }
        assert!((100.0 - p.vitals.stamina.current - plain / 2.0).abs() < 0.2);
        // Treading water costs a quarter of swimming.
        p.vitals.stamina.current = 100.0;
        p.set_swimming(true, 1.0);
        for _ in 0..60 {
            p.update_stamina(1.0 / 60.0, &Motion { swimming: true, ..Default::default() });
        }
        assert!((100.0 - p.vitals.stamina.current - SWIM_REST_DRAIN).abs() < 0.2);
        // Out of stamina: worn out, nothing more is spent and it comes back after a rest.
        p.vitals.stamina.current = 0.5;
        for _ in 0..120 {
            p.update_stamina(1.0 / 60.0, &swimming);
        }
        assert!(p.is_worn_out() && p.vitals.stamina.current > 1.0);
    }

    #[test]
    fn a_jump_costs_stamina_and_needs_enough_of_it() {
        if mpq::find_wow_dir().is_none() {
            return eprintln!("skipped: no game data");
        }
        let mut p = player_for_tests();
        assert!(p.try_jump());
        assert_eq!(p.vitals.stamina.current, 90.0);
        assert_eq!(p.take_events(), [Sfx::Jump]);
        p.vitals.stamina.current = 9.0;
        assert!(!p.try_jump());
        assert_eq!(p.vitals.stamina.current, 9.0);
    }

    #[test]
    fn blocking_costs_stamina_only_when_a_hit_is_blocked() {
        if mpq::find_wow_dir().is_none() {
            return eprintln!("skipped: no game data");
        }
        let mut p = player_for_tests();
        // Holding the shield up costs nothing by itself.
        p.set_block(true);
        p.update_stamina(5.0, &Motion::default());
        assert_eq!((p.vitals.stamina.current, p.vitals.health.current), (100.0, 100.0));
        // A blocked hit costs stamina and no health, and rings on the shield.
        assert_eq!(p.take_hit(10.0), None);
        assert_eq!(p.take_events(), [Sfx::Block]);
        assert_eq!((p.vitals.stamina.current, p.vitals.health.current), (80.0, 100.0));
        // Without the shield it hurts and costs no stamina.
        p.set_block(false);
        assert_eq!(p.take_hit(10.0), Some(10));
        assert_eq!(p.take_events(), [Sfx::Wound]);
        assert_eq!((p.vitals.stamina.current, p.vitals.health.current), (80.0, 90.0));
        // With too little stamina the block fails: the hit lands and the stamina is gone.
        p.set_block(true);
        p.vitals.stamina.current = 5.0;
        assert_eq!(p.take_hit(10.0), Some(10));
        assert_eq!((p.vitals.stamina.current, p.vitals.health.current), (0.0, 80.0));
        // The last hit kills: a death cry instead of a wound.
        p.take_events();
        p.take_hit(200.0);
        assert_eq!(p.take_events(), [Sfx::Death]);
    }

    #[test]
    fn footsteps_follow_the_stride_and_landing_thuds() {
        let walk = Motion { speed: 2.5, forward_speed: 2.5, ..Default::default() };
        let run = Motion { speed: 7.0, forward_speed: 7.0, ..Default::default() };
        // 8 seconds of walking = 20 yards = 16 strides of 1.25; of running = 56 yards = 24 strides of 2.3.
        let count = |v: Vec<Sfx>| v.iter().filter(|&&s| s == Sfx::Footstep).count();
        assert!((count(steps_over(8.0, walk)) as i32 - 16).abs() <= 1);
        assert!((count(steps_over(8.0, run)) as i32 - 24).abs() <= 1);
        assert!(steps_over(2.0, Motion::default()).is_empty());
        // Jumping makes no steps and one thud when the feet touch down.
        let mut stepper = Stepper::default();
        let mut out = Vec::new();
        stepper.update(0.5, &Motion { airborne: true, ..Default::default() }, &mut out);
        assert!(out.is_empty());
        stepper.update(0.016, &Motion::default(), &mut out);
        stepper.update(0.016, &Motion::default(), &mut out);
        assert_eq!(out, [Sfx::Land]);
    }
}
