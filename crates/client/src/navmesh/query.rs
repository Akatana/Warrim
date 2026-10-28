//! The queries of Detour (`dtNavMeshQuery`) that CMaNGOS's PathFinder uses, on a `NavMesh`. All positions are in Detour's axes (see `mesh`).

use super::mesh::*;
use glam::Vec3;
use std::collections::{BinaryHeap, HashMap, VecDeque};

/// `dtQueryFilter`: which polygons a walker may enter. (All areas cost the same here; only the distance counts.)
#[derive(Clone, Copy, Default)]
pub struct Filter {
    pub include: u16,
    pub exclude: u16,
}

impl Filter {
    fn passes(&self, poly: &Poly) -> bool {
        poly.flags & self.include != 0 && poly.flags & self.exclude == 0
    }
}

/// How a path came out (`dtStatus`): whole, or only as far as it got towards the end.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Reach {
    Whole,
    Partial,
}

/// `DT_STRAIGHTPATH_END`.
pub const STRAIGHT_END: u8 = 2;
const H_SCALE: f32 = 0.999;

fn dist_sqr(a: Vec3, b: Vec3) -> f32 {
    (a - b).length_squared()
}

fn dist_2d_sqr(a: Vec3, b: Vec3) -> f32 {
    (a.x - b.x) * (a.x - b.x) + (a.z - b.z) * (a.z - b.z)
}

/// `dtDistancePtSegSqr2D`: the squared distance in the plane from `pt` to the segment `p`-`q`, and where along it the nearest point is.
fn dist_pt_seg_sqr_2d(pt: Vec3, p: Vec3, q: Vec3) -> (f32, f32) {
    let (pqx, pqz) = (q.x - p.x, q.z - p.z);
    let (dx, dz) = (pt.x - p.x, pt.z - p.z);
    let d = pqx * pqx + pqz * pqz;
    let mut t = pqx * dx + pqz * dz;
    if d > 0.0 {
        t /= d;
    }
    let t = t.clamp(0.0, 1.0);
    let (dx, dz) = (p.x + t * pqx - pt.x, p.z + t * pqz - pt.z);
    (dx * dx + dz * dz, t)
}

/// `dtDistancePtPolyEdgesSqr`: whether `pt` is inside the polygon (in the plane), and its squared distance to each edge with where along it.
fn dist_pt_poly_edges(pt: Vec3, verts: &[Vec3]) -> (bool, Vec<(f32, f32)>) {
    let nv = verts.len();
    let mut inside = false;
    let mut edges = vec![(0.0, 0.0); nv];
    let mut j = nv - 1;
    for i in 0..nv {
        let (vi, vj) = (verts[i], verts[j]);
        if ((vi.z > pt.z) != (vj.z > pt.z)) && (pt.x < (vj.x - vi.x) * (pt.z - vi.z) / (vj.z - vi.z) + vi.x) {
            inside = !inside;
        }
        edges[j] = dist_pt_seg_sqr_2d(pt, vj, vi);
        j = i;
    }
    (inside, edges)
}

