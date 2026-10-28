//! CMaNGOS's `PathFinder` (`MotionGenerators/PathFinder.cpp`): the path of a creature from one place to another over the navigation mesh, as the points it walks
//! through. Positions are WoW's (x north, y west, z up); the mesh's own axes are `{y, z, x}` of these.

use super::mesh::{from_nav, tile_of, to_nav, NavMesh, PolyRef, NAV_GROUND, NAV_MAGMA_SLIME, NAV_WATER, NO_POLY};
use super::query::{Filter, Query, STRAIGHT_END};
use glam::Vec3;

pub const MAX_POINT_PATH_LENGTH: usize = 74;
const SMOOTH_PATH_STEP_SIZE: f32 = 4.0;
const SMOOTH_PATH_SLOP: f32 = 0.3;
const SKIP_POINT_LIMIT: u8 = 6;
const LINE_FAULT: f32 = 0.5;
const NEAR_POLY_SEARCH_BOUND: Vec3 = Vec3::new(5.0, 5.0, 5.0);
const FAR_POLY_SEARCH_BOUND: Vec3 = Vec3::new(10.0, 10.0, 10.0);

/// `PathType`
pub const PATHFIND_BLANK: u8 = 0x00;
/// A normal path.
pub const PATHFIND_NORMAL: u8 = 0x01;
/// Through obstacles, terrain, air, as the old straight line went.
pub const PATHFIND_SHORTCUT: u8 = 0x02;
/// Only part of the way: it gets closer to the target.
pub const PATHFIND_INCOMPLETE: u8 = 0x04;
/// No valid path at all, or an error in making one.
pub const PATHFIND_NOPATH: u8 = 0x08;
/// Used for creatures that fly or swim, and on maps without a navigation mesh.
pub const PATHFIND_NOT_USING_PATH: u8 = 0x10;
/// The path is as long as the limit allows.
pub const PATHFIND_SHORT: u8 = 0x20;

pub struct PathFinder {
    pub path: Vec<Vec3>,
    pub kind: u8,
    use_straight_path: bool,
    force_destination: bool,
    straight_line: bool,
    point_path_limit: usize,
    poly_refs: Vec<PolyRef>,
    poly_length: usize,
    start: Vec3,
    end: Vec3,
    actual_end: Vec3,
    filter: Filter,
    /// A path of an instance may be longer.
    dungeon: bool,
    /// The width of the creature that walks (`GetCollisionWidth`).
    pub collision_width: f32,
}

/// Detour positions are `{y, z, x}`: reads a point as the WoW one.
fn wow(p: Vec3) -> Vec3 {
    from_nav(p)
}

fn in_range_yzx(v1: Vec3, v2: Vec3, r: f32, h: f32) -> bool {
    let (dx, dy, dz) = (v2.x - v1.x, v2.y - v1.y, v2.z - v1.z);
    dx * dx + dz * dz < r * r && dy.abs() < h
}

impl PathFinder {
    /// The finder of a creature that walks (the filter of `createFilter`: ground).
    pub fn new(dungeon: bool) -> PathFinder {
        PathFinder {
            path: Vec::new(),
            kind: PATHFIND_BLANK,
            use_straight_path: false,
            force_destination: false,
            straight_line: false,
            point_path_limit: MAX_POINT_PATH_LENGTH,
            poly_refs: vec![NO_POLY; MAX_POINT_PATH_LENGTH],
            poly_length: 0,
            start: Vec3::ZERO,
            end: Vec3::ZERO,
            actual_end: Vec3::ZERO,
            // (createFilter: a creature that walks goes on the ground, one that swims in water, and creatures take no harm from magma and slime. The content does not
            // say which creatures swim: all do, as most of the database's do.)
            filter: Filter { include: NAV_GROUND | NAV_WATER | NAV_MAGMA_SLIME, exclude: 0 },
            dungeon,
            collision_width: 0.5,
        }
    }

