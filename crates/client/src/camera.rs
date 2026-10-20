//! Camera and player-style movement.

use crate::movement::{TriangleSource, Unit, DEFAULT_HEIGHT};
use glam::{Mat4, Vec2, Vec3};

/// Movement constants (yards, seconds). Gravity, terminal velocity and the collision shape live in
/// `movement.rs` (read from WoW.exe); run speed and jump velocity are the commonly documented
/// vanilla values: the client takes them from per-unit movement state, so they are not confirmed
/// from the binary.
pub const RUN_SPEED: f32 = 7.0;
/// Sprinting is this much faster than running.
pub const SPRINT_FACTOR: f32 = 1.5;
/// A sneaking character moves this much of the running speed.
pub const SNEAK_FACTOR: f32 = 0.45;
pub const JUMP_VELOCITY: f32 = 7.955_547;
pub const EYE_HEIGHT: f32 = 2.0;
/// Swimming speeds, yards a second (the vanilla values the server sends: the client has no defaults for them). A swimmer who has
/// run out of stamina manages `TIRED_FACTOR` of it and sinks `SINK_SPEED`.
pub const SWIM_SPEED: f32 = 4.722_222;
pub const SWIM_BACK_SPEED: f32 = 2.5;
const TIRED_FACTOR: f32 = 0.5;
const SINK_SPEED: f32 = 1.5;
/// The client starts swimming when the water is deeper than this share of the unit's height and stops when it is less deep than that by
/// `SWIM_STOP` (`FUN_006030c0`: height * 0.75, and 1/36 below it).
pub const SWIM_DEPTH: f32 = 0.75;
const SWIM_STOP: f32 = 1.0 / 36.0;

/// The vertical field of view the original makes of a field of view it is given (a game's default, a model camera's): the given angle is the one
/// across the corner of the picture (`FUN_005c3cc0`: tan of half of `fov / sqrt(aspect * aspect + 1)`), `aspect` being width over height.
pub fn vertical_fov(fov: f32, aspect: f32) -> f32 {
    fov / (aspect * aspect + 1.0).sqrt()
}

pub struct Camera {
    /// Render-space position.
    pub pos: Vec3,
    /// Radians; 0 = north (-Z), pi/2 = east (+X).
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

impl Camera {
    pub fn new(pos: Vec3, yaw: f32, pitch: f32) -> Self {
        Self { pos, yaw, pitch, fov_y: 60f32.to_radians(), aspect: 16.0 / 9.0, near: 0.3, far: 6000.0 }
    }

    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp)
    }

    /// Projection * rotation-only view. Geometry is submitted relative to the
    /// camera, which keeps f32 precise even 17000 yards from the map origin.
    pub fn view_proj_relative(&self) -> Mat4 {
        let proj = Mat4::perspective_rh(self.fov_y, self.aspect, self.near, self.far);
        proj * Mat4::look_to_rh(Vec3::ZERO, self.forward(), Vec3::Y)
    }
}

#[derive(Default)]
pub struct Input {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub fast: bool,
    /// Sprinting on foot (the game sets it from `fast` while there is stamina left).
    pub sprint: bool,
    /// Sneaking on foot (set by the game from the player).
    pub sneak: bool,
    pub jump_pressed: bool,
    /// The share of the running speed that a steed adds (0.6 for 60 percent; 0 on foot).
    pub mount_bonus: f32,
    /// The share of the running speed that a heavy load takes (0 for none, set by the game from the player's weight).
    pub burden: f32,
    /// The surface of the water under the feet, if there is any (render space, set by the game each frame).
    pub water: Option<f32>,
    /// Worn out: a swimmer sinks.
    pub tired: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Walk,
    Fly,
}

pub struct Controller {
    pub mode: Mode,
    pub fly_speed: f32,
    /// How much faster than usual the mouse turns the view, and whether pushing it forward looks down (the interface options).
    pub look_scale: f32,
    pub invert_y: bool,
    /// The ported client movement state; created from the camera when walking starts.
    unit: Option<Unit>,
    /// In the water, off the bottom and swimming.
    /// How tall it is to the water (`CreatureModelData.dbc`, times its scale): swimming starts at three quarters of it.
    height: f32,
    swimming: bool,
    /// Leapt out of the water and has not yet left it behind: no swimming again till then.
    leapt: bool,
}

/// Render space (X east, Y up, Z south) -> WoW (x north, y west, z up).
fn render_to_wow64(p: Vec3) -> [f64; 3] {
    [-(p.z as f64), -(p.x as f64), p.y as f64]
}

