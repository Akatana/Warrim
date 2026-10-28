//! The navigation mesh as Detour holds it (`dtNavMesh`): tiles of convex polygons that are linked to their neighbours, within a tile by shared edges and across
//! tiles by the portals on their borders. Positions are in Detour's axes, as CMaNGOS's PathFinder gives them: x west, y up, z north (`{y, z, x}` of WoW).

use super::build::{self, HALF_WORLD, TILE};
use glam::Vec3;
use std::collections::HashMap;

/// A polygon of the mesh: the tile it is in (counted from 1, so that 0 is none) and its place there.
pub type PolyRef = u32;
pub const NO_POLY: PolyRef = 0;
/// A polygon edge that leads to another tile, with the side it is on in the low bits (`DT_EXT_LINK`).
pub const EXT_LINK: u16 = 0x8000;
/// The link of an edge between two polygons of one tile has no side.
pub const NO_SIDE: u8 = 0xff;
pub const MAX_VERTS: usize = 6;

/// `NavTerrainFlag`: what a polygon is made of; a creature's filter lets some of them in.
pub const NAV_GROUND: u16 = 1;
pub const NAV_GROUND_STEEP: u16 = 2;
pub const NAV_WATER: u16 = 4;
pub const NAV_MAGMA_SLIME: u16 = 8;

pub struct Poly {
    pub verts: [u16; MAX_VERTS],
    /// Per edge: 0 for a wall, the polygon behind it + 1, or `EXT_LINK | side` (0 +x, 2 +z, 4 -x, 6 -z) for a portal to the next tile.
    pub neis: [u16; MAX_VERTS],
    pub count: usize,
    pub area: u8,
    pub flags: u16,
}

#[derive(Clone, Copy)]
pub struct Link {
    pub to: PolyRef,
    pub edge: u8,
    pub side: u8,
    /// The part of the portal edge that the link covers, in 255ths (for the links across tiles).
    pub min: u8,
    pub max: u8,
}

pub struct MeshTile {
    /// Where it is in the grid of tiles: x counts west, y counts north.
    pub x: i32,
    pub y: i32,
    pub verts: Vec<Vec3>,
    pub polys: Vec<Poly>,
    pub links: Vec<Vec<Link>>,
    /// The detail mesh per polygon: its vertices and its triangles (indices into the vertices).
    pub detail: Vec<(Vec<Vec3>, Vec<[u8; 3]>)>,
}

#[derive(Default)]
pub struct NavMesh {
    pub tiles: Vec<MeshTile>,
    index: HashMap<(i32, i32), usize>,
}

/// WoW's (x north, y west, z up) as Detour's axes and back.
pub fn to_nav(wow: Vec3) -> Vec3 {
    Vec3::new(wow.y, wow.z, wow.x)
}
pub fn from_nav(nav: Vec3) -> Vec3 {
    Vec3::new(nav.z, nav.x, nav.y)
}

/// The tile (x west, y north) that Detour's position `p` is in (`dtNavMesh::calcTileLoc`).
pub fn tile_of(p: Vec3) -> (i32, i32) {
    (((p.x as f64 + HALF_WORLD) / TILE).floor() as i32, ((p.z as f64 + HALF_WORLD) / TILE).floor() as i32)
}

pub fn make_ref(slot: usize, poly: usize) -> PolyRef {
    ((slot as u32 + 1) << 16) | poly as u32
}

