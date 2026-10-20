//! The player's animation engine, after the client's: a state machine picks the M2 sequence for what the
//! character is doing; changing sequence fades over the new one's blend time (a smoothstep of the weight);
//! a sequence with siblings (the idle fidgets, the sword swings) is repeated a random number of times and
//! then replaced by a weighted-random sibling; the upper body has a channel of its own for the swing and
//! the combat-ready stance while the legs keep walking.

use crate::anim::{Mix, Pose};
use wowfmt::m2::M2;

/// Animation ids of the M2 sequences used.
pub const STAND: u16 = 0;
pub const WALK: u16 = 4;
pub const RUN: u16 = 5;
/// The crouched walk and stand of a sneaking character (the original's stealth animations).
pub const STEALTH_WALK: u16 = 119;
pub const STEALTH_STAND: u16 = 120;
pub const SHUFFLE_LEFT: u16 = 11;
pub const SHUFFLE_RIGHT: u16 = 12;
pub const WALK_BACK: u16 = 13;
pub const ATTACK_1H: u16 = 17;
/// The swing of a heavy attack: the two-handed one.
pub const ATTACK_2H: u16 = 18;
/// Drawing a bow and loosing it, and the same with a gun.
pub const ATTACK_BOW: u16 = 46;
pub const ATTACK_RIFLE: u16 = 49;
/// The share of the bow's animation that is the drawing: it is held there until the string is let go.
pub const DRAW_SHARE: f32 = 0.6;
/// Casting a spell at a target, and all around.
pub const SPELL_CAST_DIRECTED: u16 = 53;
pub const SPELL_CAST_OMNI: u16 = 54;
pub const READY_1H: u16 = 26;
pub const SHIELD_BLOCK: u16 = 24;
pub const JUMP_START: u16 = 37;
pub const JUMP: u16 = 38;
pub const JUMP_END: u16 = 39;
pub const FALL: u16 = 40;
/// Swimming: treading water, swimming ahead, to the left and to the right, and backwards.
pub const SWIM_IDLE: u16 = 41;
pub const SWIM: u16 = 42;
pub const SWIM_LEFT: u16 = 43;
pub const SWIM_RIGHT: u16 = 44;
pub const SWIM_BACK: u16 = 45;
/// The pose of a rider in the saddle.
pub const MOUNT: u16 = 91;
/// Putting the weapons away or drawing them: the hand goes to the hip (the other, 89, to the back).
pub const HIP_SHEATH: u16 = 90;

/// The shield block animation raises the shield in its first 350 ms, keeps it up until about 650 ms and then
/// lowers it. While the button is held it stays at this time, with the shield fully up.
const BLOCK_HOLD_MS: f32 = 400.0;
/// Seconds the combat-ready stance lasts after a swing.
const COMBAT_SECONDS: f32 = 6.0;

/// What the movement code tells the animator about this frame.
#[derive(Clone, Copy, Default, Debug)]
pub struct Motion {
    /// Horizontal speed in yards per second.
    pub speed: f32,
    /// Speed along the facing direction (negative when backing up).
    pub forward_speed: f32,
    /// Speed to the right (negative to the left); only a steed side-steps with it (the character on foot leaves it 0).
    pub side_speed: f32,
    /// Vertical speed, yards per second (negative when falling).
    pub vertical_speed: f32,
    /// Turning speed in radians per second (positive to the right).
    pub yaw_rate: f32,
    pub airborne: bool,
    /// Sneaking: the crouched animations replace walking, running and standing.
    pub sneaking: bool,
    /// Riding: the character sits in the saddle (and its steed does the walking).
    pub mounted: bool,
    /// In the water, off the bottom: the swimming animations replace the walking ones.
    pub swimming: bool,
}

pub(crate) struct Rng(pub(crate) u32);

impl Rng {
    pub(crate) fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
}