impl Controller {
    pub fn new(mode: Mode) -> Self {
        Self { mode, fly_speed: 60.0, look_scale: 1.0, invert_y: false, unit: None, height: DEFAULT_HEIGHT, swimming: false, leapt: false }
    }

    /// How tall the character is (from its model).
    pub fn set_height(&mut self, height: f32) {
        self.height = height;
    }

    /// Whether the walking character is off the ground (jumping or falling).
    pub fn airborne(&self) -> bool {
        !self.swimming && self.unit.as_ref().is_some_and(|u| u.is_falling())
    }

    pub fn swimming(&self) -> bool {
        self.swimming && self.mode == Mode::Walk
    }

    /// Forget the walking state, so walking restarts from wherever the camera was put.
    pub fn reset(&mut self) {
        self.unit = None;
        self.swimming = false;
    }

    pub fn toggle_mode(&mut self) {
        self.mode = if self.mode == Mode::Walk { Mode::Fly } else { Mode::Walk };
        // Walking restarts from wherever the camera is, falling to the ground.
        self.unit = None;
    }

    pub fn look(&self, cam: &mut Camera, dx: f32, dy: f32) {
        let sens = 0.0025 * self.look_scale;
        let dy = if self.invert_y { -dy } else { dy };
        cam.yaw = (cam.yaw + dx * sens).rem_euclid(std::f32::consts::TAU);
        cam.pitch = (cam.pitch - dy * sens).clamp(-1.55, 1.55);
    }

    pub fn update(&mut self, cam: &mut Camera, input: &mut Input, world: &impl TriangleSource, dt: f32) {
        let (sy, cy) = cam.yaw.sin_cos();
        let fwd_flat = Vec3::new(sy, 0.0, -cy);
        let right = Vec3::new(cy, 0.0, sy);
        let axis = |pos: bool, neg: bool| (pos as i32 - neg as i32) as f32;

        match self.mode {
            Mode::Fly => {
                let speed = self.fly_speed * if input.fast { 4.0 } else { 1.0 };
                let mut v = cam.forward() * axis(input.forward, input.back) + right * axis(input.right, input.left);
                v.y += axis(input.up, input.down);
                cam.pos += v.normalize_or_zero() * speed * dt;
            }
            Mode::Walk => {
                let wish = (fwd_flat * axis(input.forward, input.back) + right * axis(input.right, input.left)).normalize_or_zero();
                let speed = RUN_SPEED * (1.0 + input.mount_bonus) * (1.0 - input.burden) * if input.sprint { SPRINT_FACTOR } else if input.sneak { SNEAK_FACTOR } else { 1.0 };
                let feet = cam.pos - Vec3::Y * EYE_HEIGHT;
                let unit = self.unit.get_or_insert_with(|| Unit::new(render_to_wow64(feet)));
                // Do not simulate (and fall through the void) while the terrain is still streaming in.
                if world.is_loaded(unit.pos) {
                    let (z, reach) = (unit.pos[2] as f32, self.height * SWIM_DEPTH);
                    let depth = input.water.map_or(f32::MIN, |level| level - z);
                    self.leapt &= depth >= reach - SWIM_STOP;
                    self.swimming = depth >= reach - SWIM_STOP && (self.swimming || depth > reach && !self.leapt) && input.mount_bonus == 0.0;
                    if self.swimming {
                        // The surface holds the swimmer up with the head out: the feet cannot rise beyond the depth at which swimming starts.
                        let top = input.water.map_or(z, |level| level - reach);
                        if input.jump_pressed && z >= top - 0.05 {
                            // At the surface a jump leaps out (onto the bank).
                            let v = Vec2::new(-fwd_flat.z, -fwd_flat.x) * RUN_SPEED * axis(input.forward, input.back).max(0.0);
                            (self.swimming, self.leapt) = (false, true);
                            unit.launch(JUMP_VELOCITY, v);
                            unit.update(world, dt, v);
                        } else {
                            // The swimmer goes the way the camera looks, up and down too; jump swims up.
                            let a = axis(input.forward, input.back);
                            let mut dir = cam.forward() * a + right * axis(input.right, input.left);
                            dir.y += input.up as i32 as f32;
                            let tired = if input.tired { TIRED_FACTOR } else { 1.0 };
                            let pace = if a < 0.0 && !input.right && !input.left { SWIM_BACK_SPEED } else { SWIM_SPEED } * tired;
                            let dir = dir.normalize_or_zero() * pace;
                            let up = (dir.y - if input.tired { SINK_SPEED } else { 0.0 }).min((top - z).max(0.0) / dt.max(1e-3));
                            unit.swim(world, dt, Vec3::new(-dir.z, -dir.x, up));
                        }
                    } else {
                        // render (east, south) -> WoW (north, west)
                        let velocity = Vec2::new(-wish.z * speed, -wish.x * speed);
                        if input.jump_pressed && !unit.is_falling() {
                            unit.jump(JUMP_VELOCITY, velocity);
                        }
                        unit.update(world, dt, velocity);
                    }
                }
                cam.pos = Vec3::new(-unit.pos[1] as f32, unit.pos[2] as f32 + EYE_HEIGHT, -unit.pos[0] as f32);
            }
        }
        input.jump_pressed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::Tri;

    /// A flat floor in WoW coordinates at height `z`.
    struct Floor(f32);

    impl TriangleSource for Floor {
        fn gather(&self, origin: [f64; 3], min: Vec3, max: Vec3, out: &mut Vec<Tri>) {
            let o = Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32);
            if o.z + min.z <= self.0 && self.0 <= o.z + max.z {
                let z = self.0 - o.z;
                let (a, b, c, d) = (Vec3::new(-100.0, -100.0, z), Vec3::new(100.0, -100.0, z), Vec3::new(100.0, 100.0, z), Vec3::new(-100.0, 100.0, z));
                out.extend([Tri::new(a, b, c), Tri::new(a, c, d)].into_iter().flatten());
            }
        }
    }

