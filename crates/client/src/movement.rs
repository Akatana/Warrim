//! Player movement and collision, ported from the WoW 1.12.1 client (see docs/RE-FINDINGS.md).
//!
//! Original functions (addresses in WoW.exe): `FUN_00634040` (per-update integrator),
//! `FUN_006367b0` (ground move), `FUN_00636100` (step probe), `FUN_00635c00` (slope projection),
//! `FUN_00635d80` (wall slide), `FUN_006334a0` (walkability), `FUN_00635b00`/`FUN_00635600`
//! (falling), `FUN_007c5e70` (fall distance), `FUN_00632ba0` (swept hull test).
//!
//! Everything works in WoW world coordinates (x north, y west, z up), like the original, and
//! relative to an origin near the unit so f32 stays precise anywhere on the map.
//!
//! The one part that is *not* a transcription is the swept hull-versus-triangle intersection: the
//! client's version is x87-heavy plane clipping, so this module sweeps the same hull (a square
//! prism with a pyramid foot) with an exact separating-axis test instead.
//!
//! Swimming (`Unit::swim`) is a straight 3-D sweep as in `FUN_00634640`. Not ported: flying, hovering and vehicles (client code paths for other movement
//! flags), and `FUN_00635f80`, an additional "would we fall back" check inside the step probe.

use glam::{Vec2, Vec3};

// ---- constants read from WoW.exe ------------------------------------------------------------
/// cos 50 degrees (0x80dffc): `normal.z` above this is walkable.
pub const COS_50: f32 = 0.642_787_6;
/// tan 50 degrees (0x80e008).
pub const TAN_50: f32 = 1.191_753_6;
/// Hull ring height as a multiple of the radius (0x80c740).
pub const FOOT_RATIO: f32 = 1.849_399;
/// Collision skin (0x80dfec).
pub const SKIN: f32 = 0.001_388_889;
/// 9.5367e-7 (0x8026bc): "zero" length.
pub const EPS_LEN: f32 = 9.536_743e-7;
/// 2.3842e-7 (0x8029d4): "zero" for directions and normals.
pub const EPS_DIR: f32 = 2.384_186e-7;
/// Downward cast margin during a step (0x8029d0).
pub const STEP_MARGIN: f32 = 0.01;
/// 1/36 (0x7ff9d0): added to the ground-snap cast.
pub const SNAP_BASE: f32 = 0.027_777_78;
/// 0.001 (0x801360): milliseconds to seconds, and the wall-slide push.
pub const MS: f32 = 0.001;
/// Gravity and terminal velocity (0x81da58, 0x87d894).
pub const GRAVITY: f32 = 19.291_105;
pub const TERMINAL_VELOCITY: f32 = 60.148;
/// Unit defaults from the movement constructor (`FUN_00616fa0`): radius, height, reach.
pub const DEFAULT_RADIUS: f32 = 0.333_333_34;
pub const DEFAULT_HEIGHT: f32 = 2.027_777_7;
pub const DEFAULT_REACH: f32 = 1.0;

// ---- movement flags (client-internal values) -------------------------------------------------
/// Any of forward/back/strafe (0x1..0x8 in the original).
pub const MOVING: u32 = 0x1;
pub const FALLING: u32 = 0x2000;
/// Set while climbing stairs/steps (0x4000000 in the client).
pub const STEPPED: u32 = 0x400_0000;

/// A collision triangle (the client's 0x34-byte record holds the normal, plane distance and
/// three vertices). Triangles are double-sided here.
#[derive(Clone, Copy, Debug)]
pub struct Tri {
    pub n: Vec3,
    pub v: [Vec3; 3],
}

impl Tri {
    pub fn new(a: Vec3, b: Vec3, c: Vec3) -> Option<Tri> {
        let n = (b - a).cross(c - a);
        let len = n.length();
        (len > 1e-9).then(|| Tri { n: n / len, v: [a, b, c] })
    }
}

/// Source of collision triangles in WoW coordinates.
pub trait TriangleSource {
    /// Append the triangles overlapping the box `[origin + min, origin + max]`. Vertices are
    /// returned relative to `origin` (computed in f64 by the implementation).
    fn gather(&self, origin: [f64; 3], min: Vec3, max: Vec3, out: &mut Vec<Tri>);

    /// Whether collision data exists around `origin` (so a unit does not fall through a void
    /// while the world is still streaming in).
    fn is_loaded(&self, _origin: [f64; 3]) -> bool {
        true
    }
}

/// Triangles hit within this distance of the first impact count as touched together.
const CONTACT_TOLERANCE: f32 = 1e-4;

/// Result of a sweep: distance travelled and the triangle that stopped it.
#[derive(Clone, Copy, Debug)]
struct Sweep {
    t: f32,
    tri: Option<usize>,
}

/// Outcome of a step probe (`FUN_00636100`).
#[derive(PartialEq, Eq, Debug)]
enum Step {
    /// The unit cannot step here (the original returns 1).
    No,
    /// A step up is possible (the original returns 2).
    Yes,
}

/// Convex hull of a unit: apex at the feet, a square ring at `radius * FOOT_RATIO`, a square top.
struct Hull {
    verts: [Vec3; 9],
    face_normals: Vec<Vec3>,
    edge_dirs: Vec<Vec3>,
}

