//! Triangle-soup collision for placed objects (WMOs and M2 collision meshes).
//!
//! Triangles are bucketed into a uniform XZ grid. Queries are the two things the
//! player controller needs: the highest floor under a point, and a push-out that
//! keeps a vertical cylinder away from wall-like triangles.
//! Triangles are treated as double-sided: the winding of the source data is not relied on.

use glam::Vec3;
use std::collections::HashMap;

const CELL: f32 = 4.0;
/// cos(50 degrees): surfaces flatter than this can be stood on, steeper ones are walls.
pub const WALKABLE_NORMAL_Y: f32 = 0.6428;

#[derive(Clone, Copy)]
pub struct Tri {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    /// Unit normal; its sign is meaningless (see module docs).
    pub n: Vec3,
    /// Highest vertex; lets wall tests skip anything entirely below the step height.
    pub max_y: f32,
}

impl Tri {
    pub fn new(a: Vec3, b: Vec3, c: Vec3) -> Option<Tri> {
        let n = (b - a).cross(c - a);
        let len = n.length();
        (len > 1e-6).then(|| Tri { a, b, c, n: n / len, max_y: a.y.max(b.y).max(c.y) })
    }
}

#[derive(Default)]
pub struct CollisionMesh {
    tris: Vec<Tri>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

fn cell_of(v: f32) -> i32 {
    (v / CELL).floor() as i32
}

impl CollisionMesh {
    pub fn build(tris: Vec<Tri>) -> CollisionMesh {
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, t) in tris.iter().enumerate() {
            let (min_x, max_x) = (t.a.x.min(t.b.x).min(t.c.x), t.a.x.max(t.b.x).max(t.c.x));
            let (min_z, max_z) = (t.a.z.min(t.b.z).min(t.c.z), t.a.z.max(t.b.z).max(t.c.z));
            for cz in cell_of(min_z)..=cell_of(max_z) {
                for cx in cell_of(min_x)..=cell_of(max_x) {
                    grid.entry((cx, cz)).or_default().push(i as u32);
                }
            }
        }
        CollisionMesh { tris, grid }
    }

    /// Centroids of triangles whose angle from horizontal lies in `[min_deg, max_deg]` (developer tool).
    pub fn centroids_with_slope(&self, min_deg: f32, max_deg: f32) -> Vec<Vec3> {
        self.tris
            .iter()
            .filter(|t| {
                let deg = t.n.y.abs().clamp(0.0, 1.0).acos().to_degrees();
                deg >= min_deg && deg <= max_deg
            })
            .map(|t| (t.a + t.b + t.c) / 3.0)
            .collect()
    }

    /// Triangles whose centroid lies within `radius` of `(x, z)` in this mesh's space (developer tool).
    pub fn triangles_near(&self, x: f32, z: f32, radius: f32) -> Vec<[Vec3; 3]> {
        self.tris
            .iter()
            .filter(|t| {
                let c = (t.a + t.b + t.c) / 3.0;
                (c.x - x).hypot(c.z - z) <= radius
            })
            .map(|t| [t.a, t.b, t.c])
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.tris.len()
    }

    /// Highest walkable surface under `(x, z)` that is not above `max_y`.
    pub fn floor(&self, x: f32, z: f32, max_y: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        for &i in self.grid.get(&(cell_of(x), cell_of(z)))? {
            let t = &self.tris[i as usize];
            if t.n.y.abs() < WALKABLE_NORMAL_Y || !inside_xz(t, x, z) {
                continue;
            }
            let y = t.a.y - (t.n.x * (x - t.a.x) + t.n.z * (z - t.a.z)) / t.n.y;
            if y <= max_y && best.is_none_or(|b| y > b) {
                best = Some(y);
            }
        }
        best
    }

    /// Where the segment `from -> to` first hits a triangle (either side), as a fraction of the segment.
    pub fn raycast(&self, from: Vec3, to: Vec3) -> Option<f32> {
        let dir = to - from;
        let mut best: Option<f32> = None;
        self.for_each_in_box(from.min(to), from.max(to), |t| {
            // Moeller-Trumbore.
            let (e1, e2) = (t.b - t.a, t.c - t.a);
            let p = dir.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-9 {
                return;
            }
            let s = from - t.a;
            let u = s.dot(p) / det;
            let q = s.cross(e1);
            let v = dir.dot(q) / det;
            let h = e2.dot(q) / det;
            if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && (0.0..=1.0).contains(&h) && best.is_none_or(|b| h < b) {
                best = Some(h);
            }
        });
        best
    }

