//! The lifts, trams, ships and zeppelins of the original game (`content/classic_lifts.ron`, made by `tools/import_lifts.py`): objects that move on a fixed path, round and round,
//! by the time alone. They are drawn apart from the map like the pickups, are solid for the player (`Solid`), and carry the player who stands on them (or jumps on them). A
//! ship's round crosses maps: its entries are each there from their first key to their last, and a rider is taken along into the next map. A building (a ship) is drawn with the
//! doodads of its own, the ones that move (the sails, the candles) as effects. They sound: a loop while they go, a sound as they set off and as they stop.

use crate::collision::{Collider, CollisionMesh, Tri};
use crate::content;
use crate::effects::Effects;
use crate::models::{placement_matrix, wmo_doodad_matrix, BatchCpu, ModelCache, ModelCpu, ModelVertex};
use crate::movement::TriangleSource;
use crate::renderer::{Gpu, Renderer};
use crate::world::World;
use glam::{DVec3, Mat4, Quat, Vec3};
use mpq::Mount;
use serde::Deserialize;
use std::sync::Arc;

/// How far (yards) under the feet a lift's floor still holds the player, and how far above them (a floor that rises meets the feet).
const HOLD: f32 = 0.3;
/// How far above a lift's floor a jumper is still carried by it.
const JUMP: f32 = 8.0;
/// Yards a second under which a lift stands still, for the sounds.
const STILL: f32 = 0.05;

/// One entry of `classic_lifts.ron`: where the lift stands and the offsets (milliseconds, north, west, up) of its path.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    model: String,
    map: String,
    at: (f32, f32, f32),
    heading: f32,
    scale: f32,
    path: Vec<(u32, f32, f32, f32)>,
    /// A ship (or zeppelin) whose round crosses maps: its name, shared by its entries, each of which is there from its first key to its last.
    #[serde(default)]
    route: String,
    /// Milliseconds in a round, where it is not the last key's time.
    #[serde(default)]
    period: u32,
    /// The heading (degrees) at each key, for what turns on its way.
    #[serde(default)]
    turns: Vec<f32>,
    /// `SoundEntries` numbers of a loop heard while it goes and of a sound as it stops, where its model names none of its own (0: none).
    #[serde(default)]
    hum: u32,
    #[serde(default)]
    horn: u32,
}

/// Where something that goes round and round is, by the time.
struct Track {
    /// The path: seconds and offset (render space) of each key.
    path: Vec<(f32, Vec3)>,
    turns: Vec<f32>,
    /// The heading it stands with, which `turns` is measured against.
    rest: f32,
    period: f32,
    /// Only there from the first key to the last.
    windowed: bool,
}

impl Track {
    fn new(s: &Spec) -> Track {
        // WoW (north, west, up) -> render (east, up, south).
        let path: Vec<_> = s.path.iter().map(|&(ms, n, w, up)| (ms as f32 / 1000.0, Vec3::new(-w, up, -n))).collect();
        let period = if s.period > 0 { s.period as f32 / 1000.0 } else { path.last().map_or(0.0, |k| k.0) };
        Track { path, turns: s.turns.clone(), rest: s.heading, period, windowed: !s.route.is_empty() }
    }

    /// The offset and the turn (degrees from its rest heading) `seconds` after the start, going round at the period; `None` while it is not there.
    fn pose(&self, seconds: f32) -> Option<(Vec3, f32)> {
        let (first, last) = (self.path.first()?, self.path.last()?);
        if self.period <= 0.0 {
            return Some((first.1, 0.0));
        }
        let t = seconds.rem_euclid(self.period);
        if self.windowed && (t < first.0 || t > last.0) {
            return None;
        }
        let next = self.path.partition_point(|k| k.0 <= t).min(self.path.len() - 1);
        let prev = next.saturating_sub(1);
        let ((t0, a), (t1, b)) = (self.path[prev], self.path[next]);
        let f = if t1 > t0 { ((t - t0) / (t1 - t0)).clamp(0.0, 1.0) } else { 1.0 };
        let turn = match (self.turns.get(prev), self.turns.get(next)) {
            // By the short way round.
            (Some(&h0), Some(&h1)) => h0 + ((h1 - h0 + 540.0).rem_euclid(360.0) - 180.0) * f - self.rest,
            _ => 0.0,
        };
        Some((a.lerp(b, f), turn))
    }
}

