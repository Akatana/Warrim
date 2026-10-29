//! How a creature gets from one place to another, as the original's server has it: the PathFinder makes the points of its way over the navigation mesh, and the
//! creature walks them at its speed (the client draws the movement between two points as a straight line, the heights of the points included; nothing sets a
//! creature on the ground afterwards). The generators of the original are here: the chase (`ChaseMovementGenerator`: looks every 250 ms whether the target
//! has gone from where the way leads, backs off from a target that stands too close, fans out from a creature that stands where it wants to, runs from a
//! target to keep its distance), the way home (`HomeMovementGenerator`), the strolls and the flight in terror (`AbstractRandomMovementGenerator`: a ray over
//! the mesh to a random point), the patrol (`WaypointMovementGenerator`: a path to each place) and the run to a friend.

use super::*;
use crate::coords::{render_to_wow, wow_to_render};
use crate::navmesh::{NavMesh, PathFinder, PATHFIND_NOPATH, PATHFIND_SHORTCUT, PATHFIND_NOT_USING_PATH};

/// The steepest slope a creature walks, rise over run (about 60 degrees).
const STEEPEST_WALK: f32 = 1.75;
/// Seconds between two looks at whether the target has gone from where the way leads.
const RECHECK: f32 = 0.25;
/// Seconds a creature stands at its target before it looks whether it stands too close or in another's place (`CHASE_CLOSENESS_TIMER`).
const CLOSENESS: f32 = 2.0;
/// The speed of a creature that is put back at its home in a straight line (the original sends such a spline at 400 yards a second).
const TELEPORT_SPEED: f32 = 400.0;
/// A creature further from home than this is not walked back: it goes and is back after a few seconds (outside the instances).
const FAR_HOME: f32 = 150.0;
/// Another creature that stands this close to the target, not moving, and fights it too, is in the way: the creature moves aside (`FanOut`).
const FANNING_RADIUS: f32 = 1.0;
const FAN_ANGLE: (f32, f32) = (std::f32::consts::PI / 5.0, std::f32::consts::PI / 4.0);
/// The most of the combined reach that backing off counts from.
const BACKPEDAL_REACH: f32 = 3.75;
/// A creature in terror runs to places round the one it flees from: closer to it than the first number it runs further, beyond the second it runs less (the original's quiet distance).
const QUIET_DISTANCE: (f32, f32) = (28.0, 43.0);
/// Seconds between the places a creature in terror runs to.
const FEAR_WAIT: (f32, f32) = (0.5, 1.5);

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Mode {
    Normal,
    /// Walking back from a target that stands too close.
    Backpedal,
    /// Running from the target to keep a distance (a caster).
    Distancing,
    /// Moving aside for another creature that stands where this one wants to.
    Fanning,
}

/// The state of the chase (`ChaseMovementGenerator`).
#[derive(Clone, Copy, Debug)]
pub(super) struct Chase {
    pub mode: Mode,
    /// It has come to where it was going (and the target has not gone since).
    pub reached: bool,
    /// The target can be got at: the last way ended within reach of it.
    pub reachable: bool,
    last_target: Vec3,
    /// Seconds until it looks whether it stands too close, and whether that time has gone.
    timer: f32,
    expired: bool,
}

impl Chase {
    pub fn new() -> Chase {
        Chase { mode: Mode::Normal, reached: false, reachable: true, last_target: Vec3::ZERO, timer: 0.0, expired: false }
    }
}

fn wow(p: Vec3) -> Vec3 {
    Vec3::from(render_to_wow(p))
}

impl Mob {
    /// Make the way to `goal` (`DispatchSplineToPosition`): a ray over the mesh where the goal is near and in sight, else a path round whatever is in the way.
    /// `force` has it end at the goal even where the mesh does not reach. Returns the kind of path; the course is set to it.
    pub(super) fn plan(&mut self, goal: Vec3, straight_first: bool, force: bool, nav: &NavMesh, blocked: &impl Fn(Vec3, Vec3) -> bool) -> u8 {
        let (from, to) = (wow(self.pos), wow(goal));
        let eye = Vec3::Y * EYE;
        let finder = self.finder.get_or_insert_with(|| Box::new(PathFinder::new(false)));
        let mut generated = false;
        if straight_first && from.distance(to) < 200.0 && (from.z - to.z).abs() < 5.0 && !blocked(self.pos + eye, goal + eye) {
            finder.calculate(nav, from, to, false, true);
            generated = true;
        }
        if !generated || finder.kind & (crate::navmesh::PATHFIND_NOPATH | crate::navmesh::PATHFIND_INCOMPLETE) != 0 {
            finder.calculate(nav, from, to, force, false);
        }
        let kind = finder.kind;
        self.take_course(kind);
        kind
    }

