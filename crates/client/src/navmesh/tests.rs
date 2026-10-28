use super::mesh::*;
use super::query::*;
use super::pathfinder::*;
use crate::movement::{Tri, TriangleSource};
use glam::Vec3;

/// Triangles in WoW coordinates (x north, y west, z up), and triangles of liquid.
struct Soup(Vec<Tri>, Vec<crate::movement::Wet>);

impl TriangleSource for Soup {
    fn gather_liquid(&self, origin: [f64; 3], _min: Vec3, _max: Vec3, out: &mut Vec<crate::movement::Wet>) {
        let o = Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32);
        out.extend(self.1.iter().map(|w| crate::movement::Wet { v: w.v.map(|p| p - o), ..*w }));
    }

    fn gather(&self, origin: [f64; 3], _min: Vec3, _max: Vec3, out: &mut Vec<Tri>) {
        let o = Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32);
        out.extend(self.0.iter().filter_map(|t| Tri::new(t.v[0] - o, t.v[1] - o, t.v[2] - o)));
    }
}

fn quad(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> [Tri; 2] {
    [Tri::new(a, b, c).unwrap(), Tri::new(a, c, d).unwrap()]
}

/// A floor 120 yards square round the origin, with a wall 5 yards high across x = 10 from y = -30 to 30, as a mesh of tiles.
pub(crate) fn walled_floor() -> NavMesh {
    let mut tris = Vec::new();
    let s = 60.0;
    tris.extend(quad(Vec3::new(-s, -s, 0.0), Vec3::new(s, -s, 0.0), Vec3::new(s, s, 0.0), Vec3::new(-s, s, 0.0)));
    tris.extend(quad(Vec3::new(10.0, -30.0, 0.0), Vec3::new(10.0, 30.0, 0.0), Vec3::new(10.0, 30.0, 5.0), Vec3::new(10.0, -30.0, 5.0)));
    let soup = Soup(tris, Vec::new());
    let mut mesh = NavMesh::default();
    let (mid, reach) = (800, 4);
    for tx in mid - reach..=mid + reach {
        for ty in mid - reach..=mid + reach {
            if let Some(tile) = super::build(&soup, tx, ty) {
                mesh.add(&tile);
            }
        }
    }
    mesh
}

fn ground() -> Filter {
    Filter { include: NAV_GROUND, exclude: 0 }
}

#[test]
fn a_path_over_the_mesh_goes_round_a_wall_and_across_the_tiles() {
    let mesh = walled_floor();
    assert!(mesh.tile_count() > 20, "{} tiles", mesh.tile_count());
    let q = Query { mesh: &mesh, filter: ground() };
    let (start, end) = (to_nav(Vec3::new(0.0, 0.0, 0.0)), to_nav(Vec3::new(20.0, 0.0, 0.0)));
    let extents = Vec3::splat(5.0);
    let (sp, sc) = q.find_nearest_poly(start, extents).expect("a polygon under the start");
    let (ep, ec) = q.find_nearest_poly(end, extents).expect("a polygon under the end");
    assert!((sc.y - 0.0).abs() < 0.5 && (ec.y - 0.0).abs() < 0.5, "heights {sc:?} {ec:?}");
    let (polys, reach) = q.find_path(sp, ep, sc, ec, 74).expect("a path");
    assert_eq!(reach, Reach::Whole);
    let points = q.find_straight_path(sc, ec, &polys, 74).expect("corners");
    let length: f32 = points.windows(2).map(|w| w[0].0.distance(w[1].0)).sum();
    // Round the end of the wall (30 yards to the side) and back: much further than the 20 yards across.
    assert!(length > 60.0 && length < 90.0, "length {length} over {} corners {:?}", points.len(), points.iter().map(|p| from_nav(p.0)).collect::<Vec<_>>());
    // No leg of the way crosses the wall.
    for w in points.windows(2) {
        let (a, b) = (from_nav(w[0].0), from_nav(w[1].0));
        let crosses = (a.x - 10.0) * (b.x - 10.0) < 0.0 && {
            let t = (10.0 - a.x) / (b.x - a.x);
            (a.y + t * (b.y - a.y)).abs() < 30.0
        };
        assert!(!crosses, "{a:?} to {b:?} goes through the wall");
    }
    // A ray over the mesh is stopped by the wall, and reaches a point on the same side.
    let (hit, _) = q.raycast(sp, sc, ec, 74).expect("a ray");
    assert!(hit.is_some_and(|t| t < 0.6), "{hit:?}");
    let (near, _) = to_nav_pair(&q, sp, sc, Vec3::new(-10.0, 5.0, 0.0));
    assert!(near.is_none(), "{near:?}");
}

fn to_nav_pair(q: &Query, poly: PolyRef, from: Vec3, to_wow: Vec3) -> (Option<f32>, Vec<PolyRef>) {
    q.raycast(poly, from, to_nav(to_wow), 74).unwrap()
}

#[test]
fn the_path_finder_goes_round_the_wall_and_a_ray_of_it_stops_at_the_wall() {
    let mesh = walled_floor();
    let mut finder = PathFinder::new(false);
    let (start, end) = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 0.0, 0.0));
    assert!(finder.calculate(&mesh, start, end, false, false));
    assert_eq!(finder.kind, PATHFIND_NORMAL, "{:?}", finder.path);
    let length: f32 = finder.path.windows(2).map(|w| w[0].distance(w[1])).sum();
    assert!(length > 60.0 && length < 90.0, "length {length}: {:?}", finder.path);
    assert!(finder.path.first().unwrap().distance(start) < 1.0 && finder.path.last().unwrap().distance(end) < 1.0, "{:?}", finder.path);
    for w in finder.path.windows(2) {
        let crosses = (w[0].x - 10.0) * (w[1].x - 10.0) < 0.0 && {
            let t = (10.0 - w[0].x) / (w[1].x - w[0].x);
            (w[0].y + t * (w[1].y - w[0].y)).abs() < 30.0
        };
        assert!(!crosses, "{:?} to {:?} goes through the wall", w[0], w[1]);
    }
    // The straight line over the mesh stops short at the wall: no path or an incomplete one.
    let mut ray = PathFinder::new(false);
    ray.calculate(&mesh, start, end, false, true);
    assert!(ray.kind & (PATHFIND_NOPATH | PATHFIND_INCOMPLETE) != 0, "kind {} {:?}", ray.kind, ray.path);
    // On open ground the ray goes the whole way, in steps of about four yards.
    let mut open = PathFinder::new(false);
    open.calculate(&mesh, start, Vec3::new(-30.0, 10.0, 0.0), false, true);
    assert_eq!(open.kind, PATHFIND_NORMAL);
    assert!(open.path.len() >= 8 && open.path.last().unwrap().distance(Vec3::new(-30.0, 10.0, 0.0)) < 0.5, "{:?}", open.path);
    // Far from any tile: a shortcut in a straight line, not using a path.
    let mut far = PathFinder::new(false);
    far.calculate(&mesh, start, Vec3::new(5000.0, 0.0, 0.0), false, false);
    assert_eq!(far.kind, PATHFIND_NORMAL | PATHFIND_NOT_USING_PATH);
    assert_eq!(far.path.len(), 2);
}