    pub fn end_position(&self) -> Vec3 {
        self.end
    }

    pub fn actual_end_position(&self) -> Vec3 {
        self.actual_end
    }

    fn clear(&mut self) {
        self.poly_refs.iter_mut().for_each(|r| *r = NO_POLY);
        self.poly_length = 0;
        self.path.clear();
    }

    /// Make the path from `start` to `dest`. With `force_dest` it ends at `dest` even where the mesh does not reach; with `straight_line` it is a ray over the mesh.
    pub fn calculate(&mut self, nav: &NavMesh, start: Vec3, dest: Vec3, force_dest: bool, straight_line: bool) -> bool {
        if !start.is_finite() || !dest.is_finite() {
            return false;
        }
        self.start = start;
        self.end = dest;
        self.actual_end = dest;
        self.force_destination = force_dest;
        self.straight_line = straight_line;
        // Make sure the mesh works: the start and the end need a tile (can we pass via a tile that is not loaded on the way?).
        let have_tile = |p: Vec3| {
            let (x, y) = tile_of(to_nav(p));
            nav.has_tile(x, y)
        };
        if !have_tile(start) || !have_tile(dest) {
            self.build_shortcut();
            self.kind = PATHFIND_NORMAL | PATHFIND_NOT_USING_PATH;
            return true;
        }
        self.build_poly_path(nav, start, dest);
        true
    }

    /// `ComputePathToRandomPoint`: a ray over the mesh from `curr` to `random_point` (WoW's axes; its height is the height of the place round which it was picked).
    /// The kind of path is no path where the ray gets nowhere.
    pub fn compute_path_to_random_point(&mut self, nav: &NavMesh, curr: Vec3, random_point: Vec3) {
        self.clear();
        self.kind = PATHFIND_NOPATH;
        // Use only a straight line.
        self.straight_line = true;
        self.force_destination = false;
        // Fast check to see if the point is far enough.
        if curr.distance_squared(random_point) < 0.01 {
            return;
        }
        (self.start, self.end, self.actual_end) = (curr, random_point, random_point);
        let have_tile = |p: Vec3| {
            let (x, y) = tile_of(to_nav(p));
            nav.has_tile(x, y)
        };
        if !have_tile(curr) || !have_tile(random_point) {
            self.build_shortcut();
            self.kind = PATHFIND_NORMAL | PATHFIND_SHORTCUT;
            return;
        }
        let mut point = to_nav(random_point);
        let (centre_poly, _) = self.poly_by_location(nav, point);
        if centre_poly != NO_POLY {
            // The height first, before the path is made.
            if let Some(h) = (Query { mesh: nav, filter: self.filter }).poly_height(centre_poly, point) {
                point.y = h;
                let end = wow(point);
                (self.end, self.actual_end) = (end, end);
                self.build_poly_path(nav, curr, end);
            }
        }
    }

    /// The height of the mesh at `p` (WoW's axes) nearest to the height `p` has: where a place of the content without a height stands.
    pub fn height_near(&self, nav: &NavMesh, p: Vec3) -> Option<f32> {
        let q = Query { mesh: nav, filter: self.filter };
        q.find_nearest_poly(to_nav(p), Vec3::new(3.0, 30.0, 3.0)).filter(|(_, c)| (c.x - to_nav(p).x).abs() < 3.0 && (c.z - to_nav(p).z).abs() < 3.0).map(|(_, c)| c.y)
    }

    fn build_shortcut(&mut self) {
        self.clear();
        // Two points: where it is and where it goes. (`NormalizePath` is off: PathFinder.NormalizeZ is not set by default.)
        self.path = vec![self.start, self.actual_end];
        self.kind = PATHFIND_SHORTCUT;
    }