fn point_in_polygon(pt: Vec3, verts: &[Vec3]) -> bool {
    let mut inside = false;
    let mut j = verts.len() - 1;
    for i in 0..verts.len() {
        let (vi, vj) = (verts[i], verts[j]);
        if ((vi.z > pt.z) != (vj.z > pt.z)) && (pt.x < (vj.x - vi.x) * (pt.z - vi.z) / (vj.z - vi.z) + vi.x) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// `dtClosestHeightPointTriangle`: the height of the triangle `a b c` under `p`, if `p` is over it.
fn height_in_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (v0, v1, v2) = (c - a, b - a, p - a);
    let d00 = v0.x * v0.x + v0.z * v0.z;
    let d01 = v0.x * v1.x + v0.z * v1.z;
    let d02 = v0.x * v2.x + v0.z * v2.z;
    let d11 = v1.x * v1.x + v1.z * v1.z;
    let d12 = v1.x * v2.x + v1.z * v2.z;
    let denom = d00 * d11 - d01 * d01;
    if denom < 1e-6 {
        return None;
    }
    let u = d11 * d02 - d01 * d12;
    let v = d00 * d12 - d01 * d02;
    // (A little leeway at the edges, as Detour has it.)
    let eps = -1e-4 * denom;
    (u >= eps && v >= eps && u + v <= denom * (1.0 + 1e-4)).then(|| a.y + v0.y * u / denom + v1.y * v / denom)
}

/// `dtTriArea2D`: twice the signed area of the triangle in the plane.
fn tri_area_2d(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let (ax, az) = (b.x - a.x, b.z - a.z);
    let (bx, bz) = (c.x - a.x, c.z - a.z);
    bx * az - ax * bz
}

fn perp_2d(u: Vec3, v: Vec3) -> f32 {
    u.z * v.x - u.x * v.z
}

/// `dtIntersectSegmentPoly2D`: where the segment `p0`-`p1` enters and leaves the polygon (fractions of it) and by which edges (-1: none).
fn intersect_segment_poly_2d(p0: Vec3, p1: Vec3, verts: &[Vec3]) -> Option<(f32, f32, i32, i32)> {
    const EPS: f32 = 1e-6;
    let (mut tmin, mut tmax, mut seg_min, mut seg_max) = (0.0f32, 1.0f32, -1i32, -1i32);
    let dir = p1 - p0;
    let mut j = verts.len() - 1;
    for i in 0..verts.len() {
        let edge = verts[i] - verts[j];
        let diff = p0 - verts[j];
        let n = perp_2d(edge, diff);
        let d = perp_2d(dir, edge);
        if d.abs() < EPS {
            // Nearly parallel to this edge.
            if n < 0.0 {
                return None;
            }
        } else {
            let t = n / d;
            if d < 0.0 {
                // Entering across this edge.
                if t > tmin {
                    tmin = t;
                    seg_min = j as i32;
                    if tmin > tmax {
                        return None;
                    }
                }
            } else if t < tmax {
                // Leaving across this edge.
                tmax = t;
                seg_max = j as i32;
                if tmax < tmin {
                    return None;
                }
            }
        }
        j = i;
    }
    Some((tmin, tmax, seg_min, seg_max))
}

pub struct Query<'a> {
    pub mesh: &'a NavMesh,
    pub filter: Filter,
}

impl<'a> Query<'a> {
    fn corners(&self, r: PolyRef) -> Vec<Vec3> {
        let (tile, poly) = self.mesh.poly(r);
        tile.corners(poly)
    }

    /// The polygons whose bounds meet the box `center` +- `extents` and that pass the filter (`queryPolygons`).
    fn query_polygons(&self, center: Vec3, extents: Vec3) -> Vec<PolyRef> {
        let (lo, hi) = (center - extents, center + extents);
        let (t0, t1) = (tile_of(lo), tile_of(hi));
        let mut out = Vec::new();
        for y in t0.1..=t1.1 {
            for x in t0.0..=t1.0 {
                let Some(slot) = self.mesh.tile_slot(x, y) else { continue };
                let tile = &self.mesh.tiles[slot];
                for (i, poly) in tile.polys.iter().enumerate() {
                    if !self.filter.passes(poly) {
                        continue;
                    }
                    let (mut pmin, mut pmax) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                    for k in 0..poly.count {
                        let v = tile.verts[poly.verts[k] as usize];
                        pmin = pmin.min(v);
                        pmax = pmax.max(v);
                    }
                    if pmin.x <= hi.x && pmax.x >= lo.x && pmin.y <= hi.y && pmax.y >= lo.y && pmin.z <= hi.z && pmax.z >= lo.z {
                        out.push(make_ref(slot, i));
                    }
                }
            }
        }
        out
    }

    /// The polygon nearest to `center` within `extents`, and the nearest point on it (`findNearestPoly`).
    pub fn find_nearest_poly(&self, center: Vec3, extents: Vec3) -> Option<(PolyRef, Vec3)> {
        let mut nearest: Option<(PolyRef, Vec3)> = None;
        let mut nearest_dist = f32::MAX;
        for r in self.query_polygons(center, extents) {
            let Some((closest, over)) = self.closest_point_on_poly(r, center) else { continue };
            // If a point is directly over a polygon and closer than climb height, favour that instead of straight distance.
            let d = if over {
                let climb = super::build::WALKABLE_CLIMB as f32 * super::build::CELL;
                let dy = (center.y - closest.y).abs() - climb;
                if dy > 0.0 { dy * dy } else { 0.0 }
            } else {
                dist_sqr(center, closest)
            };
            if d < nearest_dist {
                nearest_dist = d;
                nearest = Some((r, closest));
            }
        }
        nearest
    }

