//! Which groups of a building are drawn, following the original client's portal traversal
//! (`FUN_006b3b20` camera inside, `FUN_006b3dd0` outside, `FUN_006b41c0` the recursive walk).
//!
//! Drawing starts at the group(s) the camera is in, or at the outdoor groups (`0x8`) when it is
//! outside. From a group the walk crosses a portal into the neighbouring group only if the camera is
//! on the portal's near side and the portal's screen rectangle overlaps the rectangle seen so far
//! (which it then narrows). Standalone groups (`0x10000`) are drawn on their own. A group that no
//! walk reaches - e.g. Stormwind's spires, seen only through a few canal portals - stays hidden.

use crate::models::WmoCpu;
use glam::{Mat4, Vec3, Vec4};
use wowfmt::wmo::PortalRef;

/// WMO model space (x east, y north, z up) to render space (x east, y up, z south).
pub const MODEL_TO_RENDER: Mat4 = Mat4::from_cols_array(&[1., 0., 0., 0., 0., 0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1.]);

const OUTDOOR: u32 = 0x8;
const STANDALONE: u32 = 0x10000;
const MAX_DEPTH: u32 = 32;
/// Smallest screen-space extent (NDC) a narrowed rectangle may have.
const MIN_EXTENT: f32 = 1e-4;
/// Vertices closer than this (clip-space w) are behind the near plane.
const NEAR_W: f32 = 1e-3;
/// How far below the camera a group's floor may be for the camera to count as inside the group.
const FLOOR_REACH: f32 = 12.0;
/// How close to a portal (and its frame) the camera counts as standing in it.
const DOORWAY: f32 = 0.75;

#[derive(Clone, Copy, Debug)]
struct Rect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

const FULL: Rect = Rect { x0: -1.0, y0: -1.0, x1: 1.0, y1: 1.0 };

impl Rect {
    fn intersect(self, o: Rect) -> Option<Rect> {
        let r = Rect { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) };
        (r.x1 - r.x0 >= MIN_EXTENT && r.y1 - r.y0 >= MIN_EXTENT).then_some(r)
    }
    fn union(self, o: Rect) -> Rect {
        Rect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
    fn contains(self, o: Rect) -> bool {
        self.x0 <= o.x0 && self.y0 <= o.y0 && o.x1 <= self.x1 && o.y1 <= self.y1
    }
}

/// Screen rectangle of a convex polygon after clipping it to the near plane; `None` if it is
/// entirely behind the camera.
fn project(points: &[Vec3], to_clip: &Mat4) -> Option<Rect> {
    let clip: Vec<Vec4> = points.iter().map(|&p| *to_clip * p.extend(1.0)).collect();
    let mut poly = Vec::with_capacity(clip.len() + 2);
    for i in 0..clip.len() {
        let (a, b) = (clip[i], clip[(i + 1) % clip.len()]);
        let (ain, bin) = (a.w >= NEAR_W, b.w >= NEAR_W);
        if ain {
            poly.push(a);
        }
        if ain != bin {
            poly.push(a.lerp(b, (NEAR_W - a.w) / (b.w - a.w)));
        }
    }
    let first = poly.first()?;
    let mut r = Rect { x0: first.x / first.w, y0: first.y / first.w, x1: first.x / first.w, y1: first.y / first.w };
    for v in &poly {
        r = r.union(Rect { x0: v.x / v.w, y0: v.y / v.w, x1: v.x / v.w, y1: v.y / v.w });
    }
    Some(r)
}

fn bbox_corners(b: &([f32; 3], [f32; 3])) -> [Vec3; 8] {
    let (lo, hi) = (Vec3::from(b.0), Vec3::from(b.1));
    std::array::from_fn(|i| {
        Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    })
}

/// Screen rectangle of a group's bounds. A box straddling the near plane counts as covering the screen.
fn bbox_rect(b: &([f32; 3], [f32; 3]), to_clip: &Mat4) -> Option<Rect> {
    let corners = bbox_corners(b);
    let behind = corners.iter().filter(|c| (*to_clip * c.extend(1.0)).w < NEAR_W).count();
    match behind {
        0 => project(&corners, to_clip),
        8 => None,
        _ => Some(FULL),
    }
}