/// A floor 120 yards square, with a platform 5 yards high on it: x from -20 to 0, y from -10 to 10, its sides as steep as a wall.
pub(crate) fn platform_on_floor() -> NavMesh {
    let mut tris = Vec::new();
    let s = 60.0;
    tris.extend(quad(Vec3::new(-s, -s, 0.0), Vec3::new(s, -s, 0.0), Vec3::new(s, s, 0.0), Vec3::new(-s, s, 0.0)));
    let (x0, x1, y0, y1, h) = (-20.0, 0.0, -10.0, 10.0, 5.0);
    tris.extend(quad(Vec3::new(x0, y0, h), Vec3::new(x1, y0, h), Vec3::new(x1, y1, h), Vec3::new(x0, y1, h)));
    for (a, b) in [((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)), ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0))] {
        tris.extend(quad(Vec3::new(a.0, a.1, 0.0), Vec3::new(b.0, b.1, 0.0), Vec3::new(b.0, b.1, h), Vec3::new(a.0, a.1, h)));
    }
    let soup = Soup(tris, Vec::new());
    let mut mesh = NavMesh::default();
    for tx in 796..=804 {
        for ty in 796..=804 {
            if let Some(tile) = super::build(&soup, tx, ty) {
                mesh.add(&tile);
            }
        }
    }
    mesh
}

