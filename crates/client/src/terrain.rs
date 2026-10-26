//! CPU-side terrain: turns a parsed ADT into vertex/index data, texture mip
//! chains and a height field for collision queries. No GPU types in here.

use crate::collision::{CollisionMesh, Tri};
use crate::content::Prop;
use crate::coords::TileCoord;
use crate::models::{placement_matrix, wmo_doodad_matrix, DoodadInstance, ModelCache, WmoInstance};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use mpq::Mount;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wowfmt::adt::{Adt, ALPHA_DIM, CHUNK_SIZE, HEIGHTS_PER_CHUNK, TILE_SIZE, UNIT_SIZE};
use wowfmt::blp::Blp;
use wowfmt::wdt::Wdt;

pub const GROUND_TEX_SIZE: u32 = 256;
pub const GROUND_TEX_MIPS: u32 = 9;
pub const NO_LAYER: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct TerrainVertex {
    /// Relative to the tile's north-west corner (render space).
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// Detail texture coordinates: one repeat per map cell.
    pub uv: [f32; 2],
    /// 0..1 across the chunk, for the alpha blend maps.
    pub alpha_uv: [f32; 2],
    /// Indices into the tile's texture array, `NO_LAYER` if unused.
    pub layers: [u32; 4],
    /// Index of the chunk (0..256) = layer in the alpha-map array.
    pub chunk: u32,
}

/// One decoded ground texture: 256x256 RGBA8 with a full mip chain.
pub struct GroundTexture {
    pub mips: Vec<Vec<u8>>,
}

/// Footstep surfaces: the ids of `TerrainType.dbc`.
pub const SURFACE_DIRT: u8 = 0;
pub const SURFACE_STONE: u8 = 2;
pub const SURFACE_SNOW: u8 = 3;
pub const SURFACE_WOOD: u8 = 4;
pub const SURFACE_GRASS: u8 = 5;
pub const SURFACE_LEAVES: u8 = 6;
pub const SURFACE_SAND: u8 = 7;
pub const SURFACE_SOGGY: u8 = 8;

/// The footstep surface a ground texture stands for. The tiles carry no surface type, so it is read off
/// the texture's file name (`Tileset\Elwynn\ElwynnGrassBase.blp`); unknown names count as dirt.
pub fn surface_of_texture(name: &str) -> u8 {
    const WORDS: [(u8, &[&str]); 7] = [
        (SURFACE_SNOW, &["snow", "ice", "frost", "frozen", "glacier"]),
        (SURFACE_SAND, &["sand", "beach", "desert", "dune"]),
        (SURFACE_LEAVES, &["leaf", "leaves", "pine", "needle"]),
        (SURFACE_GRASS, &["grass", "moss", "fern", "flower", "clover", "forest", "jungle", "vine", "brush", "weed", "crop", "plant", "thorn", "straw", "root", "bush"]),
        (SURFACE_SOGGY, &["muck", "water", "shore", "swamp", "bog", "marsh"]),
        (SURFACE_WOOD, &["wood", "plank", "log", "bridge", "board"]),
        (SURFACE_STONE, &["rock", "stone", "cobble", "brick", "slate", "cliff", "marble", "tile", "pave", "granite", "cave", "lava", "ash", "obsidian", "magma", "rubble", "shale", "crack", "charcoal", "floor", "city"]),
    ];
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name).to_ascii_lowercase();
    WORDS.iter().find(|(_, words)| words.iter().any(|w| file.contains(w))).map_or(SURFACE_DIRT, |&(surface, _)| surface)
}

/// A placed model with particle emitters: its file and its tile-local transform.
pub struct Fire {
    pub model: String,
    pub matrix: glam::Mat4,
    /// Its mesh moves too, so it is shown by `fires.rs` and not with the tile.
    pub mesh: bool,
    /// How it is lit, if it stands in a room of a building.
    pub light: Option<crate::models::InteriorLight>,
}

pub struct TileCpu {
    pub coord: TileCoord,
    pub fires: Vec<Fire>,
    pub vertices: Vec<TerrainVertex>,
    pub indices: Vec<u32>,
    pub textures: Vec<Arc<GroundTexture>>,
    /// 256 chunks x 64x64 RGBA8 (R,G,B = blend weight of layers 1,2,3).
    pub alpha: Vec<u8>,
    pub heights: HeightField,
    pub areas: Vec<u32>,
    /// Footstep surface per 2-yard cell: 256 chunks x 16x16 (`surface_of_texture` ids).
    pub surfaces: Vec<u8>,
    pub water: Vec<WaterVertex>,
    /// The liquid of each chunk (256 slots), for what is heard of it and where it is waded.
    pub liquids: Vec<Option<wowfmt::adt::Liquid>>,
    /// The liquid cells of the buildings, each its four corners (tile space) and its kind (0 water, 1 ocean, 2 magma, 3 slime).
    pub wmo_liquids: Vec<([Vec3; 4], u32)>,
    /// What the placed models play all the time: where (tile-local) and the `SoundEntries` number.
    pub sounds: Vec<(Vec3, u32)>,
    /// Where the beds are (tile-local): the models of the map and of its buildings that are made to sleep in.
    pub beds: Vec<Vec3>,
    pub doodads: Vec<DoodadInstance>,
    /// Buildings: their groups are shown according to portal visibility each frame.
    pub wmos: Vec<WmoInstance>,
    /// The shadows baked into the ground, 256 chunks of 512 bytes (see `ground_shadowed`); empty if the tile has none.
    pub shadow: Vec<u8>,
    pub collision: CollisionMesh,
}