/// The group the camera is standing in: of the groups whose bounds contain it, the one with the
/// highest solid floor below it (within `FLOOR_REACH`). Model space, like the portals.
fn camera_group(wmo: &WmoCpu, cam: Vec3) -> Option<usize> {
    // Collision meshes are in render space (x, z, -y).
    let c = Vec3::new(cam.x, cam.z, -cam.y);
    let mut best: Option<(usize, f32)> = None;
    for (g, group) in wmo.groups.iter().enumerate() {
        let (lo, hi) = (Vec3::from(group.bbox.0) - 0.5, Vec3::from(group.bbox.1) + 0.5);
        let Some(model) = &group.model else { continue };
        if cam.cmplt(lo).any() || cam.cmpgt(hi).any() {
            continue;
        }
        for t in model.col_indices.chunks_exact(3) {
            let p = |i: u32| model.col_vertices[i as usize];
            let (a, b, d) = (p(t[0]), p(t[1]), p(t[2]));
            // Barycentric coordinates in the xz plane.
            let det = (b[2] - d[2]) * (a[0] - d[0]) + (d[0] - b[0]) * (a[2] - d[2]);
            if det.abs() < 1e-9 {
                continue;
            }
            let l1 = ((b[2] - d[2]) * (c.x - d[0]) + (d[0] - b[0]) * (c.z - d[2])) / det;
            let l2 = ((d[2] - a[2]) * (c.x - d[0]) + (a[0] - d[0]) * (c.z - d[2])) / det;
            let l3 = 1.0 - l1 - l2;
            if l1 < 0.0 || l2 < 0.0 || l3 < 0.0 {
                continue;
            }
            let floor = l1 * a[1] + l2 * b[1] + l3 * d[1];
            // An indoor group's bounds already stop at its roof, so a camera mid-fall in a tall room is still in it.
            let reach = if group.flags & OUTDOOR != 0 { FLOOR_REACH } else { f32::INFINITY };
            if floor <= c.y + 0.5 && c.y - floor <= reach && best.map_or(true, |(_, f)| floor > f) {
                best = Some((g, floor));
            }
        }
    }
    // A camera far above its floor may be in a group above it (a gallery over the hall): when it is behind a
    // portal into a group whose bounds hold it, it is in that group.
    let (mut g, floor) = best?;
    for _ in 0..if c.y - floor > FLOOR_REACH { MAX_DEPTH } else { 0 } {
        let (first, count) = wmo.groups[g].portals;
        let next = (first as usize..(first + count) as usize).filter_map(|i| wmo.portal_refs.get(i)).find(|r| {
            let Some(dest) = wmo.groups.get(r.group as usize) else { return false };
            let (lo, hi) = (Vec3::from(dest.bbox.0) - 0.5, Vec3::from(dest.bbox.1) + 0.5);
            r.group as usize != g && !cam.cmplt(lo).any() && !cam.cmpgt(hi).any() && behind(wmo, r, cam)
        });
        match next {
            Some(r) => g = r.group as usize,
            None => break,
        }
    }
    Some(g)
}

/// Whether `cam` is within `DOORWAY` of a portal's plane and of its frame.
fn in_doorway(wmo: &WmoCpu, portal: usize, cam: Vec3) -> bool {
    let p = wmo.portals[portal];
    let Some(verts) = wmo.portal_verts.get(p.start as usize..(p.start + p.count) as usize) else { return false };
    let dist = Vec3::new(p.plane[0], p.plane[1], p.plane[2]).dot(cam) + p.plane[3];
    let (lo, hi) = verts.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), &v| (lo.min(Vec3::from(v)), hi.max(Vec3::from(v))));
    dist.abs() < DOORWAY && !cam.cmplt(lo - DOORWAY).any() && !cam.cmpgt(hi + DOORWAY).any()
}

/// Whether `cam` is on the far side of the portal a group reference points through. Standing in the
/// doorway it is on neither: the floor lookup flips between the groups there, and the view must not.
fn behind(wmo: &WmoCpu, r: &PortalRef, cam: Vec3) -> bool {
    let Some(p) = wmo.portals.get(r.portal as usize) else { return false };
    if in_doorway(wmo, r.portal as usize, cam) {
        return false;
    }
    let side = Vec3::new(p.plane[0], p.plane[1], p.plane[2]).dot(cam) + p.plane[3];
    (if r.side < 0 { -side } else { side }) < 0.0
}

/// The `WMOAreaTable` numbers (building, group) of the room a point (model space) is in.
pub fn room_at(wmo: &WmoCpu, point: Vec3) -> Option<(u32, u32)> {
    camera_group(wmo, point).map(|g| (wmo.id, wmo.groups[g].id))
}

/// Whether a point (model space) is inside the building, in one of its groups that is not an outdoor one (a street or a courtyard).
pub fn indoors(wmo: &WmoCpu, point: Vec3) -> bool {
    camera_group(wmo, point).is_some_and(|g| wmo.groups[g].flags & OUTDOOR == 0)
}