/// A floor 120 yards square with a pond cut into it, 5 cells of the terrain across, `depth` yards deep, its surface at z = 0: the pond's walls are steep, its bed is flat.
/// The liquid is a cell of the terrain's (None: the pool of a building) and covers the pond.
fn pond(depth: f32, terrain: bool) -> NavMesh {
    let mut tris = Vec::new();
    let s = 60.0;
    let unit = wowfmt::adt::UNIT_SIZE;
    // The pond lies on the cells of the terrain: its corner is the first border of a cell after -10.
    let lo = ((-10.0 + super::HALF_WORLD) / unit as f64).ceil() * unit as f64 - super::HALF_WORLD;
    let (lo, hi) = (lo as f32, lo as f32 + 5.0 * unit);
    // The floor round the pond: four strips.
    for (x0, x1, y0, y1) in [(-s, lo, -s, s), (hi, s, -s, s), (lo, hi, -s, lo), (lo, hi, hi, s)] {
        tris.extend(quad(Vec3::new(x0, y0, 0.0), Vec3::new(x1, y0, 0.0), Vec3::new(x1, y1, 0.0), Vec3::new(x0, y1, 0.0)));
    }
    // The walls, and the bed and the liquid in cells.
    for (a, b) in [((lo, lo), (hi, lo)), ((hi, lo), (hi, hi)), ((hi, hi), (lo, hi)), ((lo, hi), (lo, lo))] {
        tris.extend(quad(Vec3::new(a.0, a.1, -depth), Vec3::new(b.0, b.1, -depth), Vec3::new(b.0, b.1, 0.0), Vec3::new(a.0, a.1, 0.0)));
    }
    let mut wet = Vec::new();
    for i in 0..5 {
        for j in 0..5 {
            let (x, y) = (lo + i as f32 * unit, lo + j as f32 * unit);
            let corners = |z: f32| [Vec3::new(x, y, z), Vec3::new(x + unit, y, z), Vec3::new(x + unit, y + unit, z), Vec3::new(x, y + unit, z)];
            let c = corners(-depth);
            tris.extend(quad(c[0], c[1], c[2], c[3]));
            let key = terrain.then(|| ((((x + unit / 2.0) as f64 + super::HALF_WORLD) / unit as f64).floor() as i32, (((y + unit / 2.0) as f64 + super::HALF_WORLD) / unit as f64).floor() as i32));
            let c = corners(0.0);
            wet.extend(quad(c[0], c[1], c[2], c[3]).map(|t| crate::movement::Wet { v: t.v, area: super::AREA_WATER, cell: key }));
        }
    }
    let soup = Soup(tris, wet);
    let mut mesh = NavMesh::default();
    for tx in 796..=804 {
        for ty in 796..=804 {
            if let Some(tile) = super::build(&soup, tx, ty) {
                mesh.add(&tile);
            }
        }
    }
    mesh
}

/// Whether the mesh has a polygon within a yard and a half of `p`.
fn has_polygon_near(q: &Query, p: Vec3) -> bool {
    q.find_nearest_poly(p, Vec3::splat(2.0)).is_some_and(|(_, c)| c.distance(p) < 1.5)
}