    fn path_poly_by_position(&self, nav: &NavMesh, polys: &[PolyRef], point: Vec3, max_dist: f32) -> (PolyRef, Option<f32>) {
        if polys.is_empty() {
            return (NO_POLY, None);
        }
        let q = Query { mesh: nav, filter: self.filter };
        let (mut nearest, mut min_dist) = (NO_POLY, f32::MAX);
        for &r in polys {
            let Some((closest, _)) = q.closest_point_on_poly(r, point) else { continue };
            let d = (point - closest).length_squared();
            if d < min_dist {
                min_dist = d;
                nearest = r;
            }
            if min_dist < 1.0 {
                break;
            }
        }
        (if min_dist <= max_dist { nearest } else { NO_POLY }, Some(min_dist.sqrt()))
    }

    /// The polygon `point` is on or nearest to, and how far it is from it (`getPolyByLocation`). First the current path is looked at.
    fn poly_by_location(&mut self, nav: &NavMesh, point: Vec3) -> (PolyRef, f32) {
        let q = Query { mesh: nav, filter: self.filter };
        let (r, d) = self.path_poly_by_position(nav, &self.poly_refs[..self.poly_length.min(self.poly_refs.len())], point, 3.0);
        let mut distance = d.unwrap_or(0.0);
        if r != NO_POLY {
            return (r, distance);
        }
        // More stored polygons: search those too.
        if self.poly_refs.len() > self.poly_length.max(self.point_path_limit) {
            let (r, d) = self.path_poly_by_position(nav, &self.poly_refs[self.point_path_limit..], point, 7.0);
            if let Some(d) = d {
                distance = d;
            }
            if r != NO_POLY {
                return (r, distance);
            }
        }
        // Not on the old path: look for it, first near, then in a bigger box.
        for bound in [NEAR_POLY_SEARCH_BOUND, FAR_POLY_SEARCH_BOUND] {
            if let Some((r, closest)) = q.find_nearest_poly(point, bound) {
                let new_distance = closest.distance(point);
                // Only store the polygon found if it is closer than the one found from the path.
                if distance == 0.0 || new_distance < distance {
                    self.poly_refs.push(r);
                }
                return (r, new_distance);
            }
        }
        (NO_POLY, distance)
    }