fn length(m2: &M2, seq: usize) -> f32 {
    (m2.sequences[seq].end - m2.sequences[seq].start) as f32
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Whether the animation repeats until the state changes.
fn looping(id: u16) -> bool {
    // The last four are the creatures' death, combat wound and unarmed attack (the wolf's bite).
    !matches!(id, JUMP_START | JUMP_END | ATTACK_1H | ATTACK_2H | ATTACK_BOW | ATTACK_RIFLE | SPELL_CAST_DIRECTED | SPELL_CAST_OMNI | SHIELD_BLOCK | HIP_SHEATH | 89 | 1 | 9 | 16 | 57 | 113 | 181)
}

/// Walking and running step in the same rhythm, so a switch between them keeps the stride's phase.
fn locomotion(id: u16) -> bool {
    matches!(id, WALK | RUN | WALK_BACK | STEALTH_WALK)
}

/// A sequence of animation `id`: a weighted-random one of its variations (`vary`) or the first.
fn pick(m2: &M2, id: u16, vary: bool, rng: &mut Rng) -> Option<usize> {
    let all: Vec<usize> = (0..m2.sequences.len()).filter(|&i| m2.sequences[i].id == id).collect();
    let total: u32 = all.iter().map(|&i| m2.sequences[i].probability as u32).sum();
    if !vary || total == 0 {
        return all.first().copied();
    }
    let mut r = rng.next() % total;
    all.into_iter().find(|&i| {
        let p = m2.sequences[i].probability as u32;
        let hit = r < p;
        r = r.saturating_sub(p);
        hit
    })
}

#[derive(Clone, Copy)]
struct Playing {
    id: u16,
    seq: usize,
    /// Milliseconds into the sequence.
    time: f32,
    /// Plays of the sequence before a sibling replaces it, and plays so far.
    replays: u32,
    played: u32,
}

fn play(m2: &M2, id: u16, seq: usize, rng: &mut Rng) -> Playing {
    let s = &m2.sequences[seq];
    let span = s.replay_max.saturating_sub(s.replay_min) + 1;
    Playing { id, seq, time: 0.0, replays: (s.replay_min + rng.next() % span).max(1), played: 0 }
}

/// One layer of animation: what plays now, and what is being faded out.
pub(crate) struct Channel {
    cur: Playing,
    prev: Option<Playing>,
    /// 0..1 progress of the fade into `cur` over `fade_ms`.
    fade: f32,
    fade_ms: f32,
    /// A one-shot animation has reached its last frame.
    done: bool,
    /// Plays whatever the body plays instead of its own animation.
    following: bool,
}

impl Channel {
    pub(crate) fn new(m2: &M2, id: u16, rng: &mut Rng) -> Channel {
        let seq = pick(m2, id, false, rng).unwrap_or(0);
        Channel { cur: play(m2, id, seq, rng), prev: None, fade: 1.0, fade_ms: 0.0, done: false, following: false }
    }

    fn begin(&mut self, m2: &M2, next: Playing) {
        let len = length(m2, self.cur.seq);
        self.prev = Some(Playing { time: self.cur.time.min(len), ..self.cur });
        self.fade = 0.0;
        self.fade_ms = m2.sequences[next.seq].blend_ms as f32;
        self.cur = next;
        self.done = false;
        self.following = false;
    }

    /// Switch to animation `id` unless it already plays.
    pub(crate) fn request(&mut self, m2: &M2, id: u16, vary: bool, rng: &mut Rng) {
        if self.cur.id == id && !self.following {
            return;
        }
        let Some(seq) = pick(m2, id, vary, rng) else { return };
        let mut next = play(m2, id, seq, rng);
        if locomotion(self.cur.id) && locomotion(id) {
            next.time = self.cur.time / length(m2, self.cur.seq).max(1.0) * length(m2, seq);
        }
        self.begin(m2, next);
    }

    /// Play what `body` plays, in step with it.
    fn follow(&mut self, m2: &M2, body: &Channel) {
        if !self.following || self.cur.seq != body.cur.seq {
            self.begin(m2, body.cur);
            self.following = true;
        } else {
            self.cur = body.cur;
        }
    }

    pub(crate) fn advance(&mut self, m2: &M2, dt_ms: f32, rate: f32, vary: bool, rng: &mut Rng) {
        self.fade = if self.fade_ms > 0.0 { (self.fade + dt_ms / self.fade_ms).min(1.0) } else { 1.0 };
        if self.fade >= 1.0 {
            self.prev = None;
        }
        if let Some(p) = &mut self.prev {
            let len = length(m2, p.seq).max(1.0);
            p.time = if looping(p.id) { (p.time + dt_ms * rate) % len } else { (p.time + dt_ms * rate).min(len) };
        }
        if self.following {
            return;
        }
        self.cur.time += dt_ms * rate;
        let len = length(m2, self.cur.seq).max(1.0);
        if self.cur.time < len {
            return;
        }
        if !looping(self.cur.id) {
            self.cur.time = len;
            self.done = true;
            return;
        }
        self.cur.time %= len;
        self.cur.played += 1;
        if vary && self.cur.played >= self.cur.replays {
            let id = self.cur.id;
            match pick(m2, id, true, rng) {
                Some(seq) if seq != self.cur.seq => self.begin(m2, play(m2, id, seq, rng)),
                Some(seq) => self.cur = Playing { time: self.cur.time, ..play(m2, id, seq, rng) },
                None => {}
            }
        }
    }

    /// The sequence that plays now.
    pub(crate) fn seq(&self) -> usize {
        self.cur.seq
    }

    pub(crate) fn mix(&self, m2: &M2) -> Mix {
        let pose = |p: Playing| Pose { seq: p.seq, time: m2.sequences[p.seq].start as f32 + p.time.min(length(m2, p.seq)) };
        Mix { from: self.prev.map(pose), to: pose(self.cur), weight: smoothstep(self.fade) }
    }
}

pub struct Animator {
    /// The animation held on the upper body while a spell is cast.
    pub precast: Option<u16>,
    body: Channel,
    upper: Channel,
    rng: Rng,
    /// Smoothed speed, forward speed and turning speed, so a frame's jitter does not flip the animation.
    speed: f32,
    forward: f32,
    side: f32,
    turn: f32,
    moving: bool,
    running: bool,
    backing: bool,
    /// Seconds the turn animation keeps playing after the last turning, and its direction.
    turn_hold: f32,
    turn_dir: f32,
    airborne: bool,
    air_time: f32,
    attacking: bool,
    /// The one-shot on the upper body is the sheathing: no ready stance follows it.
    sheathing: bool,
    attack_rate: f32,
    /// Where in the upper body's animation (milliseconds) it is held while a bow stays drawn.
    hold: Option<f32>,
    /// The block button is down, and the block animation is held at its raised pose.
    blocking: bool,
    block_held: bool,
    /// Seconds of combat-ready stance left.
    combat: f32,
}

impl Animator {
    pub fn new(m2: &M2, seed: u32) -> Animator {
        let mut rng = Rng(seed | 1);
        let (body, upper) = (Channel::new(m2, STAND, &mut rng), Channel::new(m2, STAND, &mut rng));
        Animator {
            body,
            upper,
            rng,
            speed: 0.0,
            forward: 0.0,
            side: 0.0,
            turn: 0.0,
            moving: false,
            running: false,
            backing: false,
            turn_hold: 0.0,
            turn_dir: 1.0,
            airborne: false,
            air_time: 0.0,
            attacking: false,
            sheathing: false,
            attack_rate: 1.0,
            hold: None,
            precast: None,
            blocking: false,
            block_held: false,
            combat: 0.0,
        }
    }

    /// Start a sword swing lasting `seconds`: a random one of the attack animations, on the upper body.
    pub fn start_attack(&mut self, m2: &M2, seconds: f32) {
        self.start_upper(m2, ATTACK_1H, seconds);
    }

    /// Play a one-shot animation `id` on the upper body, lasting `seconds`.
    pub fn start_upper(&mut self, m2: &M2, id: u16, seconds: f32) {
        let Some(seq) = pick(m2, id, true, &mut self.rng) else { return };
        let next = play(m2, id, seq, &mut self.rng);
        self.upper.begin(m2, next);
        self.attacking = true;
        self.hold = None;
        self.attack_rate = length(m2, seq) / (seconds * 1000.0).max(1.0);
    }

    /// Put the weapons away (or draw them) with the hand's trip to the hip, over `seconds`.
    pub fn start_sheath(&mut self, m2: &M2, seconds: f32) {
        self.start_upper(m2, HIP_SHEATH, seconds);
        self.sheathing = true;
    }

    /// Draw a bow (animation `id`) in `seconds` and hold it drawn until `loose`.
    pub fn start_draw(&mut self, m2: &M2, id: u16, seconds: f32) {
        self.start_upper(m2, id, seconds / DRAW_SHARE);
        self.hold = Some(length(m2, self.upper.cur.seq) * DRAW_SHARE);
    }

    /// Whether a bow or a gun is being drawn or loosed.
    pub fn aiming(&self) -> bool {
        self.attacking && matches!(self.upper.cur.id, ATTACK_BOW | ATTACK_RIFLE)
    }

    /// Let the string go: the rest of the animation plays at its own pace.
    pub fn loose(&mut self) {
        self.hold = None;
        self.attack_rate = 1.0;
    }

    /// Raise the shield and keep it up while `on`.
    pub fn set_block(&mut self, on: bool) {
        self.blocking = on;
    }

    /// Whether a swing is playing.
    #[cfg(test)]
    pub fn attacking(&self) -> bool {
        self.attacking
    }

    /// The animation id the legs play.
    #[cfg(test)]
    pub fn body_id(&self) -> u16 {
        self.body.cur.id
    }

    /// Advance `dt` seconds. `vary` lets idle animations change to their variations.
    pub fn update(&mut self, m2: &M2, dt: f32, m: &Motion, vary: bool) {
        let k = 1.0 - (-dt / 0.06).exp();
        self.speed += (m.speed - self.speed) * k;
        self.forward += (m.forward_speed - self.forward) * k;
        self.side += (m.side_speed - self.side) * k;
        self.turn += (m.yaw_rate - self.turn) * k;
        self.moving = if self.moving { self.speed > 0.2 } else { self.speed > 0.45 };
        self.running = if self.running { self.speed > 4.5 } else { self.speed > 5.5 };
        self.backing = self.forward < if self.backing { -0.25 } else { -0.5 } * self.speed;
        if self.turn.abs() > 0.5 {
            (self.turn_hold, self.turn_dir) = (0.3, self.turn.signum());
        } else {
            self.turn_hold = (self.turn_hold - dt).max(0.0);
        }

        let (landed, took_off) = (self.airborne && !m.airborne, !self.airborne && m.airborne);
        self.airborne = m.airborne;
        self.air_time = if m.airborne { self.air_time + dt } else { 0.0 };
        let was = self.body.cur.id;
        let id = if m.mounted {
            MOUNT
        } else if m.swimming {
            if !(self.moving || m.vertical_speed.abs() > 0.5) {
                SWIM_IDLE
            } else if self.moving && self.side.abs() > 0.7 * self.speed && self.forward.abs() < 0.5 * self.speed {
                if self.side > 0.0 { SWIM_RIGHT } else { SWIM_LEFT }
            } else if self.moving && self.backing {
                SWIM_BACK
            } else {
                SWIM
            }
        } else if m.airborne {
            if took_off || (was == JUMP_START && !self.body.done) {
                JUMP_START
            } else if was == FALL || (self.air_time > 0.5 && m.vertical_speed < -9.0) {
                FALL
            } else {
                JUMP
            }
        } else if landed && !self.moving || was == JUMP_END && !self.body.done && !self.moving {
            JUMP_END
        } else if self.moving {
            // Mostly sideways: the shuffle steps of the sides.
            if self.side.abs() > 0.7 * self.speed && self.forward.abs() < 0.5 * self.speed {
                if self.side > 0.0 { SHUFFLE_RIGHT } else { SHUFFLE_LEFT }
            } else if self.backing {
                WALK_BACK
            } else if m.sneaking {
                STEALTH_WALK
            } else if self.running {
                RUN
            } else {
                WALK
            }
        } else if m.sneaking {
            STEALTH_STAND
        } else if self.turn_hold > 0.0 {
            if self.turn_dir > 0.0 { SHUFFLE_RIGHT } else { SHUFFLE_LEFT }
        } else {
            STAND
        };
        self.body.request(m2, id, vary, &mut self.rng);
        let move_speed = m2.sequences[self.body.cur.seq].move_speed;
        let rate = if locomotion(id) && move_speed > 0.0 { (self.speed / move_speed).clamp(0.5, 2.0) } else { 1.0 };
        self.body.advance(m2, dt * 1000.0, rate, vary, &mut self.rng);

        if self.attacking && self.upper.done {
            (self.attacking, self.combat) = (false, if std::mem::take(&mut self.sheathing) { 0.0 } else { COMBAT_SECONDS });
        }
        if self.attacking {
            self.upper.advance(m2, dt * 1000.0, self.attack_rate, false, &mut self.rng);
            if let Some(hold) = self.hold {
                self.upper.cur.time = self.upper.cur.time.min(hold);
                self.upper.done = false;
            }
        } else if let Some(id) = self.precast {
            // A spell being cast: the hands held ready for it.
            self.upper.request(m2, id, false, &mut self.rng);
            self.upper.advance(m2, dt * 1000.0, 1.0, false, &mut self.rng);
        } else if self.blocking {
            self.upper.request(m2, SHIELD_BLOCK, false, &mut self.rng);
            self.upper.advance(m2, dt * 1000.0, 1.0, false, &mut self.rng);
            self.upper.cur.time = self.upper.cur.time.min(BLOCK_HOLD_MS);
            self.upper.done = false;
            self.block_held = true;
        } else if self.upper.cur.id == SHIELD_BLOCK && !self.upper.following && !self.upper.done {
            // Released: the shield goes down the way it came up, from the matching point of the animation.
            if std::mem::take(&mut self.block_held) {
                self.upper.cur.time = (length(m2, self.upper.cur.seq) - self.upper.cur.time.min(BLOCK_HOLD_MS)).max(0.0);
            }
            self.upper.advance(m2, dt * 1000.0, 1.0, false, &mut self.rng);
        } else {
            self.combat = if m.mounted { 0.0 } else { (self.combat - dt).max(0.0) };
            if self.combat > 0.0 && !m.airborne && !m.swimming {
                self.upper.request(m2, READY_1H, vary, &mut self.rng);
                self.upper.advance(m2, dt * 1000.0, 1.0, vary, &mut self.rng);
            } else {
                self.upper.follow(m2, &self.body);
                self.upper.advance(m2, dt * 1000.0, 1.0, false, &mut self.rng);
            }
        }
    }

    /// What the legs and what the upper body play now.
    pub fn mixes(&self, m2: &M2) -> (Mix, Mix) {
        let body = self.body.mix(m2);
        // An upper body that follows the legs also shows their fades.
        let upper = if self.upper.following && self.upper.fade >= 1.0 { body } else { self.upper.mix(m2) };
        (body, upper)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpq::Mount;

    fn human() -> Option<M2> {
        let mount = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok())?;
        Some(M2::parse(&mount.read(r"Character\Human\Male\HumanMale.m2").ok()?).expect("model parses"))
    }

    /// Run `seconds` at 60 fps with a constant motion.
    fn run(a: &mut Animator, m2: &M2, seconds: f32, m: Motion) {
        for _ in 0..(seconds * 60.0) as usize {
            a.update(m2, 1.0 / 60.0, &m, true);
        }
    }

    const WALKING: Motion = Motion { speed: 2.5, forward_speed: 2.5, vertical_speed: 0.0, yaw_rate: 0.0, airborne: false, sneaking: false, mounted: false, swimming: false, side_speed: 0.0 };

    #[test]
    fn changing_animation_fades_over_the_blend_time() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        let mut a = Animator::new(&m2, 1);
        run(&mut a, &m2, 0.05, WALKING);
        let (body, _) = a.mixes(&m2);
        assert!(body.from.is_some() && body.weight > 0.0 && body.weight < 1.0, "{body:?}");
        assert_eq!(a.body_id(), WALK);
        run(&mut a, &m2, 0.5, WALKING);
        let (body, _) = a.mixes(&m2);
        assert!(body.from.is_none() && body.weight == 1.0);
    }

    #[test]
    fn a_swimmer_treads_water_swims_ahead_backwards_and_to_the_sides_even_straight_up() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        let mut a = Animator::new(&m2, 1);
        let swim = Motion { swimming: true, ..Motion::default() };
        run(&mut a, &m2, 0.5, swim);
        assert_eq!(a.body_id(), SWIM_IDLE);
        run(&mut a, &m2, 0.5, Motion { speed: 4.7, forward_speed: 4.7, ..swim });
        assert_eq!(a.body_id(), SWIM);
        run(&mut a, &m2, 0.5, Motion { speed: 2.5, forward_speed: -2.5, ..swim });
        assert_eq!(a.body_id(), SWIM_BACK);
        run(&mut a, &m2, 0.5, Motion { speed: 4.7, side_speed: -4.7, ..swim });
        assert_eq!(a.body_id(), SWIM_LEFT);
        run(&mut a, &m2, 0.5, Motion { speed: 4.7, side_speed: 4.7, ..swim });
        assert_eq!(a.body_id(), SWIM_RIGHT);
        // Diving straight down moves no yard sideways and is still swimming.
        run(&mut a, &m2, 0.5, Motion { vertical_speed: -3.0, ..swim });
        assert_eq!(a.body_id(), SWIM);
        run(&mut a, &m2, 0.5, Motion::default());
        assert_eq!(a.body_id(), STAND);
    }

    #[test]
    fn a_sneaking_character_crouches_standing_and_walking() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        let mut a = Animator::new(&m2, 1);
        run(&mut a, &m2, 0.5, Motion { sneaking: true, ..Motion::default() });
        assert_eq!(a.body_id(), STEALTH_STAND);
        run(&mut a, &m2, 0.5, Motion { sneaking: true, speed: 3.0, forward_speed: 3.0, ..Motion::default() });
        assert_eq!(a.body_id(), STEALTH_WALK);
        // Standing up again.
        run(&mut a, &m2, 0.5, Motion::default());
        assert_eq!(a.body_id(), STAND);
    }

    #[test]
    fn walk_and_run_keep_the_stride_phase_and_jitter_does_not_flip_the_state() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        // Walking to running keeps the point in the stride.
        let mut channel = Channel::new(&m2, WALK, &mut Rng(7));
        channel.cur.time = 400.0;
        channel.request(&m2, RUN, false, &mut Rng(7));
        let (walk, run_len) = (length(&m2, channel.prev.unwrap().seq), length(&m2, channel.cur.seq));
        assert!((channel.cur.time / run_len - 400.0 / walk).abs() < 1e-3, "{} of {run_len} vs 400 of {walk}", channel.cur.time);
        let mut a = Animator::new(&m2, 1);
        run(&mut a, &m2, 0.5, Motion { speed: 7.0, forward_speed: 7.0, ..WALKING });
        assert_eq!(a.body_id(), RUN);
        // A speed dip while turning around does not drop to standing.
        run(&mut a, &m2, 0.3, Motion { speed: 0.3, forward_speed: 0.3, ..WALKING });
        assert_ne!(a.body_id(), STAND);
        run(&mut a, &m2, 0.3, Motion { speed: 7.0, forward_speed: 7.0, ..WALKING });
        assert_eq!(a.body_id(), RUN);
        // Backing up, a diagonal step back, and a strafe.
        run(&mut a, &m2, 0.5, Motion { speed: 2.5, forward_speed: -2.5, ..WALKING });
        assert_eq!(a.body_id(), WALK_BACK);
        run(&mut a, &m2, 0.5, Motion { speed: 2.5, forward_speed: 0.0, ..WALKING });
        assert_eq!(a.body_id(), WALK);
    }

    #[test]
    fn standing_turns_shuffle_and_idles_fidget() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        let mut a = Animator::new(&m2, 1);
        run(&mut a, &m2, 0.3, Motion { yaw_rate: 2.0, ..Default::default() });
        assert_eq!(a.body_id(), SHUFFLE_RIGHT);
        run(&mut a, &m2, 0.3, Motion { yaw_rate: -2.0, ..Default::default() });
        assert_eq!(a.body_id(), SHUFFLE_LEFT);
        run(&mut a, &m2, 0.6, Motion::default());
        assert_eq!(a.body_id(), STAND);
        // Over two minutes of standing the idle changes to its fidget variations and back.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..7200 {
            a.update(&m2, 1.0 / 60.0, &Motion::default(), true);
            seen.insert(a.body.cur.seq);
        }
        assert!(seen.len() >= 2, "idle never varied: {seen:?}");
        // Without variations (first person) it stays on the first one.
        let mut still = Animator::new(&m2, 1);
        for _ in 0..7200 {
            still.update(&m2, 1.0 / 60.0, &Motion::default(), false);
            assert_eq!(still.body.cur.seq, pick(&m2, STAND, false, &mut Rng(1)).unwrap());
        }
    }

    #[test]
    fn sheathing_plays_the_hand_to_the_hip_once_and_leaves_no_ready_stance() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        let mut a = Animator::new(&m2, 1);
        a.start_sheath(&m2, 1.0);
        run(&mut a, &m2, 0.5, Motion::default());
        assert_eq!(a.upper.cur.id, HIP_SHEATH);
        run(&mut a, &m2, 1.0, Motion::default());
        assert!(!a.attacking());
        assert_eq!(a.upper.cur.id, STAND, "the stance after a swing would be Ready1H");
    }

    #[test]
    fn moving_sideways_shuffles_to_the_side_and_moving_on_a_slant_runs() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        let mut a = Animator::new(&m2, 1);
        run(&mut a, &m2, 0.5, Motion { speed: 7.0, side_speed: 7.0, ..Default::default() });
        assert_eq!(a.body_id(), SHUFFLE_RIGHT);
        run(&mut a, &m2, 0.5, Motion { speed: 7.0, side_speed: -7.0, ..Default::default() });
        assert_eq!(a.body_id(), SHUFFLE_LEFT);
        run(&mut a, &m2, 0.5, Motion { speed: 7.0, forward_speed: 5.0, side_speed: 5.0, ..Default::default() });
        assert_eq!(a.body_id(), RUN);
    }

    #[test]
    fn jumps_take_off_loop_in_the_air_and_land() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        let mut a = Animator::new(&m2, 1);
        let air = Motion { airborne: true, vertical_speed: 3.0, ..Default::default() };
        run(&mut a, &m2, 0.1, air);
        assert_eq!(a.body_id(), JUMP_START);
        // The take-off is 0.83 s long.
        run(&mut a, &m2, 0.9, air);
        assert_eq!(a.body_id(), JUMP);
        // A long fall.
        run(&mut a, &m2, 0.6, Motion { vertical_speed: -15.0, ..air });
        assert_eq!(a.body_id(), FALL);
        // Landing in place plays the landing, then stands; landing on the run just runs on.
        run(&mut a, &m2, 0.05, Motion::default());
        assert_eq!(a.body_id(), JUMP_END);
        run(&mut a, &m2, 1.5, Motion::default());
        assert_eq!(a.body_id(), STAND);
        let running_jump = Motion { speed: 7.0, forward_speed: 7.0, ..air };
        run(&mut a, &m2, 0.3, running_jump);
        run(&mut a, &m2, 0.05, Motion { speed: 7.0, forward_speed: 7.0, ..Default::default() });
        assert_eq!(a.body_id(), RUN);
    }

    #[test]
    fn the_block_holds_the_raised_shield_while_pressed_and_lowers_on_release() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        let mut a = Animator::new(&m2, 1);
        run(&mut a, &m2, 0.2, Motion::default());
        a.set_block(true);
        run(&mut a, &m2, 0.2, Motion::default());
        assert_eq!(a.upper.cur.id, SHIELD_BLOCK);
        assert!(a.upper.cur.time < BLOCK_HOLD_MS);
        // Held for ten seconds: stuck at the raised pose, never finishing.
        run(&mut a, &m2, 10.0, Motion::default());
        assert_eq!((a.upper.cur.id, a.upper.cur.time, a.upper.done), (SHIELD_BLOCK, BLOCK_HOLD_MS, false));
        // Released: it plays out and the upper body follows the legs again.
        a.set_block(false);
        run(&mut a, &m2, 0.2, Motion::default());
        assert!(a.upper.cur.id == SHIELD_BLOCK && a.upper.cur.time > 600.0);
        run(&mut a, &m2, 1.0, Motion::default());
        assert!(a.upper.following);
    }

    #[test]
    fn a_heavy_swing_plays_once_and_ends() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        let mut a = Animator::new(&m2, 1);
        a.start_upper(&m2, ATTACK_2H, 1.1);
        run(&mut a, &m2, 1.3, Motion::default());
        assert!(!a.attacking(), "the heavy swing keeps repeating");
    }

    #[test]
    fn a_drawn_bow_is_held_until_the_string_is_let_go_and_then_plays_out() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        let mut a = Animator::new(&m2, 1);
        a.start_draw(&m2, ATTACK_BOW, 2.3);
        run(&mut a, &m2, 6.0, Motion::default());
        assert!(a.attacking(), "the bow stays drawn");
        let held = a.upper.cur.time;
        assert!(held > 0.0 && held < length(&m2, a.upper.cur.seq), "{held}");
        a.loose();
        run(&mut a, &m2, 1.2, Motion::default());
        assert!(!a.attacking(), "the shot plays out once");
    }

    #[test]
    fn swings_play_on_the_upper_body_then_hold_the_ready_stance() {
        let Some(m2) = human() else { return eprintln!("skipped: no game data") };
        let mut a = Animator::new(&m2, 1);
        run(&mut a, &m2, 0.5, WALKING);
        let mut swings = std::collections::HashSet::new();
        for _ in 0..16 {
            a.start_attack(&m2, 0.65);
            swings.insert(a.upper.cur.seq);
            assert!(a.attacking());
            run(&mut a, &m2, 0.3, WALKING);
            // The legs keep walking while the upper body swings.
            assert_eq!(a.body_id(), WALK);
            assert_eq!(a.upper.cur.id, ATTACK_1H);
            run(&mut a, &m2, 0.4, WALKING);
            assert!(!a.attacking());
            assert_eq!(a.upper.cur.id, READY_1H);
        }
        assert!(swings.len() >= 2, "only one swing animation: {swings:?}");
        // After the combat time the upper body follows the legs again.
        run(&mut a, &m2, 6.5, WALKING);
        assert!(a.upper.following);
        let (body, upper) = a.mixes(&m2);
        assert_eq!(body.to.seq, upper.to.seq);
    }
}