/// Whether the ground at (`x`, `z`), yards east and south of the tile's corner, has a shadow baked into it: the 64 x 64 bits of its chunk (WoW.exe FUN_0069b350).
pub fn ground_shadowed(shadow: &[u8], x: f32, z: f32) -> bool {
    let (cx, cz) = ((x / CHUNK_SIZE).floor() as i32, (z / CHUNK_SIZE).floor() as i32);
    if !(0..16).contains(&cx) || !(0..16).contains(&cz) {
        return false;
    }
    let cell = |v: f32, c: i32| (((v - c as f32 * CHUNK_SIZE) * 64.0 / CHUNK_SIZE) as usize).min(63);
    let (ix, iz) = (cell(x, cx), cell(z, cz));
    shadow.get((cz * 16 + cx) as usize * 512 + iz * 8 + ix / 8).is_some_and(|b| b >> (ix & 7) & 1 != 0)
}

/// Liquid sets, in the order of the water texture array: 30 animation frames each.
pub const WATER_KINDS: usize = 4;
pub const WATER_FRAMES: usize = 30;
const WATER_TEXTURES: [&str; WATER_KINDS] = [
    "XTextures\\river\\lake_a.{}.blp",
    "XTextures\\ocean\\ocean_h.{}.blp",
    "XTextures\\lava\\lava.{}.blp",
    "XTextures\\slime\\slime.{}.blp",
];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct WaterVertex {
    /// Relative to the tile's north-west corner (render space).
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    /// 0 water, 1 ocean, 2 magma, 3 slime.
    pub kind: u32,
}

/// All animation frames of every liquid type, `kind * WATER_FRAMES + frame`.
pub fn load_water_textures(mount: &Mount, cache: &TextureCache) -> Vec<Arc<GroundTexture>> {
    let mut out = Vec::with_capacity(WATER_KINDS * WATER_FRAMES);
    for pattern in WATER_TEXTURES {
        for frame in 1..=WATER_FRAMES {
            out.push(cache.get(mount, &pattern.replace("{}", &frame.to_string())));
        }
    }
    out
}

/// Heights of one tile, in render space relative to the tile corner.
pub struct HeightField {
    heights: Vec<[f32; HEIGHTS_PER_CHUNK]>,
    holes: Vec<u32>,
}

impl HeightField {
    /// Collision triangles (tile-local render space) of every non-hole cell overlapping the box
    /// `[min_x, max_x] x [min_z, max_z]`: four triangles around each cell's centre vertex, as in the
    /// client's terrain collision.
    pub fn cell_triangles(&self, min_x: f32, max_x: f32, min_z: f32, max_z: f32, out: &mut Vec<[Vec3; 3]>) {
        if max_x < 0.0 || max_z < 0.0 || min_x >= TILE_SIZE || min_z >= TILE_SIZE {
            return;
        }
        let chunk_range = |lo: f32, hi: f32| {
            let a = ((lo.max(0.0) / CHUNK_SIZE) as usize).min(15);
            let b = ((hi.min(TILE_SIZE - 0.001) / CHUNK_SIZE) as usize).min(15);
            a..=b
        };
        for cz in chunk_range(min_z, max_z) {
            for cx in chunk_range(min_x, max_x) {
                let chunk = cz * 16 + cx;
                let (ox, oz) = (cx as f32 * CHUNK_SIZE, cz as f32 * CHUNK_SIZE);
                let h = &self.heights[chunk];
                let cell_range = |lo: f32, hi: f32, o: f32| {
                    let a = (((lo - o) / UNIT_SIZE).floor().max(0.0) as usize).min(7);
                    let b = (((hi - o) / UNIT_SIZE).floor().max(0.0) as usize).min(7);
                    a..=b
                };
                for iz in cell_range(min_z, max_z, oz) {
                    for ix in cell_range(min_x, max_x, ox) {
                        if self.holes[chunk] & (1 << ((iz / 2) * 4 + ix / 2)) != 0 {
                            continue;
                        }
                        let tl = iz * 17 + ix;
                        let (tr, bl, br, c) = (tl + 1, tl + 17, tl + 18, tl + 9);
                        let p = |i: usize| {
                            let (lx, lz) = grid_xz(i);
                            Vec3::new(ox + lx, h[i], oz + lz)
                        };
                        let (ptl, ptr, pbl, pbr, pc) = (p(tl), p(tr), p(bl), p(br), p(c));
                        out.push([ptl, pc, ptr]);
                        out.push([ptr, pc, pbr]);
                        out.push([pbr, pc, pbl]);
                        out.push([pbl, pc, ptl]);
                    }
                }
            }
        }
    }

    /// Height at tile-local `(x, z)` (render-space offsets from the tile corner).
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        if !(0.0..TILE_SIZE).contains(&x) || !(0.0..TILE_SIZE).contains(&z) {
            return None;
        }
        let (cx, cz) = ((x / CHUNK_SIZE) as usize, (z / CHUNK_SIZE) as usize);
        let (cx, cz) = (cx.min(15), cz.min(15));
        let chunk = cz * 16 + cx;
        let (u, v) = ((x - cx as f32 * CHUNK_SIZE) / UNIT_SIZE, (z - cz as f32 * CHUNK_SIZE) / UNIT_SIZE);
        let (ix, iz) = ((u as usize).min(7), (v as usize).min(7));
        if self.holes[chunk] & (1 << ((iz / 2) * 4 + ix / 2)) != 0 {
            return None;
        }
        let (fu, fv) = (u - ix as f32, v - iz as f32);
        let h = &self.heights[chunk];
        let base = iz * 17 + ix;
        let (tl, tr, bl, br, c) = (h[base], h[base + 1], h[base + 17], h[base + 18], h[base + 9]);
        // The cell is four triangles meeting at the centre vertex.
        let (dx, dz) = (fu - 0.5, fv - 0.5);
        // Pick the triangle (outer edge a->b plus the centre c), the position `t`
        // along that edge and the distance `d` from it (0 at the edge, 0.5 at c).
        let (a, b, t, d) = if dz.abs() >= dx.abs() {
            if dz < 0.0 { (tl, tr, fu, fv) } else { (bl, br, fu, 1.0 - fv) }
        } else if dx < 0.0 {
            (tl, bl, fv, fu)
        } else {
            (tr, br, fv, 1.0 - fu)
        };
        // Barycentric weights: w for the centre, the remainder split along a->b.
        let w = (d * 2.0).clamp(0.0, 1.0);
        let e = t - 0.5 * w;
        Some(a * (1.0 - w - e) + b * e + c * w)
    }
}