struct Lift {
    /// One entity for each part it is drawn with (the groups of a building).
    entities: Vec<usize>,
    /// Where it stands with no offset (render space) and how it is turned.
    base: Vec3,
    orient: Mat4,
    track: Track,
    /// The ship it is a part of the round of (empty for a lift).
    route: String,
    /// Its solid triangles, turned, around its own origin.
    mesh: CollisionMesh,
    offset: Vec3,
    /// How far it has turned from how it stands, round the vertical (radians, the way the heading grows).
    yaw: f32,
    /// Whether it is there now.
    here: bool,
    /// The effects of the doodads of its own that move, and where each stands on it.
    effects: Vec<(usize, Mat4)>,
    /// `SoundEntries` numbers: the loop while it goes, the sound as it sets off, the sound as it stops (0: none).
    sounds: [Sound; 3],
    /// Whether it went last time.
    moving: bool,
}

impl Lift {
    /// Go where the track has it `clock` seconds after the start.
    fn place(&mut self, clock: f32) {
        self.here = match self.track.pose(clock) {
            Some((offset, turn)) => {
                (self.offset, self.yaw) = (offset, turn.to_radians());
                true
            }
            None => false,
        };
    }

    fn at(&self) -> Vec3 {
        self.base + self.offset
    }

    /// Where its sound `i` is made (render space): where its model has it, turned with the lift.
    fn spot(&self, i: usize) -> Vec3 {
        self.at() + self.turn() * self.orient.transform_point3(self.sounds[i].1)
    }

    fn turn(&self) -> Quat {
        Quat::from_rotation_y(self.yaw)
    }

    /// Whether the feet (render space) stand on its floor.
    fn holds(&self, feet: Vec3) -> bool {
        let rel = self.turn().inverse() * (feet - self.at());
        self.here && self.mesh.floor(rel.x, rel.z, rel.y + HOLD).is_some_and(|y| y >= rel.y - HOLD)
    }

    /// Whether the feet (render space) are over its floor, no more than a jump above it.
    fn over(&self, feet: Vec3) -> bool {
        let rel = self.turn().inverse() * (feet - self.at());
        self.here && self.mesh.floor(rel.x, rel.z, rel.y + HOLD).is_some_and(|y| rel.y - y < JUMP)
    }

    /// The height of its floor over `(x, z)` (render space) no higher than `max_y`.
    fn floor(&self, x: f32, z: f32, max_y: f32) -> Option<f32> {
        let at = self.at();
        let rel = self.turn().inverse() * Vec3::new(x - at.x, 0.0, z - at.z);
        self.here.then(|| self.mesh.floor(rel.x, rel.z, max_y - at.y)).flatten().map(|y| y + at.y)
    }
}

/// A part of a ship's round that is in another map.
struct Away {
    map: String,
    route: String,
    track: Track,
}

/// What the lift under the player does to him in a moment.
#[derive(Default)]
pub struct Ride {
    /// How far he is carried (render space).
    pub by: Vec3,
    /// How far his lift turned (radians, the way the heading grows): what he sees turns the other way.
    pub turn: f32,
    /// His ship has sailed into another map.
    pub cross: Option<Cross>,
    /// Sounds to make: the entry and where.
    pub bangs: Vec<(u32, Vec3)>,
}

/// A rider whose ship has left this map.
pub struct Cross {
    /// The map the ship is in now.
    pub map: String,
    route: String,
    /// Where he stood on the ship (turned back to how it stands) and how the ship was turned.
    local: Vec3,
    yaw: f32,
}