impl NavMesh {
    pub fn has_tile(&self, x: i32, y: i32) -> bool {
        self.index.contains_key(&(x, y))
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    pub fn tile_slot(&self, x: i32, y: i32) -> Option<usize> {
        self.index.get(&(x, y)).copied()
    }

    pub fn tile(&self, r: PolyRef) -> &MeshTile {
        &self.tiles[(r >> 16) as usize - 1]
    }

    pub fn poly(&self, r: PolyRef) -> (&MeshTile, &Poly) {
        let tile = self.tile(r);
        (tile, &tile.polys[(r & 0xffff) as usize])
    }

    pub fn links(&self, r: PolyRef) -> &[Link] {
        &self.tile(r).links[(r & 0xffff) as usize]
    }

    /// Put a built tile in (`dtNavMesh::addTile`), linked with the tiles round it.
    pub fn add(&mut self, built: &build::Tile) {
        self.add_tile(MeshTile::from_built(built));
    }

    /// Put a tile in, linked with the tiles round it.
    pub fn add_tile(&mut self, tile: MeshTile) {
        let (key, slot) = ((tile.x, tile.y), self.tiles.len());
        if self.index.contains_key(&key) {
            return;
        }
        self.tiles.push(tile);
        self.index.insert(key, slot);
        self.connect_int_links(slot);
        // The eight tiles round it, in Detour's order of sides (0 is +x, going round through +z).
        for side in 0..8 {
            let (dx, dy) = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)][side];
            let Some(&other) = self.index.get(&(key.0 + dx, key.1 + dy)) else { continue };
            self.connect_ext_links(slot, other, side as u8);
            self.connect_ext_links(other, slot, ((side + 4) & 7) as u8);
        }
    }

    fn connect_int_links(&mut self, slot: usize) {
        let tile = &mut self.tiles[slot];
        for (i, poly) in tile.polys.iter().enumerate() {
            for j in 0..poly.count {
                let nei = poly.neis[j];
                if nei == 0 || nei & EXT_LINK != 0 {
                    continue;
                }
                tile.links[i].push(Link { to: make_ref(slot, nei as usize - 1), edge: j as u8, side: NO_SIDE, min: 0, max: 0 });
            }
        }
    }

    /// Link the portal edges of tile `slot` that face the side `side` to the polygons of `other` (`dtNavMesh::connectExtLinks`).
    fn connect_ext_links(&mut self, slot: usize, other: usize, side: u8) {
        let mut added: Vec<(usize, Link)> = Vec::new();
        {
            let (tile, target) = (&self.tiles[slot], &self.tiles[other]);
            for (i, poly) in tile.polys.iter().enumerate() {
                for j in 0..poly.count {
                    if poly.neis[j] & EXT_LINK == 0 || (poly.neis[j] & 0xff) as u8 != side {
                        continue;
                    }
                    let (va, vb) = (tile.verts[poly.verts[j] as usize], tile.verts[poly.verts[(j + 1) % poly.count] as usize]);
                    for (k, (lo, hi)) in find_connecting_polys(va, vb, target, (side + 4) & 7) {
                        let (min, max) = if side == 0 || side == 4 {
                            (((lo - va.z) / (vb.z - va.z)).clamp(0.0, 1.0), ((hi - va.z) / (vb.z - va.z)).clamp(0.0, 1.0))
                        } else {
                            (((lo - va.x) / (vb.x - va.x)).clamp(0.0, 1.0), ((hi - va.x) / (vb.x - va.x)).clamp(0.0, 1.0))
                        };
                        let (min, max) = (min.min(max), min.max(max));
                        added.push((i, Link { to: make_ref(other, k), edge: j as u8, side, min: (min * 255.0) as u8, max: (max * 255.0) as u8 }));
                    }
                }
            }
        }
        for (i, link) in added {
            self.tiles[slot].links[i].push(link);
        }
    }
}

/// The slab of an edge on a tile border seen along the border: (position along it, height) at both ends, the lower first.
fn slab_ends(va: Vec3, vb: Vec3, side: u8) -> ([f32; 2], [f32; 2]) {
    let (a, b) = if side == 0 || side == 4 { ([va.z, va.y], [vb.z, vb.y]) } else { ([va.x, va.y], [vb.x, vb.y]) };
    if a[0] < b[0] { (a, b) } else { (b, a) }
}

/// Whether two slabs overlap along the border and are at about the same height (`overlapSlabs`).
fn overlap_slabs(amin: [f32; 2], amax: [f32; 2], bmin: [f32; 2], bmax: [f32; 2], px: f32, py: f32) -> bool {
    let (minx, maxx) = ((amin[0] + px).max(bmin[0] + px), (amax[0] - px).min(bmax[0] - px));
    if minx > maxx {
        return false;
    }
    let ad = (amax[1] - amin[1]) / (amax[0] - amin[0]);
    let ak = amin[1] - ad * amin[0];
    let bd = (bmax[1] - bmin[1]) / (bmax[0] - bmin[0]);
    let bk = bmin[1] - bd * bmin[0];
    let (aminy, amaxy) = (ad * minx + ak, ad * maxx + ak);
    let (bminy, bmaxy) = (bd * minx + bk, bd * maxx + bk);
    let (dmin, dmax) = (bminy - aminy, bmaxy - amaxy);
    // Crossing segments always overlap.
    if dmin * dmax < 0.0 {
        return true;
    }
    let threshold = (py * 2.0) * (py * 2.0);
    dmin * dmin <= threshold || dmax * dmax <= threshold
}

