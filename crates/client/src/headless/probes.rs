//! Diagnostic probes of the loaded world, asked for by command line switches: they print or log, nothing is drawn.

use super::*;

fn floor_map(args: &Args, world: &World, camera: &Camera, z_given: bool) {
    // 1 yard per character, north at the top. ' ' = nothing, '.' = terrain level,
    // digit = floor that many yards above the terrain, '+' = 10 or more, '-' = below it.
    let (cx, cz) = (camera.pos.x, camera.pos.z);
    println!("floor map centred on ({:.0}, {:.0}); north up, 1 char = 1 yard, 80x50", render_to_wow(camera.pos)[0], render_to_wow(camera.pos)[1]);
    for row in -25..25 {
        let line: String = (-40..40)
            .map(|col| {
                let (x, z) = (cx + col as f32 * args.floor_scale, cz + row as f32 * args.floor_scale);
                let terrain = world.height_at(x, z);
                match (world.floor(x, z, if z_given { camera.pos.y + 1.0 } else { 10_000.0 }), terrain) {
                    (None, _) => ' ',
                    (Some(_), None) => '#',
                    (Some(f), Some(t)) => match f - t {
                        d if d < -0.3 => '-',
                        d if d < 0.3 => '.',
                        d if d < 9.5 => char::from_digit((d + 0.5) as u32, 10).unwrap_or('+'),
                        _ => '+',
                    },
                }
            })
            .collect();
        println!("{line}");
    }
}

fn find_holes(world: &World, camera: &Camera) {
    // A hole: nothing walkable within [top-3.5, top] at this spot, yet walkable surface at that height on both sides.
    let top = camera.pos.y;
    let has = |x: f32, z: f32| world.floor(x, z, top).is_some_and(|f| f > top - 3.5);
    let mut holes = Vec::new();
    for iz in -75..=75 {
        for ix in -75..=75 {
            let (x, z) = (camera.pos.x + ix as f32 * 0.2, camera.pos.z + iz as f32 * 0.2);
            if has(x, z) {
                continue;
            }
            let d = 0.5;
            if (has(x - d, z) && has(x + d, z)) || (has(x, z - d) && has(x, z + d)) {
                holes.push((x, z));
            }
        }
    }
    log::info!("holes: {} grid cells (0.2 yd) have no floor but floor on opposite sides", holes.len());
    // Cluster into 1 yd cells for a readable list.
    let mut clusters = std::collections::BTreeMap::<(i32, i32), usize>::new();
    for (x, z) in &holes {
        *clusters.entry((x.floor() as i32, z.floor() as i32)).or_default() += 1;
    }
    for ((cx, cz), n) in clusters.iter().take(25) {
        let [wx, wy, _] = render_to_wow(glam::Vec3::new(*cx as f32 + 0.5, 0.0, *cz as f32 + 0.5));
        log::info!("  {n:>3} cells near wow ({wx:.1}, {wy:.1})");
    }
}

fn stress_walk(world: &World, camera: &Camera) {
    // Random steering from random starts on the floor near --pos; report drops after a climb.
    let mut rng = 0x2545_F491u32;
    let mut next = move || {
        rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (rng >> 8) as f32 / (1u32 << 24) as f32
    };
    let centre = camera.pos;
    let (mut runs, mut falls) = (0, 0);
    let mut by_height = std::collections::BTreeMap::<i32, usize>::new();
    for _ in 0..600 {
        let (x, zz) = (centre.x + (next() - 0.5) * 6.0, centre.z + (next() - 0.5) * 6.0);
        let Some(ground) = world.floor(x, zz, centre.y) else { continue };
        let mut cam = Camera::new(glam::Vec3::new(x, ground + EYE_HEIGHT, zz), (80.0 + next() * 80.0).to_radians(), 0.0);
        let mut c = Controller::new(Mode::Walk);
        let mut input = Input { forward: true, ..Default::default() };
        runs += 1;
        let mut best = ground;
        let mut prev = cam.pos;
        let mut fast_frames = 0;
        let mut trace: Vec<String> = Vec::new();
        let mut history: Vec<f32> = Vec::new();
        for frame in 0..(4 * 60) {
            if frame % 20 == 0 {
                cam.yaw += (next() - 0.5) * 50f32.to_radians();
            }
            prev = cam.pos;
            c.update(&mut cam, &mut input, world, 1.0 / 60.0);
            let feet = cam.pos.y - EYE_HEIGHT;
            history.push(feet);
            {
                let [tx, ty, tz] = render_to_wow(glam::Vec3::new(cam.pos.x, feet, cam.pos.z));
                trace.push(format!(
                    "f{frame:03} wow ({tx:.2},{ty:.2}) feet {tz:.3} floor@step {:?} yaw {:.0}",
                    world.floor(cam.pos.x, cam.pos.z, feet + 0.8).map(|v| (v * 1000.0).round() / 1000.0),
                    cam.yaw.to_degrees().rem_euclid(360.0)
                ));
            }
            best = best.max(feet);
            // A fall is sustained downward speed beyond what descending stairs reaches (~7 yd/s at 45 degrees).
            let speed = (prev.y - cam.pos.y) * 60.0;
            fast_frames = if speed > 9.0 { fast_frames + 1 } else { 0 };
            if best - ground > 0.6 && fast_frames >= 3 {
                falls += 1;
                if falls == 3 {
                    for line in trace.iter().rev().take(14).rev() {
                        log::info!("  {line}");
                    }
                }
                let pf = prev.y - EYE_HEIGHT;
                let started_from = history[history.len().saturating_sub(25)];
                let _ = pf;
                *by_height.entry((started_from * 2.0).round() as i32).or_default() += 1;
                let [wx, wy, wz] = render_to_wow(glam::Vec3::new(prev.x, pf, prev.z));
                log::debug!(
                    "fall at t={:.2}s: feet were ({wx:.2}, {wy:.2}, {wz:.2}) yaw {:.0}; floor within step {:?}, floor within 3 yd {:?}, floor below {:?}",
                    frame as f32 / 60.0,
                    cam.yaw.to_degrees().rem_euclid(360.0),
                    world.floor(prev.x, prev.z, pf + 0.8),
                    world.floor(prev.x, prev.z, pf + 3.0),
                    world.floor(prev.x, prev.z, pf - 0.01)
                );
                break;
            }
        }
        let _ = prev;
    }
    log::info!("stress: {falls} falls in {runs} runs");
    for (h, n) in &by_height {
        log::info!("  fall started near feet height {:.1}: {n}", *h as f32 / 2.0);
    }
}

