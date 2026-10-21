//! Ragdolls for the dead: when a creature dies its skeleton is turned into a few rigid bodies (the torso, the head, the limbs...) joined at the bones'
//! pivots, put into the physics world (`physics.rs`) in the pose it died in, and let fall. Every frame the poses of the bodies become the bone matrices the
//! model is skinned with, until all of them have gone to sleep; the creature is drawn from then on as it lies.
//!
//! The parts are picked from the bones that move enough of the mesh (the rest follow the part above them, keeping the pose they had); a part is the convex
//! hull of the vertices its bones move most. Parts do not collide with each other or with the player, only with the ground and the props.
//!
//! shortcut: the joints are balls that bend a fixed amount either way from the pose of the death (no limits taken from the model, which has none), and
//! the parts may pass through each other; upgrade to per-joint limits when a corpse looks wrong.

use crate::anim;
use crate::models::ModelCpu;
use crate::physics::{from, ragdoll_groups, to, Bodies};
use crate::wmovis::MODEL_TO_RENDER;
use glam::{Mat4, Quat, Vec3};
use rapier3d::prelude::*;
use wowfmt::m2::M2;

/// The most parts a corpse is made of, and the share of the weight of all the vertices below which a bone is not worth a part.
const MAX_PARTS: usize = 14;
const MIN_SHARE: f32 = 0.01;
/// How far (radians) a joint bends either way, about each axis, from the pose of the death.
const BEND: f32 = 1.0;

pub struct Ragdoll {
    /// The physics world these bodies are in.
    owner: u64,
    parts: Vec<RigidBodyHandle>,
    /// For each part, the inverse of the place it started in (world space).
    start: Vec<Mat4>,
    /// For each bone, the part that carries it.
    part_of: Vec<usize>,
    /// The bone matrices of the pose of the death (model space).
    death: Vec<Mat4>,
    /// From where the model is drawn (entity space, render axes) to the world.
    placement: Mat4,
    /// The bone matrices last handed out were those of a corpse that has come to rest.
    settled: bool,
    /// How many frames it has been drawn since it fell.
    frames: u32,
    /// Which part is the root, and where it started (world space): where the corpse is follows it.
    root: usize,
    root_start: Vec3,
}

/// A corpse lies still after this many frames whatever a loose limb is doing (a joint that will not quite settle keeps the engine from sleeping it).
const SETTLE_FRAMES: u32 = 480;

/// The nearest bone above `b` (not `b`) that is kept.
fn up(m2: &M2, b: usize, kept: &[bool]) -> Option<usize> {
    let mut at = b;
    for _ in 0..m2.bones.len() {
        let p = m2.bones[at].parent;
        if p < 0 || p as usize >= m2.bones.len() {
            return None;
        }
        at = p as usize;
        if kept[at] {
            return Some(at);
        }
    }
    None
}