/// Shared cache so neighbouring tiles do not decode the same BLP twice.
#[derive(Default)]
pub struct TextureCache {
    map: Mutex<HashMap<String, Arc<GroundTexture>>>,
}

impl TextureCache {
    pub fn get(&self, mount: &Mount, path: &str) -> Arc<GroundTexture> {
        let key = path.to_ascii_lowercase();
        if let Some(t) = self.map.lock().unwrap().get(&key) {
            return t.clone();
        }
        let tex = match mount.read(path).map_err(anyhow::Error::from).and_then(|d| Ok(Blp::parse(&d)?)) {
            Ok(blp) => Arc::new(prepare_ground_texture(&blp)),
            Err(e) => {
                log::warn!("texture {path}: {e}");
                Arc::new(checker_texture())
            }
        };
        self.map.lock().unwrap().insert(key, tex.clone());
        tex
    }
}

fn checker_texture() -> GroundTexture {
    let mut base = Vec::with_capacity((GROUND_TEX_SIZE * GROUND_TEX_SIZE * 4) as usize);
    for y in 0..GROUND_TEX_SIZE {
        for x in 0..GROUND_TEX_SIZE {
            let on = ((x / 32) + (y / 32)) % 2 == 0;
            base.extend_from_slice(if on { &[200, 0, 200, 255] } else { &[40, 40, 40, 255] });
        }
    }
    GroundTexture { mips: build_mips(base, GROUND_TEX_SIZE as usize) }
}

fn prepare_ground_texture(blp: &Blp) -> GroundTexture {
    let size = GROUND_TEX_SIZE as usize;
    let src = &blp.mips[0];
    let base = if blp.width as usize == size && blp.height as usize == size {
        src.clone()
    } else {
        resize_bilinear(src, blp.width as usize, blp.height as usize, size, size)
    };
    GroundTexture { mips: build_mips(base, size) }
}

fn build_mips(base: Vec<u8>, size: usize) -> Vec<Vec<u8>> {
    let mut mips = vec![base];
    let mut s = size;
    while s > 1 {
        let n = s / 2;
        let prev = mips.last().unwrap();
        let mut next = vec![0u8; n * n * 4];
        for y in 0..n {
            for x in 0..n {
                for c in 0..4 {
                    let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                        .iter()
                        .map(|&(dx, dy)| prev[((y * 2 + dy) * s + x * 2 + dx) * 4 + c] as u32)
                        .sum();
                    next[(y * n + x) * 4 + c] = ((sum + 2) / 4) as u8;
                }
            }
        }
        mips.push(next);
        s = n;
    }
    mips
}

fn resize_bilinear(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        let fy = ((y as f32 + 0.5) * sh as f32 / dh as f32 - 0.5).max(0.0);
        let (y0, ty) = (fy as usize, fy.fract());
        let y1 = (y0 + 1).min(sh - 1);
        for x in 0..dw {
            let fx = ((x as f32 + 0.5) * sw as f32 / dw as f32 - 0.5).max(0.0);
            let (x0, tx) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(sw - 1);
            for c in 0..4 {
                let p = |xx: usize, yy: usize| src[(yy * sw + xx) * 4 + c] as f32;
                let top = p(x0, y0) * (1.0 - tx) + p(x1, y0) * tx;
                let bot = p(x0, y1) * (1.0 - tx) + p(x1, y1) * tx;
                out[(y * dw + x) * 4 + c] = (top * (1.0 - ty) + bot * ty + 0.5) as u8;
            }
        }
    }
    out
}

/// Vertex position inside a chunk for slot `i` of the interleaved 145-grid, in
/// chunk-local render space (x east, z south).
fn grid_xz(i: usize) -> (f32, f32) {
    let row = i / 17;
    let col = i % 17;
    if col < 9 {
        (col as f32 * UNIT_SIZE, row as f32 * UNIT_SIZE)
    } else {
        ((col - 9) as f32 * UNIT_SIZE + UNIT_SIZE * 0.5, row as f32 * UNIT_SIZE + UNIT_SIZE * 0.5)
    }
}

pub fn load_tile(mount: &Mount, cache: &TextureCache, models: &ModelCache, map: &str, coord: TileCoord, props: &[Prop]) -> Result<TileCpu> {
    let path = format!("World\\Maps\\{map}\\{map}_{}_{}.adt", coord.0, coord.1);
    let mut adt = match mount.read(&path) {
        Ok(bytes) => Adt::parse(&bytes).with_context(|| format!("parsing {path}"))?,
        // A dungeon that is one building has no tiles: its one tile holds the building of the WDT.
        Err(e) => global_adt(mount, map).ok_or(e).with_context(|| format!("reading {path}"))?,
    };
    if adt.foreign.is_empty() {
        adt.foreign = unlisted_elsewhere(mount, map, coord, &adt);
    }
    add_props(&mut adt, props, coord, map);
    Ok(build_tile(mount, cache, models, &adt, coord))
}

/// The tile (as the map files number them) that holds a placement.
fn owner_of(position: [f32; 3]) -> TileCoord {
    ((position[0] / TILE_SIZE).floor() as i32, (position[2] / TILE_SIZE).floor() as i32)
}