/// Whether anything of the content stands in or under a building, a tree or a wall: objects with something at head height
/// within two yards of them.
fn check_spawns(world: &World) {
    let content = crate::content::get();
    let places = content.spawns.iter().map(|s| (s.creature.as_str(), s.at)).chain(content.chests.iter().map(|c| (c.id.as_str(), c.at)));
    // A spawn at a height of its own (a floor of a building) must find that floor, or its creature drops to the ground below.
    for s in &content.spawns {
        let Some(h) = s.height else { continue };
        let p = crate::coords::wow_to_render([s.at.0, s.at.1, h]);
        let Some(ground) = world.height_at(p.x, p.z) else { continue };
        let floor = crate::collision::Collider::floor(world, p.x, p.z, p.y + 1.0);
        if p.y > ground + 1.5 && floor.is_none_or(|f| (f - p.y).abs() > 1.0) {
            log::warn!("{} at {:?}: stands at {h} (ground {ground:.1}), floor found {floor:?}", s.creature, s.at);
        }
    }
    for (name, at) in places {
        let p = crate::coords::wow_to_render([at.0, at.1, 0.0]);
        let Some(ground) = world.height_at(p.x, p.z) else { log::warn!("{name} at {at:?}: no ground loaded"); continue };
        let blocked = world.triangles_near(p.x, p.z, 2.0).iter().any(|t| t.iter().all(|v| v.y > ground + 0.5 && v.y < ground + 2.5));
        if blocked {
            log::warn!("{name} at {at:?}: something stands there");
        }
    }
    log::info!("checked the places of the content");
}