struct Walk<'a> {
    wmo: &'a WmoCpu,
    cam: Vec3,
    to_clip: &'a Mat4,
    /// The group the camera is in, if any.
    home: Option<usize>,
    visible: Vec<bool>,
    /// Union of the rectangles each group was already walked with.
    seen: Vec<Option<Rect>>,
    portal_rects: Vec<Option<Option<Rect>>>,
    /// Rectangles through which an outdoor group was entered (camera inside only).
    outdoor_views: Option<Vec<Rect>>,
}

impl Walk<'_> {
    fn portal_rect(&mut self, portal: usize) -> Option<Rect> {
        *self.portal_rects[portal].get_or_insert_with(|| {
            let p = self.wmo.portals[portal];
            let verts = self.wmo.portal_verts.get(p.start as usize..(p.start + p.count) as usize)?;
            let pts: Vec<Vec3> = verts.iter().map(|&v| Vec3::from(v)).collect();
            // A camera standing in the doorway sees the polygon edge-on or behind the near plane: it covers the screen.
            if in_doorway(self.wmo, portal, self.cam) {
                return Some(FULL);
            }
            project(&pts, self.to_clip)
        })
    }

    fn visit(&mut self, group: usize, from: Option<usize>, rect: Rect, depth: u32) {
        self.visible[group] = true;
        if depth >= MAX_DEPTH {
            return;
        }
        let (first, count) = self.wmo.groups[group].portals;
        for i in first as usize..(first + count) as usize {
            let Some(&r) = self.wmo.portal_refs.get(i) else { break };
            let dest = r.group as usize;
            if dest >= self.visible.len() || Some(dest) == from || r.portal as usize >= self.wmo.portals.len() {
                continue;
            }
            // The camera's own group is left through any of its portals: in a doorway the floor lookup flips
            // between the two groups while the camera crosses the plane, and the side test must not flicker with it.
            let home = from.is_none() && self.home == Some(group);
            if !home && behind(self.wmo, &r, self.cam) {
                continue;
            }
            let Some(clipped) = self.portal_rect(r.portal as usize).and_then(|p| rect.intersect(p)) else { continue };
            // Already walked with a rectangle at least this big: nothing new can show up.
            if self.seen[dest].is_some_and(|s| s.contains(clipped)) {
                continue;
            }
            self.seen[dest] = Some(self.seen[dest].map_or(clipped, |s| s.union(clipped)));
            if let (Some(views), true) = (&mut self.outdoor_views, self.wmo.groups[dest].flags & OUTDOOR != 0) {
                views.push(clipped);
            }
            self.visit(dest, Some(group), clipped, depth + 1);
        }
    }
}