pub struct Lifts {
    list: Vec<Lift>,
    /// The parts of ships' rounds in other maps.
    away: Vec<Away>,
    /// Seconds since the game began: the lifts go by this alone.
    clock: f32,
    /// The lift (index) the player stood on last, which a jump does not take him from.
    riding: Option<usize>,
}

/// The meshes a model is drawn with: the model's own, or the groups of a building.
fn parts(cache: &ModelCache, mount: &Mount, path: &str) -> Vec<Arc<ModelCpu>> {
    if path.to_ascii_lowercase().ends_with(".wmo") {
        cache.get_wmo(mount, path).groups.iter().filter_map(|g| g.model.clone()).collect()
    } else {
        cache.get(mount, path).into_iter().collect()
    }
}

/// A sound (`SoundEntries` number, 0 for none) and where it is made, in the model's own space (render space, as the model's mesh).
type Sound = (u32, Vec3);

/// The sounds a model makes, where the model has them (its `$DSL` event: a loop while it goes; its first and second `$DSO`: as it sets off and as it stops), each at the
/// place the model gives it: the pivot of its bone and its own offset.
fn ride_sounds(mount: &Mount, path: &str) -> [Sound; 3] {
    let Some(m) = mount.read(path).ok().and_then(|b| wowfmt::m2::M2::parse(&b).ok()) else { return [(0, Vec3::ZERO); 3] };
    let mut out = [(0, Vec3::ZERO); 3];
    let mut once = 1;
    for e in &m.events {
        let pivot = m.bones.get(e.bone as usize).map_or([0.0; 3], |b| b.pivot);
        let p = [pivot[0] + e.position[0], pivot[1] + e.position[1], pivot[2] + e.position[2]];
        let sound = (e.data, Vec3::new(p[0], p[2], -p[1]));
        match &e.id {
            b"$DSL" => out[0] = sound,
            b"$DSO" if once < 3 => {
                out[once] = sound;
                once += 1;
            }
            _ => {}
        }
    }
    out
}

/// Still models placed on a building as models of their own, merged (a ship has over a hundred), each with room for 16-bit indices.
fn merge(key: &str, placed: &[(Arc<ModelCpu>, Mat4)]) -> Vec<ModelCpu> {
    let mut out: Vec<ModelCpu> = Vec::new();
    for (m, place) in placed {
        if out.last().is_none_or(|o| o.vertices.len() + m.vertices.len() > 65_000) {
            out.push(ModelCpu { key: format!("{key}#doodads{}", out.len()), vertices: vec![], indices: vec![], batches: vec![], col_vertices: vec![], col_indices: vec![], tracks: None });
        }
        let o = out.last_mut().unwrap();
        let (base, first) = (o.vertices.len() as u16, o.indices.len() as u32);
        o.vertices.extend(m.vertices.iter().map(|v| ModelVertex { pos: place.transform_point3(Vec3::from(v.pos)).to_array(), normal: place.transform_vector3(Vec3::from(v.normal)).normalize_or_zero().to_array(), ..*v }));
        o.indices.extend(m.indices.iter().map(|&i| i + base));
        o.batches.extend(m.batches.iter().map(|b| BatchCpu { index_start: b.index_start + first, ..b.clone() }));
    }
    out
}

