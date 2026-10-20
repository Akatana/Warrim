//! Props with `physics` (`content/props.ron`): bottles, mugs, books, clutter. They are drawn apart from the map like the pickups and simulated as rigid
//! bodies (rapier): each is solid as the convex hull of its model, falls, tips, rolls, comes to rest on the ground and on the others, and goes to sleep. The
//! player is a cylinder that shoves them when he walks into them, and the interact key picks one up and carries it in front of him while it is held.
//!
//! The ground is not a body of the world's own: around the player a patch of terrain (sampled) and the collision triangles of the objects there are made
//! into static colliders (the arena), made again when the player has walked off from its middle or more of the map has loaded. Bodies sleep where they
//! lie, so only those the player has woken, and the arena they need, cost anything. Nothing here is saved: a game that is loaded has them all back in
//! their places.
//!
//! shortcut: a hull is made from every vertex of the model, so a hollow thing (a cup, a bowl) is solid and a thing with arms (a chair) is a lump; upgrade
//! to several hulls per model if that matters. The terrain is sampled every half yard, not the game's own triangles.

use crate::content::Prop;
use crate::models::{placement_matrix, ModelCache};
use crate::renderer::{Gpu, Renderer};
use crate::world::World;
use anyhow::{Context, Result};
use glam::{Mat4, Quat, Vec3};
use mpq::Mount;
use rapier3d::prelude::*;

/// How far (yards) the player can be from a body (its box) to take hold of it.
pub const GRAB_RANGE: f32 = 2.0;
/// A yard is about a metre.
const GRAVITY: f32 = 10.7;
const TICK: f32 = 1.0 / 60.0;
/// The player: a cylinder of a third of a yard across and 1.7 high that starts a little above the feet (he steps over a flat book, not over a bottle).
const PLAYER_RADIUS: f32 = 0.33;
const PLAYER_HALF: f32 = 0.85;
const PLAYER_LIFT: f32 = 0.12;
/// How much higher than the feet a body is still in the player's way, and how far below.
const REACH_UP: f32 = 1.8;
const REACH_DOWN: f32 = 0.3;
/// How far in front of the player and how high above his feet a held body floats, how hard it is pulled there, and how far from there it is let go.
const HOLD_DISTANCE: f32 = 1.4;
const HOLD_HEIGHT: f32 = 1.0;
const HOLD_PULL: f32 = 12.0;
const HOLD_LOSE: f32 = 3.5;
/// The fastest a body is carried or thrown (yards a second).
const FASTEST: f32 = 12.0;
/// The arena: the half width of the terrain patch and the radius of the objects around its middle (yards), how far the player may walk from the middle
/// before it is made again, how far a body may roam from the middle before it is put to sleep, and how long (seconds) more ground loading waits.
const PATCH: f32 = 16.0;
const OBJECTS: f32 = 20.0;
const REMAKE: f32 = 5.0;
const ROAM: f32 = 15.0;
const LOADING_WAIT: f32 = 1.0;
const PATCH_STEP: f32 = 0.5;

/// What the world under the bodies gives: terrain heights and the collision triangles of its objects (render space).
pub trait Ground {
    fn height(&self, x: f32, z: f32) -> Option<f32>;
    fn triangles(&self, x: f32, z: f32, radius: f32) -> Vec<[Vec3; 3]>;
    /// A number that changes as the ground loads.
    fn stamp(&self) -> usize;
}

impl Ground for World {
    fn height(&self, x: f32, z: f32) -> Option<f32> {
        self.height_at(x, z)
    }

    fn triangles(&self, x: f32, z: f32, radius: f32) -> Vec<[Vec3; 3]> {
        self.triangles_near(x, z, radius)
    }

    fn stamp(&self) -> usize {
        self.loaded_count()
    }
}

fn to(v: Vec3) -> Vector {
    Vector::new(v.x, v.y, v.z)
}

fn from(v: Vector) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn flat(a: Vec3, b: Vec3) -> f32 {
    (a.x - b.x).hypot(a.z - b.z)
}

/// The bodies collide with the player, the others and the ground; the one that is held does not collide with the player.
fn body_groups() -> InteractionGroups {
    InteractionGroups::new(Group::GROUP_1, Group::ALL, InteractionTestMode::And)
}

fn held_groups() -> InteractionGroups {
    InteractionGroups::new(Group::GROUP_1, Group::ALL ^ Group::GROUP_2, InteractionTestMode::And)
}