impl Ragdoll {
    /// Make the ragdoll of a model that dies in the pose `bones` (model space), drawn through `placement`, and give it a push of `push` yards a second.
    /// `template` holds the vertices (in the order of the model's) and the indices of the mesh that is drawn. None if there is nothing to make it of.
    pub fn spawn(physics: &mut Bodies, m2: &M2, template: &ModelCpu, bones: Vec<Mat4>, placement: Mat4, push: Vec3) -> Option<Ragdoll> {
        let n = m2.bones.len();
        if n == 0 || bones.len() != n {
            return None;
        }
        let mut used = vec![false; template.vertices.len().min(m2.vertices.len())];
        template.indices.iter().for_each(|&i| used.get_mut(i as usize).map(|u| *u = true).unwrap_or_default());
        // How much of the mesh each bone moves.
        let mut weight = vec![0.0f32; n];
        let mut dominant = vec![None; used.len()];
        for (i, v) in m2.vertices.iter().enumerate().take(used.len()).filter(|&(i, _)| used[i]) {
            for k in 0..4 {
                if let Some(w) = weight.get_mut(v.bones[k] as usize) {
                    *w += v.weights[k] as f32 / 255.0;
                }
            }
            dominant[i] = (0..4).filter(|&k| v.weights[k] > 0 && (v.bones[k] as usize) < n).max_by_key(|&k| v.weights[k]).map(|k| v.bones[k] as usize);
        }
        let total: f32 = weight.iter().sum();
        let mut kept: Vec<bool> = weight.iter().map(|&w| total > 0.0 && w >= total * MIN_SHARE).collect();
        // Too many: the lightest end of a limb gives its weight to the bone above, until there are few enough.
        while kept.iter().filter(|&&k| k).count() > MAX_PARTS {
            let leaf = |b: usize| !(0..n).any(|c| kept[c] && up(m2, c, &kept) == Some(b));
            let Some(lightest) = (0..n).filter(|&b| kept[b] && leaf(b)).min_by(|&a, &b| weight[a].total_cmp(&weight[b])) else { break };
            kept[lightest] = false;
            if let Some(above) = up(m2, lightest, &kept) {
                weight[above] += weight[lightest];
            }
        }
        // The root of the ragdoll: of the kept bones with none above them, the one with the most weight in it and under it.
        let top_of = |mut b: usize| {
            while let Some(a) = up(m2, b, &kept) {
                b = a;
            }
            b
        };
        let mut tree = vec![0.0f32; n];
        (0..n).filter(|&b| kept[b]).for_each(|b| tree[top_of(b)] += weight[b]);
        let root_bone = (0..n).filter(|&b| kept[b] && up(m2, b, &kept).is_none()).max_by(|&a, &b| tree[a].total_cmp(&tree[b]))?;
        let bones_of_parts: Vec<usize> = (0..n).filter(|&b| kept[b]).collect();
        let index_of = |bone: usize| bones_of_parts.iter().position(|&b| b == bone);
        let root = index_of(root_bone)?;
        let mut parent: Vec<Option<usize>> = bones_of_parts.iter().map(|&b| if b == root_bone { None } else { Some(up(m2, b, &kept).and_then(index_of).unwrap_or(root)) }).collect();
        let mut part_of: Vec<usize> = (0..n).map(|b| if kept[b] { index_of(b) } else { up(m2, b, &kept).and_then(index_of) }.unwrap_or(root)).collect();
        // The vertices of each part, where they are in the world now.
        let mut posed = Vec::new();
        anim::skin(m2, &bones, &template.vertices, &mut posed);
        let mut points: Vec<Vec<Vec3>> = vec![Vec::new(); bones_of_parts.len()];
        for (i, d) in dominant.iter().enumerate() {
            if let Some(d) = d {
                points[part_of[*d]].push(placement.transform_point3(Vec3::from(posed[i].pos)));
            }
        }
        // A part with nothing to make a body of is carried by the part above.
        let mut alive = vec![true; points.len()];
        for p in 0..points.len() {
            if p != root && points[p].is_empty() {
                alive[p] = false;
                let above = parent[p].unwrap_or(root);
                parent.iter_mut().filter(|q| **q == Some(p)).for_each(|q| *q = Some(above));
                part_of.iter_mut().filter(|q| **q == p).for_each(|q| *q = above);
            }
        }
        if points[root].is_empty() {
            return None;
        }
        // The bodies, at the pivots of their bones.
        let pivot = |p: usize| placement.transform_point3(MODEL_TO_RENDER.transform_point3(bones[bones_of_parts[p]].transform_point3(Vec3::from(m2.bones[bones_of_parts[p]].pivot))));
        let id = physics.new_ragdoll();
        let groups = ragdoll_groups(false);
        let mut handles: Vec<Option<RigidBodyHandle>> = vec![None; points.len()];
        for p in (0..points.len()).filter(|&p| alive[p]) {
            let origin = pivot(p);
            let local: Vec<Vector> = points[p].iter().map(|&q| to(q - origin)).collect();
            let shape = ColliderBuilder::convex_hull(&local).unwrap_or_else(|| {
                let (lo, hi) = points[p].iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &q| (lo.min(q - origin), hi.max(q - origin)));
                let half = ((hi - lo) / 2.0).max(Vec3::splat(0.03));
                ColliderBuilder::cuboid(half.x, half.y, half.z).translation(to((lo + hi) / 2.0))
            });
            let body = RigidBodyBuilder::dynamic().translation(to(origin)).linvel(to(push)).linear_damping(0.5).angular_damping(3.0).user_data(id).additional_solver_iterations(2);
            let (handle, _) = physics.sim.insert(body, shape.density(300.0).friction(0.8).restitution(0.05).collision_groups(groups));
            handles[p] = Some(handle);
        }
        for p in (0..points.len()).filter(|&p| alive[p]) {
            let Some(q) = parent[p] else { continue };
            let (child, above) = (handles[p]?, handles[q]?);
            let mut joint = SphericalJointBuilder::new().local_anchor1(to(pivot(p) - pivot(q))).local_anchor2(to(Vec3::ZERO)).contacts_enabled(false);
            for axis in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ] {
                joint = joint.limits(axis, [-BEND, BEND]);
            }
            physics.sim.insert_impulse_joint(above, child, joint.build());
        }
        let alive_parts: Vec<usize> = (0..points.len()).filter(|&p| alive[p]).collect();
        let slot = |p: usize| alive_parts.iter().position(|&a| a == p).unwrap_or(0);
        let parts = alive_parts.iter().filter_map(|&p| handles[p]).collect();
        let start = alive_parts.iter().map(|&p| Mat4::from_translation(-pivot(p))).collect();
        let part_of = part_of.iter().map(|&p| slot(p)).collect();
        let (root_slot, root_start) = (slot(root), pivot(root));
        Some(Ragdoll { owner: physics.id, parts, start, part_of, death: bones, placement, settled: false, frames: 0, root: root_slot, root_start })
    }

    /// Whether its bodies are in `physics` (the world of the props can be made again, taking them with it).
    pub fn alive(&self, physics: &Bodies) -> bool {
        self.owner == physics.id
    }

    /// Where the model is drawn from (it does not move; the bones do).
    pub fn origin(&self) -> Vec3 {
        self.placement.w_axis.truncate()
    }

    /// Where the corpse is now: the place it died at, moved as far as the root part has moved (the height is that of the root part).
    pub fn corpse_at(&self, physics: &Bodies) -> Option<Vec3> {
        let now = from(physics.sim.bodies.get(*self.parts.get(self.root)?)?.translation());
        let at = self.origin();
        Some(Vec3::new(at.x + now.x - self.root_start.x, now.y, at.z + now.z - self.root_start.z))
    }

    /// Take the bodies out of the world.
    pub fn remove(self, physics: &mut Bodies) {
        if self.alive(physics) {
            self.parts.iter().for_each(|&h| {
                physics.sim.remove_body(h);
            });
        }
    }

    /// The bone matrices (model space) for the bodies as they are now, or none when nothing has changed since it came to rest.
    pub fn bones(&mut self, physics: &mut Bodies) -> Option<Vec<Mat4>> {
        if !self.alive(physics) {
            return None;
        }
        let mut asleep = self.parts.iter().all(|&h| physics.sim.bodies[h].is_sleeping());
        // Disturbed after it lay still (shoved, dragged): the time to settle starts again.
        if self.settled && !asleep {
            self.frames = 0;
        }
        self.frames = self.frames.saturating_add(1);
        if self.frames >= SETTLE_FRAMES && !asleep && !physics.holding() {
            self.parts.iter().for_each(|&h| physics.sim.bodies[h].sleep());
            asleep = true;
        }
        if asleep && self.settled {
            return None;
        }
        self.settled = asleep;
        // What each part has done since the death, in the world; as seen from the model.
        let (into, out_of) = (MODEL_TO_RENDER.transpose() * self.placement.inverse(), self.placement * MODEL_TO_RENDER);
        let moved: Vec<Mat4> = self
            .parts
            .iter()
            .zip(&self.start)
            .map(|(&h, start)| {
                let pose = physics.sim.bodies[h].position();
                let now = Mat4::from_rotation_translation(Quat::from_xyzw(pose.rotation.x, pose.rotation.y, pose.rotation.z, pose.rotation.w), from(pose.translation));
                into * now * *start * out_of
            })
            .collect();
        Some(self.death.iter().enumerate().map(|(b, m)| moved[self.part_of[b]] * *m).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::{Appearance, Character};
    use crate::models::{ModelCache, ModelCpu};
    use crate::physics::Ground;
    use mpq::Mount;

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

    /// A model dies standing on flat ground: it is made of a few parts, falls, does not sink into the ground, and comes to rest.
    fn falls(name: &str, m2: &M2, template: &ModelCpu) {
        let seq = m2.sequences.iter().position(|s| s.id == 0).unwrap_or(0);
        let bones = anim::bone_matrices(m2, seq, m2.sequences[seq].start as f32, 0.0);
        let mut physics = Bodies::empty();
        let feet = Vec3::new(4.0, 0.0, 4.0);
        let mut ragdoll = Ragdoll::spawn(&mut physics, m2, template, bones.clone(), Mat4::IDENTITY, Vec3::new(2.0, 1.5, 0.0)).unwrap_or_else(|| panic!("{name}: a ragdoll"));
        assert!((2..=MAX_PARTS).contains(&ragdoll.parts.len()), "{name}: {} parts", ragdoll.parts.len());
        let mut posed = Vec::new();
        let lowest_and_highest = |bones: &[Mat4], posed: &mut Vec<crate::models::ModelVertex>| {
            anim::skin(m2, bones, &template.vertices, posed);
            posed.iter().map(|v| v.pos[1]).fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)))
        };
        let (_, standing) = lowest_and_highest(&bones, &mut posed);
        // How far the mesh reaches from its middle: a body that holds together keeps about this size.
        let reach = |posed: &[crate::models::ModelVertex]| {
            let middle = posed.iter().map(|v| Vec3::from(v.pos)).sum::<Vec3>() / posed.len() as f32;
            posed.iter().map(|v| (Vec3::from(v.pos) - middle).length()).fold(0.0, f32::max)
        };
        let before = reach(&posed);
        let mut last = bones;
        for _ in 0..SETTLE_FRAMES + 60 {
            physics.step(1.0 / 60.0, feet, Vec3::X, &Flat);
            if let Some(b) = ragdoll.bones(&mut physics) {
                last = b;
            }
        }
        assert!(last.iter().all(|m| m.is_finite()), "{name}: finite bones");
        let (lowest, highest) = lowest_and_highest(&last, &mut posed);
        assert!(lowest > -0.25, "{name}: sunk into the ground by {lowest}");
        assert!(reach(&posed) < before * 1.5, "{name}: torn apart: {} across, was {before}", reach(&posed));
        assert!(highest < standing * 0.8, "{name}: still {highest} high (stood {standing})");
        assert!(ragdoll.parts.iter().all(|&h| physics.sim.bodies[h].is_sleeping()), "{name}: still moving after the time to settle");
        assert!(ragdoll.bones(&mut physics).is_none(), "{name}: nothing more to draw once it rests");
        ragdoll.remove(&mut physics);
        assert_eq!(physics.sim.bodies.len(), 1, "{name}: only the player is left");
    }

    #[test]
    fn beasts_and_people_fall_to_the_ground_in_a_few_parts_and_come_to_rest() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let cache = ModelCache::default();
        for path in [r"Creature\Ogre\Ogre.m2", r"Creature\Murloc\Murloc.m2", r"Creature\Wolf\Wolf.m2"] {
            let Ok(bytes) = mount.read(path) else { continue };
            let m2 = M2::parse(&bytes).unwrap();
            let template = cache.creature(&mount, path, &[]).unwrap();
            falls(path, &m2, &template);
        }
        let character = Character::load(&mount, &cache, "test/ragdoll", &Appearance::default()).unwrap();
        falls("human", &character.m2, &character.model);
    }
}