    fn walker(y: f32) -> (Camera, Controller, Input) {
        (Camera::new(Vec3::new(0.0, y, 0.0), 0.0, 0.0), Controller::new(Mode::Walk), Input::default())
    }

    fn run(c: &mut Controller, cam: &mut Camera, input: &mut Input, world: &impl TriangleSource, secs: f32) {
        for _ in 0..(secs * 60.0) as usize {
            c.update(cam, input, world, 1.0 / 60.0);
        }
    }

    #[test]
    fn lands_on_flat_ground_and_stays() {
        let (mut cam, mut c, mut input) = walker(50.0);
        run(&mut c, &mut cam, &mut input, &Floor(10.0), 5.0);
        assert!((cam.pos.y - (10.0 + EYE_HEIGHT)).abs() < 0.02, "y = {}", cam.pos.y);
    }

    #[test]
    fn running_speed_is_seven_yards_per_second_and_yaw_zero_is_north() {
        let world = Floor(2.0);
        let (mut cam, mut c, mut input) = walker(2.0 + EYE_HEIGHT);
        run(&mut c, &mut cam, &mut input, &world, 0.5);
        let start = cam.pos;
        input.forward = true;
        run(&mut c, &mut cam, &mut input, &world, 2.0);
        let dist = (cam.pos - start).length();
        assert!((dist - 14.0).abs() < 0.3, "ran {dist} yards in 2s");
        assert!(cam.pos.z < start.z, "yaw 0 must head north (-Z in render space)");
    }

    #[test]
    fn sneaking_is_slower_than_running() {
        let world = Floor(2.0);
        let (mut cam, mut c, mut input) = walker(2.0 + EYE_HEIGHT);
        run(&mut c, &mut cam, &mut input, &world, 0.5);
        let start = cam.pos;
        input.forward = true;
        input.sneak = true;
        run(&mut c, &mut cam, &mut input, &world, 2.0);
        let dist = (cam.pos - start).length();
        assert!((dist - 14.0 * SNEAK_FACTOR).abs() < 0.4, "sneaked {dist} yards in 2s");
    }

    #[test]
    fn sprinting_is_half_again_as_fast_as_running() {
        let world = Floor(2.0);
        let (mut cam, mut c, mut input) = walker(2.0 + EYE_HEIGHT);
        run(&mut c, &mut cam, &mut input, &world, 0.5);
        let start = cam.pos;
        input.forward = true;
        input.sprint = true;
        run(&mut c, &mut cam, &mut input, &world, 2.0);
        let dist = (cam.pos - start).length();
        assert!((dist - 14.0 * SPRINT_FACTOR).abs() < 0.4, "sprinted {dist} yards in 2s");
    }

    #[test]
    fn a_steed_adds_its_share_to_the_running_speed() {
        let world = Floor(2.0);
        let (mut cam, mut c, mut input) = walker(2.0 + EYE_HEIGHT);
        run(&mut c, &mut cam, &mut input, &world, 0.5);
        let start = cam.pos;
        input.forward = true;
        input.mount_bonus = 0.6;
        run(&mut c, &mut cam, &mut input, &world, 2.0);
        let dist = (cam.pos - start).length();
        assert!((dist - 14.0 * 1.6).abs() < 0.5, "rode {dist} yards in 2s");
    }