    /// The point of the polygon nearest to `pos`, and whether `pos` is over the polygon (`closestPointOnPoly`).
    pub fn closest_point_on_poly(&self, r: PolyRef, pos: Vec3) -> Option<(Vec3, bool)> {
        if r == NO_POLY || (r >> 16) as usize > self.mesh.tiles.len() {
            return None;
        }
        let (tile, poly) = self.mesh.poly(r);
        let verts = tile.corners(poly);
        let (over, edges) = dist_pt_poly_edges(pos, &verts);
        if over {
            let mut closest = pos;
            if let Some(h) = self.height_in_detail(r, pos) {
                closest.y = h;
            }
            return Some((closest, true));
        }
        let (imin, _) = edges.iter().enumerate().min_by(|a, b| a.1 .0.total_cmp(&b.1 .0))?;
        let (va, vb) = (verts[imin], verts[(imin + 1) % verts.len()]);
        Some((va.lerp(vb, edges[imin].1), false))
    }

    /// The point of the polygon's border nearest to `pos` (`pos` itself if it is over the polygon).
    pub fn closest_point_on_poly_boundary(&self, r: PolyRef, pos: Vec3) -> Option<Vec3> {
        if r == NO_POLY || (r >> 16) as usize > self.mesh.tiles.len() {
            return None;
        }
        let verts = self.corners(r);
        let (inside, edges) = dist_pt_poly_edges(pos, &verts);
        if inside {
            return Some(pos);
        }
        let (imin, _) = edges.iter().enumerate().min_by(|a, b| a.1 .0.total_cmp(&b.1 .0))?;
        Some(verts[imin].lerp(verts[(imin + 1) % verts.len()], edges[imin].1))
    }