impl Lifts {
    /// The lifts of `map`, `clock` seconds after the game began.
    #[allow(clippy::too_many_arguments)]
    pub fn load(mount: &Mount, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu, effects: &mut Effects, map: &str, clock: f32) -> anyhow::Result<Lifts> {
        let text = content::source("classic_lifts.ron", include_str!("../../../content/classic_lifts.ron"));
        let specs: Vec<Spec> = ron::from_str(&text)?;
        let (mut list, mut away) = (Vec::new(), Vec::new());
        for s in specs.into_iter().filter(|s| !s.path.is_empty()) {
            if !s.map.eq_ignore_ascii_case(map) {
                if !s.route.is_empty() {
                    away.push(Away { map: s.map.clone(), route: s.route.clone(), track: Track::new(&s) });
                }
                continue;
            }
            let models = parts(cache, mount, &s.model);
            if models.is_empty() {
                log::warn!("lift model {} does not load", s.model);
                continue;
            }
            let orient = placement_matrix(Vec3::ZERO, [0.0, s.heading, 0.0], s.scale);
            let mut tris = Vec::new();
            for model in &models {
                let point = |i: u32| orient.transform_point3(Vec3::from(model.col_vertices[i as usize]));
                tris.extend(model.col_indices.chunks_exact(3).filter_map(|t| Tri::new(point(t[0]), point(t[1]), point(t[2]))));
            }
            let mut entities: Vec<usize> = models.iter().map(|m| renderer.add_entity(gpu, m)).collect();
            let mut sounds = ride_sounds(mount, &s.model);
            // The doodads of a building: the still ones are drawn together, the ones that move are effects.
            let (mut still, mut living) = (Vec::new(), Vec::new());
            if s.model.to_ascii_lowercase().ends_with(".wmo") {
                for (_, d) in cache.get_wmo(mount, &s.model).active_doodads(0) {
                    let place = wmo_doodad_matrix(d);
                    match cache.living(mount, &d.name) {
                        Some(l) => {
                            // The loop of a doodad (the engine of a zeppelin) is heard where the doodad has it on the building.
                            let (entry, at) = ride_sounds(mount, &l.file)[0];
                            if sounds[0].0 == 0 && entry != 0 {
                                sounds[0] = (entry, place.transform_point3(at));
                            }
                            living.push((l, place));
                        }
                        None => still.extend(cache.get(mount, &d.name).map(|m| (m, place))),
                    }
                }
            }
            entities.extend(merge(&s.model, &still).iter().map(|m| renderer.add_entity(gpu, m)));
            let mut spawned = Vec::new();
            for (l, place) in living {
                let id = if l.mesh { effects.spawn_placed(renderer, gpu, &l.file, None, Mat4::IDENTITY) } else { effects.spawn_emitters(renderer, gpu, &l.file, Mat4::IDENTITY) };
                if let Some(id) = id {
                    effects.set_hidden(id, true);
                    spawned.push((id, place));
                }
            }
            // What the model has no sounds for (a ship) is heard from the middle of it.
            if sounds[0].0 == 0 && s.hum != 0 {
                sounds[0] = (s.hum, Vec3::ZERO);
            }
            if sounds[2].0 == 0 && s.horn != 0 {
                sounds[2] = (s.horn, Vec3::ZERO);
            }
            let mut lift = Lift {
                entities,
                base: crate::coords::wow_to_render(s.at.into()),
                orient,
                track: Track::new(&s),
                route: s.route,
                mesh: CollisionMesh::build(tris),
                offset: Vec3::ZERO,
                yaw: 0.0,
                here: false,
                effects: spawned,
                sounds,
                moving: false,
            };
            lift.place(clock);
            list.push(lift);
        }
        Ok(Lifts { list, away, clock, riding: None })
    }

    /// Take them out of the world, giving their entities back. Returns the clock, for the lifts of the next map to go on from.
    pub fn retire(self, renderer: &mut Renderer, effects: &mut Effects) -> f32 {
        self.list.iter().flat_map(|l| &l.entities).for_each(|&e| renderer.remove_entity(e));
        self.list.iter().flat_map(|l| &l.effects).for_each(|&(id, _)| effects.stop(id));
        self.clock
    }

    /// The sounds that loop where the lifts are, as `Audio::set_places` wants them: those of the lifts that go.
    pub fn hums(&self) -> Vec<(u64, u32, Vec3)> {
        let going = self.list.iter().enumerate().filter(|(_, l)| l.here && l.moving && l.sounds[0].0 != 0);
        going.map(|(i, l)| ((3u64 << 60) | i as u64, l.sounds[0].0, l.spot(0))).collect()
    }