/// The unique ids of the placements of this tile's file that stand in another tile, whose own file does not list them. A model on the border of
/// two tiles is listed by both and spawned by the one it stands in; the dungeons list some in a single file, of a tile next to the one they stand in
/// (the graveyard of the Scarlet Monastery), and nobody would spawn those.
fn unlisted_elsewhere(mount: &Mount, map: &str, coord: TileCoord, adt: &Adt) -> Vec<u32> {
    let mut others: HashMap<TileCoord, Option<Adt>> = HashMap::new();
    let mut keep = Vec::new();
    let placements = adt.doodads.iter().map(|d| (d.position, d.unique_id, false)).chain(adt.wmos.iter().map(|w| (w.position, w.unique_id, true)));
    for (position, id, wmo) in placements {
        let owner = owner_of(position);
        if owner == coord {
            continue;
        }
        let other = others.entry(owner).or_insert_with(|| {
            let path = format!(r"World\Maps\{map}\{map}_{}_{}.adt", owner.0, owner.1);
            mount.read(&path).ok().and_then(|bytes| Adt::parse(&bytes).ok())
        });
        let listed = other.as_ref().is_some_and(|o| if wmo { o.wmos.iter().any(|w| w.unique_id == id) } else { o.doodads.iter().any(|d| d.unique_id == id) });
        if !listed {
            keep.push(id);
        }
    }
    keep
}

/// The tile of a map with no terrain: no ground, the one building of its WDT.
fn global_adt(mount: &Mount, map: &str) -> Option<Adt> {
    let global = Wdt::parse(&mount.read(&format!("World\\Maps\\{map}\\{map}.wdt")).ok()?).ok()?.global?;
    // The map files measure from the north-west corner of the map; the WDT, from its origin.
    let [x, y, z] = global.position;
    let corner = 32.0 * TILE_SIZE;
    let wmo = wowfmt::adt::WmoPlacement { name_index: 0, unique_id: u32::MAX, position: [x + corner, y, z + corner], rotation: global.rotation, flags: 0, doodad_set: global.doodad_set };
    Some(Adt { textures: Vec::new(), m2_names: Vec::new(), wmo_names: vec![global.name], doodads: Vec::new(), wmos: vec![wmo], chunks: Vec::new(), foreign: vec![u32::MAX] })
}

/// Whether a model (by its file name) is something to sleep in: a bed or a hammock, not a wardrobe.
pub fn is_bed(model: &str) -> bool {
    let file = model.rsplit(['\\', '/']).next().unwrap_or(model).to_ascii_lowercase();
    (file.contains("bed") && !file.contains("wardrobe")) || file.contains("hammock")
}

/// The tile (as the map files number them) that holds a prop.
pub fn tile_of_prop(p: &Prop) -> TileCoord {
    let at = crate::coords::wow_to_render([p.at.0, p.at.1, p.at.2]);
    crate::coords::tile_of(at.x, at.z)
}

/// Put the props of the tile among the placements of its map file, so that they are drawn, solid, lit and heard like the rest.
fn add_props(adt: &mut Adt, props: &[Prop], coord: TileCoord, map: &str) {
    // The pickups and the bodies of the physics are drawn apart from the map (`pickups.rs`, `physics.rs`).
    for p in props.iter().filter(|p| p.pickup.is_none() && !p.physics &&p.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(map)) && tile_of_prop(p) == coord) {
        // The map files measure from the north-west corner of the map: render space shifted by half the map's width.
        let at = crate::coords::wow_to_render([p.at.0, p.at.1, p.at.2]);
        let position = [at.x + 32.0 * TILE_SIZE, at.y, at.z + 32.0 * TILE_SIZE];
        let model = p.model.to_ascii_lowercase();
        if model.ends_with(".wmo") {
            adt.wmo_names.push(p.model.clone());
            adt.wmos.push(wowfmt::adt::WmoPlacement { name_index: adt.wmo_names.len() - 1, unique_id: 0, position, rotation: p.rotation, flags: 0, doodad_set: 0 });
        } else {
            adt.m2_names.push(p.model.clone());
            adt.doodads.push(wowfmt::adt::DoodadPlacement { name_index: adt.m2_names.len() - 1, unique_id: 0, position, rotation: p.rotation, scale: p.scale });
        }
    }
}