/// Visibility of every group of `wmo` for a camera at `cam` (model space), where `to_clip` maps
/// model space to clip space.
pub fn visible_groups(wmo: &WmoCpu, cam: Vec3, to_clip: &Mat4) -> Vec<bool> {
    let n = wmo.groups.len();
    // Buildings without outdoor groups (dungeons) are only seen from inside: show all of them.
    if !wmo.groups.iter().any(|g| g.flags & OUTDOOR != 0) {
        return vec![true; n];
    }
    let inside = camera_group(wmo, cam);
    let mut walk = Walk {
        wmo,
        cam,
        to_clip,
        home: inside,
        visible: vec![false; n],
        seen: vec![None; n],
        portal_rects: vec![None; wmo.portals.len()],
        outdoor_views: inside.map(|_| Vec::new()),
    };
    let outdoor = |g: usize| wmo.groups[g].flags & (OUTDOOR | STANDALONE) == OUTDOOR;
    let mut views = vec![FULL];
    if let Some(g) = inside {
        walk.visit(g, None, FULL, 0);
        views = walk.outdoor_views.take().unwrap_or_default();
        walk.outdoor_views = Some(Vec::new());
    }
    // Outdoor groups are not joined by portals: every one whose bounds show in a view is a start.
    for view in views {
        for g in (0..n).filter(|&g| outdoor(g)) {
            if let Some(r) = bbox_rect(&wmo.groups[g].bbox, to_clip).and_then(|b| view.intersect(b)) {
                walk.visit(g, None, r, 0);
            }
        }
    }
    for g in (0..n).filter(|&g| wmo.groups[g].flags & STANDALONE != 0) {
        if bbox_rect(&wmo.groups[g].bbox, to_clip).is_some() {
            walk.visible[g] = true;
        }
    }
    walk.visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{WmoCpu, WmoGroupCpu};
    use wowfmt::wmo::{PortalRef, WmoPortal};

    /// Camera at the origin looking down -z (model space is used directly as view space).
    fn clip() -> Mat4 {
        Mat4::perspective_rh(1.0, 1.0, 0.1, 500.0)
    }

    /// Group 0 is outdoor with a portal square 10 ahead leading to group 1; group 2 is unconnected;
    /// group 3 sits behind a portal the camera is on the far side of.
    fn building() -> WmoCpu {
        let sq = |z: f32, x: f32| [[x - 2.0, -2.0, z], [x + 2.0, -2.0, z], [x + 2.0, 2.0, z], [x - 2.0, 2.0, z]];
        let mut portal_verts = Vec::new();
        for v in sq(-10.0, 0.0).into_iter().chain(sq(-10.0, 0.0)) {
            portal_verts.push(v);
        }
        let bbox = ([-50.0, -50.0, -50.0], [50.0, 50.0, 50.0]);
        let group = |flags, portals| WmoGroupCpu { id: 0, flags, bbox, portals, ..Default::default() };
        WmoCpu {
            groups: vec![group(OUTDOOR, (0, 2)), group(0x2000, (2, 0)), group(0x2000, (2, 0)), group(0x2000, (2, 0))],
            portal_verts,
            portals: vec![
                WmoPortal { start: 0, count: 4, plane: [0.0, 0.0, 1.0, 10.0] },
                WmoPortal { start: 4, count: 4, plane: [0.0, 0.0, 1.0, 10.0] },
            ],
            portal_refs: vec![
                PortalRef { portal: 0, group: 1, side: 1 },
                // Same polygon, but the camera is on the negative side of this reference.
                PortalRef { portal: 1, group: 3, side: -1 },
            ],
            ..Default::default()
        }
    }

    /// From the street inside Stormwind's gate no heading shows the floating spire groups 109 and
    /// 223, which only the canal groups can see through one-way portals. (Group 110 is reachable
    /// through a portal of the gate street itself and shows as the distant thin spire.)
    #[test]
    fn real_stormwind_hides_the_spire_groups_from_the_trade_district() {
        use crate::camera::Camera;
        use crate::models::{placement_matrix, ModelCache};
        let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else {
            return eprintln!("skipped: no game data");
        };
        let wmo = ModelCache::default().get_wmo(&mount, r"World\wmo\Azeroth\Buildings\Stormwind\Stormwind.wmo");
        // File-space placement of the building (tile 32,48 of Azeroth) and a camera at WoW (-8875, 588, 95).
        let matrix = placement_matrix(Vec3::new(16527.373, 101.957, 25998.3), [0.0, 38.5, 0.0], 1.0);
        let cam = Vec3::new(16478.67, 95.0, 25941.67);
        let r = matrix.inverse().transform_point3(cam);
        let cam_model = Vec3::new(r.x, -r.z, r.y);
        let mut most = 0;
        for yaw in (0..360).step_by(15) {
            for pitch in [0.0f32, 25.0, 50.0] {
                let vp = Camera::new(cam, (yaw as f32).to_radians(), pitch.to_radians()).view_proj_relative();
                let to_clip = vp * Mat4::from_translation(-cam) * matrix * MODEL_TO_RENDER;
                let visible = visible_groups(&wmo, cam_model, &to_clip);
                assert!(!visible[109] && !visible[223], "spires visible at yaw {yaw} pitch {pitch}");
                most = most.max(visible.iter().filter(|&&v| v).count());
            }
        }
        assert!(most > 10, "only {most} groups ever visible: the street itself must draw");
    }

    #[test]
    fn portals_are_followed_only_in_view_and_from_the_right_side() {
        let wmo = building();
        let view = clip();
        // Looking at the portal: 0 (outdoor start) and 1 (through the portal); 2 unreachable, 3 wrong side.
        assert_eq!(visible_groups(&wmo, Vec3::ZERO, &view), [true, true, false, false]);
        // Looking the other way: the portal is behind the camera, so only the start shows.
        let away = view * Mat4::from_rotation_y(std::f32::consts::PI);
        assert_eq!(visible_groups(&wmo, Vec3::ZERO, &away), [true, false, false, false]);
    }

    #[test]
    fn standing_in_the_doorway_sees_both_sides_whichever_way_the_camera_leans() {
        let wmo = building();
        for z in [-9.8, -10.0, -10.3] {
            assert!(visible_groups(&wmo, Vec3::new(0.0, 0.0, z), &clip())[1], "group 1 hidden at z {z}");
        }
        // Well past the plane the portal is crossed no more.
        assert!(!visible_groups(&wmo, Vec3::new(0.0, 0.0, -12.0), &clip())[1]);
    }
}