fn player_groups() -> InteractionGroups {
    InteractionGroups::new(Group::GROUP_2, Group::GROUP_1, InteractionTestMode::And)
}

struct Body {
    handle: RigidBodyHandle,
    collider: ColliderHandle,
    scale: f32,
    entity: usize,
    /// The ground under it is known (so it is there to see, and may be woken).
    ready: bool,
    /// What the renderer was last told.
    synced: Option<(Vec3, Quat)>,
}

/// The static colliders around a point.
struct Arena {
    center: Vec3,
    stamp: usize,
    made: f32,
    colliders: Vec<ColliderHandle>,
}

pub struct Bodies {
    sim: PhysicsWorld,
    list: Vec<Body>,
    held: Option<usize>,
    player: RigidBodyHandle,
    /// Where the cylinder was (its middle) at the last step, and where the player's feet were.
    cylinder: Option<Vec3>,
    arena: Option<Arena>,
    /// Seconds not yet simulated, and the time since the start.
    owed: f32,
    time: f32,
}

impl Bodies {
    fn empty() -> Bodies {
        let mut sim = PhysicsWorld::default();
        sim.gravity = to(Vec3::NEG_Y * GRAVITY);
        let (player, _) = sim.insert(
            RigidBodyBuilder::kinematic_position_based().can_sleep(false),
            ColliderBuilder::cylinder(PLAYER_HALF, PLAYER_RADIUS).collision_groups(player_groups()),
        );
        Bodies { sim, list: Vec::new(), held: None, player, cylinder: None, arena: None, owed: 0.0, time: 0.0 }
    }