#[cfg(test)]
mod moving {
    use super::*;
    use crate::models::ModelCache;
    use crate::physics::{Ground, GRAB_RANGE};
    use mpq::Mount;

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

    #[test]
    fn a_corpse_is_shoved_by_a_walker_and_dragged_by_a_held_part() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let path = r"Creature\Ogre\Ogre.m2";
        let m2 = M2::parse(&mount.read(path).unwrap()).unwrap();
        let template = ModelCache::default().creature(&mount, path, &[]).unwrap();
        let seq = m2.sequences.iter().position(|s| s.id == 0).unwrap_or(0);
        let bones = anim::bone_matrices(&m2, seq, m2.sequences[seq].start as f32, 0.0);
        let mut physics = Bodies::empty();
        let mut ragdoll = Ragdoll::spawn(&mut physics, &m2, &template, bones, Mat4::IDENTITY, Vec3::ZERO).unwrap();
        let far = Vec3::new(0.0, 0.0, 6.0);
        for _ in 0..SETTLE_FRAMES + 60 {
            physics.step(1.0 / 60.0, far, Vec3::X, &Flat);
            ragdoll.bones(&mut physics);
        }
        let lies = ragdoll.corpse_at(&physics).unwrap();
        // Someone walks east through it at three yards a second: it is pushed along.
        for k in 0..300 {
            physics.step(1.0 / 60.0, Vec3::new(-4.0 + 3.0 * k as f32 / 60.0, 0.0, lies.z), Vec3::X, &Flat);
        }
        let shoved = ragdoll.corpse_at(&physics).unwrap();
        assert!(shoved.x > lies.x + 0.4, "pushed from {lies} to {shoved}");
        // Held by the part nearest to him it comes along behind him, six yards on.
        let start = Vec3::new(shoved.x + 1.5, 0.0, shoved.z);
        physics.step(1.0 / 60.0, start, Vec3::X, &Flat);
        let part = physics.near_corpse(start, GRAB_RANGE).expect("a part in reach");
        physics.grab(part);
        for k in 0..480 {
            physics.step(1.0 / 60.0, start + Vec3::X * (6.0 * k as f32 / 480.0), Vec3::X, &Flat);
        }
        let dragged = ragdoll.corpse_at(&physics).unwrap();
        assert!(physics.holding() && dragged.x > shoved.x + 3.0, "dragged from {shoved} to {dragged}");
        physics.release();
        assert!(!physics.holding());
    }
}