    fn build_poly_path(&mut self, nav: &NavMesh, start_pos: Vec3, end_pos: Vec3) {
        if self.dungeon {
            let distance = start_pos.distance(end_pos);
            if distance > 300.0 {
                self.point_path_limit = MAX_POINT_PATH_LENGTH * 4;
            } else if distance > 200.0 {
                self.point_path_limit = MAX_POINT_PATH_LENGTH * 3;
            } else if distance > 100.0 {
                self.point_path_limit = MAX_POINT_PATH_LENGTH * 2;
            }
            if self.point_path_limit > self.poly_refs.len() {
                self.poly_refs.resize(self.point_path_limit, NO_POLY);
            }
        }
        let (start_point, mut end_point) = (to_nav(start_pos), to_nav(end_pos));
        let (start_poly, dist_to_start) = self.poly_by_location(nav, start_point);
        let (end_poly, dist_to_end) = self.poly_by_location(nav, end_point);
        // A hole in the mesh: a shortcut, marked as no path (a creature that flies or swims would be let through; this one walks).
        if start_poly == NO_POLY || end_poly == NO_POLY {
            self.build_shortcut();
            self.kind = PATHFIND_NOPATH;
            return;
        }
        let q = Query { mesh: nav, filter: self.filter };
        // Far from the polygon: the path ends at the nearest point of the mesh.
        let far_from_poly = dist_to_start > 7.0 || dist_to_end > 7.0;
        if far_from_poly {
            if let Some((closest, _)) = q.closest_point_on_poly(end_poly, end_point) {
                end_point = closest;
                self.actual_end = wow(end_point);
            }
            self.kind = PATHFIND_INCOMPLETE;
        }
        // Start and end on one polygon: a straight line.
        if start_poly == end_poly {
            self.build_shortcut();
            self.poly_refs[0] = start_poly;
            self.poly_length = 1;
            self.kind = if far_from_poly { PATHFIND_INCOMPLETE } else { PATHFIND_NORMAL };
            return;
        }
        // Look for the start and end polygons in the current path.
        let (mut start_found, mut end_found) = (false, false);
        let (mut path_start, mut path_end) = (0usize, 0usize);
        if self.poly_length > 0 {
            while path_start < self.poly_length {
                if self.poly_refs[path_start] == NO_POLY {
                    break;
                }
                if self.poly_refs[path_start] == start_poly {
                    start_found = true;
                    break;
                }
                path_start += 1;
            }
            path_end = self.poly_length - 1;
            while path_end > path_start {
                if self.poly_refs[path_end] == end_poly {
                    end_found = true;
                    break;
                }
                path_end -= 1;
            }
        }
        if start_found && end_found {
            // It moved along the path and the target did not leave it: cut the part that is left out.
            self.poly_length = path_end - path_start + 1;
            self.poly_refs.copy_within(path_start..path_start + self.poly_length, 0);
        } else {
            // No path yet, or it is not followed: make a new one.
            self.clear();
            if !self.straight_line {
                match q.find_path(start_poly, end_poly, start_point, end_point, self.point_path_limit) {
                    Some((polys, _)) => {
                        self.poly_length = polys.len();
                        self.poly_refs[..polys.len()].copy_from_slice(&polys);
                    }
                    None => self.poly_length = 0,
                }
            } else {
                let Some((hit, polys)) = q.raycast(start_poly, start_point, end_point, self.point_path_limit) else {
                    self.poly_length = 0;
                    self.build_shortcut();
                    self.kind = PATHFIND_NOPATH;
                    return;
                };
                self.poly_length = polys.len();
                self.poly_refs[..polys.len()].copy_from_slice(&polys);
                // The ray hit something: the end moves back to it.
                if let Some(hit) = hit {
                    // (The original takes the collision width, in yards, off this fraction of the ray.)
                    let hit = hit - self.collision_width;
                    if hit < 0.1 {
                        self.kind = PATHFIND_NOPATH;
                        return;
                    }
                    let mut hit_pos = start_point.lerp(end_point, hit);
                    let (poly, _) = self.poly_by_location(nav, hit_pos);
                    if poly != NO_POLY {
                        // The height first, before the hit test.
                        if let Some(h) = q.poly_height(poly, hit_pos) {
                            hit_pos.y = h;
                        }
                        let hit_vec = wow(hit_pos);
                        if (start_pos - hit_vec).length_squared() > 0.01 {
                            self.end = hit_vec;
                            self.build_point_path(nav, start_point, hit_pos);
                            self.kind = PATHFIND_INCOMPLETE;
                            return;
                        }
                    }
                    self.kind = PATHFIND_NOPATH;
                    return;
                }
            }
            if self.poly_length == 0 {
                self.build_shortcut();
                self.kind = PATHFIND_NOPATH;
                return;
            }
        }
        // By now we know what type of path we can get.
        self.kind = if self.poly_refs[self.poly_length - 1] == end_poly && self.kind & PATHFIND_INCOMPLETE == 0 { PATHFIND_NORMAL } else { PATHFIND_INCOMPLETE };
        self.build_point_path(nav, start_point, end_point);
    }