    /// Let `dt` seconds go by and show each lift where it is now (hidden where `loaded` says the ground is not there yet). Returns what the lift that the feet (render
    /// space) stood on does to the player, who is to be carried along.
    pub fn advance(&mut self, dt: f32, feet: Vec3, airborne: bool, loaded: impl Fn(f32, f32) -> bool, renderer: &mut Renderer, effects: &mut Effects) -> Ride {
        let ride = self.follow(dt, feet, airborne);
        for l in &self.list {
            let at = l.at();
            let orient = Mat4::from_quat(l.turn()) * l.orient;
            let show = l.here && loaded(at.x, at.z);
            l.entities.iter().for_each(|&e| renderer.set_entity(e, at, orient, show));
            for &(id, place) in &l.effects {
                effects.set_world(id, Mat4::from_translation(at) * orient * place);
                effects.set_hidden(id, !show);
            }
        }
        ride
    }

    /// Let `dt` seconds go by; what the lift under the feet (or under a jump from it) does to the one who is on it.
    fn follow(&mut self, dt: f32, feet: Vec3, airborne: bool) -> Ride {
        let on = self.list.iter().position(|l| l.holds(feet)).or_else(|| self.riding.filter(|&i| airborne && self.list[i].over(feet)));
        self.riding = on;
        let was = on.map(|i| (self.list[i].at(), self.list[i].yaw));
        self.clock += dt;
        let mut bangs = Vec::new();
        for l in &mut self.list {
            let (was_here, was_at) = (l.here, l.at());
            l.place(self.clock);
            let moving = l.here && was_here && dt > 0.0 && (l.at() - was_at).length() / dt > STILL;
            // It sets off or stops: a sound, where it is (not as it comes into this map or leaves it).
            if moving != l.moving && l.here {
                let which = if moving { 1 } else { 2 };
                bangs.extend(Some(l.sounds[which].0).filter(|&e| e != 0).map(|e| (e, l.spot(which))));
            }
            l.moving = moving;
        }
        let (Some(i), Some((was_at, was_yaw))) = (on, was) else { return Ride { bangs, ..Default::default() } };
        let l = &self.list[i];
        let local = Quat::from_rotation_y(-was_yaw) * (feet - was_at);
        // He goes on with the lift, or with the next part of its round (a ship that sails on, into this map or another).
        let next = if l.here { Some(l) } else { self.list.iter().find(|o| o.here && !l.route.is_empty() && o.route == l.route) };
        match next {
            Some(n) => Ride { by: n.at() + Quat::from_rotation_y(n.yaw) * local - feet, turn: n.yaw - was_yaw, cross: None, bangs },
            None => {
                let there = self.away.iter().find(|a| !l.route.is_empty() && a.route == l.route && a.track.pose(self.clock).is_some());
                Ride { cross: there.map(|a| Cross { map: a.map.clone(), route: a.route.clone(), local, yaw: was_yaw }), bangs, ..Default::default() }
            }
        }
    }

    /// Where a rider that crossed lands (feet, render space) on the ship of his route as it is now, and where he looks if he looked to `camera_yaw`.
    pub fn arrive(&self, c: &Cross, camera_yaw: f32) -> Option<(Vec3, f32)> {
        let l = self.list.iter().find(|l| l.here && l.route == c.route)?;
        Some((l.at() + l.turn() * c.local, camera_yaw - (l.yaw - c.yaw)))
    }

    /// The height of the highest floor of a lift over `(x, z)` (render space) no higher than `max_y`.
    fn floor(&self, x: f32, z: f32, max_y: f32) -> Option<f32> {
        self.list.iter().filter_map(|l| l.floor(x, z, max_y)).reduce(f32::max)
    }