#[test]
fn a_pond_has_water_polygons_over_its_bed_and_a_deep_one_has_neither() {
    let count = |mesh: &NavMesh, flag: u16| mesh.tiles.iter().flat_map(|t| &t.polys).filter(|p| p.flags == flag).count();
    let middle = to_nav(Vec3::new(0.0, 0.0, 0.0));
    let at = |z: f32| Vec3::new(middle.x, z, middle.z);
    // 2 yards deep (more than a creature climbs, less than "deep"): the bed can be walked and the surface swum on.
    let shallow = pond(2.0, false);
    assert!(count(&shallow, NAV_WATER) > 0, "no water polygons");
    assert!(has_polygon_near(&Query { mesh: &shallow, filter: Filter { include: NAV_WATER, exclude: 0 } }, at(0.0)), "no water polygon over the middle");
    assert!(has_polygon_near(&Query { mesh: &shallow, filter: Filter { include: NAV_GROUND, exclude: 0 } }, at(-2.0)), "no ground under the water");
    // 6 yards deep, in the cells of the terrain: no bed, no surface.
    let deep = pond(6.0, true);
    assert_eq!(count(&deep, NAV_WATER), 0);
    let q = Query { mesh: &deep, filter: Filter { include: NAV_GROUND | NAV_WATER, exclude: 0 } };
    assert!(!has_polygon_near(&q, at(-6.0)) && !has_polygon_near(&q, at(0.0)));
    // The floor round it is still there.
    assert!(has_polygon_near(&q, to_nav(Vec3::new(30.0, 0.0, 0.0))));
}

#[test]
fn tiles_kept_in_the_cache_come_back_as_they_were_and_a_changed_stamp_or_a_cut_file_loses_them() {
    use super::cache::NavCache;
    let mesh = walled_floor();
    let dir = std::env::temp_dir().join(format!("rsworld-navcache-test-{}", std::process::id()));
    let path = dir.join("test.nav");
    let _ = std::fs::remove_file(&path);
    let tile = &mesh.tiles[mesh.tiles.len() / 2];
    {
        let mut cache = NavCache::open(&path, 7).unwrap();
        assert!(cache.load(1, 2).is_none());
        cache.save(1, 2, Some(tile)).unwrap();
        cache.save(3, 4, None).unwrap();
        // Read back at once, and the tile as it was.
        let back = cache.load(1, 2).unwrap().expect("a tile");
        assert_eq!((back.x, back.y, back.verts.len(), back.polys.len()), (tile.x, tile.y, tile.verts.len(), tile.polys.len()));
        assert!(back.verts == tile.verts && back.detail == tile.detail);
        assert!(back.polys.iter().zip(&tile.polys).all(|(a, b)| a.verts == b.verts && a.neis == b.neis && a.count == b.count && a.area == b.area && a.flags == b.flags));
        assert!(cache.load(3, 4).unwrap().is_none());
    }
    // A mesh made of the tiles that come back from the file is the mesh that went in: the same path over it.
    let mut kept = NavCache::open(&path, 7).unwrap();
    let mut again = NavMesh::default();
    {
        let mut cache = NavCache::open(&dir.join("all.nav"), 7).unwrap();
        for t in &mesh.tiles {
            cache.save(t.y, t.x, Some(t)).unwrap();
        }
        for t in &mesh.tiles {
            again.add_tile(cache.load(t.y, t.x).unwrap().unwrap());
        }
    }
    let mut finder = PathFinder::new(false);
    finder.calculate(&again, Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 0.0, 0.0), false, false);
    assert_eq!(finder.kind, PATHFIND_NORMAL);
    assert!(finder.path.len() > 3, "{:?}", finder.path);
    assert!(kept.load(1, 2).is_some());
    // Another stamp: nothing is kept. A file cut short loses the record that was cut, not the others.
    drop(kept);
    assert!(NavCache::open(&path, 8).unwrap().load(1, 2).is_none());
    let mut cache = NavCache::open(&path, 9).unwrap();
    cache.save(5, 6, None).unwrap();
    cache.save(1, 2, Some(tile)).unwrap();
    drop(cache);
    let length = std::fs::metadata(&path).unwrap().len();
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(length - 10).unwrap();
    let mut cache = NavCache::open(&path, 9).unwrap();
    assert!(cache.load(5, 6).is_some() && cache.load(1, 2).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
