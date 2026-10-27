//! Editing props in the world with the pointer: picking one by the box of its model, the gizmo drawn over the selected one (arrows to
//! move along an axis, a spot to move over the ground, a ring to turn it, a knob to scale it) and what dragging it does, and the
//! ghost models that show a prop while it is put down or dragged (the world makes its tile again only when the drag is over).

use crate::anim;
use crate::camera::Camera;
use crate::character::Appearance;
use crate::creature::Person;
use crate::content::Prop;
use crate::coords::{render_to_wow, wow_to_render};
use crate::models::{placement_matrix, ModelCache, ModelCpu, ModelVertex};
use crate::renderer::{Gpu, Renderer};
use crate::ui::{Quad, Ui, FULL_UV};
use glam::{Mat4, Vec2, Vec3, Vec4};
use mpq::Mount;
use wowfmt::m2::M2;
use std::collections::HashMap;
use std::sync::Arc;

/// Where the camera is and how it sees (pixels), to put points of the world on the screen and the pointer into the world.
#[derive(Clone, Copy)]
pub struct Eye {
    view_proj: Mat4,
    inverse: Mat4,
    pos: Vec3,
    size: Vec2,
    fov_y: f32,
}

impl Eye {
    /// Where the eye is (render space).
    pub fn position(&self) -> Vec3 {
        self.pos
    }

    pub fn of(camera: &Camera, size: (u32, u32)) -> Eye {
        let view_proj = camera.view_proj_relative();
        Eye { view_proj, inverse: view_proj.inverse(), pos: camera.pos, size: Vec2::new(size.0 as f32, size.1 as f32), fov_y: camera.fov_y }
    }

    /// The pixel a point is seen at (not for what is behind the camera).
    pub fn project(&self, p: Vec3) -> Option<Vec2> {
        let c = self.view_proj * (p - self.pos).extend(1.0);
        (c.w > 0.01).then(|| Vec2::new((c.x / c.w * 0.5 + 0.5) * self.size.x, (0.5 - c.y / c.w * 0.5) * self.size.y))
    }

    /// The ray through a pixel: where it starts and its direction (unit).
    pub fn ray(&self, pixel: Vec2) -> (Vec3, Vec3) {
        let far = self.inverse * Vec4::new(pixel.x / self.size.x * 2.0 - 1.0, 1.0 - pixel.y / self.size.y * 2.0, 1.0, 1.0);
        (self.pos, (far.truncate() / far.w).normalize())
    }

    /// How long, in the world, something at `at` is that appears `pixels` long.
    fn world_length(&self, at: Vec3, pixels: f32) -> f32 {
        (at - self.pos).length() * 2.0 * (self.fov_y / 2.0).tan() * pixels / self.size.y
    }
}

/// Where a prop stands (render space) and how its model is turned and scaled.
pub fn position(p: &Prop) -> Vec3 {
    wow_to_render([p.at.0, p.at.1, p.at.2])
}

pub fn orient(p: &Prop) -> Mat4 {
    placement_matrix(Vec3::ZERO, p.rotation, p.scale)
}

/// How far along the ray the box `low..high` of a prop's model is hit, if it is.
fn hit_box(origin: Vec3, dir: Vec3, prop: &Prop, (low, high): (Vec3, Vec3)) -> Option<f32> {
    let inverse = orient(prop).inverse();
    let (o, d) = (inverse.transform_point3(origin - position(prop)), inverse.transform_vector3(dir));
    let (mut near, mut far) = (0.0f32, f32::MAX);
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            if o[i] < low[i] || o[i] > high[i] {
                return None;
            }
            continue;
        }
        let (a, b) = ((low[i] - o[i]) / d[i], (high[i] - o[i]) / d[i]);
        (near, far) = (near.max(a.min(b)), far.min(a.max(b)));
    }
    (near <= far).then_some(near)
}