impl Hull {
    fn new(radius: f32, height: f32) -> Hull {
        let h = radius * FOOT_RATIO;
        let ring = |z: f32| {
            [Vec3::new(-radius, -radius, z), Vec3::new(-radius, radius, z), Vec3::new(radius, radius, z), Vec3::new(radius, -radius, z)]
        };
        let (lo, hi) = (ring(h), ring(height));
        let verts = [Vec3::ZERO, lo[0], lo[1], lo[2], lo[3], hi[0], hi[1], hi[2], hi[3]];
        let mut face_normals = vec![Vec3::X, Vec3::Y, Vec3::Z];
        for i in 0..4 {
            face_normals.push(lo[i].cross(lo[(i + 1) % 4]).normalize_or_zero());
        }
        let mut edge_dirs = vec![Vec3::X, Vec3::Y, Vec3::Z];
        edge_dirs.extend(lo.iter().map(|p| p.normalize_or_zero()));
        Hull { verts, face_normals, edge_dirs }
    }
}

/// The movement state the original keeps in the unit object.
pub struct Unit {
    /// Feet position, WoW coordinates (absolute, f64).
    pub pos: [f64; 3],
    pub flags: u32,
    pub radius: f32,
    pub height: f32,
    pub reach: f32,
    /// +0x80: height of the last ground stood on without stepping.
    last_ground_z: f32,
    /// Falling state (+0x78, +0x7c, +0xa0): ms since the fall began, start height, initial
    /// downward speed (negative = upwards).
    fall_time_ms: f32,
    fall_start_z: f32,
    fall_v0: f32,
    /// Horizontal velocity kept while falling (yards/second, x north / y west).
    fall_velocity: Vec2,
    ms_carry: f32,
    // --- per-update scratch (local frame) ---
    base: [f64; 3],
    lp: Vec3,
    hull: Hull,
    triangles: Vec<Tri>,
}

impl Unit {
    pub fn new(pos: [f64; 3]) -> Unit {
        let mut u = Unit {
            pos,
            flags: 0,
            radius: DEFAULT_RADIUS,
            height: DEFAULT_HEIGHT,
            reach: DEFAULT_REACH,
            last_ground_z: pos[2] as f32,
            fall_time_ms: 0.0,
            fall_start_z: pos[2] as f32,
            fall_v0: 0.0,
            fall_velocity: Vec2::ZERO,
            ms_carry: 0.0,
            base: pos,
            lp: Vec3::ZERO,
            hull: Hull::new(DEFAULT_RADIUS, DEFAULT_HEIGHT),
            triangles: Vec::new(),
        };
        u.start_fall(0.0);
        u
    }

    pub fn is_falling(&self) -> bool {
        self.flags & FALLING != 0
    }

    /// `FUN_007c61f0`: enter the falling state with initial downward speed `v0` (negative = up).
    fn start_fall(&mut self, v0: f32) {
        self.flags = (self.flags & !0x0420_0000) | FALLING;
        self.fall_time_ms = 0.0;
        self.fall_start_z = self.pos[2] as f32;
        self.fall_v0 = v0;
    }

    /// Jump with the given initial upward speed, keeping `horizontal` (x north / y west) in the air.
    pub fn jump(&mut self, speed: f32, horizontal: Vec2) {
        if !self.is_falling() {
            self.launch(speed, horizontal);
        }
    }

    /// Throw the unit up at `speed` whatever it is doing (a swimmer leaping out of the water).
    pub fn launch(&mut self, speed: f32, horizontal: Vec2) {
        self.fall_velocity = horizontal;
        self.start_fall(-speed);
    }

    /// Swimming (`FUN_00634640`): move `velocity` (yards/second, x north / y west / z up) in a straight line through the water, with no
    /// gravity, sliding along whatever the hull meets. The unit is left in the falling state with nothing fallen yet, so that leaving the
    /// water drops it where it is and it lands on the first ground.
    pub fn swim(&mut self, world: &impl TriangleSource, dt: f32, velocity: Vec3) {
        self.start_fall(0.0);
        self.fall_velocity = velocity.truncate();
        let mut remaining = velocity * dt;
        self.gather(world, remaining.length());
        for _ in 0..6 {
            let len = remaining.length();
            if len < EPS_LEN {
                break;
            }
            let dir = remaining / len;
            let s = self.sweep(dir, len);
            self.lp += dir * s.t;
            let Some(i) = s.tri else { break };
            // A surface the hull only touches and is moving off from does not hold it (the sweep counts any touch).
            let (tri, centre) = (self.triangles[i], self.lp + self.hull.verts.iter().sum::<Vec3>() / 9.0);
            if s.t < EPS_LEN && (centre - tri.v[0]).dot(tri.n).signum() * dir.dot(tri.n) > 1e-4 {
                self.triangles.swap_remove(i);
                continue;
            }
            let n = self.oriented(i, dir);
            let left = remaining - dir * s.t;
            remaining = left - n * left.dot(n) + n * SKIN;
        }
        self.commit();
    }

    /// `FUN_007c5e70`: distance fallen after `t` seconds, accelerating at `GRAVITY` up to terminal speed.
    fn fall_distance(&self, t: f32) -> f32 {
        let v0 = self.fall_v0.min(TERMINAL_VELOCITY);
        if v0 + GRAVITY * t <= TERMINAL_VELOCITY {
            (v0 + 0.5 * GRAVITY * t) * t
        } else {
            let t_term = (TERMINAL_VELOCITY - v0) / GRAVITY;
            (v0 + 0.5 * GRAVITY * t_term) * t_term + TERMINAL_VELOCITY * (t - t_term)
        }
    }

    fn z(&self) -> f32 {
        (self.pos[2] + self.lp.z as f64) as f32
    }

    // ---- public update ---------------------------------------------------------------------