/// The polygons of `target` with an edge on its border `side` that meets the edge `va`-`vb`, and where along the border they meet (`findConnectingPolys`).
fn find_connecting_polys(va: Vec3, vb: Vec3, target: &MeshTile, side: u8) -> Vec<(usize, (f32, f32))> {
    let (amin, amax) = slab_ends(va, vb, side);
    // The border, in the position along the other axis (x for +x/-x sides, where the edge runs along z).
    let apos = if side == 0 || side == 4 { va.x } else { va.z };
    let mut found = Vec::new();
    for (i, poly) in target.polys.iter().enumerate() {
        for j in 0..poly.count {
            if poly.neis[j] & EXT_LINK == 0 || (poly.neis[j] & 0xff) as u8 != side {
                continue;
            }
            let (vc, vd) = (target.verts[poly.verts[j] as usize], target.verts[poly.verts[(j + 1) % poly.count] as usize]);
            let bpos = if side == 0 || side == 4 { vc.x } else { vc.z };
            if (apos - bpos).abs() > 0.01 {
                continue;
            }
            let (bmin, bmax) = slab_ends(vc, vd, side);
            if !overlap_slabs(amin, amax, bmin, bmax, 0.01, build::WALKABLE_CLIMB as f32 * build::CELL) {
                continue;
            }
            found.push((i, (amin[0].max(bmin[0]), amax[0].min(bmax[0]))));
        }
    }
    found
}

impl MeshTile {
    /// The tile as Detour holds it, from what the builder made (`dtCreateNavMeshData`).
    pub fn from_built(built: &build::Tile) -> MeshTile {
        let polys = &built.polys;
        let (cs, ch) = (polys.cell_size, polys.cell_height);
        // Detour's axes: x is west and z north, from the corner of the tile (which is in WoW's north and west).
        let place = |x: f32, y: f32, z: f32| Vec3::new(built.corner.1 as f32 + x, y, built.corner.0 as f32 + z);
        let verts: Vec<Vec3> = polys.vertices.iter().map(|v| place(polys.aabb.min.x + v.x as f32 * cs, polys.aabb.min.y + v.y as f32 * ch, polys.aabb.min.z + v.z as f32 * cs)).collect();
        let nvp = polys.max_vertices_per_polygon as usize;
        let mut out = Vec::new();
        for p in 0..polys.polygon_count() {
            let (vs, ns) = (&polys.polygons[p * nvp..(p + 1) * nvp], &polys.polygon_neighbors[p * nvp..(p + 1) * nvp]);
            let count = vs.iter().position(|&v| v == rerecast::PolygonNavmesh::NO_INDEX).unwrap_or(nvp);
            let mut poly = Poly { verts: [0; MAX_VERTS], neis: [0; MAX_VERTS], count, area: polys.areas[p].0, flags: 0 };
            for j in 0..count {
                poly.verts[j] = vs[j];
                poly.neis[j] = match ns[j] {
                    rerecast::PolygonNavmesh::NO_CONNECTION => 0,
                    // A portal: Recast counts its sides 0 x-, 1 z+, 2 x+, 3 z-, Detour 4, 2, 0, 6.
                    n if n & EXT_LINK != 0 => match n & 0xf { 0 => EXT_LINK | 4, 1 => EXT_LINK | 2, 2 => EXT_LINK, 3 => EXT_LINK | 6, _ => 0 },
                    n => n + 1,
                };
            }
            poly.flags = match poly.area {
                build::AREA_GROUND => NAV_GROUND,
                build::AREA_STEEP => NAV_GROUND_STEEP,
                build::AREA_WATER => NAV_WATER,
                build::AREA_MAGMA_SLIME => NAV_MAGMA_SLIME,
                _ => 0,
            };
            out.push(poly);
        }
        let detail = built
            .detail
            .meshes
            .iter()
            .map(|m| {
                let vs = built.detail.vertices[m.base_vertex_index as usize..][..m.vertex_count as usize].iter().map(|v| place(v.x, v.y, v.z)).collect();
                let ts = built.detail.triangles[m.base_triangle_index as usize..][..m.triangle_count as usize].to_vec();
                (vs, ts)
            })
            .collect();
        let (x, y) = (((built.corner.1 + build::BORDER_YARDS + HALF_WORLD) / TILE).round() as i32, ((built.corner.0 + build::BORDER_YARDS + HALF_WORLD) / TILE).round() as i32);
        let links = vec![Vec::new(); out.len()];
        MeshTile { x, y, verts, polys: out, links, detail }
    }

    /// The corners of polygon `p`.
    pub fn corners(&self, p: &Poly) -> Vec<Vec3> {
        (0..p.count).map(|i| self.verts[p.verts[i] as usize]).collect()
    }
}