/// The prop whose model the ray hits first, and how far along the ray. `bounds` gives the box of a model file.
pub fn pick_hit(props: &[Prop], origin: Vec3, dir: Vec3, mut bounds: impl FnMut(&str) -> (Vec3, Vec3)) -> Option<(usize, f32)> {
    props
        .iter()
        .enumerate()
        .filter_map(|(i, p)| hit_box(origin, dir, p, bounds(&p.model)).map(|t| (i, t)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

#[cfg(test)]
pub fn pick(props: &[Prop], origin: Vec3, dir: Vec3, bounds: impl FnMut(&str) -> (Vec3, Vec3)) -> Option<usize> {
    pick_hit(props, origin, dir, bounds).map(|(i, _)| i)
}

/// How far along the ray the box `low..high` (render space, not turned) is hit, if it is: for a creature or a chest, which have no model
/// box of their own here but a size.
pub fn hit_standing(origin: Vec3, dir: Vec3, low: Vec3, high: Vec3) -> Option<f32> {
    let (mut near, mut far) = (0.0f32, f32::MAX);
    for i in 0..3 {
        if dir[i].abs() < 1e-9 {
            if origin[i] < low[i] || origin[i] > high[i] {
                return None;
            }
            continue;
        }
        let (a, b) = ((low[i] - origin[i]) / dir[i], (high[i] - origin[i]) / dir[i]);
        (near, far) = (near.max(a.min(b)), far.min(a.max(b)));
    }
    (near <= far).then_some(near)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Move,
    Turn,
    Scale,
}

/// A part of the gizmo that can be dragged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    /// An arrow: east, up, south.
    Axis(usize),
    /// The spot in the middle: over the ground.
    Floor,
    Ring,
    Knob,
}

const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
const COLORS: [[f32; 4]; 3] = [[1.0, 0.3, 0.3, 1.0], [0.4, 1.0, 0.4, 1.0], [0.4, 0.6, 1.0, 1.0]];
const HOT: [f32; 4] = [1.0, 0.9, 0.2, 1.0];
/// How long the arrows look, and how near the pointer must be to a part to take it, in pixels.
const LENGTH: f32 = 110.0;
const REACH: f32 = 9.0;

enum Shape {
    Line(Vec<Vec2>),
    Spot(Vec2, f32),
}

/// The gizmo of a prop: its parts as they are seen.
pub struct Gizmo {
    parts: Vec<(Handle, [f32; 4], Shape)>,
}

impl Gizmo {
    /// `vertical`: the up arrow too (not for a creature or a chest, which stand on the ground).
    pub fn new(eye: &Eye, at: Vec3, mode: Mode, vertical: bool) -> Gizmo {
        let len = eye.world_length(at, LENGTH);
        let mut parts = Vec::new();
        let line = |handle, color, points: Vec<Vec3>, parts: &mut Vec<(Handle, [f32; 4], Shape)>| {
            let seen: Vec<Vec2> = points.iter().filter_map(|p| eye.project(*p)).collect();
            if seen.len() == points.len() {
                parts.push((handle, color, Shape::Line(seen)));
            }
        };
        match mode {
            Mode::Move => {
                for (i, axis) in AXES.iter().enumerate().filter(|(i, _)| vertical || *i != 1) {
                    line(Handle::Axis(i), COLORS[i], vec![at, at + *axis * len], &mut parts);
                    if let Some(tip) = eye.project(at + *axis * len) {
                        parts.push((Handle::Axis(i), COLORS[i], Shape::Spot(tip, 5.0)));
                    }
                }
                if let Some(middle) = eye.project(at) {
                    parts.insert(0, (Handle::Floor, [1.0, 1.0, 1.0, 1.0], Shape::Spot(middle, REACH)));
                }
            }
            Mode::Turn => {
                let ring = (0..=48).map(|i| i as f32 / 48.0 * std::f32::consts::TAU).map(|a| at + Vec3::new(a.cos(), 0.0, a.sin()) * len * 0.8).collect();
                line(Handle::Ring, COLORS[1], ring, &mut parts);
            }
            Mode::Scale => {
                let top = at + Vec3::Y * len;
                line(Handle::Knob, COLORS[1], vec![at, top], &mut parts);
                if let Some(knob) = eye.project(top) {
                    parts.push((Handle::Knob, COLORS[1], Shape::Spot(knob, 8.0)));
                }
            }
        }
        Gizmo { parts }
    }

    /// The part under the pointer (pixels), if one is.
    pub fn grab(&self, mouse: Vec2) -> Option<Handle> {
        self.parts
            .iter()
            .find(|(_, _, shape)| match shape {
                Shape::Spot(at, radius) => at.distance(mouse) <= radius + 2.0,
                Shape::Line(points) => points.windows(2).any(|w| distance_to_segment(mouse, w[0], w[1]) <= REACH),
            })
            .map(|(h, _, _)| *h)
    }

    /// Draw it with dots in pixels, the part `hot` in yellow.
    pub fn draw(&self, ui: &mut Ui, white: usize, hot: Option<Handle>) {
        let mut dot = |at: Vec2, half: f32, color: [f32; 4]| ui.quad(Quad { texture: white, rect: [at.x - half, at.y - half, 2.0 * half, 2.0 * half], uv: FULL_UV, color, additive: false });
        for (handle, color, shape) in &self.parts {
            let color = if hot == Some(*handle) { HOT } else { *color };
            match shape {
                Shape::Spot(at, radius) => dot(*at, radius.min(8.0) * 0.6 + 1.0, color),
                Shape::Line(points) => {
                    for w in points.windows(2) {
                        let steps = (w[0].distance(w[1]) / 4.0).ceil().max(1.0) as usize;
                        for s in 0..steps {
                            dot(w[0].lerp(w[1], s as f32 / steps as f32), 1.5, color);
                        }
                    }
                }
            }
        }
    }
}

fn distance_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() < 1e-6 { 0.0 } else { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
    p.distance(a + ab * t)
}

/// Draw a mark on the ground at `at` with an arrow that points the way `facing` (degrees, 0 north, clockwise) says: where a creature or a
/// chest that is put down will stand.
pub fn draw_marker(ui: &mut Ui, white: usize, eye: &Eye, at: Vec3, facing: f32, color: [f32; 4]) {
    let mut dot = |p: Vec3, half: f32| {
        if let Some(px) = eye.project(p) {
            ui.quad(Quad { texture: white, rect: [px.x - half, px.y - half, 2.0 * half, 2.0 * half], uv: FULL_UV, color, additive: false });
        }
    };
    let lifted = at + Vec3::Y * 0.05;
    for i in 0..32 {
        let a = i as f32 / 32.0 * std::f32::consts::TAU;
        dot(lifted + Vec3::new(a.cos(), 0.0, a.sin()) * 0.7, 1.5);
    }
    let f = facing.to_radians();
    let dir = Vec3::new(f.sin(), 0.0, -f.cos());
    for step in 1..=10 {
        dot(lifted + dir * (0.7 + 1.3 * step as f32 / 10.0), 1.5);
    }
    dot(lifted + dir * 2.0, 4.0);
}

/// A part being dragged: what the prop was, and where the pointer took hold.
pub struct Drag {
    pub handle: Handle,
    start: Prop,
    mouse: Vec2,
    /// Where on the axis, the ground or the ring the pointer took hold.
    grip: f32,
    spot: Vec3,
}

/// The place on the line through `p` along `axis` (a unit vector) that is nearest the ray, as a distance from `p`.
fn along(origin: Vec3, dir: Vec3, p: Vec3, axis: Vec3) -> Option<f32> {
    let (w, b) = (origin - p, dir.dot(axis));
    let denom = 1.0 - b * b;
    (denom > 1e-3).then(|| (axis.dot(w) - b * dir.dot(w)) / denom)
}

/// Where the ray meets the horizontal plane at height `y`.
fn on_floor(origin: Vec3, dir: Vec3, y: f32) -> Option<Vec3> {
    (dir.y.abs() > 1e-3).then(|| (y - origin.y) / dir.y).filter(|t| *t > 0.0).map(|t| origin + dir * t)
}

/// The angle (degrees, counter-clockwise seen from above, 0 = east) of a point round `centre`.
fn bearing(point: Vec3, centre: Vec3) -> f32 {
    (-(point.z - centre.z)).atan2(point.x - centre.x).to_degrees()
}

impl Drag {
    pub fn begin(handle: Handle, prop: &Prop, eye: &Eye, mouse: Vec2) -> Option<Drag> {
        let (origin, dir) = eye.ray(mouse);
        let at = position(prop);
        let (grip, spot) = match handle {
            Handle::Axis(i) => (along(origin, dir, at, AXES[i])?, Vec3::ZERO),
            Handle::Floor => (0.0, on_floor(origin, dir, at.y)?),
            Handle::Ring => (bearing(on_floor(origin, dir, at.y)?, at), Vec3::ZERO),
            Handle::Knob => (0.0, Vec3::ZERO),
        };
        Some(Drag { handle, start: prop.clone(), mouse, grip, spot })
    }

    /// The prop as the pointer has dragged it by now (`None` when the pointer is where nothing can be told, such as the sky for the ground).
    pub fn update(&self, eye: &Eye, mouse: Vec2) -> Option<Prop> {
        let (origin, dir) = eye.ray(mouse);
        let at = position(&self.start);
        let mut prop = self.start.clone();
        let place = |prop: &mut Prop, to: Vec3| [prop.at.0, prop.at.1, prop.at.2] = render_to_wow(to);
        match self.handle {
            Handle::Axis(i) => place(&mut prop, at + AXES[i] * (along(origin, dir, at, AXES[i])? - self.grip)),
            Handle::Floor => place(&mut prop, at + (on_floor(origin, dir, at.y)? - self.spot)),
            Handle::Ring => prop.rotation[1] = (self.start.rotation[1] + bearing(on_floor(origin, dir, at.y)?, at) - self.grip).rem_euclid(360.0),
            Handle::Knob => prop.scale = (self.start.scale * ((self.mouse.y - mouse.y) / 120.0).exp()).clamp(0.05, 50.0),
        }
        Some(prop)
    }
}

/// What stands in the world as a ghost: the parts of a model, or a dressed person (he is animated and has his own entities).
enum Ghost {
    Parts(Vec<usize>),
    Person(Person),
}

/// The models of the game for props: their parts and boxes (loaded on this thread, once), and the ghosts that show a prop, a creature or
/// a chest in the world before it is put down (one at a time; the entities of those shown before are kept, hidden).
pub struct Models {
    mount: Arc<Mount>,
    pub cache: Arc<ModelCache>,
    boxes: HashMap<String, (Vec3, Vec3)>,
    ghosts: HashMap<String, Ghost>,
    /// The ghost that is shown, and where (to hide a person).
    shown: Option<(String, Vec3)>,
}

impl Models {
    pub fn new(mount: Arc<Mount>) -> Models {
        Models { mount, cache: Arc::new(ModelCache::default()), boxes: HashMap::new(), ghosts: HashMap::new(), shown: None }
    }

    /// The parts of a model that have something to draw.
    fn parts(&self, file: &str) -> Vec<Arc<ModelCpu>> {
        let parts: Vec<Arc<ModelCpu>> = if file.to_ascii_lowercase().ends_with(".wmo") { self.cache.get_wmo(&self.mount, file).groups.iter().filter_map(|g| g.model.clone()).collect() } else { self.cache.get(&self.mount, file).into_iter().collect() };
        parts.into_iter().filter(|m| !m.vertices.is_empty()).collect()
    }

    /// The box round a model (a yard wide for one with nothing to see).
    pub fn bounds(&mut self, file: &str) -> (Vec3, Vec3) {
        if let Some(b) = self.boxes.get(&file.to_ascii_lowercase()) {
            return *b;
        }
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for v in self.parts(file).iter().flat_map(|m| &m.vertices) {
            (low, high) = (low.min(Vec3::from(v.pos)), high.max(Vec3::from(v.pos)));
        }
        let b = if low.x > high.x { (Vec3::splat(-0.5), Vec3::splat(0.5)) } else { (low, high) };
        self.boxes.insert(file.to_ascii_lowercase(), b);
        b
    }

    /// Hide the ghost shown before, unless it is the one called `key`.
    fn switch(&mut self, renderer: &mut Renderer, key: &str) {
        if self.shown.as_ref().is_some_and(|(shown, _)| shown != key) {
            self.hide(renderer);
        }
    }

    /// Show the model `file` at `at`, turned and scaled by `orient`, in the world (the one shown before is hidden). False if it has
    /// nothing to show.
    pub fn show(&mut self, renderer: &mut Renderer, gpu: &Gpu, file: &str, at: Vec3, orient: Mat4) -> bool {
        let key = file.to_ascii_lowercase();
        self.switch(renderer, &key);
        if !self.ghosts.contains_key(&key) {
            let ids = self.parts(file).iter().map(|m| renderer.add_entity(gpu, m)).collect();
            self.ghosts.insert(key.clone(), Ghost::Parts(ids));
        }
        self.place(renderer, key, at, orient)
    }

    /// Show a creature that is an animal: its model with these skins, standing, at `at`, facing `yaw` (radians, 0 north, clockwise)
    /// and `scale` times its size: as the creatures themselves are placed.
    #[allow(clippy::too_many_arguments)]
    pub fn show_beast(&mut self, renderer: &mut Renderer, gpu: &Gpu, model: &str, skins: &[(u32, String)], at: Vec3, yaw: f32, scale: f32) -> bool {
        let key = format!("beast|{model}|{skins:?}").to_ascii_lowercase();
        self.switch(renderer, &key);
        if !self.ghosts.contains_key(&key) {
            let ids = self.beast(renderer, gpu, model, skins).map_err(|e| log::warn!("no ghost of {model}: {e:#}")).unwrap_or_default();
            self.ghosts.insert(key.clone(), Ghost::Parts(ids));
        }
        self.place(renderer, key, at, Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - yaw) * Mat4::from_scale(Vec3::splat(scale)))
    }

    /// The entity of an animal in the first frame of its standing animation, and the vertices it has there.
    pub fn beast_entity(&self, renderer: &mut Renderer, gpu: &Gpu, model: &str, skins: &[(u32, String)]) -> anyhow::Result<(usize, Vec<ModelVertex>)> {
        let m2 = M2::parse(&self.mount.read(model)?)?;
        let skins: Vec<(u32, &str)> = skins.iter().map(|(slot, file)| (*slot, file.as_str())).collect();
        let template = self.cache.creature(&self.mount, model, &skins)?;
        let entity = renderer.add_entity(gpu, &template);
        let mut vertices = template.vertices.clone();
        if let Some(seq) = m2.sequences.iter().position(|s| s.id == 0) {
            let bones = anim::bone_matrices(&m2, seq, m2.sequences[seq].start as f32, 0.0);
            anim::skin(&m2, &bones, &template.vertices, &mut vertices);
            renderer.update_entity_vertices(gpu, entity, &vertices);
        }
        Ok((entity, vertices))
    }

    fn beast(&self, renderer: &mut Renderer, gpu: &Gpu, model: &str, skins: &[(u32, String)]) -> anyhow::Result<Vec<usize>> {
        Ok(vec![self.beast_entity(renderer, gpu, model, skins)?.0])
    }

    /// A person dressed in `outfit` (indices of the items) with his looks.
    pub fn person(&self, renderer: &mut Renderer, gpu: &Gpu, outfit: &[usize], look: &Appearance, key: &str) -> anyhow::Result<Person> {
        Person::load(outfit, look, &self.mount, renderer, gpu, key)
    }

    /// Show a creature that is a person: dressed in `outfit` (indices of the items) with his looks, standing at `at` and facing `yaw`.
    #[allow(clippy::too_many_arguments)]
    pub fn show_person(&mut self, renderer: &mut Renderer, gpu: &Gpu, outfit: &[usize], look: &Appearance, dt: f32, at: Vec3, yaw: f32) -> bool {
        let key = format!("person|{outfit:?}|{look:?}").to_ascii_lowercase();
        self.switch(renderer, &key);
        if !self.ghosts.contains_key(&key) {
            match self.person(renderer, gpu, outfit, look, &key) {
                Ok(person) => {
                    self.ghosts.insert(key.clone(), Ghost::Person(person));
                }
                Err(e) => {
                    log::warn!("no ghost of a person: {e:#}");
                    return false;
                }
            }
        }
        if let Some(Ghost::Person(person)) = self.ghosts.get_mut(&key) {
            person.stand_facing(dt, at, yaw, renderer, gpu);
        }
        self.shown = Some((key, at));
        true
    }

    fn place(&mut self, renderer: &mut Renderer, key: String, at: Vec3, orient: Mat4) -> bool {
        let Some(Ghost::Parts(ids)) = self.ghosts.get(&key) else { return false };
        for &id in ids {
            renderer.set_entity(id, at, orient, true);
        }
        let any = !ids.is_empty();
        self.shown = Some((key, at));
        any
    }

    pub fn hide(&mut self, renderer: &mut Renderer) {
        if let Some((key, at)) = self.shown.take() {
            match self.ghosts.get(&key) {
                Some(Ghost::Parts(ids)) => ids.iter().for_each(|&id| renderer.set_entity(id, Vec3::ZERO, Mat4::IDENTITY, false)),
                Some(Ghost::Person(person)) => person.hide(at, renderer),
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(rotation: [f32; 3]) -> Prop {
        Prop { model: "x.m2".into(), map: None, zone: None, pickup: None, sleep: false, physics: false, station: 0, page: 0, title: None, at: (-8900.0, -100.0, 80.0), rotation, scale: 1.0 }
    }

    fn eye() -> Eye {
        // Standing south-east of the prop at its height, looking at it.
        let target = position(&prop([0.0; 3]));
        let mut camera = Camera::new(target + Vec3::new(10.0, 4.0, 10.0), 0.0, 0.0);
        let to = target - camera.pos;
        camera.yaw = to.x.atan2(-to.z);
        camera.pitch = (to.y / to.length()).asin();
        camera.aspect = 1.0;
        Eye::of(&camera, (800, 800))
    }

    #[test]
    fn a_pixel_and_the_ray_through_it_agree() {
        let eye = eye();
        let p = position(&prop([0.0; 3])) + Vec3::new(1.0, 0.5, -2.0);
        let pixel = eye.project(p).unwrap();
        let (origin, dir) = eye.ray(pixel);
        assert!((origin + dir * (p - origin).length() - p).length() < 0.05);
    }

    #[test]
    fn the_ray_hits_the_box_of_a_prop_that_is_turned_and_not_the_one_beside_it() {
        let eye = eye();
        let bounds = |_: &str| (Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0));
        let here = prop([0.0, 30.0, 0.0]);
        let mut beside = here.clone();
        beside.at.1 += 20.0;
        let centre = position(&here) + Vec3::Y;
        let (origin, dir) = eye.ray(eye.project(centre).unwrap());
        assert_eq!(pick(&[beside.clone(), here.clone()], origin, dir, bounds), Some(1));
        let far = eye.project(position(&beside)).unwrap();
        let (origin, dir) = eye.ray(far);
        assert_eq!(pick(&[beside, here], origin, dir, bounds), Some(0));
    }

    #[test]
    fn dragging_an_arrow_moves_the_prop_along_it_and_by_as_much_as_the_pointer_did() {
        let eye = eye();
        let start = prop([0.0; 3]);
        let at = position(&start);
        let gizmo = Gizmo::new(&eye, at, Mode::Move, true);
        // The arrow tip, seen at its tip, is taken (the spot in the middle comes first, the arrows after it).
        let tip = eye.project(at + Vec3::X * eye.world_length(at, LENGTH)).unwrap();
        assert_eq!(gizmo.grab(tip), Some(Handle::Axis(0)));
        assert_eq!(gizmo.grab(eye.project(at).unwrap()), Some(Handle::Floor));
        // Pulling the grip of the east arrow two yards further east.
        let grip = eye.project(at + Vec3::X * 3.0).unwrap();
        let drag = Drag::begin(Handle::Axis(0), &start, &eye, grip).unwrap();
        let moved = drag.update(&eye, eye.project(at + Vec3::X * 5.0).unwrap()).unwrap();
        let to = position(&moved);
        assert!((to - (at + Vec3::X * 2.0)).length() < 0.05, "{to} from {at}");
    }

    #[test]
    fn dragging_over_the_ground_keeps_the_height_and_dragging_the_ring_turns_the_prop_as_far_as_the_pointer_went() {
        let eye = eye();
        let start = prop([0.0, 40.0, 0.0]);
        let at = position(&start);
        let floor = Drag::begin(Handle::Floor, &start, &eye, eye.project(at).unwrap()).unwrap();
        let there = at + Vec3::new(3.0, 0.0, -4.0);
        let moved = floor.update(&eye, eye.project(there).unwrap()).unwrap();
        assert!((position(&moved) - there).length() < 0.05);
        // A point of the ring a quarter turn round, counter-clockwise seen from above (towards north), turns it by 90 degrees.
        let ring = Drag::begin(Handle::Ring, &start, &eye, eye.project(at + Vec3::X * 3.0).unwrap()).unwrap();
        let turned = ring.update(&eye, eye.project(at - Vec3::Z * 3.0).unwrap()).unwrap();
        // The model turned that way, its east side goes to the north: the same as `placement_matrix` does for a bigger heading.
        let (before, after) = (orient(&start).transform_vector3(Vec3::X), orient(&turned).transform_vector3(Vec3::X));
        let angle = (bearing(after, Vec3::ZERO) - bearing(before, Vec3::ZERO)).rem_euclid(360.0);
        assert!((angle - 90.0).abs() < 0.5, "turned by {angle}");
    }

    #[test]
    fn a_standing_box_is_hit_from_the_side_and_not_beside_it() {
        let (low, high) = (Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 2.0, 0.5));
        assert!(hit_standing(Vec3::new(5.0, 1.0, 0.0), -Vec3::X, low, high).is_some_and(|t| (t - 4.5).abs() < 1e-4));
        assert!(hit_standing(Vec3::new(5.0, 1.0, 2.0), -Vec3::X, low, high).is_none());
        assert!(hit_standing(Vec3::new(5.0, 3.0, 0.0), -Vec3::X, low, high).is_none());
    }

    #[test]
    fn the_knob_scales_by_the_pointer_going_up_or_down() {
        let eye = eye();
        let start = prop([0.0; 3]);
        let drag = Drag::begin(Handle::Knob, &start, &eye, Vec2::new(400.0, 400.0)).unwrap();
        assert!(drag.update(&eye, Vec2::new(400.0, 280.0)).unwrap().scale > 2.0);
        assert!(drag.update(&eye, Vec2::new(400.0, 520.0)).unwrap().scale < 0.5);
    }
}