    /// Advance by `dt` seconds with horizontal velocity `velocity` (yards/second, x north / y west).
    pub fn update(&mut self, world: &impl TriangleSource, dt: f32, velocity: Vec2) {
        self.ms_carry += dt * 1000.0;
        let ms = self.ms_carry as u32;
        self.ms_carry -= ms as f32;
        if ms == 0 {
            return;
        }
        let moving = velocity.length() > EPS_DIR;
        if moving {
            self.flags |= MOVING;
        } else {
            self.flags &= !MOVING;
        }
        if !self.is_falling() && !moving {
            // Standing still: the original runs no movement at all.
            return;
        }
        let v = if self.is_falling() { self.fall_velocity } else { velocity };
        self.integrate(world, ms, v * (ms as f32 * MS));
        if !self.is_falling() {
            self.fall_velocity = velocity;
        }
    }

    /// `FUN_00634040`: integrate `total_ms` of movement with horizontal displacement `disp`.
    fn integrate(&mut self, world: &impl TriangleSource, total_ms: u32, disp: Vec2) {
        let len = disp.length();
        let speed = len / (total_ms as f32 * MS);
        let dir = if len >= EPS_LEN { disp / len } else { Vec2::ZERO };
        let (mut done, mut guard) = (0u32, 0);
        while done < total_ms && guard < 24 {
            guard += 1;
            if self.flags & (0xf | FALLING) == 0 {
                break;
            }
            let remaining = total_ms - done;
            let dist = remaining as f32 * MS * speed;
            self.gather(world, dist);
            let consumed = if self.flags & FALLING == 0 {
                self.ground_move(remaining, dist, dir)
            } else {
                self.fall_move(remaining, dist, dir)
            };
            self.commit();
            done += consumed;
        }
        if self.flags & (FALLING | STEPPED) == 0 {
            self.last_ground_z = self.pos[2] as f32;
        }
    }

    /// Fold the local offset back into the absolute position.
    fn commit(&mut self) {
        for i in 0..3 {
            self.pos[i] += self.lp[i] as f64;
        }
        self.lp = Vec3::ZERO;
        self.base = self.pos;
    }

    /// `FUN_00633840`: collect the triangles around the unit, relative to its position.
    fn gather(&mut self, world: &impl TriangleSource, dist: f32) {
        self.base = self.pos;
        self.lp = Vec3::ZERO;
        let r = self.radius + dist.abs() + 1.5;
        let down = dist.abs() * TAN_50 + self.reach + 2.0 + if self.is_falling() { 64.0 } else { 0.0 };
        let up = self.height + (2.0 * self.reach).max(dist.abs()) + 2.0;
        self.triangles.clear();
        world.gather(self.base, Vec3::new(-r, -r, -down), Vec3::new(r, r, up), &mut self.triangles);
    }

    // ---- sweep -------------------------------------------------------------------------------

    /// `FUN_00632ba0`: sweep the hull from the local position along unit `dir` for `dist`.
    fn sweep(&self, dir: Vec3, dist: f32) -> Sweep {
        if dist.abs() < EPS_LEN {
            return Sweep { t: 0.0, tri: None };
        }
        let verts: Vec<Vec3> = self.hull.verts.iter().map(|v| *v + self.lp).collect();
        let (mut hmin, mut hmax) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for v in &verts {
            hmin = hmin.min(*v);
            hmax = hmax.max(*v);
        }
        let end = dir * dist;
        let (smin, smax) = (hmin.min(hmin + end) - Vec3::splat(SKIN), hmax.max(hmax + end) + Vec3::splat(SKIN));

        // Every triangle that is hit, with its time of impact.
        let mut hits: Vec<(f32, usize)> = Vec::new();
        let mut best_t = dist;
        for (i, tri) in self.triangles.iter().enumerate() {
            let (tmin, tmax) = (tri.v[0].min(tri.v[1]).min(tri.v[2]), tri.v[0].max(tri.v[1]).max(tri.v[2]));
            if tmin.cmpgt(smax).any() || tmax.cmplt(smin).any() {
                continue;
            }
            if let Some(t) = self.sweep_triangle(&verts, tri, dir, best_t + CONTACT_TOLERANCE) {
                best_t = best_t.min(t);
                hits.push((t, i));
            }
        }
        // Several triangles can be touched at once (e.g. a step's riser and tread share an edge).
        // Like the client's contact list, pick the one whose plane opposes the motion the most,
        // so that the riser blocks and the tread does not pretend to.
        let mut best = Sweep { t: dist, tri: None };
        let mut best_dot = f32::MAX;
        for &(t, i) in &hits {
            if t > best_t + CONTACT_TOLERANCE {
                continue;
            }
            let d = self.triangles[i].n.dot(dir).abs();
            let opposing = -d; // more negative = more opposing
            if best.tri.is_none() || opposing < best_dot - 1e-6 {
                best = Sweep { t: best_t.min(t), tri: Some(i) };
                best_dot = opposing;
            }
        }
        if best.tri.is_some() {
            best.t = (best_t - SKIN).max(0.0);
        }
        best
    }