    #[test]
    fn a_steed_gallops_half_again_as_fast_when_sprinting() {
        let world = Floor(2.0);
        let (mut cam, mut c, mut input) = walker(2.0 + EYE_HEIGHT);
        run(&mut c, &mut cam, &mut input, &world, 0.5);
        let start = cam.pos;
        (input.forward, input.sprint, input.mount_bonus) = (true, true, 0.6);
        run(&mut c, &mut cam, &mut input, &world, 2.0);
        let dist = (cam.pos - start).length();
        assert!((dist - 14.0 * 1.6 * SPRINT_FACTOR).abs() < 0.8, "galloped {dist} yards in 2s");
    }

    #[test]
    fn jump_apex_matches_ballistics() {
        let world = Floor(2.0);
        let (mut cam, mut c, mut input) = walker(2.0 + EYE_HEIGHT);
        run(&mut c, &mut cam, &mut input, &world, 1.0);
        input.jump_pressed = true;
        let mut apex = cam.pos.y;
        for _ in 0..240 {
            c.update(&mut cam, &mut input, &world, 1.0 / 240.0);
            apex = apex.max(cam.pos.y);
        }
        let expected = JUMP_VELOCITY * JUMP_VELOCITY / (2.0 * crate::movement::GRAVITY); // ~1.64 yards
        assert!(((apex - 2.0 - EYE_HEIGHT) - expected).abs() < 0.06, "apex {}, expected {expected}", apex - 2.0 - EYE_HEIGHT);
    }

    #[test]
    fn deep_water_makes_a_swimmer_who_goes_the_way_the_camera_looks_and_stops_at_the_surface() {
        // A floor at 0 with the water surface at 5: the water is deeper than three quarters of the height, so the character swims.
        let world = Floor(0.0);
        let (mut cam, mut c, mut input) = walker(EYE_HEIGHT);
        input.water = Some(5.0);
        run(&mut c, &mut cam, &mut input, &world, 0.5);
        assert!(c.swimming() && !c.airborne());
        let start = cam.pos;
        input.forward = true;
        run(&mut c, &mut cam, &mut input, &world, 2.0);
        let dist = (cam.pos - start).length();
        assert!((dist - 2.0 * SWIM_SPEED).abs() < 0.3, "swam {dist} yards in 2s");
        // Looking up, forward swims up, but the head stays out of the water, not above it.
        cam.pitch = 0.6;
        run(&mut c, &mut cam, &mut input, &world, 4.0);
        let top = 5.0 - 2.027_777_7 * SWIM_DEPTH + EYE_HEIGHT;
        assert!((cam.pos.y - top).abs() < 0.05, "y = {}, surface holds at {top}", cam.pos.y);
        // A tired swimmer sinks to the bottom.
        (input.tired, input.forward) = (true, false);
        run(&mut c, &mut cam, &mut input, &world, 5.0);
        assert!((cam.pos.y - EYE_HEIGHT).abs() < 0.05, "y = {}", cam.pos.y);
    }

    #[test]
    fn shallow_water_is_waded_and_a_jump_at_the_surface_leaps_out() {
        let world = Floor(0.0);
        let (mut cam, mut c, mut input) = walker(EYE_HEIGHT);
        input.water = Some(1.0);
        run(&mut c, &mut cam, &mut input, &world, 0.5);
        assert!(!c.swimming(), "knee deep is not swimming");
        input.water = Some(5.0);
        run(&mut c, &mut cam, &mut input, &world, 0.1);
        input.forward = true;
        cam.pitch = 0.6;
        run(&mut c, &mut cam, &mut input, &world, 3.0);
        assert!(c.swimming());
        input.jump_pressed = true;
        c.update(&mut cam, &mut input, &world, 1.0 / 60.0);
        assert!(!c.swimming() && cam.pos.y > 5.0 - 1.52 + EYE_HEIGHT);
        run(&mut c, &mut cam, &mut input, &world, 0.1);
        assert!(!c.swimming(), "does not dive straight back in while rising out");
    }

    #[test]
    fn holds_still_until_the_world_is_loaded() {
        struct Empty;
        impl TriangleSource for Empty {
            fn gather(&self, _: [f64; 3], _: Vec3, _: Vec3, _: &mut Vec<Tri>) {}
            fn is_loaded(&self, _: [f64; 3]) -> bool {
                false
            }
        }
        let (mut cam, mut c, mut input) = walker(30.0);
        run(&mut c, &mut cam, &mut input, &Empty, 2.0);
        assert_eq!(cam.pos.y, 30.0, "must not free-fall through unloaded terrain");
    }
}