    /// The triangles of the lifts overlapping the box (see `TriangleSource::gather`).
    fn gather(&self, origin: [f64; 3], min: Vec3, max: Vec3, out: &mut Vec<crate::movement::Tri>) {
        // WoW (x, y, z) -> render (X = -y, Y = z, Z = -x).
        let lo = DVec3::new(-(origin[1] + max.y as f64), origin[2] + min.z as f64, -(origin[0] + max.x as f64));
        let hi = DVec3::new(-(origin[1] + min.y as f64), origin[2] + max.z as f64, -(origin[0] + min.x as f64));
        for l in self.list.iter().filter(|l| l.here) {
            let at = l.at().as_dvec3();
            let (q, back) = (l.turn(), l.turn().inverse());
            // The box in the lift's own frame: the one that holds the turned box.
            let (a, b) = ((lo - at).as_vec3(), (hi - at).as_vec3());
            let corners = (0..8).map(|i| back * Vec3::new(if i & 1 == 0 { a.x } else { b.x }, if i & 2 == 0 { a.y } else { b.y }, if i & 4 == 0 { a.z } else { b.z }));
            let (from, to) = corners.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(m, n), c| (m.min(c), n.max(c)));
            l.mesh.for_each_in_box(from, to, |t| {
                let w = [t.a, t.b, t.c].map(|v| {
                    let p = at + (q * v).as_dvec3();
                    Vec3::new((-p.z - origin[0]) as f32, (-p.x - origin[1]) as f32, (p.y - origin[2]) as f32)
                });
                out.extend(crate::movement::Tri::new(w[0], w[1], w[2]));
            });
        }
    }
}

/// The world with its lifts, for what the player walks on.
pub struct Solid<'a>(pub &'a World, pub Option<&'a Lifts>);

impl TriangleSource for Solid<'_> {
    fn gather(&self, origin: [f64; 3], min: Vec3, max: Vec3, out: &mut Vec<crate::movement::Tri>) {
        self.0.gather(origin, min, max, out);
        if let Some(lifts) = self.1 {
            lifts.gather(origin, min, max, out);
        }
    }

    fn is_loaded(&self, origin: [f64; 3]) -> bool {
        self.0.is_loaded(origin)
    }
}