    /// Exact swept hull vs triangle (separating axes). Returns the time of impact in `[0, limit]`.
    fn sweep_triangle(&self, verts: &[Vec3], tri: &Tri, dir: Vec3, limit: f32) -> Option<f32> {
        let tv = tri.v;
        let tri_edges = [tv[1] - tv[0], tv[2] - tv[1], tv[0] - tv[2]];
        let mut axes: Vec<Vec3> = Vec::with_capacity(32);
        axes.push(tri.n);
        axes.extend(self.hull.face_normals.iter().copied());
        for e in &self.hull.edge_dirs {
            for te in &tri_edges {
                axes.push(e.cross(*te));
            }
        }

        let (mut t_enter, mut t_exit) = (f32::MIN, f32::MAX);
        let mut enter_axis = Vec3::ZERO;
        for a in axes {
            let len = a.length();
            if len < 1e-6 {
                continue;
            }
            let a = a / len;
            let (mut bmin, mut bmax) = (f32::MAX, f32::MIN);
            for v in verts {
                let d = v.dot(a);
                bmin = bmin.min(d);
                bmax = bmax.max(d);
            }
            let (mut tmin, mut tmax) = (f32::MAX, f32::MIN);
            for v in &tv {
                let d = v.dot(a);
                tmin = tmin.min(d);
                tmax = tmax.max(d);
            }
            let vel = dir.dot(a);
            if vel.abs() < 1e-7 {
                // Parallel: must overlap (touching counts as separated).
                if bmax <= tmin + 1e-5 || bmin >= tmax - 1e-5 {
                    return None;
                }
                continue;
            }
            let (enter, exit) = if vel > 0.0 {
                ((tmin - bmax) / vel, (tmax - bmin) / vel)
            } else {
                ((tmax - bmin) / vel, (tmin - bmax) / vel)
            };
            if enter > t_enter {
                t_enter = enter;
                enter_axis = if vel > 0.0 { -a } else { a };
            }
            t_exit = t_exit.min(exit);
            if t_enter > t_exit || t_exit < 0.0 || t_enter > limit {
                return None;
            }
        }
        if t_enter < 0.0 {
            // Already overlapping: only a hit if we are moving further into the triangle.
            if dir.dot(enter_axis) >= 0.0 || t_enter < -0.25 {
                return None;
            }
            return Some(0.0);
        }
        Some(t_enter)
    }

    /// Triangle normal facing against `dir` (the sweep direction).
    fn oriented(&self, tri: usize, dir: Vec3) -> Vec3 {
        let n = self.triangles[tri].n;
        if n.dot(dir) > 0.0 {
            -n
        } else {
            n
        }
    }

    /// Normal of a surface met by a downward cast: facing up.
    fn oriented_up(&self, tri: usize) -> Vec3 {
        let n = self.triangles[tri].n;
        if n.z < 0.0 {
            -n
        } else {
            n
        }
    }

    // ---- classification ---------------------------------------------------------------------

    /// `FUN_006334a0`: is the surface walkable (slide along it) rather than a wall (try to step)?
    fn walkable(&self, n: Vec3) -> bool {
        if n.z > COS_50 {
            return true;
        }
        if n.z >= 0.0 {
            // Steep, upward-facing: counts as walkable only while climbing steps.
            return self.flags & STEPPED != 0;
        }
        false
    }

    /// `FUN_00635d80`: slide vector for a wall (horizontal, pushes out along the wall normal).
    fn wall_slide(&self, dir: Vec3, dist: f32, n: Vec3) -> Vec3 {
        if n.z < 0.0 && -n.z <= COS_50 {
            return Vec3::ZERO;
        }
        let l = (n.x * n.x + n.y * n.y).sqrt();
        let (nx, ny) = if l >= EPS_DIR { (n.x / l, n.y / l) } else { (n.x, n.y) };
        let mut proj = (-n.x * dir.x - n.y * dir.y - n.z * dir.z) * dist;
        let denom = nx * n.x + ny * n.y;
        if denom.abs() >= EPS_DIR {
            proj /= denom;
        }
        Vec3::new(nx * (proj + MS), ny * (proj + MS), 0.0)
    }

    /// `FUN_00635c00`: vertical offset that keeps the move on a slope's plane, clamped to the
    /// reach; shortens `remaining` when the slope is too tall.
    fn slope_project(&self, dir: Vec3, remaining: &mut f32, n: Vec3) -> Vec3 {
        let reach = self.reach;
        let rise_num = (-n.x * dir.x - n.y * dir.y - n.z * dir.z) * *remaining;
        let rise = if n.z.abs() < EPS_DIR {
            if rise_num >= 0.0 {
                f32::MAX
            } else {
                f32::MIN
            }
        } else {
            rise_num / n.z
        };
        let z;
        if self.flags & STEPPED == 0 || rise >= 0.0 {
            if rise <= reach {
                if -reach <= rise {
                    z = rise;
                } else {
                    *remaining = -((reach / rise) * *remaining);
                    z = -reach;
                }
            } else {
                *remaining = (reach / rise) * *remaining;
                z = reach;
            }
        } else {
            *remaining = 0.0;
            z = reach;
        }
        Vec3::new(0.0, 0.0, z)
    }

    // ---- step probe -------------------------------------------------------------------------