    /// The bodies among `props` that stand in `map` (the models that can be drawn as objects: `.m2` files).
    pub fn load(mount: &Mount, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu, props: &[Prop], map: &str) -> Result<Bodies> {
        let mut bodies = Bodies::empty();
        for p in props.iter().filter(|p| p.physics && p.pickup.is_none() && p.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(map))) {
            let model = cache.get(mount, &p.model).with_context(|| format!("physics model {}", p.model))?;
            let entity = renderer.add_entity(gpu, &model);
            let turn = placement_matrix(Vec3::ZERO, p.rotation, 1.0);
            let points: Vec<Vec3> = model.vertices.iter().map(|v| Vec3::from(v.pos)).collect();
            bodies.add(&points, crate::coords::wow_to_render(p.at.into()), Quat::from_mat4(&turn), if p.scale > 0.0 { p.scale } else { 1.0 }, entity);
        }
        Ok(bodies)
    }

    /// A body shaped as the convex hull of `points` (model space, scaled by `scale`), asleep where it is put.
    fn add(&mut self, points: &[Vec3], pos: Vec3, turn: Quat, scale: f32, entity: usize) {
        let scaled: Vec<Vector> = points.iter().map(|&p| to(p * scale)).collect();
        let shape = ColliderBuilder::convex_hull(&scaled).unwrap_or_else(|| {
            // Flat or a line: a box around it, a little thick.
            let (lo, hi) = points.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &p| (lo.min(p * scale), hi.max(p * scale)));
            let (lo, hi) = if lo.x > hi.x { (Vec3::splat(-0.1), Vec3::splat(0.1)) } else { (lo, hi) };
            let half = ((hi - lo) / 2.0).max(Vec3::splat(0.02));
            ColliderBuilder::cuboid(half.x, half.y, half.z).translation(to((lo + hi) / 2.0))
        });
        let body = RigidBodyBuilder::dynamic().pose(Pose::from_parts(to(pos), Rotation::from_xyzw(turn.x, turn.y, turn.z, turn.w))).sleeping(true).ccd_enabled(true).linear_damping(0.1).angular_damping(0.8);
        let (handle, collider) = self.sim.insert(body, shape.density(400.0).friction(0.7).restitution(0.1).collision_groups(body_groups()));
        self.list.push(Body { handle, collider, scale, entity, ready: false, synced: None });
    }

    /// Take them out of the world, giving their entities back.
    pub fn retire(self, renderer: &mut Renderer) {
        self.list.iter().for_each(|b| renderer.remove_entity(b.entity));
    }

    pub fn holding(&self) -> bool {
        self.held.is_some()
    }

    fn pos(&self, i: usize) -> Vec3 {
        from(self.sim.bodies[self.list[i].handle].translation())
    }

    /// The nearest one within `range` yards of `from` (feet, render space) that is not already held.
    pub fn near(&self, from_feet: Vec3, range: f32) -> Option<usize> {
        let gap = |i: usize| {
            let aabb = self.sim.colliders[self.list[i].collider].compute_aabb();
            let (dx, dz) = ((aabb.mins.x - from_feet.x).max(from_feet.x - aabb.maxs.x).max(0.0), (aabb.mins.z - from_feet.z).max(from_feet.z - aabb.maxs.z).max(0.0));
            (dx.hypot(dz), aabb.mins.y < from_feet.y + REACH_UP && aabb.maxs.y > from_feet.y - REACH_DOWN)
        };
        (0..self.list.len())
            .filter(|&i| self.held != Some(i) && self.list[i].ready)
            .map(|i| (i, gap(i)))
            .filter(|&(_, (d, reach))| d <= range && reach)
            .min_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
            .map(|(i, _)| i)
    }

    /// Take hold of body `i`: it follows the player until it is let go.
    pub fn grab(&mut self, i: usize) {
        if i >= self.list.len() {
            return;
        }
        self.release();
        self.held = Some(i);
        let (handle, collider) = (self.list[i].handle, self.list[i].collider);
        let body = &mut self.sim.bodies[handle];
        body.wake_up(true);
        body.set_gravity_scale(0.0, true);
        self.sim.colliders[collider].set_collision_groups(held_groups());
    }

    /// Let go: it falls from where it is, with the speed it was carried at.
    pub fn release(&mut self) {
        let Some(i) = self.held.take() else { return };
        let (handle, collider) = (self.list[i].handle, self.list[i].collider);
        let body = &mut self.sim.bodies[handle];
        body.set_gravity_scale(1.0, true);
        let speed = from(body.linvel()).clamp_length_max(FASTEST);
        body.set_linvel(to(speed), true);
        self.sim.colliders[collider].set_collision_groups(body_groups());
    }

    /// Simulate, and show each body where it is. `feet` is the player's (render space), `forward` the way he faces (level).
    pub fn update(&mut self, dt: f32, feet: Vec3, forward: Vec3, ground: &impl Ground, renderer: &mut Renderer) {
        self.step(dt, feet, forward, ground);
        for b in &mut self.list {
            let body = &self.sim.bodies[b.handle];
            let now = (from(body.translation()), Quat::from_xyzw(body.rotation().x, body.rotation().y, body.rotation().z, body.rotation().w));
            if b.synced != Some(now) {
                b.synced = Some(now);
                renderer.set_entity(b.entity, now.0, Mat4::from_quat(now.1) * Mat4::from_scale(Vec3::splat(b.scale)), b.ready);
            }
        }
    }

    fn step(&mut self, dt: f32, feet: Vec3, forward: Vec3, ground: &impl Ground) {
        let dt = dt.clamp(0.0, 0.1);
        self.time += dt;
        self.owed += dt;
        for i in 0..self.list.len() {
            if !self.list[i].ready {
                let p = self.pos(i);
                self.list[i].ready = ground.height(p.x, p.z).is_some();
            }
        }
        // The arena is there while a body is near the player.
        let near = (0..self.list.len()).any(|i| self.list[i].ready && flat(self.pos(i), feet) < PATCH);
        let stale = self.arena.as_ref().is_none_or(|a| flat(a.center, feet) > REMAKE || (a.stamp != ground.stamp() && self.time - a.made > LOADING_WAIT));
        if near && stale {
            self.make_arena(feet, ground);
        }
        // The player's cylinder goes where he is (from the last place, in the ticks owed: the speed he walks into things with).
        let middle = feet + Vec3::Y * (PLAYER_LIFT + PLAYER_HALF);
        let ticks = (self.owed / TICK) as u32;
        let from_middle = match self.cylinder.replace(middle) {
            Some(last) if last.distance(middle) < 3.0 => last,
            _ => {
                self.sim.bodies[self.player].set_translation(to(middle), true);
                middle
            }
        };
        if self.arena.is_none() {
            self.owed = 0.0;
        } else {
            for k in 1..=ticks {
                self.sim.bodies[self.player].set_next_kinematic_translation(to(from_middle.lerp(middle, k as f32 / ticks as f32)));
                self.pull_held(feet, forward);
                self.sim.step();
            }
            self.owed -= ticks as f32 * TICK;
        }
        // Whatever rolled off the arena (or was left awake when the player went away) lies where it is.
        if let Some(center) = self.arena.as_ref().map(|a| a.center) {
            for i in 0..self.list.len() {
                if self.held != Some(i) && flat(self.pos(i), center) > ROAM {
                    self.sim.bodies[self.list[i].handle].sleep();
                }
            }
        }
    }

    /// The held body is pulled to the place in front of the player, and let go when it cannot get there (behind a wall).
    fn pull_held(&mut self, feet: Vec3, forward: Vec3) {
        let Some(i) = self.held else { return };
        let target = feet + forward * HOLD_DISTANCE + Vec3::Y * HOLD_HEIGHT;
        let away = target - self.pos(i);
        if away.length() > HOLD_LOSE {
            self.release();
            return;
        }
        let body = &mut self.sim.bodies[self.list[i].handle];
        body.set_linvel(to((away * HOLD_PULL).clamp_length_max(FASTEST)), true);
        body.set_angvel(to(Vec3::ZERO), true);
    }

    /// Make the static colliders around `center`: a patch of the terrain and the collision triangles of the objects.
    fn make_arena(&mut self, center: Vec3, ground: &impl Ground) {
        if let Some(old) = self.arena.take() {
            // (The bodies on it stay asleep where they are.)
            for h in old.colliders {
                self.sim.colliders.remove(h, &mut self.sim.islands, &mut self.sim.bodies, &mut self.sim.soft_bodies, false);
            }
        }
        let mut colliders = Vec::new();
        let n = (2.0 * PATCH / PATCH_STEP) as usize + 1;
        let (x0, z0) = (center.x - PATCH, center.z - PATCH);
        let heights: Vec<Option<f32>> = (0..n * n).map(|k| ground.height(x0 + (k % n) as f32 * PATCH_STEP, z0 + (k / n) as f32 * PATCH_STEP)).collect();
        let vertices: Vec<Vector> = heights.iter().enumerate().map(|(k, h)| Vector::new(x0 + (k % n) as f32 * PATCH_STEP, h.unwrap_or(0.0), z0 + (k / n) as f32 * PATCH_STEP)).collect();
        let mut indices = Vec::new();
        for (z, x) in (0..n - 1).flat_map(|z| (0..n - 1).map(move |x| (z, x))) {
            let k = z * n + x;
            if [k, k + 1, k + n, k + n + 1].iter().all(|&c| heights[c].is_some()) {
                let k = k as u32;
                let n = n as u32;
                indices.push([k, k + n, k + 1]);
                indices.push([k + 1, k + n, k + n + 1]);
            }
        }
        let flags = TriMeshFlags::FIX_INTERNAL_EDGES | TriMeshFlags::DELETE_DEGENERATE_TRIANGLES;
        match ColliderBuilder::trimesh_with_flags(vertices, indices, flags) {
            Ok(terrain) => colliders.push(self.sim.insert_collider(terrain.friction(0.8), None)),
            Err(e) => log::warn!("no terrain for the physics: {e}"),
        }
        let triangles = ground.triangles(center.x, center.z, OBJECTS);
        if !triangles.is_empty() {
            let vertices: Vec<Vector> = triangles.iter().flatten().map(|&p| to(p)).collect();
            let indices: Vec<[u32; 3]> = (0..triangles.len() as u32).map(|t| [3 * t, 3 * t + 1, 3 * t + 2]).collect();
            match ColliderBuilder::trimesh_with_flags(vertices, indices, TriMeshFlags::DELETE_DEGENERATE_TRIANGLES) {
                Ok(objects) => colliders.push(self.sim.insert_collider(objects.friction(0.8), None)),
                Err(e) => log::warn!("no objects for the physics: {e}"),
            }
        }
        self.arena = Some(Arena { center, stamp: ground.stamp(), made: self.time, colliders });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat ground at height 0.
    struct Flat;

    impl Ground for Flat {
        fn height(&self, _: f32, _: f32) -> Option<f32> {
            Some(0.0)
        }

        fn triangles(&self, _: f32, _: f32, _: f32) -> Vec<[Vec3; 3]> {
            Vec::new()
        }

        fn stamp(&self) -> usize {
            0
        }
    }

    fn cuboid(half: Vec3) -> Vec<Vec3> {
        (0..8).map(|k| Vec3::new(if k & 1 == 0 { -half.x } else { half.x }, if k & 2 == 0 { 0.0 } else { 2.0 * half.y }, if k & 4 == 0 { -half.z } else { half.z })).collect()
    }

    fn run(bodies: &mut Bodies, frames: usize, feet: impl Fn(usize) -> Vec3) {
        for k in 0..frames {
            bodies.step(TICK, feet(k), Vec3::X, &Flat);
        }
    }

    const FAR: Vec3 = Vec3::new(50.0, 0.0, 50.0);

    fn up(bodies: &Bodies, i: usize) -> f32 {
        (bodies.sim.bodies[bodies.list[i].handle].rotation().mul_vec3(to(Vec3::Y))).y
    }

    #[test]
    fn a_body_stands_on_the_ground_and_on_another_and_a_walker_shoves_and_topples_them() {
        // Two boxes dropped, one over the other: they land, stack, and fall asleep.
        let mut bodies = Bodies::empty();
        bodies.add(&cuboid(Vec3::new(0.1, 0.1, 0.1)), Vec3::new(0.0, 0.5, 0.0), Quat::IDENTITY, 1.0, 0);
        bodies.add(&cuboid(Vec3::new(0.1, 0.1, 0.1)), Vec3::new(0.0, 1.5, 0.0), Quat::IDENTITY, 1.0, 1);
        // Asleep where they are put: nothing near, nothing happens. The player comes near and they are woken by hand (they fall).
        bodies.step(TICK, Vec3::new(3.0, 0.0, 0.0), Vec3::X, &Flat);
        assert_eq!(bodies.pos(0).y, 0.5);
        bodies.list.iter().for_each(|b| bodies.sim.bodies[b.handle].wake_up(true));
        run(&mut bodies, 300, |_| Vec3::new(3.0, 0.0, 0.0));
        assert!(bodies.pos(0).y.abs() < 0.03 && (bodies.pos(1).y - 0.2).abs() < 0.03, "{} {}", bodies.pos(0).y, bodies.pos(1).y);
        assert!(bodies.sim.bodies[bodies.list[1].handle].is_sleeping());

        // A walker at 3 yards a second runs into a bottle-sized box: it is shoved ahead of him.
        let mut bodies = Bodies::empty();
        bodies.add(&cuboid(Vec3::new(0.1, 0.15, 0.1)), Vec3::ZERO, Quat::IDENTITY, 1.0, 0);
        run(&mut bodies, 180, |k| Vec3::new(-3.0 + 3.0 * k as f32 * TICK, 0.0, 0.0));
        assert!(bodies.pos(0).x > 0.8, "shoved to {}", bodies.pos(0).x);

        // A tall thin one is knocked over by the same walk: it ends up lying down.
        let mut bodies = Bodies::empty();
        bodies.add(&cuboid(Vec3::new(0.06, 0.4, 0.06)), Vec3::ZERO, Quat::IDENTITY, 1.0, 0);
        run(&mut bodies, 300, |k| Vec3::new(-2.0 + 3.0 * (k as f32 * TICK).min(1.0), 0.0, 0.0));
        assert!(up(&bodies, 0).abs() < 0.3, "still upright: {}", up(&bodies, 0));
    }

    #[test]
    fn a_held_body_follows_the_player_and_falls_when_let_go() {
        let mut bodies = Bodies::empty();
        bodies.add(&cuboid(Vec3::new(0.1, 0.1, 0.1)), Vec3::new(10.5, 0.0, 10.0), Quat::IDENTITY, 1.0, 0);
        let feet = Vec3::new(10.0, 0.0, 10.0);
        bodies.step(TICK, feet, Vec3::X, &Flat);
        let i = bodies.near(feet, GRAB_RANGE).expect("near enough to take");
        bodies.grab(i);
        run(&mut bodies, 120, |_| feet);
        let target = feet + Vec3::X * HOLD_DISTANCE + Vec3::Y * HOLD_HEIGHT;
        assert!((bodies.pos(0) - target).length() < 0.1, "held at {}", bodies.pos(0));
        assert_eq!(bodies.near(feet, GRAB_RANGE), None);
        bodies.release();
        run(&mut bodies, 240, |_| feet);
        assert!(bodies.pos(0).y < 0.05 && !bodies.holding());
    }
}