impl Collider for Solid<'_> {
    fn floor(&self, x: f32, z: f32, max_y: f32) -> Option<f32> {
        [Collider::floor(self.0, x, z, max_y), self.1.and_then(|l| l.floor(x, z, max_y))].into_iter().flatten().reduce(f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(path: Vec<(f32, Vec3)>, turns: Vec<f32>, period: f32, windowed: bool) -> Track {
        Track { path, turns, rest: 0.0, period, windowed }
    }

    /// A lift with a 4 x 4 floor at its origin, going up 10 yards in 2 seconds and down again by 4.
    fn lift() -> Lift {
        let (a, b, c, d) = (Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, -2.0), Vec3::new(2.0, 0.0, 2.0), Vec3::new(-2.0, 0.0, 2.0));
        let tris = [Tri::new(a, b, c), Tri::new(a, c, d)].into_iter().flatten().collect();
        let path = vec![(0.0, Vec3::ZERO), (2.0, Vec3::new(0.0, 10.0, 0.0)), (4.0, Vec3::ZERO)];
        Lift {
            entities: vec![],
            base: Vec3::new(100.0, 5.0, 100.0),
            orient: Mat4::IDENTITY,
            track: track(path, vec![], 4.0, false),
            route: String::new(),
            mesh: CollisionMesh::build(tris),
            offset: Vec3::ZERO,
            yaw: 0.0,
            here: true,
            effects: vec![],
            sounds: [(0, Vec3::ZERO), (11, Vec3::new(1.0, 2.0, 0.0)), (12, Vec3::ZERO)],
            moving: false,
        }
    }

    fn offset(l: &Lift, seconds: f32) -> Vec3 {
        l.track.pose(seconds).unwrap().0
    }

    #[test]
    fn the_lift_goes_round_its_path_and_holds_only_what_stands_on_its_floor() {
        let mut l = lift();
        assert_eq!(offset(&l, 1.0), Vec3::new(0.0, 5.0, 0.0));
        assert_eq!(offset(&l, 3.0), Vec3::new(0.0, 5.0, 0.0));
        assert_eq!(offset(&l, 4.0 + 1.0), Vec3::new(0.0, 5.0, 0.0));
        assert!(l.holds(Vec3::new(101.0, 5.0, 99.0)));
        assert!(!l.holds(Vec3::new(105.0, 5.0, 100.0)), "beside it");
        assert!(!l.holds(Vec3::new(100.0, 8.0, 100.0)), "above it");
        l.offset = Vec3::new(0.0, 10.0, 0.0);
        assert!(!l.holds(Vec3::new(100.0, 5.0, 100.0)), "it went up");
        assert!(l.holds(Vec3::new(100.0, 15.0, 100.0)));
    }

    #[test]
    fn its_floor_is_a_triangle_of_the_world_where_it_is_now() {
        let mut lifts = Lifts { list: vec![lift()], away: vec![], clock: 0.0, riding: None };
        lifts.list[0].offset = Vec3::new(0.0, 10.0, 0.0);
        let mut out = Vec::new();
        // The floor is at render (100, 15, 100) = WoW (north -100, west -100, up 15); look around that, from an origin of WoW (-100, -100, 0).
        lifts.gather([-100.0, -100.0, 0.0], Vec3::splat(-1.0) + Vec3::Z * 15.0, Vec3::splat(1.0) + Vec3::Z * 15.0, &mut out);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|t| t.v.iter().all(|v| (v.z - 15.0).abs() < 1e-4)));
        assert_eq!(lifts.floor(100.0, 100.0, 20.0), Some(15.0));
        assert_eq!(lifts.floor(100.0, 100.0, 10.0), None);
    }

    #[test]
    fn a_part_of_a_round_is_there_only_from_its_first_key_to_its_last_and_turns_the_short_way() {
        let t = track(vec![(10.0, Vec3::ZERO), (20.0, Vec3::new(10.0, 0.0, 0.0))], vec![350.0, 10.0], 30.0, true);
        assert!(t.pose(5.0).is_none() && t.pose(25.0).is_none());
        assert!(t.pose(10.0).is_some() && t.pose(20.0).is_some());
        assert_eq!(t.pose(35.0 + 10.0).unwrap().0, Vec3::new(5.0, 0.0, 0.0), "round again");
        // Halfway from 350 to 10 degrees is 360, not 180.
        assert!((t.pose(15.0).unwrap().1 - 360.0).abs() < 1e-3);
    }

    #[test]
    fn a_rider_turns_with_the_ship_and_goes_on_with_its_next_part() {
        // A deck 4 x 4 at the origin that turns a quarter (90 degrees) in 10 seconds, and then is gone; the next part (same route) is the same deck, elsewhere.
        let mut ship = lift();
        ship.route = "ship".into();
        ship.base = Vec3::ZERO;
        ship.track = track(vec![(0.0, Vec3::ZERO), (10.0, Vec3::ZERO)], vec![0.0, 90.0], 20.0, true);
        let mut next = lift();
        next.route = "ship".into();
        next.base = Vec3::new(1000.0, 0.0, 0.0);
        next.track = track(vec![(10.0, Vec3::ZERO), (20.0, Vec3::ZERO)], vec![90.0, 90.0], 20.0, true);
        let mut lifts = Lifts { list: vec![ship, next], away: vec![], clock: 0.0, riding: None };
        lifts.list[0].place(0.0);
        lifts.list[1].place(0.0);
        let feet = Vec3::new(1.0, 0.0, 0.0);
        // A fifth of the way, the rider has gone round with the deck.
        let ride = lifts.follow(5.0, feet, false);
        assert!((ride.turn - 45f32.to_radians()).abs() < 1e-4);
        let rel = feet + ride.by;
        assert!((rel.length() - 1.0).abs() < 1e-4, "same distance from the middle: {rel}");
        assert!((rel - Quat::from_rotation_y(45f32.to_radians()) * feet).length() < 1e-4);
        // When the part is over, he is on the next one at the same place on the deck.
        let ride = lifts.follow(5.5, rel, false);
        let end = rel + ride.by;
        assert!((end - (Vec3::new(1000.0, 0.0, 0.0) + Quat::from_rotation_y(90f32.to_radians()) * feet)).length() < 1e-3, "{end}");
    }

    #[test]
    fn a_jump_from_a_lift_goes_on_with_it_until_he_is_off_its_floor() {
        let mut lifts = Lifts { list: vec![lift()], away: vec![], clock: 0.0, riding: None };
        // Standing on it as it sets off up, with a sound; then in the air a yard above the floor it still takes him along.
        let stand = Vec3::new(100.0, 5.0, 100.0);
        let ride = lifts.follow(0.5, stand, false);
        assert!((ride.by.y - 2.5).abs() < 1e-4);
        assert_eq!(ride.bangs.iter().map(|b| b.0).collect::<Vec<_>>(), vec![11]);
        // It is made where the model has it (a yard east and 2 yards up from the middle of the lift, which has gone 2.5 up by now).
        assert!((ride.bangs[0].1 - (Vec3::new(101.0, 5.0 + 2.5 + 2.0, 100.0))).length() < 1e-4, "{:?}", ride.bangs[0].1);
        let ride = lifts.follow(0.5, stand + Vec3::new(0.0, 2.5 + 1.0, 0.0), true);
        assert!((ride.by.y - 2.5).abs() < 1e-4, "{:?}", ride.by);
        // Over the side, or far above, or not jumping: not taken.
        assert_eq!(lifts.follow(0.5, Vec3::new(110.0, 11.0, 100.0), true).by, Vec3::ZERO);
        lifts.riding = Some(0);
        assert_eq!(lifts.follow(0.1, Vec3::new(100.0, 40.0, 100.0), true).by, Vec3::ZERO);
        lifts.riding = Some(0);
        assert_eq!(lifts.follow(0.1, Vec3::new(100.0, 11.0, 100.0), false).by, Vec3::ZERO);
    }

    #[test]
    fn the_cars_of_the_game_have_a_floor_to_stand_on() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return };
        let specs: Vec<Spec> = ron::from_str(include_str!("../../../content/classic_lifts.ron")).unwrap();
        let cache = ModelCache::default();
        for s in specs.iter().filter(|s| !s.model.contains("Door")) {
            let models = parts(&cache, &mount, &s.model);
            assert!(!models.is_empty(), "{} does not load", s.model);
            assert!(models.iter().any(|m| !m.col_indices.is_empty()), "{} has no solid parts", s.model);
            // The cars that carry people make sounds (the plunger of the Gnomeregan lift, a counterweight, does not).
            if ["ElevatorCar.m2", "UndeadElevator.m2", "SubwayCar.m2", "GnomeElevatorCar01.m2"].iter().any(|n| s.model.ends_with(n)) {
                assert!(ride_sounds(&mount, &s.model).iter().all(|&(e, _)| e != 0), "{} makes no sounds", s.model);
            }
        }
    }

    #[test]
    fn the_parts_of_a_ships_round_follow_one_another_all_the_way_round() {
        let specs: Vec<Spec> = ron::from_str(include_str!("../../../content/classic_lifts.ron")).unwrap();
        let mut routes: std::collections::BTreeMap<&str, Vec<&Spec>> = Default::default();
        for s in specs.iter().filter(|s| !s.route.is_empty()) {
            routes.entry(&s.route).or_default().push(s);
        }
        assert!(routes.len() >= 9, "the nine ships and zeppelins");
        for (name, mut parts) in routes {
            parts.sort_by_key(|s| s.path[0].0);
            assert_eq!(parts[0].path[0].0, 0, "{name} begins at the start");
            for pair in parts.windows(2) {
                let (end, start) = (pair[0].path.last().unwrap().0 as i64, pair[1].path[0].0 as i64);
                assert!((end - start).abs() <= 2, "{name}: a part ends at {end} ms and the next begins at {start}");
            }
            let end = parts.last().unwrap().path.last().unwrap().0 as i64;
            assert!((end - parts[0].period as i64).abs() <= 2, "{name}: ends at {end} ms of {}", parts[0].period);
            assert!(parts.iter().all(|s| s.turns.len() == s.path.len()), "{name}: a heading for every key");
        }
    }
}