    /// `FUN_00636100`: can the unit step over the obstacle it just hit? Leaves the position
    /// unchanged; the caller applies the step via `slope_project`.
    fn step_probe(&mut self, dir: Vec2, n: Vec3) -> Step {
        let saved = self.lp;
        let reach = self.reach;
        let mut probe = self.radius + SKIN;
        if probe < reach * TAN_50 {
            probe = reach * TAN_50;
        }
        let mut d0 = dir;
        if n.z >= 0.0 && n.z < COS_50 {
            let l = (n.x * n.x + n.y * n.y).sqrt();
            if l > 0.0 {
                let into = Vec3::new(-n.x / l, -n.y / l, 0.0);
                let s = self.sweep(into, probe);
                // Only keep the into-the-wall direction when we hit that same surface.
                if let Some(i) = s.tri {
                    let hn = self.triangles[i].n;
                    if (hn - n).length() < 1e-4 || (hn + n).length() < 1e-4 {
                        d0 = Vec2::new(into.x, into.y);
                    }
                }
            }
        }

        // Headroom: how far up can the unit go?
        let up = self.sweep(Vec3::Z, reach);
        if up.t.abs() < EPS_DIR {
            return Step::No;
        }
        let mut raised = up.t;
        self.lp.z += raised;

        // Forward at the raised height.
        let fwd = Vec3::new(d0.x, d0.y, 0.0);
        let s = self.sweep(fwd, probe);
        self.lp += fwd * s.t;
        let mut travelled = s.t;
        let mut hit = s.tri;
        if (s.t - probe).abs() >= EPS_LEN {
            if let Some(i) = s.tri {
                let hn = self.oriented(i, fwd);
                if self.walkable(hn) {
                    // Hit a walkable slope: slide up it.
                    let mut remaining = probe - s.t;
                    let slide = self.slope_project(fwd, &mut remaining, hn);
                    let v = Vec3::new(slide.x + fwd.x * remaining, slide.y + fwd.y * remaining, slide.z);
                    let vl = v.length();
                    if vl >= EPS_DIR {
                        let u = v / vl;
                        let s2 = self.sweep(u, vl);
                        self.lp += u * s2.t;
                        travelled += Vec2::new(u.x * s2.t, u.y * s2.t).length();
                        raised += u.z * s2.t;
                        hit = s2.tri;
                    }
                }
            }
        }

        let mut result = Step::No;
        if hit.is_none() || self.radius < travelled {
            // Drop back down and see what we land on.
            let down = self.sweep(-Vec3::Z, raised + STEP_MARGIN);
            self.lp.z -= down.t;
            let landing_ok = match down.tri {
                None => true,
                Some(i) => self.oriented_up(i).z > COS_50,
            };
            if landing_ok {
                result = Step::Yes;
            }
        }
        self.lp = saved;
        if result == Step::Yes && self.flags & STEPPED == 0 {
            self.last_ground_z = self.z();
        }
        result
    }

    // ---- ground move ------------------------------------------------------------------------

    /// `FUN_006367b0`. Returns the milliseconds consumed (0 if the unit started to fall).
    fn ground_move(&mut self, time_ms: u32, dist: f32, dir2: Vec2) -> u32 {
        let total_time = time_ms as f32;
        if dist.abs() < EPS_LEN {
            return time_ms;
        }
        if self.triangles.is_empty() {
            self.start_fall(0.0);
            return 0;
        }

        let mut remaining_dist = dist;
        let mut horiz_left = dist;
        let mut remaining_time = total_time;
        let mut time_used = 0.0f32;
        let mut dir = Vec3::new(dir2.x, dir2.y, 0.0);
        let mut step_dir = dir2;
        let mut iter = 0u32;
        let mut gave_up = false;
        let mut blocked = false;
        let mut sw = self.sweep(dir, remaining_dist);
        let mut last_t;

        // The main collide-and-slide loop. `done` means "skip ground snapping".
        let mut done = false;
        loop {
            self.lp += dir * sw.t;
            last_t = sw.t;
            let Some(ti) = sw.tri else {
                // Nothing hit: spend any time left at the original heading.
                time_used += remaining_time;
                let rest = total_time - time_used;
                if rest >= 1.0 {
                    dir = Vec3::new(dir2.x, dir2.y, 0.0);
                    step_dir = dir2;
                    remaining_dist = (rest / total_time) * dist;
                    remaining_time = rest;
                    sw = self.sweep(dir, remaining_dist);
                    continue;
                }
                break;
            };

            let frac_time = (sw.t / remaining_dist) * remaining_time;
            remaining_dist -= sw.t;
            horiz_left -= Vec2::new(dir.x * sw.t, dir.y * sw.t).length();
            let iter_cur;
            if frac_time < 1.0 {
                iter_cur = iter + 1;
                if iter_cur > 5 {
                    gave_up = true;
                    done = true;
                    break;
                }
            } else {
                iter_cur = 1;
            }
            time_used += frac_time;
            remaining_time -= frac_time;
            if total_time - time_used < 1.0 {
                if (total_time - time_used).abs() >= EPS_LEN {
                    gave_up = true;
                }
                done = true;
                break;
            }

            let n = self.oriented(ti, dir);
            let slide: Vec3;
            let mut prev_dist = remaining_dist;
            if self.walkable(n) {
                slide = self.slope_project(dir, &mut remaining_dist, n);
            } else {
                gave_up = true;
                match self.step_probe(step_dir, n) {
                    Step::Yes => {
                        self.flags |= STEPPED;
                        prev_dist = remaining_dist;
                        slide = self.slope_project(dir, &mut remaining_dist, n);
                    }
                    Step::No => {
                        slide = self.wall_slide(dir, remaining_dist, n);
                        prev_dist = remaining_dist;
                    }
                }
            }
            if prev_dist.abs() > 0.0 {
                remaining_time = (remaining_dist / prev_dist) * remaining_time;
            }

            // New motion = what is left of the old direction plus the slide.
            let v = Vec3::new(dir.x * remaining_dist + slide.x, dir.y * remaining_dist + slide.y, dir.z * remaining_dist + slide.z);
            remaining_dist = v.length();
            if remaining_dist < MS {
                gave_up = true;
                done = true;
                break;
            }
            dir = v / remaining_dist;
            let horiz = Vec2::new(v.x, v.y).length();
            step_dir = if horiz >= EPS_DIR { Vec2::new(v.x, v.y) / horiz } else { Vec2::new(v.x, v.y) };
            if horiz_left - horiz > EPS_LEN {
                gave_up = true;
            }
            iter = iter_cur;
            horiz_left = horiz;

            // While climbing, never rise more than `reach` above the last ground.
            if self.flags & STEPPED != 0 && v.z > EPS_LEN {
                let target = v.z + self.z();
                let limit = self.reach + self.last_ground_z;
                if limit < target {
                    let allowed = limit - self.z();
                    remaining_dist = (allowed / v.z) * remaining_dist;
                    if (allowed - v.z).abs() >= EPS_LEN {
                        gave_up = true;
                    }
                    if remaining_dist < MS {
                        blocked = true;
                        done = true;
                        break;
                    }
                }
            }
            sw = self.sweep(dir, remaining_dist);
        }

        if !done {
            // Ground snap: cast down far enough to follow the surface (and the steps just taken).
            let mut drop = last_t * FOOT_RATIO + SNAP_BASE;
            if self.flags & STEPPED != 0 {
                drop += self.reach;
            }
            let down = self.sweep(-Vec3::Z, drop);
            self.lp.z -= down.t;
            match down.tri {
                None => blocked = true,
                Some(i) => {
                    let n = self.oriented_up(i);
                    if n.z > COS_50 {
                        self.flags &= !STEPPED;
                    } else {
                        match self.step_probe(step_dir, n) {
                            Step::Yes => self.flags |= STEPPED,
                            Step::No => blocked = true,
                        }
                    }
                }
            }
            if down.tri.is_none() || blocked {
                self.start_fall(0.0);
                return time_ms;
            }
        } else if blocked {
            self.start_fall(0.0);
            return time_ms;
        }
        let _ = gave_up;
        time_ms
    }