/// Build the navigation mesh round the camera (tiles within 100 yards), then say how the spawns of the content stand on it and how paths between neighbours come out.
fn navmesh_probe(args: &Args, world: &World, camera: &Camera) {
    use crate::navmesh::{from_nav, to_nav, NavMesh, PathFinder, HALF_WORLD, PATHFIND_NORMAL, TILE};
    let [north, west, _] = render_to_wow(camera.pos);
    let (ny, nx) = (((north as f64 + HALF_WORLD) / TILE).floor() as i32, ((west as f64 + HALF_WORLD) / TILE).floor() as i32);
    let began = std::time::Instant::now();
    let mut mesh = NavMesh::default();
    let span = (100.0 / TILE).ceil() as i32;
    for dy in -span..=span {
        for dx in -span..=span {
            if let Some(tile) = crate::navmesh::build(world, ny + dy, nx + dx) {
                mesh.add(&tile);
            }
        }
    }
    log::info!("navmesh: {} tiles of {} built in {:?}", mesh.tile_count(), (2 * span + 1).pow(2), began.elapsed());
    let mut kinds = [0usize; 4];
    for t in &mesh.tiles {
        for p in &t.polys {
            kinds[p.flags.trailing_zeros().min(3) as usize] += 1;
        }
    }
    log::info!("navmesh: polygons of ground {}, steep {}, water {}, magma or slime {}", kinds[0], kinds[1], kinds[2], kinds[3]);
    let near = |at: (f32, f32)| (at.0 - north).hypot(at.1 - west) < 90.0;
    let spawns: Vec<_> = crate::content::get().spawns.iter().filter(|s| near(s.at) && s.height.is_some() && s.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(&args.map))).collect();
    let q = crate::navmesh::Query { mesh: &mesh, filter: crate::navmesh::Filter { include: crate::navmesh::NAV_GROUND, exclude: 0 } };
    let (mut on, mut high, mut off) = (0, 0, Vec::new());
    for s in &spawns {
        let p = to_nav(glam::Vec3::new(s.at.0, s.at.1, s.height.unwrap_or(0.0)));
        match q.find_nearest_poly(p, glam::Vec3::splat(5.0)) {
            Some((_, c)) if (c.y - p.y).abs() < 1.5 => {
                on += 1;
                high += (c.y > world.height_at(-s.at.1, -s.at.0).unwrap_or(c.y) + 1.5) as i32;
            }
            _ => {
                let r = crate::coords::wow_to_render([s.at.0, s.at.1, s.height.unwrap_or(0.0)]);
                off.push((s.creature.as_str(), s.at, p.y, crate::collision::Collider::floor(world, r.x, r.z, r.y + 1.0)))
            }
        }
    }
    log::info!("navmesh: of {} spawns here {on} are on a polygon at their height ({high} of those above the ground), {} are not", spawns.len(), off.len());
    for (name, at, z, floor) in off.iter().take(8) {
        log::info!("  off the mesh: {name} at {at:?} height {z:.1}, the player's floor there {floor:?}");
    }
    // Paths between spawns 20 to 60 yards apart.
    let mut finder = PathFinder::new(false);
    let (mut paths, mut whole, mut longest, mut worst) = (0, 0, 0.0f32, 1.0f32);
    for (i, a) in spawns.iter().enumerate() {
        for b in spawns.iter().skip(i + 1).take(6) {
            let (pa, pb) = (glam::Vec3::new(a.at.0, a.at.1, a.height.unwrap_or(0.0)), glam::Vec3::new(b.at.0, b.at.1, b.height.unwrap_or(0.0)));
            let d = pa.distance(pb);
            if !(20.0..60.0).contains(&d) {
                continue;
            }
            finder.calculate(&mesh, pa, pb, false, false);
            paths += 1;
            if finder.kind == PATHFIND_NORMAL {
                whole += 1;
                let len: f32 = finder.path.windows(2).map(|w| w[0].distance(w[1])).sum();
                longest = longest.max(len);
                worst = worst.max(len / d);
            }
        }
    }
    log::info!("navmesh: {paths} paths between spawns, {whole} whole (longest {longest:.0} yd, at most {worst:.1} times the straight line)");
    let _ = from_nav;
}

fn dump_tris(world: &World, camera: &Camera) {
    let mut tris = world.triangles_near(camera.pos.x, camera.pos.z, 3.0);
    tris.sort_by(|a, b| (a[0].y + a[1].y + a[2].y).total_cmp(&(b[0].y + b[1].y + b[2].y)));
    for t in tris.iter().take(60) {
        let w: Vec<String> = t.iter().map(|v| { let [x, y, z] = render_to_wow(*v); format!("({x:.2},{y:.2},{z:.2})") }).collect();
        let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
        let slope = n.y.abs().clamp(0.0, 1.0).acos().to_degrees();
        log::info!("tri {} {} {}  slope {slope:.0} deg", w[0], w[1], w[2]);
    }
}

fn find_ramps(world: &World, camera: &Camera) {
    let mut clusters: std::collections::BTreeMap<(i32, i32, i32), usize> = std::collections::BTreeMap::new();
    for c in world.ramp_centroids(camera.pos.x, camera.pos.z, 40.0, 35.0, 50.0) {
        *clusters.entry(((c.x / 2.0).floor() as i32, (c.z / 2.0).floor() as i32, (c.y / 1.0).floor() as i32)).or_default() += 1;
    }
    let mut top: Vec<_> = clusters.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    log::info!("ramp-like surfaces near the camera (2x2 yd cells, by height):");
    for ((cx, cz, cy), n) in top.iter().take(15) {
        let [wx, wy, wz] = render_to_wow(glam::Vec3::new(*cx as f32 * 2.0 + 1.0, *cy as f32 + 0.5, *cz as f32 * 2.0 + 1.0));
        log::info!("  {n:>3} triangles around ({wx:.0}, {wy:.0}) at height {wz:.1}");
    }
}