    fn height_in_detail(&self, r: PolyRef, pos: Vec3) -> Option<f32> {
        let (tile, _) = self.mesh.poly(r);
        let (verts, tris) = &tile.detail[(r & 0xffff) as usize];
        tris.iter().find_map(|t| height_in_triangle(pos, verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]))
    }

    /// The height of the polygon under `pos`, if `pos` is over it (`getPolyHeight`).
    pub fn poly_height(&self, r: PolyRef, pos: Vec3) -> Option<f32> {
        let verts = self.corners(r);
        if !point_in_polygon(pos, &verts) {
            return None;
        }
        self.height_in_detail(r, pos)
    }

    /// The two ends of the portal between neighbouring polygons `from` and `to` (`getPortalPoints`).
    fn portal_points(&self, from: PolyRef, to: PolyRef) -> Option<(Vec3, Vec3)> {
        let (tile, poly) = self.mesh.poly(from);
        let link = self.mesh.links(from).iter().find(|l| l.to == to)?;
        let (mut left, mut right) = (tile.verts[poly.verts[link.edge as usize] as usize], tile.verts[poly.verts[(link.edge as usize + 1) % poly.count] as usize]);
        // If the link is at a tile border, clamp the vertices to the link width.
        if link.side != NO_SIDE && (link.min != 0 || link.max != 255) {
            let (s, tmin, tmax) = (1.0 / 255.0, link.min as f32 / 255.0, link.max as f32 / 255.0);
            let _ = s;
            let (l, r) = (left, right);
            left = l.lerp(r, tmin);
            right = l.lerp(r, tmax);
        }
        Some((left, right))
    }

    fn edge_midpoint(&self, from: PolyRef, to: PolyRef) -> Option<Vec3> {
        let (l, r) = self.portal_points(from, to)?;
        Some((l + r) * 0.5)
    }

    fn passes(&self, r: PolyRef) -> bool {
        self.filter.passes(self.mesh.poly(r).1)
    }

    /// The polygons from `start` to `end` (A* over the polygons, `findPath`); only as far as it gets, nearest to the end, if there is no way.
    pub fn find_path(&self, start: PolyRef, end: PolyRef, start_pos: Vec3, end_pos: Vec3, max_path: usize) -> Option<(Vec<PolyRef>, Reach)> {
        if start == NO_POLY || end == NO_POLY {
            return None;
        }
        if start == end {
            return Some((vec![start], Reach::Whole));
        }
        struct Node {
            pos: Vec3,
            cost: f32,
            total: f32,
            parent: Option<usize>,
            id: PolyRef,
            open: bool,
            closed: bool,
        }
        // (The pool is keyed by polygon and the side it was entered across.)
        let mut nodes: Vec<Node> = Vec::new();
        let mut by_key: HashMap<(PolyRef, u8), usize> = HashMap::new();
        let mut open: BinaryHeap<(std::cmp::Reverse<u32>, usize)> = BinaryHeap::new();
        let push_key = |total: f32| std::cmp::Reverse(total.max(0.0).to_bits());
        nodes.push(Node { pos: start_pos, cost: 0.0, total: start_pos.distance(end_pos) * H_SCALE, parent: None, id: start, open: true, closed: false });
        by_key.insert((start, 0), 0);
        open.push((push_key(nodes[0].total), 0));
        let mut last_best = 0usize;
        let mut last_best_cost = nodes[0].total;
        while let Some((_, best)) = open.pop() {
            if !nodes[best].open {
                continue;
            }
            // Remove it from the open list and mark it closed.
            nodes[best].open = false;
            nodes[best].closed = true;
            let best_ref = nodes[best].id;
            if best_ref == end {
                last_best = best;
                break;
            }
            let parent_ref = nodes[best].parent.map(|p| nodes[p].id);
            for link in self.mesh.links(best_ref) {
                let neighbour = link.to;
                if neighbour == NO_POLY || Some(neighbour) == parent_ref || !self.passes(neighbour) {
                    continue;
                }
                // Deal explicitly with crossing tile boundaries.
                let cross = if link.side != NO_SIDE { link.side >> 1 } else { 0 };
                let idx = *by_key.entry((neighbour, cross)).or_insert_with(|| {
                    nodes.push(Node { pos: Vec3::ZERO, cost: 0.0, total: 0.0, parent: None, id: neighbour, open: false, closed: false });
                    nodes.len() - 1
                });
                if !nodes[idx].open && !nodes[idx].closed {
                    let Some(mid) = self.edge_midpoint(best_ref, neighbour) else { continue };
                    nodes[idx].pos = mid;
                }
                let (cost, heuristic) = if neighbour == end {
                    // The last node: add the way from the edge to the end.
                    (nodes[best].cost + nodes[best].pos.distance(nodes[idx].pos) + nodes[idx].pos.distance(end_pos), 0.0)
                } else {
                    (nodes[best].cost + nodes[best].pos.distance(nodes[idx].pos), nodes[idx].pos.distance(end_pos) * H_SCALE)
                };
                let total = cost + heuristic;
                // Already open or closed with a better result: skip.
                if (nodes[idx].open || nodes[idx].closed) && total >= nodes[idx].total {
                    continue;
                }
                nodes[idx].parent = Some(best);
                nodes[idx].closed = false;
                nodes[idx].cost = cost;
                nodes[idx].total = total;
                nodes[idx].open = true;
                open.push((push_key(total), idx));
                // Update the node nearest to the target so far.
                if heuristic < last_best_cost {
                    last_best_cost = heuristic;
                    last_best = idx;
                }
            }
        }
        let reach = if nodes[last_best].id == end { Reach::Whole } else { Reach::Partial };
        let mut path = Vec::new();
        let mut at = Some(last_best);
        while let Some(i) = at {
            path.push(nodes[i].id);
            at = nodes[i].parent;
        }
        path.reverse();
        path.truncate(max_path);
        Some((path, reach))
    }

    /// The corners of the shortest way through the polygons `path` (the funnel; `findStraightPath`): their positions, flags (`STRAIGHT_END` at the last) and
    /// the polygons they are in.
    pub fn find_straight_path(&self, start_pos: Vec3, end_pos: Vec3, path: &[PolyRef], max_points: usize) -> Option<Vec<(Vec3, u8, PolyRef)>> {
        if path.is_empty() || max_points == 0 {
            return None;
        }
        let closest_start = self.closest_point_on_poly_boundary(path[0], start_pos)?;
        let closest_end = self.closest_point_on_poly_boundary(*path.last()?, end_pos)?;
        let mut out: Vec<(Vec3, u8, PolyRef)> = Vec::new();
        // Appends a corner; a corner on the last one only changes its flags. False when the list is full or ended.
        fn append(out: &mut Vec<(Vec3, u8, PolyRef)>, max: usize, pos: Vec3, flags: u8, r: PolyRef) -> bool {
            if let Some(last) = out.last_mut() {
                if last.0 == pos {
                    last.1 = flags;
                    last.2 = r;
                    return flags & STRAIGHT_END == 0 && out.len() < max;
                }
            }
            out.push((pos, flags, r));
            out.len() < max && flags & STRAIGHT_END == 0
        }
        if !append(&mut out, max_points, closest_start, 1, path[0]) {
            return Some(out);
        }
        if path.len() > 1 {
            let mut apex = closest_start;
            let (mut left_v, mut right_v) = (apex, apex);
            let (mut apex_i, mut left_i, mut right_i) = (0usize, 0usize, 0usize);
            let (mut left_ref, mut right_ref) = (path[0], path[0]);
            let mut i = 0usize;
            while i < path.len() {
                let (left, right);
                if i + 1 < path.len() {
                    match self.portal_points(path[i], path[i + 1]) {
                        Some(p) => (left, right) = p,
                        None => {
                            // The path ends at the last polygon it could be followed to.
                            let end = self.closest_point_on_poly_boundary(path[i], end_pos)?;
                            append(&mut out, max_points, end, 0, path[i]);
                            return Some(out);
                        }
                    }
                    // If starting really close to the portal, advance.
                    if i == 0 && dist_pt_seg_sqr_2d(apex, left, right).0 < 0.001 * 0.001 {
                        i += 1;
                        continue;
                    }
                } else {
                    (left, right) = (closest_end, closest_end);
                }
                // Right vertex.
                if tri_area_2d(apex, right_v, right) <= 0.0 {
                    if apex == right_v || tri_area_2d(apex, left_v, right) > 0.0 {
                        right_v = right;
                        right_ref = if i + 1 < path.len() { path[i + 1] } else { NO_POLY };
                        right_i = i;
                    } else {
                        // Left over right: the left vertex becomes the new apex.
                        if !append(&mut out, max_points, left_v, 0, left_ref) {
                            return Some(out);
                        }
                        apex = left_v;
                        apex_i = left_i;
                        (left_v, right_v) = (apex, apex);
                        (left_i, right_i) = (apex_i, apex_i);
                        (left_ref, right_ref) = (NO_POLY, NO_POLY);
                        i = apex_i + 1;
                        continue;
                    }
                }
                // Left vertex.
                if tri_area_2d(apex, left_v, left) >= 0.0 {
                    if apex == left_v || tri_area_2d(apex, right_v, left) < 0.0 {
                        left_v = left;
                        left_ref = if i + 1 < path.len() { path[i + 1] } else { NO_POLY };
                        left_i = i;
                    } else {
                        if !append(&mut out, max_points, right_v, 0, right_ref) {
                            return Some(out);
                        }
                        apex = right_v;
                        apex_i = right_i;
                        (left_v, right_v) = (apex, apex);
                        (left_i, right_i) = (apex_i, apex_i);
                        (left_ref, right_ref) = (NO_POLY, NO_POLY);
                        i = apex_i + 1;
                        continue;
                    }
                }
                i += 1;
            }
        }
        append(&mut out, max_points, closest_end, STRAIGHT_END, NO_POLY);
        Some(out)
    }

    /// Slide from `start_pos` towards `end_pos` over the surface (`moveAlongSurface`): where it ends up, and the polygons it went through (the first is `start`).
    pub fn move_along_surface(&self, start: PolyRef, start_pos: Vec3, end_pos: Vec3, max_visited: usize) -> Option<(Vec3, Vec<PolyRef>)> {
        if start == NO_POLY {
            return None;
        }
        // Visited polygons with the one they were reached from.
        let mut parents: HashMap<PolyRef, Option<PolyRef>> = HashMap::from([(start, None)]);
        let mut queue = VecDeque::from([start]);
        let mut best_pos = start_pos;
        let mut best_dist = f32::MAX;
        let mut best_node: Option<PolyRef> = None;
        // Search constraint.
        let search_pos = (start_pos + end_pos) * 0.5;
        let search_rad_sqr = (start_pos.distance(end_pos) / 2.0 + 0.001).powi(2);
        while let Some(cur) = queue.pop_front() {
            let (tile, poly) = self.mesh.poly(cur);
            let verts = tile.corners(poly);
            // If the target is inside the polygon, stop the search.
            if point_in_polygon(end_pos, &verts) {
                best_node = Some(cur);
                best_pos = end_pos;
                break;
            }
            // Find wall edges and the nearest point inside the walls.
            let nv = verts.len();
            let mut j = nv - 1;
            for i in 0..nv {
                let mut neis: Vec<PolyRef> = Vec::new();
                if poly.neis[j] & EXT_LINK != 0 {
                    // Tile border.
                    for link in self.mesh.links(cur) {
                        if link.edge as usize == j && link.to != NO_POLY && self.passes(link.to) {
                            neis.push(link.to);
                        }
                    }
                } else if poly.neis[j] != 0 {
                    let r = make_ref((cur >> 16) as usize - 1, poly.neis[j] as usize - 1);
                    if self.passes(r) {
                        neis.push(r);
                    }
                }
                let (vj, vi) = (verts[j], verts[i]);
                if neis.is_empty() {
                    // Wall edge, calculate the distance.
                    let (d, t) = dist_pt_seg_sqr_2d(end_pos, vj, vi);
                    if d < best_dist {
                        best_pos = vj.lerp(vi, t);
                        best_dist = d;
                        best_node = Some(cur);
                    }
                } else {
                    for n in neis {
                        if parents.contains_key(&n) {
                            continue;
                        }
                        // Skip if the edge is too far from the search constraint.
                        if dist_pt_seg_sqr_2d(search_pos, vj, vi).0 > search_rad_sqr {
                            continue;
                        }
                        parents.insert(n, Some(cur));
                        queue.push_back(n);
                    }
                }
                j = i;
            }
        }
        let mut visited = Vec::new();
        let mut at = best_node;
        while let Some(r) = at {
            visited.push(r);
            at = parents[&r];
        }
        visited.reverse();
        visited.truncate(max_visited);
        Some((best_pos, visited))
    }

    /// Cast a ray over the polygons from `start` (`raycast`): how far along it (0..1) it gets before a wall, or `None` if it reaches the end, and the polygons
    /// it went through.
    pub fn raycast(&self, start: PolyRef, start_pos: Vec3, end_pos: Vec3, max_path: usize) -> Option<(Option<f32>, Vec<PolyRef>)> {
        if start == NO_POLY {
            return None;
        }
        let mut cur = start;
        let mut path = Vec::new();
        let mut t = 0.0f32;
        loop {
            let (tile, poly) = self.mesh.poly(cur);
            let verts = tile.corners(poly);
            let Some((_, tmax, _, seg_max)) = intersect_segment_poly_2d(start_pos, end_pos, &verts) else {
                // Could not hit the polygon: keep the old t and report a hit.
                return Some((Some(t), path));
            };
            if tmax > t {
                t = tmax;
            }
            if path.len() < max_path {
                path.push(cur);
            }
            // The ray end is completely inside the polygon.
            if seg_max == -1 {
                return Some((None, path));
            }
            // Follow the neighbours.
            let mut next = NO_POLY;
            for link in self.mesh.links(cur) {
                if link.edge as i32 != seg_max || !self.passes(link.to) {
                    continue;
                }
                // An internal link: the polygon behind it.
                if link.side == NO_SIDE {
                    next = link.to;
                    break;
                }
                // At a tile border: the crossing has to lie inside the portal.
                let e = link.edge as usize;
                let (left, right) = (verts[e], verts[(e + 1) % poly.count]);
                if link.side == 0 || link.side == 4 {
                    let (mut lmin, mut lmax) = (left.z + (right.z - left.z) * (link.min as f32 / 255.0), left.z + (right.z - left.z) * (link.max as f32 / 255.0));
                    if lmin > lmax {
                        std::mem::swap(&mut lmin, &mut lmax);
                    }
                    let z = start_pos.z + (end_pos.z - start_pos.z) * tmax;
                    if z >= lmin && z <= lmax {
                        next = link.to;
                        break;
                    }
                } else if link.side == 2 || link.side == 6 {
                    let (mut lmin, mut lmax) = (left.x + (right.x - left.x) * (link.min as f32 / 255.0), left.x + (right.x - left.x) * (link.max as f32 / 255.0));
                    if lmin > lmax {
                        std::mem::swap(&mut lmin, &mut lmax);
                    }
                    let x = start_pos.x + (end_pos.x - start_pos.x) * tmax;
                    if x >= lmin && x <= lmax {
                        next = link.to;
                        break;
                    }
                }
            }
            if next == NO_POLY {
                // No neighbour: a wall.
                return Some((Some(t), path));
            }
            cur = next;
        }
    }
}