    // ---- falling ----------------------------------------------------------------------------

    /// `FUN_00635b00`: one fall step of `time_ms` with horizontal distance `dist` along `dir2`.
    fn fall_move(&mut self, time_ms: u32, dist: f32, dir2: Vec2) -> u32 {
        if time_ms == 0 {
            return 0;
        }
        let t_new = (self.fall_time_ms + time_ms as f32) * MS;
        // FUN_007c6110: distance still to fall this step = d(t) + (z - startZ).
        let delta = self.fall_distance(t_new) + (self.z() - self.fall_start_z);
        let v = Vec3::new(dist * dir2.x, dist * dir2.y, -delta);
        if v.length() < EPS_DIR {
            self.fall_time_ms += time_ms as f32;
            return time_ms;
        }
        if self.triangles.is_empty() {
            self.lp += v;
            self.fall_time_ms += time_ms as f32;
            return time_ms;
        }
        self.fall_slice(time_ms, v);
        self.fall_time_ms += time_ms as f32;
        time_ms
    }

    /// `FUN_00635600`: move along `v`, landing on walkable surfaces and sliding along the rest.
    fn fall_slice(&mut self, _time_ms: u32, v: Vec3) {
        let mut remaining = v;
        for _ in 0..6 {
            let len = remaining.length();
            if len < EPS_LEN {
                return;
            }
            let dir = remaining / len;
            let s = self.sweep(dir, len);
            self.lp += dir * s.t;
            let Some(i) = s.tri else { return };
            let n = self.oriented(i, dir);
            if remaining.z < 0.0 && n.z > COS_50 {
                self.land();
                return;
            }
            // Slide along the surface with what is left.
            let left = remaining - dir * s.t;
            remaining = left - n * left.dot(n) + n * SKIN;
        }
    }

    /// Landing: back to ground movement.
    fn land(&mut self) {
        self.flags &= !(FALLING | STEPPED | 0x4000);
        self.fall_time_ms = 0.0;
        self.fall_v0 = 0.0;
        self.fall_start_z = self.z();
        self.last_ground_z = self.z();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed list of triangles (WoW coordinates).
    struct Scene(Vec<Tri>);

    impl TriangleSource for Scene {
        fn gather(&self, origin: [f64; 3], min: Vec3, max: Vec3, out: &mut Vec<Tri>) {
            let o = Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32);
            let (lo, hi) = (o + min, o + max);
            for t in &self.0 {
                let tmin = t.v[0].min(t.v[1]).min(t.v[2]);
                let tmax = t.v[0].max(t.v[1]).max(t.v[2]);
                if tmin.cmple(hi).all() && tmax.cmpge(lo).all() {
                    out.push(Tri { n: t.n, v: [t.v[0] - o, t.v[1] - o, t.v[2] - o] });
                }
            }
        }
    }