fn floor_profile(args: &Args, world: &World, camera: &Camera, z_given: bool) {
    let heading = args.yaw_deg.to_radians();
    let dir = glam::Vec3::new(heading.sin(), 0.0, -heading.cos());
    let mut line = String::new();
    for i in 0..=80 {
        let p = camera.pos + dir * (i as f32 * 0.25);
        let max_y = if z_given { camera.pos.y + 0.3 } else { 10_000.0 };
        match world.floor(p.x, p.z, max_y) {
            Some(f) => line.push_str(&format!("{f:.2} ")),
            None => line.push_str("-- "),
        }
    }
    log::info!("floor every 0.25 yd along the heading (max height {:.1}):
{line}", camera.pos.y + 0.3);
}

fn probe_walk(world: &World, camera: &Camera, z_given: bool) {
    // Experiment: can the player reach raised walkable surfaces by walking alone?
    let centre = camera.pos;
    let (mut runs, mut raised, mut roofed, mut climbed) = (0, Vec::new(), Vec::new(), Vec::<(f32, [f32; 3])>::new());
    for dz in (-40..=40).step_by(2) {
        for dx in (-40..=40).step_by(2) {
            for heading in 0..8 {
                let (x, z) = (centre.x + dx as f32, centre.z + dz as f32);
                // With an explicit height in --pos, start on the highest floor at or below it.
                let start_ground = if z_given { world.floor(x, z, centre.y) } else { world.height_at(x, z) };
                let Some(ground) = start_ground else { continue };
                let mut cam = Camera::new(glam::Vec3::new(x, ground + EYE_HEIGHT, z), heading as f32 * std::f32::consts::FRAC_PI_4, 0.0);
                let mut c = Controller::new(Mode::Walk);
                let mut input = Input { forward: true, ..Default::default() };
                for _ in 0..(3 * 60) {
                    c.update(&mut cam, &mut input, world, 1.0 / 60.0);
                }
                runs += 1;
                if z_given && cam.pos.y - EYE_HEIGHT - ground > 2.0 {
                    climbed.push((cam.pos.y - EYE_HEIGHT - ground, render_to_wow(cam.pos - glam::Vec3::Y * EYE_HEIGHT)));
                }
                let feet = cam.pos.y - EYE_HEIGHT;
                let terrain = world.height_at(cam.pos.x, cam.pos.z).unwrap_or(feet);
                // A walkable surface 2.5..15 yards overhead means we are indoors (or under a bridge).
                if let Some(above) = world.floor(cam.pos.x, cam.pos.z, feet + 15.0) {
                    if above - feet > 2.5 && (feet - terrain).abs() < 0.6 {
                        roofed.push(render_to_wow(cam.pos - glam::Vec3::Y * EYE_HEIGHT));
                    }
                }
                if feet - terrain > 1.2 && feet - terrain < 30.0 {
                    raised.push((feet - terrain, render_to_wow(cam.pos - glam::Vec3::Y * EYE_HEIGHT)));
                }
            }
        }
    }
    if z_given {
        climbed.sort_by(|a, b| b.0.total_cmp(&a.0));
        log::info!("probe: {} of {runs} walks from the interior floor climbed more than 2 yd", climbed.len());
        // distinct landing spots, 3 yd cells
        let mut cells = std::collections::BTreeMap::<(i32, i32), (f32, [f32; 3])>::new();
        for c in &climbed {
            cells.entry(((c.1[0] / 3.0) as i32, (c.1[1] / 3.0) as i32)).or_insert(*c);
        }
        for (h, [x, y, zz]) in cells.values().take(10) {
            log::info!("  climbed +{h:.1} yd, ended at ({x:.1}, {y:.1}, {zz:.1})");
        }
    }
    log::info!("probe: {} walks ended under a roof at ground level", roofed.len());
    for [x, y, z] in roofed.iter().take(6) {
        log::info!("  indoors at ({x:.1}, {y:.1}, {z:.1})");
    }
    raised.sort_by(|a, b| b.0.total_cmp(&a.0));
    log::info!("probe: {runs} walks, {} ended standing on a floor more than 1.2 yd above the terrain", raised.len());
    for (h, [x, y, z]) in raised.iter().take(8) {
        log::info!("  +{h:.1} yd at ({x:.1}, {y:.1}, {z:.1})");
    }
}

pub(super) fn run(args: &Args, world: &World, camera: &Camera, z_given: bool) {
    if args.floor_map { floor_map(args, world, camera, z_given); }
    if args.find_holes { find_holes(world, camera); }
    if args.stress_walk { stress_walk(world, camera); }
    if args.check_spawns { check_spawns(world); }
    if args.navmesh_probe { navmesh_probe(args, world, camera); }
    if args.probe_creatures { probe_creatures(args, world, camera); }
    if args.dump_tris { dump_tris(world, camera); }
    if args.find_ramps { find_ramps(world, camera); }
    if args.floor_profile { floor_profile(args, world, camera, z_given); }
    if args.probe_walk { probe_walk(world, camera, z_given); }
}

/// Run the real controller against the real collision data, 60 steps per second, and report where it ends.
pub(super) fn walk(args: &Args, camera: &mut Camera, world: &World) {
        let mut controller = Controller::new(Mode::Walk);
    let mut input = Input { forward: true, ..Default::default() };
    let start = camera.pos;
    let mut swam = 0.0;
    for _ in 0..(args.walk * 60.0) as usize {
        let feet = camera.pos - glam::Vec3::Y * EYE_HEIGHT;
        input.water = world.liquid_at(feet.x, feet.z, feet.y);
        controller.update(camera, &mut input, world, 1.0 / 60.0);
        swam += controller.swimming() as u32 as f32 / 60.0;
    }
    log::info!("swam {swam:.1} s of it, water level at the end {:?}", input.water);
    let [sx, sy, sz] = render_to_wow(start - glam::Vec3::Y * EYE_HEIGHT);
    let [ex, ey, ez] = render_to_wow(camera.pos - glam::Vec3::Y * EYE_HEIGHT);
    let dist = ((ex - sx).powi(2) + (ey - sy).powi(2)).sqrt();
    log::info!(
        "walked {:.1}s: ({sx:.1}, {sy:.1}, {sz:.1}) -> ({ex:.1}, {ey:.1}, {ez:.1}), {dist:.1} yards on the ground plane (free run would be {:.1})",
        args.walk,
        args.walk * 7.0
    );
    let feet = camera.pos - glam::Vec3::Y * EYE_HEIGHT;
    log::info!(
        "under the feet: terrain {:?}, highest floor {:?}, feet at {:.2}",
        world.height_at(feet.x, feet.z).map(|h| (h * 100.0).round() / 100.0),
        world.floor(feet.x, feet.z, feet.y + 1.0).map(|h| (h * 100.0).round() / 100.0),
        feet.y
    );
    log::info!("{} collision triangles loaded", world.collision_triangles());
    if let Some(a) = world.area_at(camera.pos) {
        log::info!("now in: {}", world.areas.display(a));
    }
}

/// How far a creature at `at` is from the surface it should stand on: the highest floor (terrain, building, mine) within a yard above its feet. None: there is none.
fn off_floor(world: &World, at: glam::Vec3) -> Option<f32> {
    world.walker_floor(at.x, at.z, at.y + 0.8).map(|f| at.y - f)
}

/// Tracks how long a creature has been off its floor; reports the first time it has been for half a second.
struct Watchdog {
    since: Option<(f32, glam::Vec3)>,
    reported: bool,
}

impl Watchdog {
    fn new() -> Watchdog {
        Watchdog { since: None, reported: false }
    }

    /// Returns the place and the time it began, once.
    fn look(&mut self, world: &World, at: glam::Vec3, t: f32) -> Option<(f32, glam::Vec3)> {
        let off = off_floor(world, at).is_none_or(|d| d.abs() > 0.5);
        match (off, self.since) {
            (true, None) => self.since = Some((t, at)),
            (false, _) => self.since = None,
            (true, Some((began, from))) if t - began >= 0.5 && !self.reported => {
                self.reported = true;
                return Some((began, from));
            }
            _ => {}
        }
        None
    }
}

/// The spawns near the camera, run on the real collision and navmesh: first wandering for a minute, then chasing a player who circles them, and a pet that follows him.
/// A creature that is off any floor for half a second (below the one it stood on, or in the rock) has lost it.
fn probe_creatures(args: &Args, world: &World, camera: &Camera) {
    use crate::creature::{Mob, Watch};
    use crate::navmesh::{NavMesh, HALF_WORLD, TILE};
    let [north, west, _] = render_to_wow(camera.pos);
    let (ny, nx) = (((north as f64 + HALF_WORLD) / TILE).floor() as i32, ((west as f64 + HALF_WORLD) / TILE).floor() as i32);
    let mut nav = NavMesh::default();
    let span = (100.0 / TILE).ceil() as i32;
    for dy in -span..=span {
        for dx in -span..=span {
            if let Some(tile) = crate::navmesh::build(world, ny + dy, nx + dx) {
                nav.add(&tile);
            }
        }
    }
    // The ground round the camera, 2 yards to a letter, north up: H a hole in the terrain, m a floor under the terrain (a tunnel), t terrain only; the centre is the camera.
    for row in (-20..=20).rev() {
        let line: String = (-30..=30)
            .map(|col| {
                let wow = [north + row as f32 * 2.0, west + col as f32 * 2.0, 0.0];
                let p = crate::coords::wow_to_render(wow);
                match world.height_at(p.x, p.z) {
                    None => 'H',
                    Some(top) if (row, col) == (0, 0) => { let _ = top; '@' }
                    Some(top) => match world.walker_floor(p.x, p.z, top - 3.0) {
                        Some(f) if f < top - 2.0 => 'm',
                        _ => 't',
                    },
                }
            })
            .collect();
        log::info!("map {line}");
    }
    let spawns: Vec<_> = crate::content::get().spawns.iter().filter(|s| (s.at.0 - north).hypot(s.at.1 - west) < 90.0 && s.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(&args.map))).collect();
    let ground = |x: f32, z: f32, y: f32| world.walker_floor(x, z, y);
    let blocked = |a: glam::Vec3, b: glam::Vec3| world.blocked(a, b);
    let sight = |a, b| world.ray_hit(a, b).is_none() && world.ray_hit(b, a).is_none();
    let (mut lost, mut runs) = (std::collections::BTreeMap::<&str, usize>::new(), 0);
    let kinds = std::cell::RefCell::new([0usize; 3]);
    let mut casts = std::collections::BTreeMap::<&str, (usize, usize)>::new();
    let mut crossings = 0;
    let mut skin = std::collections::BTreeMap::<&str, (usize, f32)>::new();
    let mut tunnel = std::collections::BTreeMap::<&str, usize>::new();
    let mut tunnel_shown = 0;
    let mut walls = 0;
    let report = |what: &str, scenario: &'static str, name: &str, t: f32, began: f32, from: glam::Vec3, now: glam::Vec3| {
        let [fx, fy, fz] = render_to_wow(from);
        let [x, y, z] = render_to_wow(now);
        let gap = off_floor(world, now).map_or("no floor within a yard above".to_string(), |d| format!("{d:+.1} yd from the floor"));
        log::info!("  {what} {name} {scenario}: off its floor from {began:.1} s at ({fx:.1}, {fy:.1}, {fz:.1}); at {t:.1} s ({x:.1}, {y:.1}, {z:.1}), {gap}");
        detail(world, now);
        kinds.borrow_mut()[match off_floor(world, now) { Some(d) if d > 0.6 => 0, Some(_) => 1, None => 2 }] += 1;
    };
    for s in &spawns {
        let kind = crate::content::creature_index(&s.creature);
        let mut start = crate::coords::wow_to_render([s.at.0, s.at.1, s.height.unwrap_or(0.0)]);
        if s.height.is_none() {
            start.y = world.height_at(start.x, start.z).unwrap_or(0.0);
        }
        let circle = |t: f32| {
            let a = t * 0.6;
            let at = start + glam::Vec3::new(a.cos() * 12.0, 0.0, a.sin() * 12.0);
            glam::Vec3::new(at.x, world.walker_floor(at.x, at.z, start.y).unwrap_or(start.y), at.z)
        };
        let mut mob = Mob::new(kind, start, s.facing);
        // A long run: the player stands 45 yards off on the terrain, in the direction of this spawn's number, and the creature, after him from the start, runs all the way
        // (up the ramp of a mine, over a hill).
        let outside = {
            let a = (runs as f32) * 0.785;
            let at = start + glam::Vec3::new(a.cos() * 45.0, 0.0, a.sin() * 45.0);
            glam::Vec3::new(at.x, world.height_at(at.x, at.z).unwrap_or(start.y), at.z)
        };
        for (scenario, far) in [("wanders", true), ("chases", false), ("runs out", false), ("comes to the camera", false)] {
            let mut dog = Watchdog::new();
            for tick in 0..60 * if scenario == "runs out" { 20 } else { 60 } {
                let t = tick as f32 / 60.0;
                if scenario == "runs out" && tick == 0 {
                    mob.state = crate::creature::State::Chase;
                }
                let at_camera = glam::Vec3::new(camera.pos.x, world.walker_floor(camera.pos.x, camera.pos.z, camera.pos.y - 2.0).unwrap_or(camera.pos.y), camera.pos.z);
                if scenario == "comes to the camera" && tick == 0 {
                    mob.state = crate::creature::State::Chase;
                }
                let player = if far { start + glam::Vec3::new(300.0, 0.0, 0.0) } else if scenario == "runs out" { outside } else if scenario == "comes to the camera" { at_camera } else { circle(t) };
                // Under the skin of the slope: the feet below the terrain by more than a step, within a few yards of where he stands, and not deep in a tunnel.
                if scenario == "comes to the camera" {
                    if let Some(top) = world.height_at(mob.pos.x, mob.pos.z) {
                        let under = top - mob.pos.y;
                        if under > 0.3 && under < 8.0 && flat_of(mob.pos - at_camera) < 40.0 {
                            *skin.entry(s.creature.as_str()).or_insert((0, 0.0)) = (skin.get(s.creature.as_str()).map_or(0, |e| e.0) + 1, under);
                        }
                    }
                }
                let watch = Watch { feet: player, yaw: 0.0, level: 60, health: 1.0, mana: 1.0, casting: false, moving: false, auras: &[], sneaking: None, sight: &sight };
                let before = mob.pos;
                let popped_before = mob.pos.y;
                mob.step(1.0 / 60.0, &watch, 0, &[], &[], ground, blocked, &nav);
                if std::env::var_os("PROBE_TUNNEL").is_some() && mob.pos.distance(before) > 1e-4 && mob.pos.distance(before) < 3.0 {
                    let mid = (before + mob.pos) / 2.0;
                    // Through a wall or a floor of a building or mine: the step crosses a collision triangle (a hand's breadth up from the feet, so a floor it walks on is not one).
                    let up = glam::Vec3::Y * 0.6;
                    let wall = segment_hits_triangle(before + up, mob.pos + up, &world.triangles_near(mid.x, mid.z, 2.0));
                    // Into the ground: the terrain at the new place is higher than the feet by more than a step, though the feet were on it before.
                    let into = match (world.height_at(before.x, before.z), world.height_at(mob.pos.x, mob.pos.z)) {
                        (Some(a), Some(b)) => before.y > a - 0.3 && mob.pos.y < b - 0.5,
                        _ => false,
                    };
                    // A step onto or off a ledge (up to the yard the player's own controller steps) is no tunnelling; a jump of more than that is, and so is a crossing at the same height.
                    let dy = (mob.pos.y - before.y).abs();
                    let wall = wall && (dy > 1.0 || dy < 0.3);
                    if wall || into {
                        *tunnel.entry(if wall && dy > 1.0 { "jumped over a yard through a surface" } else if wall { "crossed a surface at one height" } else { "into the terrain" }).or_default() += 1;
                        if tunnel_shown < 8 {
                            tunnel_shown += 1;
                            let [x, y, z] = render_to_wow(mob.pos);
                            let [bx, by, bz] = render_to_wow(before);
                            let layers: Vec<String> = (0..12).filter_map(|n| world.floor(mob.pos.x, mob.pos.z, before.y.max(mob.pos.y) + 4.0 - n as f32 * 0.5)).fold(Vec::<f32>::new(), |mut v, f| { if v.last().is_none_or(|l| (l - f).abs() > 0.05) { v.push(f) } v }).iter().map(|f| format!("{f:.2}")).collect();
                            log::info!("      floors under it, from above: {}; course points {:?}", layers.join(", "), mob.course_heights());
                            log::info!("    {} {scenario} {}: ({bx:.1}, {by:.1}, {bz:.1}) -> ({x:.1}, {y:.1}, {z:.1}) at {t:.1} s, terrain there {:?}", s.creature, if wall { "through a surface" } else { "into the terrain" }, world.height_at(mob.pos.x, mob.pos.z));
                        }
                    }
                }
                if mob.pos.y - popped_before > 1.5 {
                    *tunnel.entry("popped up more than 1.5 yd").or_default() += 1;
                    if tunnel_shown < 8 {
                        tunnel_shown += 1;
                        let [x, y, z] = render_to_wow(mob.pos);
                        log::info!("    {} {scenario} popped up from {popped_before:.1} to {y:.1} at ({x:.1}, {z:.1}) at {t:.1} s, terrain {:?}", s.creature, world.height_at(mob.pos.x, mob.pos.z));
                    }
                }
                if let Some((began, from)) = dog.look(world, mob.pos, t) {
                    if scenario == "runs out" && kinds.borrow().iter().sum::<usize>() < 2 {
                        let along: Vec<String> = mob.course_points().iter().rev().take(6).map(|p| format!("({:.1},{:.1},{:.2} | floor {:?})", render_to_wow(*p)[0], render_to_wow(*p)[1], p.y, world.floor(p.x, p.z, p.y + 1.0).map(|f| (f * 100.0).round() / 100.0))).collect();
                        log::info!("      its course from here: {}", along.join(" -> "));
                    }
                    *lost.entry(scenario).or_default() += 1;
                    report("creature", scenario, &s.creature, t, began, from, mob.pos);
                }
            }
        }
        // The player's pet: it follows him round the circle for half a minute, then goes for the spawn's place.
        let mut pet = Mob::new(crate::content::creature_index("c416"), start, 0.0);
        pet.make_ally(20, 0.0, 0, true, false);
        let mut dog = Watchdog::new();
        for tick in 0..60 * 60 {
            let t = tick as f32 / 60.0;
            pet.step_ally(1.0 / 60.0, circle(t), (tick >= 30 * 60).then_some(start), false, ground, blocked, |a, b| !sight(a, b), &nav);
            if let Some((began, from)) = dog.look(world, pet.pos, t) {
                *lost.entry("pet").or_default() += 1;
                report("pet near", "follows", &s.creature, t, began, from, pet.pos);
            }
        }
        runs += 1;
        // An imp with its firebolt at the spawn, once as the walker's check alone sees (as it was) and once with the sight line; every bolt begun or gone off is looked at
        // with a terrain march ten times as fine as the game's, between the places that stand above the ground.
        for (what, with_sight) in [("walker check only", false), ("with sight line", true)] {
            let mut imp = Mob::new(crate::content::creature_index("c416"), start, 0.0);
            imp.make_ally(60, 0.0, 3110, true, false);
            for tick in 0..60 * 60 {
                let t = tick as f32 / 60.0;
                let acts = imp.step_ally(1.0 / 60.0, circle(t), Some(start), false, ground, blocked, |a, b| with_sight && !sight(a, b), &nav);
                for act in acts {
                    if matches!(act, crate::creature::Act::Begin(_) | crate::creature::Act::Spell(_)) {
                        let (a, b) = (imp.pos + glam::Vec3::Y * 1.2, start + glam::Vec3::Y * 1.2);
                        let (len, under) = (a.distance(b), |p: glam::Vec3| world.height_at(p.x, p.z).is_none_or(|g| p.y < g));
                        if under(a) != under(b) {
                            // One end in a mine (or cave) and the other outside: the terrain lies between unless a mine mouth does.
                            let key = if with_sight { "crossing, with sight line" } else { "crossing, walker check only" };
                            casts.entry(key).or_insert((0, 0)).0 += 1;
                            if crossings < 6 && with_sight {
                                crossings += 1;
                                let [ax, ay, az] = render_to_wow(a);
                                let [bx, by, bz] = render_to_wow(b);
                                log::info!("    bolt across the terrain surface: imp ({ax:.1}, {ay:.1}, {az:.1}) -> foe ({bx:.1}, {by:.1}, {bz:.1}); terrain at imp {:?}, at foe {:?}", world.height_at(a.x, a.z), world.height_at(b.x, b.z));
                            }
                        }
                        // Every cast, in a mine or out: the collision triangles between, tested one by one.
                        {
                            let key = if with_sight { "all, with sight line" } else { "all, walker check only" };
                            let mid = (a + b) / 2.0;
                            let entry = casts.entry(key).or_insert((0, 0));
                            entry.0 += 1;
                            if segment_hits_triangle(a, b, &world.triangles_near(mid.x, mid.z, len / 2.0 + 1.0)) {
                                entry.1 += 1;
                                if walls < 6 && with_sight {
                                    walls += 1;
                                    let [ax, ay, az] = render_to_wow(a);
                                    let [bx, by, bz] = render_to_wow(b);
                                    log::info!("    bolt through a wall: imp ({ax:.1}, {ay:.1}, {az:.1}) -> foe ({bx:.1}, {by:.1}, {bz:.1}); terrain at imp {:?}", world.height_at(a.x, a.z));
                                }
                            }
                        }
                        if !under(a) && !under(b) {
                            casts.entry(what).or_insert((0, 0)).0 += 1;
                            if (1..len as usize * 20).any(|i| under(a.lerp(b, i as f32 / (len * 20.0)))) {
                                casts.get_mut(what).unwrap().1 += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    log::info!("imp bolts begun or gone off above ground, and how many of them went through the terrain by a fine march: {casts:?}");
    log::info!("creature ticks spent 0.3 to 8 yd under the surface within 40 yd of the camera, by kind (ticks, last depth): {skin:?}");
    log::info!("creature steps that went through a surface or into the terrain: {tunnel:?}");
    let k = kinds.borrow();
    log::info!("creatures: {runs} spawns run; lost their floor: {lost:?} ({} hovering over it, {} below it, {} with none within a yard above)", k[0], k[1], k[2]);
}

/// What the world says is under a creature that is off its floor: the terrain, and the floors at several heights from its feet.
fn detail(world: &World, at: glam::Vec3) {
    let floors: Vec<String> = [-1.0, 0.0, 1.0, 3.0, 10.0].iter().map(|up| format!("{up:+}: {:?}", world.floor(at.x, at.z, at.y + up).map(|f| (f * 100.0).round() / 100.0))).collect();
    log::info!("      terrain {:?}; floor at feet {}", world.height_at(at.x, at.z), floors.join(", "));
}

/// Whether the segment from `a` to `b` goes through any of the triangles (Moller-Trumbore, both faces), a hand's breadth from either end left out.
fn segment_hits_triangle(a: glam::Vec3, b: glam::Vec3, triangles: &[[glam::Vec3; 3]]) -> bool {
    let (dir, len) = (b - a, a.distance(b));
    triangles.iter().any(|t| {
        let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
        let p = dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-9 {
            return false;
        }
        let (s, inv) = (a - t[0], 1.0 / det);
        let u = s.dot(p) * inv;
        let q = s.cross(e1);
        let v = dir.dot(q) * inv;
        let at = e2.dot(q) * inv;
        u >= 0.0 && v >= 0.0 && u + v <= 1.0 && at > 0.15 / len && at < 1.0 - 0.15 / len
    })
}

fn flat_of(v: glam::Vec3) -> f32 {
    glam::Vec2::new(v.x, v.z).length()
}