    /// The course is the path the finder has made (its first point is where the creature is).
    fn take_course(&mut self, kind: u8) {
        let Some(finder) = self.finder.as_ref() else { return };
        self.course = finder.path.iter().skip(1).rev().map(|p| wow_to_render(p.to_array())).collect();
        self.dest = self.course.first().copied();
        self.loose = kind & (PATHFIND_SHORTCUT | PATHFIND_NOT_USING_PATH) != 0;
    }

    /// Stop where it is (`InterruptMoving`).
    pub(super) fn halt(&mut self) {
        self.course.clear();
    }

    /// Walk the course for `dt` seconds at `speed`. Where the mesh was not there to lead the way (a straight line), a wall still stops it. Returns the yards walked.
    pub(super) fn follow(&mut self, dt: f32, speed: f32, blocked: &impl Fn(Vec3, Vec3) -> bool) -> f32 {
        if self.root > 0.0 {
            return 0.0;
        }
        let (mut left, mut walked) = (speed * dt, 0.0);
        while left > 0.0 {
            let Some(&next) = self.course.last() else { break };
            // (Along the ground: the height follows from where on the leg it is.)
            let to = flat(next - self.pos);
            let length = to.length();
            let step = left.min(length);
            let target = if length <= left { next } else { self.pos + to * (step / length) + Vec3::Y * (next.y - self.pos.y) * (step / length) };
            if self.loose && blocked(self.pos, target) {
                self.course.clear();
                break;
            }
            if length > 1e-3 {
                self.yaw = to.x.atan2(-to.z);
            }
            // It climbs or falls no faster than a steep slope allows: a point of the course on another layer (the terrain over a tunnel) does not lift it there at once; the floor under
            // it (`stand`) says where it stands.
            let rise = (target.y - self.pos.y).clamp(-step * STEEPEST_WALK - 0.05, step * STEEPEST_WALK + 0.05);
            self.pos = Vec3::new(target.x, self.pos.y + rise, target.z);
            if length <= left {
                self.course.pop();
            }
            walked += step;
            left -= step.max(1e-6);
        }
        self.moving |= walked > 0.0;
        walked
    }

    /// `DispatchSplineToPosition` of a chase: whether a way was found.
    fn dispatch(&mut self, goal: Vec3, nav: &NavMesh, blocked: &impl Fn(Vec3, Vec3) -> bool) -> bool {
        if self.plan(goal, true, false, nav, blocked) & PATHFIND_NOPATH != 0 {
            self.halt();
            return false;
        }
        true
    }

    /// The place `distance` yards from `target` on the side `angle` (radians from the line target-to-creature) turns to.
    fn place_near(&self, target: Vec3, distance: f32, angle: f32) -> Vec3 {
        let from = flat(self.pos - target).normalize_or_zero();
        let (s, c) = angle.sin_cos();
        target + Vec3::new(from.x * c - from.z * s, 0.0, from.x * s + from.z * c) * distance
    }