    fn build_point_path(&mut self, nav: &NavMesh, start_point: Vec3, end_point: Vec3) {
        let q = Query { mesh: nav, filter: self.filter };
        let mut points: Vec<Vec3> = Vec::new();
        let mut ok = false;
        if self.straight_line {
            let (start_vec, end_vec) = (wow(start_point), wow(end_point));
            let dir = end_vec - start_vec;
            let length = dir.length();
            // (Not sure about the exact limit below, but it should be ok.)
            if length > 0.1 {
                let step_count_float = length / SMOOTH_PATH_STEP_SIZE;
                let mut step_count = step_count_float as u32;
                let step_part = step_count_float - step_count as f32;
                // Merge the last point with the previous one if it is 30% less than the step.
                if step_count > 1 && step_part < 0.3 {
                    step_count -= 1;
                }
                let mut prev = start_vec;
                let diff = dir * (SMOOTH_PATH_STEP_SIZE / length);
                points.push(start_vec);
                for _ in 0..step_count {
                    prev += diff;
                    // Fix the height using the detail data.
                    let converted = to_nav(prev);
                    let (poly, _) = self.poly_by_location(nav, converted);
                    if poly != NO_POLY {
                        if let Some(h) = q.poly_height(poly, converted) {
                            prev.z = h;
                        }
                    }
                    points.push(prev);
                }
                points.push(end_vec);
                ok = true;
            }
        } else {
            let found = if self.use_straight_path {
                q.find_straight_path(start_point, end_point, &self.poly_refs[..self.poly_length], self.point_path_limit).map(|p| p.into_iter().map(|c| c.0).collect::<Vec<_>>())
            } else {
                self.find_smooth_path(nav, start_point, end_point)
            };
            if let Some(found) = found {
                ok = true;
                // PathFinder.OptimizePath: points on a line with their neighbours are left out (at most `SKIP_POINT_LIMIT` in a row).
                let wow_points: Vec<Vec3> = found.iter().map(|&p| wow(p)).collect();
                if wow_points.len() > 2 {
                    let mut kept = vec![wow_points[0]];
                    let mut cut = 0u8;
                    for i in 1..wow_points.len() - 1 {
                        let (p, p1, p2) = (wow_points[i], wow_points[i - 1], wow_points[i + 1]);
                        let line_len = (p1.y - p2.y) * p.x + (p2.x - p1.x) * p.y + (p1.x * p2.y - p2.x * p1.y);
                        if line_len.abs() < LINE_FAULT && cut < SKIP_POINT_LIMIT {
                            cut += 1;
                        } else {
                            kept.push(p);
                            cut = 0;
                        }
                    }
                    kept.push(*wow_points.last().expect("three points"));
                    points = kept;
                } else {
                    points = wow_points;
                }
            }
        }
        if points.len() < 2 || !ok {
            // Only happens with bad data or a broken mesh.
            self.build_shortcut();
            self.kind = PATHFIND_NOPATH;
            return;
        }
        if points.len() == self.point_path_limit {
            self.build_shortcut();
            self.kind = PATHFIND_SHORT;
            return;
        }
        self.path = points;
        // The first point is always where it is: the next one is needed.
        self.actual_end = *self.path.last().expect("two points");
        // Force the given destination, if needed.
        if self.force_destination && (self.kind & PATHFIND_NORMAL == 0 || !self.in_range(self.end, self.actual_end, 1.0, 1.0)) {
            // A partial path may be kept.
            if self.actual_end.distance_squared(self.end) < 0.3 * self.start.distance_squared(self.end) {
                self.actual_end = self.end;
                *self.path.last_mut().expect("two points") = self.end;
            } else {
                self.actual_end = self.end;
                self.build_shortcut();
            }
            self.kind = PATHFIND_NORMAL | PATHFIND_NOT_USING_PATH;
        }
    }

    fn in_range(&self, p1: Vec3, p2: Vec3, r: f32, h: f32) -> bool {
        let d = p1 - p2;
        d.x * d.x + d.y * d.y < r * r && d.z.abs() < h
    }