    fn quad(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> Vec<Tri> {
        [Tri::new(a, b, c), Tri::new(a, c, d)].into_iter().flatten().collect()
    }

    fn floor(z: f32) -> Vec<Tri> {
        quad(Vec3::new(-200.0, -200.0, z), Vec3::new(200.0, -200.0, z), Vec3::new(200.0, 200.0, z), Vec3::new(-200.0, 200.0, z))
    }

    /// Run `seconds` at 60 fps with a constant velocity (x north, y west).
    fn run(u: &mut Unit, s: &Scene, seconds: f32, v: Vec2) {
        for _ in 0..(seconds * 60.0) as usize {
            u.update(s, 1.0 / 60.0, v);
        }
    }

    fn unit_at(x: f64, y: f64, z: f64) -> Unit {
        Unit::new([x, y, z])
    }

    #[test]
    fn falls_and_lands_on_flat_ground() {
        let s = Scene(floor(10.0));
        let mut u = unit_at(0.0, 0.0, 40.0);
        run(&mut u, &s, 3.0, Vec2::ZERO);
        assert!(!u.is_falling());
        assert!((u.pos[2] - 10.0).abs() < 0.01, "z = {}", u.pos[2]);
    }

    #[test]
    fn fall_follows_the_clients_gravity() {
        // d(t) = g t^2 / 2 until terminal speed.
        let s = Scene(floor(-1000.0));
        let mut u = unit_at(0.0, 0.0, 0.0);
        run(&mut u, &s, 1.0, Vec2::ZERO);
        let fallen = -u.pos[2] as f32;
        let expected = 0.5 * GRAVITY * 1.0;
        assert!((fallen - expected).abs() < 0.25, "fell {fallen}, expected {expected}");
    }

    #[test]
    fn walks_at_run_speed_on_flat_ground() {
        let s = Scene(floor(0.0));
        let mut u = unit_at(0.0, 0.0, 0.0);
        run(&mut u, &s, 0.5, Vec2::ZERO);
        let start = u.pos;
        run(&mut u, &s, 2.0, Vec2::new(7.0, 0.0));
        let dist = u.pos[0] - start[0];
        assert!((dist - 14.0).abs() < 0.3, "ran {dist} yards");
        assert!((u.pos[2] - 0.0).abs() < 0.01 && !u.is_falling());
    }

    #[test]
    fn a_wall_stops_the_unit() {
        let mut tris = floor(0.0);
        // wall at x = 5, spanning y in [-20, 20], z in [0, 8]
        tris.extend(quad(Vec3::new(5.0, -20.0, 0.0), Vec3::new(5.0, 20.0, 0.0), Vec3::new(5.0, 20.0, 8.0), Vec3::new(5.0, -20.0, 8.0)));
        let s = Scene(tris);
        let mut u = unit_at(0.0, 0.0, 0.0);
        run(&mut u, &s, 0.5, Vec2::ZERO);
        run(&mut u, &s, 3.0, Vec2::new(7.0, 0.0));
        assert!(u.pos[0] < 5.0 && u.pos[0] > (5.0 - u.radius - 0.2) as f64, "stopped at x = {}", u.pos[0]);
    }

    #[test]
    fn slides_along_a_diagonal_wall() {
        let mut tris = floor(0.0);
        // wall along the line y = x, from (-50,-50) to (50,50)
        tris.extend(quad(Vec3::new(-50.0, -50.0, 0.0), Vec3::new(50.0, 50.0, 0.0), Vec3::new(50.0, 50.0, 8.0), Vec3::new(-50.0, -50.0, 8.0)));
        let s = Scene(tris);
        let mut u = unit_at(3.0, 0.0, 0.0); // below the line y=x (y < x)
        run(&mut u, &s, 0.3, Vec2::ZERO);
        run(&mut u, &s, 2.0, Vec2::new(0.0, 7.0)); // walk towards +y, into the wall
        // still on its original side, and progressed along the wall (x and y both grew)
        assert!(u.pos[1] <= u.pos[0] + 0.01, "passed through: ({}, {})", u.pos[0], u.pos[1]);
        assert!(u.pos[0] > 8.0, "did not slide: ({}, {})", u.pos[0], u.pos[1]);
    }

    /// Stairs rising towards +x: `n` steps, each `rise` high and `run` deep, with risers.
    fn stairs(n: usize, rise: f32, run_len: f32, start_x: f32, width: f32) -> Vec<Tri> {
        let mut tris = Vec::new();
        for i in 0..n {
            let (x0, x1) = (start_x + i as f32 * run_len, start_x + (i + 1) as f32 * run_len);
            let top = rise * (i + 1) as f32;
            tris.extend(quad(Vec3::new(x0, -width, top), Vec3::new(x1, -width, top), Vec3::new(x1, width, top), Vec3::new(x0, width, top)));
            tris.extend(quad(Vec3::new(x0, -width, top - rise), Vec3::new(x0, width, top - rise), Vec3::new(x0, width, top), Vec3::new(x0, -width, top)));
        }
        tris
    }

    #[test]
    fn climbs_stairs_with_risers() {
        let mut tris = floor(0.0);
        tris.extend(stairs(10, 0.33, 0.45, 4.0, 1.5));
        let s = Scene(Scene(tris).0);
        let mut u = unit_at(0.0, 0.0, 0.0);
        run(&mut u, &s, 0.3, Vec2::ZERO);
        // 10.5 yd of running; the staircase ends at x = 8.5 with no landing, so the unit walks off the
        // top at the end. What matters is the highest point it reached on the way.
        let mut highest = 0.0f64;
        for _ in 0..90 {
            u.update(&s, 1.0 / 60.0, Vec2::new(7.0, 0.0));
            highest = highest.max(u.pos[2]);
        }
        assert!(highest > 3.2, "only climbed to {highest}");
        assert!(u.pos[0] > 9.0, "stuck at x = {}", u.pos[0]);
    }

    #[test]
    fn a_wide_ledge_up_to_the_reach_can_be_stepped_onto_but_not_beyond() {
        for (height, expect_up) in [(0.5f32, true), (0.9, true), (1.4, false)] {
            let mut tris = floor(0.0);
            tris.extend(quad(Vec3::new(5.0, -20.0, height), Vec3::new(40.0, -20.0, height), Vec3::new(40.0, 20.0, height), Vec3::new(5.0, 20.0, height)));
            tris.extend(quad(Vec3::new(5.0, -20.0, 0.0), Vec3::new(5.0, 20.0, 0.0), Vec3::new(5.0, 20.0, height), Vec3::new(5.0, -20.0, height)));
            let s = Scene(tris);
            let mut u = unit_at(0.0, 0.0, 0.0);
            run(&mut u, &s, 0.3, Vec2::ZERO);
            run(&mut u, &s, 2.0, Vec2::new(7.0, 0.0));
            let up = u.pos[2] > (height - 0.05) as f64 && u.pos[0] > 6.0;
            assert_eq!(up, expect_up, "ledge {height}: ended at x = {:.2}, z = {:.2}", u.pos[0], u.pos[2]);
        }
    }

    #[test]
    fn walks_down_stairs_without_falling() {
        let mut tris = floor(0.0);
        tris.extend(stairs(10, 0.33, 0.45, 4.0, 1.5));
        // a landing on top, so the unit can start up there
        tris.extend(quad(Vec3::new(8.5, -1.5, 3.3), Vec3::new(14.0, -1.5, 3.3), Vec3::new(14.0, 1.5, 3.3), Vec3::new(8.5, 1.5, 3.3)));
        let s = Scene(tris);
        let mut u = unit_at(11.0, 0.0, 3.3);
        run(&mut u, &s, 0.3, Vec2::ZERO);
        let mut fell = false;
        for _ in 0..(1.6 * 60.0) as usize {
            u.update(&s, 1.0 / 60.0, Vec2::new(-7.0, 0.0));
            if u.is_falling() && u.pos[2] > 0.3 {
                fell = true; // a short airborne frame is allowed only if it lands again immediately
            }
        }
        assert!(u.pos[2] < 0.1, "did not descend: z = {}", u.pos[2]);
        let _ = fell;
    }

    #[test]
    fn a_steep_slope_blocks_and_a_gentle_one_does_not() {
        for (rise_per_yd, expect_up) in [(0.5f32, true), (2.0, false)] {
            // ramp from x = 5 rising with the given slope for 20 yd
            let top = rise_per_yd * 20.0;
            let mut tris = floor(0.0);
            tris.extend(quad(Vec3::new(5.0, -20.0, 0.0), Vec3::new(25.0, -20.0, top), Vec3::new(25.0, 20.0, top), Vec3::new(5.0, 20.0, 0.0)));
            let s = Scene(tris);
            let mut u = unit_at(0.0, 0.0, 0.0);
            run(&mut u, &s, 0.3, Vec2::ZERO);
            run(&mut u, &s, 2.0, Vec2::new(7.0, 0.0));
            assert_eq!(u.pos[2] > 2.0, expect_up, "slope {rise_per_yd}: z = {}, x = {}", u.pos[2], u.pos[0]);
        }
    }

    #[test]
    fn jump_apex_matches_ballistics() {
        let s = Scene(floor(0.0));
        let mut u = unit_at(0.0, 0.0, 0.0);
        run(&mut u, &s, 0.5, Vec2::ZERO);
        u.jump(7.955_547, Vec2::ZERO);
        let mut apex = 0.0f64;
        for _ in 0..120 {
            u.update(&s, 1.0 / 120.0, Vec2::ZERO);
            apex = apex.max(u.pos[2]);
        }
        let expected = 7.955_547f64.powi(2) / (2.0 * GRAVITY as f64);
        assert!((apex - expected).abs() < 0.06, "apex {apex}, expected {expected}");
        assert!(!u.is_falling() && u.pos[2].abs() < 0.01, "should have landed, z = {}", u.pos[2]);
    }

    #[test]
    fn a_swimmer_moves_in_three_dimensions_and_slides_along_the_bottom() {
        let s = Scene(floor(0.0));
        let mut u = unit_at(0.0, 0.0, 0.0);
        for _ in 0..60 {
            u.swim(&s, 1.0 / 60.0, Vec3::new(3.0, 0.0, -0.5));
        }
        assert!((u.pos[0] - 3.0).abs() < 0.05 && u.pos[2] >= -0.01, "slid, {:?}", u.pos);
        for _ in 0..60 {
            u.swim(&s, 1.0 / 60.0, Vec3::new(0.0, 0.0, 3.0));
        }
        u.pos = [0.0, 0.0, 0.0];
        for _ in 0..60 {
            u.swim(&s, 1.0 / 60.0, Vec3::new(3.0, 0.0, 3.0));
        }
        assert!((u.pos[0] - 3.0).abs() < 0.05 && (u.pos[2] - 3.0).abs() < 0.05, "{:?}", u.pos);
        for _ in 0..120 {
            u.swim(&s, 1.0 / 60.0, Vec3::new(0.0, 2.0, -3.0));
        }
        assert!(u.pos[2] >= -0.01 && (u.pos[1] - 4.0).abs() < 0.2, "slid along the floor, {:?}", u.pos);
    }

    #[test]
    fn a_walkable_unit_does_not_fall_through_cracks_between_treads() {
        let mut tris = floor(-5.0);
        // start platform
        tris.extend(quad(Vec3::new(-5.0, -2.0, 0.0), Vec3::new(4.0, -2.0, 0.0), Vec3::new(4.0, 2.0, 0.0), Vec3::new(-5.0, 2.0, 0.0)));
        for i in 0..8 {
            // Step i: riser at x_i (0.33 high), tread 1 yd deep behind it; the tread ends 0.35 yd
            // before the next riser, leaving a crack in the walkable surface.
            let x = 4.0 + i as f32 * 1.35;
            let top = 0.33 * (i + 1) as f32;
            tris.extend(quad(Vec3::new(x, -1.5, top - 0.33), Vec3::new(x, 1.5, top - 0.33), Vec3::new(x, 1.5, top), Vec3::new(x, -1.5, top)));
            tris.extend(quad(Vec3::new(x, -1.5, top), Vec3::new(x + 1.0, -1.5, top), Vec3::new(x + 1.0, 1.5, top), Vec3::new(x, 1.5, top)));
        }
        let s = Scene(tris);
        let mut u = unit_at(0.0, 0.0, 0.0);
        run(&mut u, &s, 0.3, Vec2::ZERO);
        run(&mut u, &s, 1.3, Vec2::new(7.0, 0.0));
        assert!(u.pos[2] > 1.0 && !u.is_falling(), "fell through the stairs: z = {}", u.pos[2]);
    }
}