    /// Go for `target` and stand `stand` yards from it, as the chase of the original does. `crowd` are the places of the others that stand fighting it; `vs_player`:
    /// the target is the player (a creature that fights another creature does not back off from it). Returns the yards walked.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn chase_to(&mut self, target: Vec3, stand: f32, speed: f32, dt: f32, vs_player: bool, crowd: &[Vec3], nav: &NavMesh, blocked: &impl Fn(Vec3, Vec3) -> bool) -> f32 {
        if self.chase.timer > 0.0 {
            if self.chase.timer <= dt {
                (self.chase.timer, self.chase.expired) = (0.0, true);
            } else {
                self.chase.timer -= dt;
            }
        }
        self.recheck -= dt;
        if self.recheck <= 0.0 {
            self.recheck = RECHECK;
            self.recheck_chase(target, stand, vs_player, crowd, nav, blocked);
        }
        let pace = if matches!(self.chase.mode, Mode::Backpedal | Mode::Fanning) { self.walk_speed } else { speed };
        let walked = self.follow(dt, pace, blocked);
        if self.course.is_empty() && !self.chase.reached {
            // The way is walked (`HandleFinalizedMovement`): it faces the target and stands there for a while before it looks whether it stands too close.
            self.turn(flat(target - self.pos));
            self.chase = Chase { mode: Mode::Normal, reached: true, reachable: true, last_target: target, timer: CLOSENESS, expired: false };
        }
        walked
    }

    /// `HandleTargetedMovement`, every 250 ms.
    fn recheck_chase(&mut self, target: Vec3, stand: f32, vs_player: bool, crowd: &[Vec3], nav: &NavMesh, blocked: &impl Fn(Vec3, Vec3) -> bool) {
        let reach = stand.max(self.combined_reach(PLAYER_REACH, true));
        let dest = self.dest.unwrap_or(self.pos);
        if self.chase.mode != Mode::Distancing {
            if flat(target - dest).length() > reach {
                // The target has gone from where the way leads: a new way, to the place beside it on the side the creature comes from.
                let to = self.place_near(target, stand, 0.0);
                if flat(to - self.pos).length() > 0.3 {
                    if self.dispatch(to, nav, blocked) {
                        self.chase = Chase { reached: false, last_target: target, timer: 0.0, ..self.chase };
                        return;
                    }
                    if !self.chase.reachable {
                        return;
                    }
                }
                // No way found: the target is out of reach if it is further from the creature than a blow reaches.
                if flat(target - self.pos).length() > reach {
                    self.chase.reachable = false;
                }
                return;
            }
            self.chase.reachable = true;
        }
        if !self.chase.reached {
            // While the way is walked: a target that went on, between the creature and where the way ends and in reach, stops it.
            if !self.course.is_empty() && self.chase.mode == Mode::Normal && target != self.chase.last_target {
                let (from_dest, from_target) = (self.pos.distance(dest), self.pos.distance(target));
                if from_dest > from_target && from_target < reach {
                    self.halt();
                }
            }
        } else if vs_player && self.chase.expired {
            self.backpedal(target, crowd, nav, blocked);
        }
    }

    /// `Backpedal`: a target that stands too close is walked back from; else the creature looks whether another stands in its place.
    fn backpedal(&mut self, target: Vec3, crowd: &[Vec3], nav: &NavMesh, blocked: &impl Fn(Vec3, Vec3) -> bool) {
        self.chase.expired = false;
        // Just in case no way is found.
        self.chase.timer = CLOSENESS;
        let reach = self.combined_reach(PLAYER_REACH, false).min(BACKPEDAL_REACH);
        if flat(target - self.pos).length().powi(2) < reach * reach * 0.33 {
            // Too close: away from the target, to three quarters of the reach.
            let to = self.place_near(target, reach * 0.75, 0.0);
            if self.dispatch(to, nav, blocked) {
                self.chase = Chase { mode: Mode::Backpedal, reached: false, timer: 0.0, ..self.chase };
            }
            return;
        }
        self.fan_out(target, crowd, nav, blocked);
    }

    /// `FanOut`: another creature that fights the target stands within a yard: the creature goes round the target to another place, either way.
    pub(super) fn fan_out(&mut self, target: Vec3, crowd: &[Vec3], nav: &NavMesh, blocked: &impl Fn(Vec3, Vec3) -> bool) {
        if !crowd.iter().any(|p| flat(*p - self.pos).length() < FANNING_RADIUS && flat(*p - self.pos).length() > 1e-3) {
            return;
        }
        let side = if self.random() < 0.5 { -1.0 } else { 1.0 };
        for direction in [side, -side] {
            let angle = FAN_ANGLE.0 + (FAN_ANGLE.1 - FAN_ANGLE.0) * self.random();
            let to = self.place_near(target, self.combined_reach(PLAYER_REACH, false), angle * direction);
            if self.dispatch(to, nav, blocked) {
                self.chase = Chase { mode: Mode::Fanning, reached: false, timer: 0.0, ..self.chase };
                return;
            }
        }
    }

    /// `DistanceYourself`: run from the target to a place `distance` yards from it. Whether it set out.
    pub(super) fn distance_yourself(&mut self, target: Vec3, distance: f32, nav: &NavMesh, blocked: &impl Fn(Vec3, Vec3) -> bool) -> bool {
        let to = self.place_near(target, distance, 0.0);
        if self.dispatch(to, nav, blocked) {
            self.chase = Chase { mode: Mode::Distancing, reached: false, ..self.chase };
            return true;
        }
        false
    }

    /// The way home, made when it sets out (`HomeMovementGenerator`) and walked at a run: where there is no way it is a straight line, covered at once. Returns whether it is home.
    pub(super) fn go_home(&mut self, dt: f32, nav: &NavMesh, blocked: &impl Fn(Vec3, Vec3) -> bool) -> bool {
        if flat(self.home - self.pos).length() < 0.3 {
            return true;
        }
        if flat(self.home - self.pos).length() > FAR_HOME {
            self.pos = self.home;
            return true;
        }
        if self.course.is_empty() {
            let kind = self.plan(self.home, false, true, nav, blocked);
            self.fast = kind & (PATHFIND_NOPATH | PATHFIND_SHORTCUT) != 0;
            // (What is put back in a straight line is not stopped by what stands between.)
            self.loose = false;
        }
        self.follow(dt, if self.fast { TELEPORT_SPEED } else { self.run_speed }, blocked);
        self.course.is_empty()
    }

    /// A ray over the mesh to the random point `point` (`ComputePathToRandomPoint`): a stroll, or a flight in terror. Whether there is a way.
    pub(super) fn random_course(&mut self, point: Vec3, nav: &NavMesh) -> bool {
        let (from, to) = (wow(self.pos), wow(point));
        let finder = self.finder.get_or_insert_with(|| Box::new(PathFinder::new(false)));
        finder.compute_path_to_random_point(nav, from, to);
        let kind = finder.kind;
        self.take_course(kind);
        kind & PATHFIND_NOPATH == 0
    }

    /// In terror the creature runs to random places round the one it flees from, one after the other (`FleeingMovementGenerator`). Returns the yards walked.
    pub(super) fn run_in_terror(&mut self, from: Vec3, dt: f32, nav: &NavMesh, blocked: &impl Fn(Vec3, Vec3) -> bool) -> f32 {
        if self.course.is_empty() {
            self.fear_wait -= dt;
            if self.fear_wait <= 0.0 {
                let d = flat(self.pos - from).length();
                let span = QUIET_DISTANCE.1 - QUIET_DISTANCE.0;
                let radius = if d < QUIET_DISTANCE.0 {
                    (0.4 + 0.9 * self.random()) * (QUIET_DISTANCE.0 - d)
                } else if d > QUIET_DISTANCE.1 {
                    (0.4 + 0.6 * self.random()) * span
                } else {
                    (0.6 + 0.6 * self.random()) * span
                };
                let (angle, range) = (self.random() * std::f32::consts::TAU, self.random() * radius);
                let point = from + Vec3::new(angle.sin() * range, self.pos.y - from.y, angle.cos() * range);
                let found = self.random_course(point, nav);
                self.fear_wait = if found { FEAR_WAIT.0 + (FEAR_WAIT.1 - FEAR_WAIT.0) * self.random() } else { 0.5 };
            }
        }
        self.follow(dt, self.run_speed, blocked)
    }

    /// The way to the next place of a patrol (`BuildIntPath`): a path that ends there. The content gives the place no height: it is that of the mesh nearest to the creature's.
    pub(super) fn patrol_course(&mut self, place: Vec3, nav: &NavMesh, blocked: &impl Fn(Vec3, Vec3) -> bool) {
        let finder = self.finder.get_or_insert_with(|| Box::new(PathFinder::new(false)));
        let mut to = wow(Vec3::new(place.x, self.pos.y, place.z));
        to.z = finder.height_near(nav, to).unwrap_or(to.z);
        let goal = wow_to_render(to.to_array());
        self.plan(goal, false, true, nav, blocked);
    }
}