    /// Call `f` for every triangle whose XZ bounds overlap the box `[min, max]` (each once).
    pub fn for_each_in_box(&self, min: Vec3, max: Vec3, mut f: impl FnMut(&Tri)) {
        let mut seen: Vec<u32> = Vec::new();
        for cz in cell_of(min.z)..=cell_of(max.z) {
            for cx in cell_of(min.x)..=cell_of(max.x) {
                if let Some(v) = self.grid.get(&(cx, cz)) {
                    seen.extend_from_slice(v);
                }
            }
        }
        seen.sort_unstable();
        seen.dedup();
        for i in seen {
            let t = &self.tris[i as usize];
            if t.max_y >= min.y && t.a.y.min(t.b.y).min(t.c.y) <= max.y {
                f(t);
            }
        }
    }
}

fn inside_xz(t: &Tri, x: f32, z: f32) -> bool {
    // Barycentric test in the XZ plane.
    let (v0x, v0z) = (t.c.x - t.a.x, t.c.z - t.a.z);
    let (v1x, v1z) = (t.b.x - t.a.x, t.b.z - t.a.z);
    let (v2x, v2z) = (x - t.a.x, z - t.a.z);
    let d00 = v0x * v0x + v0z * v0z;
    let d01 = v0x * v1x + v0z * v1z;
    let d11 = v1x * v1x + v1z * v1z;
    let d20 = v2x * v0x + v2z * v0z;
    let d21 = v2x * v1x + v2z * v1z;
    let denom = d00 * d11 - d01 * d01;
    if denom.abs() < 1e-9 {
        return false;
    }
    let u = (d11 * d20 - d01 * d21) / denom;
    let v = (d00 * d21 - d01 * d20) / denom;
    const E: f32 = -1e-4;
    u >= E && v >= E && u + v <= 1.0 - E
}

/// What the player controller needs to know about the world.
pub trait Collider {
    /// Highest walkable surface (terrain or object) at `(x, z)` no higher than `max_y`.
    fn floor(&self, x: f32, z: f32, max_y: f32) -> Option<f32>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> Vec<Tri> {
        [Tri::new(a, b, c), Tri::new(a, c, d)].into_iter().flatten().collect()
    }

    #[test]
    fn floor_finds_the_highest_surface_below_the_limit() {
        // Two stacked floors at y = 0 and y = 5.
        let mut tris = quad(Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 10.0), Vec3::new(0.0, 0.0, 10.0));
        tris.extend(quad(Vec3::new(0.0, 5.0, 0.0), Vec3::new(10.0, 5.0, 0.0), Vec3::new(10.0, 5.0, 10.0), Vec3::new(0.0, 5.0, 10.0)));
        let m = CollisionMesh::build(tris);
        assert_eq!(m.floor(5.0, 5.0, 100.0), Some(5.0));
        assert_eq!(m.floor(5.0, 5.0, 4.0), Some(0.0));
        assert_eq!(m.floor(5.0, 5.0, -1.0), None);
        assert_eq!(m.floor(50.0, 5.0, 100.0), None);
    }

    #[test]
    fn rays_stop_at_the_first_triangle_hit() {
        // A wall in the plane x = 5 and a second one at x = 8.
        let wall = |x: f32| quad(Vec3::new(x, 0.0, 0.0), Vec3::new(x, 10.0, 0.0), Vec3::new(x, 10.0, 10.0), Vec3::new(x, 0.0, 10.0));
        let mut tris = wall(5.0);
        tris.extend(wall(8.0));
        let m = CollisionMesh::build(tris);
        let hit = m.raycast(Vec3::new(0.0, 5.0, 5.0), Vec3::new(10.0, 5.0, 5.0)).unwrap();
        assert!((hit - 0.5).abs() < 1e-4, "{hit}");
        // From the far side too, a ray that stops short, and one that misses beside the wall.
        assert!((m.raycast(Vec3::new(10.0, 5.0, 5.0), Vec3::new(0.0, 5.0, 5.0)).unwrap() - 0.2).abs() < 1e-4);
        assert!(m.raycast(Vec3::new(0.0, 5.0, 5.0), Vec3::new(4.0, 5.0, 5.0)).is_none());
        assert!(m.raycast(Vec3::new(0.0, 5.0, 15.0), Vec3::new(10.0, 5.0, 15.0)).is_none());
    }

    #[test]
    fn floor_follows_a_ramp() {
        // 30 degree ramp rising along +x: y = 0.5 * x.
        let m = CollisionMesh::build(quad(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(10.0, 5.0, 0.0),
            Vec3::new(10.0, 5.0, 10.0),
            Vec3::new(0.0, 0.0, 10.0),
        ));
        let y = m.floor(4.0, 3.0, 100.0).unwrap();
        assert!((y - 2.0).abs() < 1e-4, "{y}");
    }

    #[test]
    fn steep_surfaces_are_not_floors_but_walls() {
        // 75 degree slope: not walkable.
        let m = CollisionMesh::build(quad(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 7.0, 0.0),
            Vec3::new(2.0, 7.0, 10.0),
            Vec3::new(0.0, 0.0, 10.0),
        ));
        assert_eq!(m.floor(1.0, 5.0, 100.0), None);
    }
}