pub fn build_tile(mount: &Mount, cache: &TextureCache, models: &ModelCache, adt: &Adt, coord: TileCoord) -> TileCpu {
    let textures: Vec<Arc<GroundTexture>> = adt.textures.iter().map(|t| cache.get(mount, t)).collect();
    let n_tex = textures.len() as u32;

    let mut vertices = Vec::with_capacity(256 * HEIGHTS_PER_CHUNK);
    let mut indices = Vec::with_capacity(256 * 64 * 12);
    let mut alpha = vec![0u8; 256 * ALPHA_DIM * ALPHA_DIM * 4];
    let mut heights = Vec::with_capacity(256);
    let mut holes = Vec::with_capacity(256);
    let mut areas = Vec::with_capacity(256);
    let mut surfaces = vec![SURFACE_DIRT; 256 * 256];
    let mut water = Vec::new();
    let mut liquids = vec![None; 256];
    let mut wmo_liquids = Vec::new();
    let mut sounds = Vec::new();

    for chunk in &adt.chunks {
        // The chunk's slot in the tile comes from its header, not from file order.
        let (cx, cz) = (chunk.index_x as usize % 16, chunk.index_y as usize % 16);
        let slot = cz * 16 + cx;
        let (ox, oz) = (cx as f32 * CHUNK_SIZE, cz as f32 * CHUNK_SIZE);

        let mut layers = [NO_LAYER; 4];
        for (i, l) in chunk.layers.iter().enumerate().take(4) {
            layers[i] = if l.texture < n_tex { l.texture } else { NO_LAYER };
        }
        if layers[0] == NO_LAYER && n_tex > 0 {
            layers[0] = 0;
        }

        // Alpha maps (layers 1..3) packed into the RGB channels.
        for (li, l) in chunk.layers.iter().enumerate().take(4).skip(1) {
            let Some(a) = &l.alpha else { continue };
            for (p, &v) in a.iter().enumerate() {
                alpha[(slot * ALPHA_DIM * ALPHA_DIM + p) * 4 + (li - 1)] = v;
            }
        }

        // Footstep surface per cell: the layer that dominates the blend there (later layers cover earlier ones).
        let kinds: Vec<u8> =
            chunk.layers.iter().take(4).map(|l| adt.textures.get(l.texture as usize).map_or(SURFACE_DIRT, |t| surface_of_texture(t))).collect();
        for cell in 0..256usize {
            let (cell_x, cell_y) = (cell % 16, cell / 16);
            let mut weights = [0.0f32; 4];
            weights[0] = 1.0;
            for (li, l) in chunk.layers.iter().enumerate().take(4).skip(1) {
                let a = l.alpha.as_ref().map_or(0.0, |al| {
                    let sum: u32 = (0..16).map(|i| al[(cell_y * 4 + i / 4) * ALPHA_DIM + cell_x * 4 + i % 4] as u32).sum();
                    sum as f32 / (16.0 * 255.0)
                });
                weights[..li].iter_mut().for_each(|w| *w *= 1.0 - a);
                weights[li] = a;
            }
            let best = (0..kinds.len()).max_by(|&a, &b| weights[a].total_cmp(&weights[b]));
            surfaces[slot * 256 + cell] = best.map_or(SURFACE_DIRT, |i| kinds[i]);
        }

        let base = vertices.len() as u32;
        let mut pos = [Vec3::ZERO; HEIGHTS_PER_CHUNK];
        for (i, p) in pos.iter_mut().enumerate() {
            let (lx, lz) = grid_xz(i);
            *p = Vec3::new(ox + lx, chunk.heights[i], oz + lz);
        }

        // Cell triangles (tl, centre, tr), ... wound counter-clockwise seen from above.
        let mut normals = [Vec3::ZERO; HEIGHTS_PER_CHUNK];
        let mut tris: Vec<[u32; 3]> = Vec::with_capacity(64 * 4);
        for cell_z in 0..8usize {
            for cell_x in 0..8usize {
                let tl = cell_z * 17 + cell_x;
                let (tr, bl, br, c) = (tl + 1, tl + 17, tl + 18, tl + 9);
                let quad = [[tl, c, tr], [tr, c, br], [br, c, bl], [bl, c, tl]];
                for t in quad {
                    let n = (pos[t[1]] - pos[t[0]]).cross(pos[t[2]] - pos[t[0]]);
                    for &v in &t {
                        normals[v] += n;
                    }
                    if !chunk.is_hole(cell_x, cell_z) {
                        tris.push([t[0] as u32, t[1] as u32, t[2] as u32]);
                    }
                }
            }
        }
        for t in &tris {
            indices.extend(t.iter().map(|&i| base + i));
        }

        for i in 0..HEIGHTS_PER_CHUNK {
            let (lx, lz) = grid_xz(i);
            let n = normals[i].normalize_or_zero();
            let n = if n == Vec3::ZERO { Vec3::Y } else { n };
            vertices.push(TerrainVertex {
                pos: pos[i].to_array(),
                normal: n.to_array(),
                uv: [lx / UNIT_SIZE, lz / UNIT_SIZE],
                alpha_uv: [lx / CHUNK_SIZE, lz / CHUNK_SIZE],
                layers,
                chunk: slot as u32,
            });
        }

        liquids[slot] = chunk.liquid.clone();
        if let Some(liq) = &chunk.liquid {
            // MCNK flag bits (river, ocean, magma, slime) -> texture set.
            let kind = if liq.kind == 0 { 0 } else { liq.kind.trailing_zeros().min(3) };
            for cell_z in 0..8usize {
                for cell_x in 0..8usize {
                    if liq.cell_flags[cell_z * 8 + cell_x] & 0x0F == 0x0F {
                        continue;
                    }
                    let corner = |x: usize, z: usize| {
                        let (px, pz) = (ox + x as f32 * UNIT_SIZE, oz + z as f32 * UNIT_SIZE);
                        // One texture repeat per two cells; 64 repeats per tile keeps tiles seamless.
                        WaterVertex {
                            pos: [px, liq.heights[z * 9 + x], pz],
                            uv: [px / (UNIT_SIZE * 2.0), pz / (UNIT_SIZE * 2.0)],
                            kind,
                        }
                    };
                    let (a, b, c, d) = (
                        corner(cell_x, cell_z),
                        corner(cell_x + 1, cell_z),
                        corner(cell_x + 1, cell_z + 1),
                        corner(cell_x, cell_z + 1),
                    );
                    water.extend_from_slice(&[a, d, b, b, d, c]);
                }
            }
        }

        // Height field keyed by slot (chunks normally arrive in order, but be safe).
        if heights.len() <= slot {
            heights.resize(256, [0.0; HEIGHTS_PER_CHUNK]);
            holes.resize(256, 0);
            areas.resize(256, 0);
        }
        heights[slot] = chunk.heights;
        holes[slot] = chunk.holes;
        areas[slot] = chunk.area_id;
    }
    heights.resize(256, [0.0; HEIGHTS_PER_CHUNK]);
    // No ground at all where the map has none (a dungeon): every cell is a hole.
    holes.resize(256, if adt.chunks.is_empty() { 0xFFFF } else { 0 });
    areas.resize(256, 0);

    // Placements: file coordinates are (east, height, south) measured from the map's
    // north-west corner, so tile-local = position - tile index * tile size. Objects that
    // straddle tiles are listed in several ADTs; only the tile containing the position spawns it.
    let mut doodads = Vec::new();
    let mut fires = Vec::new();
    let mut beds = Vec::new();
    for d in &adt.doodads {
        let owner = ((d.position[0] / TILE_SIZE).floor() as i32, (d.position[2] / TILE_SIZE).floor() as i32);
        if owner != coord && !adt.foreign.contains(&d.unique_id) {
            continue;
        }
        let Some(name) = adt.m2_names.get(d.name_index) else { continue };
        let local = Vec3::new(
            d.position[0] - coord.0 as f32 * TILE_SIZE,
            d.position[1],
            d.position[2] - coord.1 as f32 * TILE_SIZE,
        );
        let matrix = placement_matrix(local, d.rotation, d.scale);
        if is_bed(name) {
            beds.push(matrix.transform_point3(Vec3::ZERO));
        }
        sounds.extend(models.sounds(mount, name).into_iter().map(|(p, id)| (matrix.transform_point3(p), id)));
        let living = models.living(mount, name);
        let moves = living.as_ref().is_some_and(|l| l.mesh);
        fires.extend(living.map(|l| Fire { model: l.file, matrix, mesh: l.mesh, light: None }));
        let Some(model) = models.get(mount, name) else { continue };
        doodads.push(DoodadInstance { model, matrix, drawn: !moves, light: None });
    }

    let on_map = doodads.len();
    let mut shadow = vec![0u8; 256 * 512];
    for c in &adt.chunks {
        if let Some(bits) = &c.shadow {
            let at = (c.index_y as usize % 16 * 16 + c.index_x as usize % 16) * 512;
            shadow[at..at + 512].copy_from_slice(&bits[..]);
        }
    }
    if adt.chunks.iter().all(|c| c.shadow.is_none()) {
        shadow.clear();
    }

    // WMOs: same coordinate convention.
    let mut wmos = Vec::new();
    for w in &adt.wmos {
        let owner = ((w.position[0] / TILE_SIZE).floor() as i32, (w.position[2] / TILE_SIZE).floor() as i32);
        if owner != coord && !adt.foreign.contains(&w.unique_id) {
            continue;
        }
        let Some(name) = adt.wmo_names.get(w.name_index) else { continue };
        let local = Vec3::new(
            w.position[0] - coord.0 as f32 * TILE_SIZE,
            w.position[1],
            w.position[2] - coord.1 as f32 * TILE_SIZE,
        );
        let matrix = placement_matrix(local, w.rotation, 1.0);
        let wmo = models.get_wmo(mount, name);
        wmos.push(WmoInstance { wmo: wmo.clone(), matrix });
        // Each cell is six vertices: a, d, b, b, d, c.
        for cell in wmo.liquid_tris.chunks_exact(6) {
            let at = |i: usize| matrix.transform_point3(Vec3::from(cell[i].0));
            wmo_liquids.push(([at(0), at(2), at(5), at(1)], cell[0].1));
        }
        for &(p, kind) in &wmo.liquid_tris {
            let q = matrix.transform_point3(Vec3::from(p));
            water.push(WaterVertex { pos: q.to_array(), uv: [q.x / (UNIT_SIZE * 2.0), q.z / (UNIT_SIZE * 2.0)], kind });
        }
        // The lamps of the building, where they are in the tile.
        let lamps = crate::models::lamps_of(&wmo, &matrix);
        for (i, d) in wmo.active_doodads(w.doodad_set) {
            let living = models.living(mount, &d.name);
            let moves = living.as_ref().is_some_and(|l| l.mesh);
            let mut light = wmo.indoor_doodads.contains(&(i as u16)).then(|| crate::models::interior_light(d.color));
            let place = matrix * wmo_doodad_matrix(d);
            if is_bed(&d.name) {
                beds.push(place.transform_point3(Vec3::ZERO));
            }
            // The lamps near it light it too.
            if let Some(light) = light.as_mut() {
                light.lamps = crate::models::lamp_lights(&lamps, place.w_axis.truncate());
            }
            fires.extend(living.map(|l| Fire { model: l.file, matrix: place, mesh: l.mesh, light }));
            if let Some(model) = models.get(mount, &d.name) {
                doodads.push(DoodadInstance { model, matrix: place, drawn: !moves, light });
            }
        }
    }

    // The models of the map that stand in a room of a building are lit by the room, as the original does.
    let lamps: Vec<Vec<crate::models::Lamp>> = wmos.iter().map(|w| crate::models::lamps_of(&w.wmo, &w.matrix)).collect();
    for d in &mut doodads[..on_map] {
        let at = d.matrix.w_axis.truncate();
        let intensity = if ground_shadowed(&shadow, at.x, at.z) { 0.5 } else { 2.5 };
        let Some((mut light, w)) = crate::models::room_light_at(wmos.iter().map(|w| (&*w.wmo, &w.matrix)), at, intensity) else { continue };
        light.lamps = crate::models::lamp_lights(&lamps[w], at);
        d.light = Some(light);
    }

    // Collision: every placed model's solid triangles, moved into tile space.
    let mut solid = Vec::new();
    let wmo_groups = wmos.iter().flat_map(|w| w.wmo.groups.iter().filter_map(|g| g.model.as_deref()).map(|m| (m, w.matrix)));
    for (m, matrix) in doodads.iter().map(|d| (&*d.model, d.matrix)).chain(wmo_groups) {
        if m.col_indices.is_empty() {
            continue;
        }
        let world: Vec<Vec3> = m.col_vertices.iter().map(|v| matrix.transform_point3(Vec3::from(*v))).collect();
        for t in m.col_indices.chunks_exact(3) {
            if let Some(tri) = Tri::new(world[t[0] as usize], world[t[1] as usize], world[t[2] as usize]) {
                solid.push(tri);
            }
        }
    }
    let collision = CollisionMesh::build(solid);

    TileCpu {
        coord,
        fires,
        vertices,
        indices,
        textures,
        alpha,
        heights: HeightField { heights, holes },
        areas,
        surfaces,
        water,
        liquids,
        wmo_liquids,
        sounds,
        beds,
        doodads,
        wmos,
        shadow,
        collision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shadow_bits_of_the_ground_are_found_by_chunk_and_cell() {
        let mut shadow = vec![0u8; 256 * 512];
        // Chunk (2, 3), cell (5, 7): byte 7 * 8 + 0 of its 512, bit 5.
        shadow[(3 * 16 + 2) * 512 + 7 * 8] |= 1 << 5;
        let cell = CHUNK_SIZE / 64.0;
        assert!(ground_shadowed(&shadow, 2.0 * CHUNK_SIZE + 5.5 * cell, 3.0 * CHUNK_SIZE + 7.5 * cell));
        assert!(!ground_shadowed(&shadow, 2.0 * CHUNK_SIZE + 6.5 * cell, 3.0 * CHUNK_SIZE + 7.5 * cell));
        assert!(!ground_shadowed(&shadow, -1.0, 3.0) && !ground_shadowed(&[], 3.0, 3.0));
    }

    #[test]
    fn beds_and_hammocks_are_slept_in_and_wardrobes_are_not() {
        for yes in [r"World\Dungeon\GoldshireInn\Innbed\InnBed.m2", r"World\Generic\Human\Passive Doodads\Beds\Bunkbed01.m2", r"World\Generic\Orc\Passive Doodads\Hammocks\Hammock02.m2"] {
            assert!(is_bed(yes), "{yes}");
        }
        for no in [r"World\Generic\Dwarf\Passive Doodads\Wardrobe\WardrobeDwarvenAverage01.m2", r"World\Kalimdor\StoneTalon\PassiveDoodads\WebbedTrees\StoneTree01_web.m2"] {
            assert!(!is_bed(no), "{no}");
        }
    }

    #[test]
    fn real_northshire_tile_has_grass_dirt_and_a_stone_road() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let tile = load_tile(&mount, &TextureCache::default(), &ModelCache::default(), "Azeroth", (32, 48), &[]).unwrap();
        let count = |surface| tile.surfaces.iter().filter(|&&s| s == surface).count();
        let total = tile.surfaces.len();
        assert!(count(SURFACE_GRASS) > total / 10, "grass {}", count(SURFACE_GRASS));
        assert!(count(SURFACE_DIRT) > total / 20, "dirt {}", count(SURFACE_DIRT));
        assert!(count(SURFACE_STONE) > 0, "no stone at all");
    }

    /// Every portal of the original leads to ground: the tile of its destination loads (a dungeon that is one building too) and has a floor within a few yards of the height given.
    #[test]
    fn every_portal_arrives_on_a_floor() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let (cache, models) = (TextureCache::default(), ModelCache::default());
        let mut places: Vec<(&str, (f32, f32, f32))> = crate::content::get().portals.iter().map(|p| (p.to_map.as_str(), p.to)).collect();
        // The Shadowfang "falling out" door puts the character in the air above the ground on purpose.
        places.retain(|&(map, to)| !(map == "Azeroth" && to.2 == 77.5589));
        places.sort_by(|a, b| a.partial_cmp(b).unwrap());
        places.dedup();
        let mut lost = Vec::new();
        for (map, (north, west, height)) in places {
            let at = crate::coords::wow_to_render([north, west, height]);
            let global = wowfmt::wdt::Wdt::parse(&mount.read(&format!(r"World\Maps\{map}\{map}.wdt")).unwrap()).unwrap().global.is_some();
            // A dungeon that is one building keeps all of it in one tile; a building of the open map may belong to a neighbour of the tile it stands in.
            let centre = if global { (32, 32) } else { crate::coords::tile_of(at.x, at.z) };
            let rings: &[i32] = if global { &[0] } else { &[0, 1] };
            let mut seen = Vec::new();
            for &ring in rings {
                for coord in (-ring..=ring).flat_map(|dz| (-ring..=ring).map(move |dx| (centre.0 + dx, centre.1 + dz))).filter(|&c| ring == 0 || c != centre) {
                    match load_tile(&mount, &cache, &models, map, coord, &[]) {
                        Ok(tile) => {
                            let (ox, oz) = crate::coords::tile_origin(coord);
                            let local = (at.x - ox as f32, at.z - oz as f32);
                            seen.extend([tile.collision.floor(local.0, local.1, height + 3.0), tile.heights.height_at(local.0, local.1)].into_iter().flatten());
                        }
                        Err(e) if coord == centre => lost.push(format!("{map} ({north}, {west}): {e:#}")),
                        Err(_) => {}
                    }
                }
                if seen.iter().any(|f| (f - height).abs() < 4.0) {
                    break;
                }
            }
            if !seen.iter().any(|f| (f - height).abs() < 4.0) {
                lost.push(format!("{map} ({north}, {west}, {height}): floors {seen:?}"));
            }
        }
        assert!(lost.is_empty(), "{} destinations without a floor:\n{}", lost.len(), lost.join("\n"));
    }

    #[test]
    fn ground_textures_map_to_footstep_surfaces_by_name() {
        for (name, want) in [
            (r"Tileset\Elwynn\ElwynnGrassBase.blp", SURFACE_GRASS),
            (r"Tileset\Elwynn\ElwynnDirtBase.blp", SURFACE_DIRT),
            (r"Tileset\Elwynn\ElwynnCobblestone.blp", SURFACE_STONE),
            (r"Tileset\Dunmorogh\DunMoroghSnow.blp", SURFACE_SNOW),
            (r"Tileset\Tanaris\TanarisSand.blp", SURFACE_SAND),
            (r"Tileset\Duskwood\DuskwoodMuck.blp", SURFACE_SOGGY),
            (r"Tileset\Generic\Black.blp", SURFACE_DIRT),
        ] {
            assert_eq!(surface_of_texture(name), want, "{name}");
        }
    }

    /// A plane is reproduced exactly by any triangulation, so every sample must match it.
    #[test]
    fn height_field_reproduces_a_plane() {
        let plane = |x: f32, z: f32| 10.0 + 0.3 * x - 0.2 * z;
        let mut heights = vec![[0f32; HEIGHTS_PER_CHUNK]; 256];
        for (slot, h) in heights.iter_mut().enumerate() {
            let (ox, oz) = ((slot % 16) as f32 * CHUNK_SIZE, (slot / 16) as f32 * CHUNK_SIZE);
            for (i, v) in h.iter_mut().enumerate() {
                let (lx, lz) = grid_xz(i);
                *v = plane(ox + lx, oz + lz);
            }
        }
        let field = HeightField { heights, holes: vec![0; 256] };
        let mut n = 0;
        for xi in 0..97 {
            for zi in 0..91 {
                let (x, z) = (xi as f32 * 5.49 + 0.13, zi as f32 * 5.83 + 0.07);
                let got = field.height_at(x, z).expect("inside the tile");
                assert!((got - plane(x, z)).abs() < 1e-2, "at ({x},{z}): {got} vs {}", plane(x, z));
                n += 1;
            }
        }
        assert!(n > 8000);
    }

    #[test]
    fn holes_and_outside_return_none() {
        let mut holes = vec![0u32; 256];
        holes[0] = 1; // cells (0..2, 0..2) of chunk 0
        let field = HeightField { heights: vec![[0.0; HEIGHTS_PER_CHUNK]; 256], holes };
        assert!(field.height_at(1.0, 1.0).is_none());
        assert!(field.height_at(UNIT_SIZE * 2.5, 1.0).is_some());
        assert!(field.height_at(-1.0, 5.0).is_none());
        assert!(field.height_at(TILE_SIZE + 1.0, 5.0).is_none());
    }

    #[test]
    fn the_abbey_tile_has_fires_among_its_models_even_without_a_mesh_to_draw() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let models = ModelCache::default();
        // Emitters are found by the model's file, whether or not the model has a mesh (the placement of one without is no longer skipped).
        assert!(models.living(&mount, r"World\Generic\PassiveDoodads\Lights\CandelabraTallWall01.mdx").is_some_and(|l| l.mesh));
        assert!(models.living(&mount, r"World\Generic\PassiveDoodads\ParticleEmitters\FountainParticles.mdx").is_some());
        // The tile of the abbey places 34 candelabras (in the building), and pyres and torches outside.
        let tile = load_tile(&mount, &TextureCache::default(), &models, "Azeroth", (32, 48), &[]).unwrap();
        let count = |name: &str| tile.fires.iter().filter(|f| f.model.ends_with(name)).count();
        assert_eq!((count("candelabratallwall01.m2"), count("stonepyre01.m2"), count("torch.m2")), (34, 2, 2));
    }

    #[test]
    fn every_model_of_the_originals_objects_loads_and_the_signs_of_stormwind_stand() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let models = ModelCache::default();
        let props = &crate::content::get().props;
        let files: std::collections::BTreeSet<&str> = props.iter().map(|p| p.model.as_str()).collect();
        let broken: Vec<_> = files.iter().filter(|f| f.to_ascii_lowercase().ends_with(".m2") && models.get(&mount, f).is_none() && models.living(&mount, f).is_none()).collect();
        assert!(broken.is_empty(), "models that do not load: {broken:?}");
        // The pointers of Trade District and the rest hang on the posts of the city, in the tiles around it.
        let signs = props.iter().filter(|p| p.map.as_deref() == Some("Azeroth") && p.zone.as_deref() == Some("Stormwind City") && p.model.to_ascii_lowercase().ends_with("woodsignpointernice01.m2")).count();
        assert!(signs > 40, "{signs} pointers in Stormwind");
    }

    #[test]
    fn the_campfires_of_the_originals_objects_burn_in_their_tile() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let models = ModelCache::default();
        let props = &crate::content::get().props;
        let fires: Vec<&Prop> = props.iter().filter(|p| p.map.as_deref() == Some("Azeroth") && p.model.to_ascii_lowercase().ends_with("elwynncampfire.m2")).collect();
        let coord = tile_of_prop(fires[0]);
        let placed = fires.iter().filter(|p| tile_of_prop(p) == coord).count();
        let tile = load_tile(&mount, &TextureCache::default(), &models, "Azeroth", coord, props).unwrap();
        let burning = tile.fires.iter().filter(|f| f.model.ends_with("elwynncampfire.m2")).count();
        assert!(placed > 0 && burning >= placed, "{burning} campfires burn of {placed} placed in {coord:?}");
    }

    #[test]
    fn a_prop_is_placed_by_the_tile_that_holds_it_and_by_no_other() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let model = r"World\Generic\PassiveDoodads\Lights\CandelabraTallWall01.m2";
        let prop = Prop { model: model.into(), map: None, zone: None, pickup: None, sleep: false, physics: false, station: 0, at: (-8905.0, -192.0, 90.0), rotation: [0.0, 45.0, 0.0], scale: 1.0 };
        assert_eq!(tile_of_prop(&prop), (32, 48));
        let (models, cache) = (ModelCache::default(), TextureCache::default());
        let plain = load_tile(&mount, &cache, &models, "Azeroth", (32, 48), &[]).unwrap();
        let with = load_tile(&mount, &cache, &models, "Azeroth", (32, 48), std::slice::from_ref(&prop)).unwrap();
        assert_eq!((with.doodads.len(), with.fires.len()), (plain.doodads.len() + 1, plain.fires.len() + 1));
        // The model stands where it was put: the matrix moves the model's origin to the place, in the tile's own coordinates.
        let (ox, oz) = crate::coords::tile_origin((32, 48));
        let want = Vec3::new(192.0 - ox as f32, 90.0, 8905.0 - oz as f32);
        assert!(with.doodads.iter().any(|d| (d.matrix.transform_point3(Vec3::ZERO) - want).length() < 0.01));
        let other = load_tile(&mount, &cache, &models, "Azeroth", (32, 49), std::slice::from_ref(&prop)).unwrap();
        let none = load_tile(&mount, &cache, &models, "Azeroth", (32, 49), &[]).unwrap();
        assert_eq!(other.doodads.len(), none.doodads.len());
    }

    #[test]
    fn the_abbey_has_lamps_that_light_the_doodads_of_its_rooms() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let tile = load_tile(&mount, &TextureCache::default(), &ModelCache::default(), "Azeroth", (32, 48), &[]).unwrap();
        // The abbey has 42 lamps (warm, a share of the way out), and what stands near them is lit by them.
        assert!(tile.wmos.iter().any(|w| w.wmo.lights.len() == 42 && w.wmo.lights.iter().all(|l| l.kind == 0 && l.attenuation.1 > l.attenuation.0)));
        assert!(tile.doodads.iter().filter(|d| d.light.is_some_and(|l| l.lamps[0].color != [0.0; 3])).count() > 30);
    }
}