    /// `fixupCorridor`: the polygons visited by a step are put at the front of the path.
    fn fixup_corridor(path: &mut Vec<PolyRef>, max_path: usize, visited: &[PolyRef]) {
        // The furthest polygon of the path that was visited (and the first place it was visited at).
        let mut furthest: Option<(usize, usize)> = None;
        for i in (0..path.len()).rev() {
            for j in (0..visited.len()).rev() {
                if path[i] == visited[j] {
                    furthest = Some((i, j));
                }
            }
            if furthest.is_some() {
                break;
            }
        }
        let Some((furthest_path, furthest_visited)) = furthest else { return };
        // Concatenate the paths: the visited at the start, then what is left of the path.
        let req = visited.len() - furthest_visited;
        if req >= max_path {
            return;
        }
        let orig = (furthest_path + 1).min(path.len());
        let mut rest: Vec<PolyRef> = path[orig..].to_vec();
        rest.truncate(max_path - req);
        let mut out: Vec<PolyRef> = (0..req).map(|i| visited[visited.len() - 1 - i]).collect();
        out.extend(rest);
        *path = out;
    }

    /// `getSteerTarget`: the corner to steer towards, at least `min_target_dist` away.
    fn steer_target(&self, nav: &NavMesh, start: Vec3, end: Vec3, min_target_dist: f32, path: &[PolyRef]) -> Option<(Vec3, u8)> {
        let q = Query { mesh: nav, filter: self.filter };
        let corners = q.find_straight_path(start, end, path, 3)?;
        if corners.is_empty() {
            return None;
        }
        // Find a vertex far enough to steer to.
        let found = corners.iter().find(|c| !in_range_yzx(c.0, start, min_target_dist, 1000.0))?;
        let mut pos = found.0;
        pos.y = start.y; // keep the height
        Some((pos, found.1))
    }

    /// `findSmoothPath`: walk towards the end in steps of four yards over the mesh, the points of the way.
    fn find_smooth_path(&mut self, nav: &NavMesh, start: Vec3, end: Vec3) -> Option<Vec<Vec3>> {
        let q = Query { mesh: nav, filter: self.filter };
        let limit = self.point_path_limit;
        let mut polys: Vec<PolyRef> = self.poly_refs[..self.poly_length].to_vec();
        let mut iter_pos = q.closest_point_on_poly_boundary(polys[0], start)?;
        let target_pos = q.closest_point_on_poly_boundary(*polys.last()?, end)?;
        let mut smooth = vec![iter_pos];
        // Move towards the target a small advancement at a time until it is reached or the path is full.
        while !polys.is_empty() && smooth.len() < limit {
            let Some((steer_pos, steer_flag)) = self.steer_target(nav, iter_pos, target_pos, SMOOTH_PATH_SLOP, &polys) else { break };
            let end_of_path = steer_flag & STRAIGHT_END != 0;
            // Find the movement delta.
            let delta = steer_pos - iter_pos;
            let mut len = delta.length();
            // If the steer target is the end of the path, do not move past it.
            len = if end_of_path && len < SMOOTH_PATH_STEP_SIZE { 1.0 } else { SMOOTH_PATH_STEP_SIZE / len };
            let move_target = iter_pos + delta * len;
            // Move.
            let (result, visited) = q.move_along_surface(polys[0], iter_pos, move_target, 16)?;
            Self::fixup_corridor(&mut polys, limit, &visited);
            let mut result = result;
            if let Some(h) = q.poly_height(polys[0], result) {
                result.y = h;
            }
            // (CMaNGOS lifts the point by 0.5 here and its server then puts the creature back on the ground; nothing does that here, so the lift would float them.)
            iter_pos = result;
            // Handle the end of the path when close enough.
            if end_of_path && in_range_yzx(iter_pos, steer_pos, SMOOTH_PATH_SLOP, 1.0) {
                iter_pos = target_pos;
                if smooth.len() < limit {
                    smooth.push(iter_pos);
                }
                break;
            }
            // Store the result.
            if smooth.len() < limit {
                smooth.push(iter_pos);
            }
        }
        // (This is most likely a loop if it is as long as the limit.)
        (smooth.len() < limit).then_some(smooth)
    }
}

impl PathFinder {
    /// Whether the path leads the whole way (and is not a straight line past what the mesh allows).
    pub fn reaches(&self) -> bool {
        self.kind & (PATHFIND_NOPATH | PATHFIND_INCOMPLETE) == 0
    }
}
