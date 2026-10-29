//! The loaded world: streams terrain tiles around the camera on worker threads
//! and answers ground-height / area-name queries.

use crate::collision::{Collider, CollisionMesh};
use crate::navmesh::{NavCache, NavMesh, BORDER_YARDS, HALF_WORLD, TILE};
use crate::coords::{tile_of, tile_origin, TileCoord};
use crate::renderer::{Gpu, Renderer, TileGpu};
use crate::models::ModelCache;
use crate::content::Prop;
use crate::terrain::{tile_of_prop, Fire, load_tile, load_water_textures, GroundTexture, HeightField, TextureCache, TileCpu};
use anyhow::{Context, Result};
use mpq::Mount;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use wowfmt::adt::{CHUNK_SIZE, UNIT_SIZE};
use wowfmt::dbc::Dbc;
use wowfmt::wdt::Wdt;

const WORKERS: usize = 3;
/// Tiles uploaded to the GPU per frame, to keep frame times even.
const UPLOADS_PER_FRAME: usize = 1;
/// How far in front of a wall or the ground a camera stops (and `ray_hit` adds it back).
const MARGIN: f32 = 0.35;

struct LoadedTile {
    gpu: TileGpu,
    heights: HeightField,
    areas: Vec<u32>,
    surfaces: Vec<u8>,
    collision: CollisionMesh,
    fires: Vec<Fire>,
    liquids: Vec<Option<wowfmt::adt::Liquid>>,
    wmo_liquids: Vec<([glam::Vec3; 4], u32)>,
    sounds: Vec<(glam::Vec3, u32)>,
    beds: Vec<glam::Vec3>,
}

/// `AreaTable.dbc`: area id -> (parent zone id, name, exploration level).
pub struct AreaNames {
    by_id: HashMap<u32, (u32, String, u32)>,
    /// `WMOAreaTable.dbc`: (building, group) -> what the room is; group `u32::MAX` is the building's own row.
    rooms: HashMap<(u32, u32), Room>,
}

/// What a room of a building has of its own in `WMOAreaTable.dbc` (0: nothing, the area's or the building's applies).
#[derive(Clone, Copy, Default)]
pub struct Room {
    pub area: u32,
    pub ambience: u32,
    pub music: u32,
}

impl AreaNames {
    pub fn load(mount: &Mount) -> Result<Self> {
        let dbc = Dbc::parse(&mount.read("DBFilesClient\\AreaTable.dbc")?)?;
        // Fields: 0 id, 1 map, 2 parent zone, 10 exploration level, 11 name (enUS).
        let by_id = dbc.iter().map(|r| (r.u32(0), (r.u32(2), r.string(11).to_string(), r.u32(10)))).collect();
        // WMOAreaTable: 0 id, 1 building, 2 name set, 3 group (-1: the building), 6 ambience, 7 zone music, 10 area id.
        // The first name set of a room is the one it has.
        let mut rooms = HashMap::new();
        for r in Dbc::parse(&mount.read(r"DBFilesClient\WMOAreaTable.dbc")?)?.iter() {
            rooms.entry((r.u32(1), r.u32(3))).or_insert(Room { area: r.u32(10), ambience: r.u32(6), music: r.u32(7) });
        }
        Ok(Self { by_id, rooms })
    }

    /// The room of building `wmo` that is group `group`: what it has of its own, and the building's for what it has not.
    fn room(&self, (wmo, group): (u32, u32)) -> Option<Room> {
        let (own, whole) = (self.rooms.get(&(wmo, group)), self.rooms.get(&(wmo, u32::MAX)));
        let (a, b) = (own.or(whole)?, whole?);
        Some(Room { area: if a.area != 0 { a.area } else { b.area }, ambience: if a.ambience != 0 { a.ambience } else { b.ambience }, music: if a.music != 0 { a.music } else { b.music } })
    }

    /// The zone an area belongs to (the area itself when it is a zone).
    pub fn zone(&self, id: u32) -> u32 {
        self.by_id.get(&id).map_or(id, |&(parent, ..)| if parent != 0 { parent } else { id })
    }

    /// The area's own name and the level it is worth exploring at.
    pub fn explore_info(&self, id: u32) -> Option<(&str, u32)> {
        self.by_id.get(&id).map(|(_, name, level)| (name.as_str(), *level))
    }

    /// "Zone - Subzone", or just the zone when the area is a zone itself.
    pub fn display(&self, id: u32) -> String {
        let Some((parent, name, _)) = self.by_id.get(&id) else { return format!("area {id}") };
        match self.by_id.get(parent) {
            Some((_, zone, _)) if *parent != 0 && *parent != id => format!("{zone} - {name}"),
            _ => name.clone(),
        }
    }
}

pub struct World {
    pub areas: AreaNames,
    pub radius: i32,
    wdt: Wdt,
    /// The map is one building (a dungeon without terrain): it is kept as one copy, in the tile the camera is in, and moved to the next tile when the camera goes there
    /// (the building can be thousands of yards long, far more than the tiles kept round the camera).
    global: bool,
    mount: Arc<Mount>,
    cache: Arc<TextureCache>,
    tiles: HashMap<TileCoord, LoadedTile>,
    pending: HashSet<TileCoord>,
    failed: HashSet<TileCoord>,
    /// The editor's props, read by the loading threads; tiles asked for before they changed are loaded again.
    props: Arc<RwLock<Vec<Prop>>>,
    stale: HashSet<TileCoord>,
    /// The loaded tiles that are being made again for new props.
    refresh: HashSet<TileCoord>,
    requests: Sender<TileCoord>,
    results: Receiver<(TileCoord, Result<TileCpu>)>,
    /// The navigation mesh that creatures find their ways on, made tile by tile near the camera, and the tiles that were tried (some have nothing to walk on).
    pub nav: NavMesh,
    nav_tried: HashSet<(i32, i32)>,
    /// The tiles of the mesh kept in a file of the application folder, from the earlier runs and for the next (none when the file cannot be had, or once the props changed).
    nav_cache: Option<NavCache>,
}

impl World {
    pub fn open(mount: Arc<Mount>, map: &str, radius: i32) -> Result<World> {
        let wdt = Wdt::parse(&mount.read(&format!("World\\Maps\\{map}\\{map}.wdt"))?)
            .with_context(|| format!("map '{map}'"))?;
        anyhow::ensure!(wdt.tile_count() > 0, "map '{map}' has no terrain tiles");
        let areas = AreaNames::load(&mount)?;

        let (req_tx, req_rx) = channel::<TileCoord>();
        let (res_tx, res_rx) = channel();
        let req_rx = Arc::new(Mutex::new(req_rx));
        let cache = Arc::new(TextureCache::default());
        let models = Arc::new(ModelCache::default());
        let props = Arc::new(RwLock::new(crate::content::get().props.clone()));
        for _ in 0..WORKERS {
            let (mount, cache, models, req_rx, res_tx, map, props) =
                (mount.clone(), cache.clone(), models.clone(), req_rx.clone(), res_tx.clone(), map.to_string(), props.clone());
            std::thread::spawn(move || loop {
                // Hold the lock only while waiting for the next request.
                let next = req_rx.lock().unwrap().recv();
                let Ok(coord) = next else { break };
                let props = props.read().unwrap_or_else(|e| e.into_inner()).clone();
                let res = load_tile(&mount, &cache, &models, &map, coord, &props);
                if res_tx.send((coord, res)).is_err() {
                    break;
                }
            });
        }

        Ok(World {
            areas,
            radius,
            global: wdt.global.is_some(),
            wdt,
            mount,
            cache,
            tiles: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            props,
            stale: HashSet::new(),
            refresh: HashSet::new(),
            requests: req_tx,
            results: res_rx,
            nav: NavMesh::default(),
            nav_tried: HashSet::new(),
            nav_cache: open_nav_cache(map),
        })
    }

    /// Request tiles near `(x, z)`, upload finished ones and drop distant ones.
    pub fn update(&mut self, x: f32, z: f32, gpu: &Gpu, renderer: &mut Renderer) {
        let (cx, cz) = tile_of(x, z);
        let r = self.radius;

        let mut wanted: Vec<TileCoord> = Vec::new();
        for dz in -r..=r {
            for dx in -r..=r {
                let t = (cx + dx, cz + dz);
                let exists = if self.global { t == (cx, cz) } else { t.0 >= 0 && t.1 >= 0 && self.wdt.has_tile(t.0 as usize, t.1 as usize) };
                if exists && !self.tiles.contains_key(&t) && !self.pending.contains(&t) && !self.failed.contains(&t) {
                    wanted.push(t);
                }
            }
        }
        // Nearest first.
        wanted.sort_by_key(|t| (t.0 - cx).pow(2) + (t.1 - cz).pow(2));
        for t in wanted {
            self.pending.insert(t);
            let _ = self.requests.send(t);
        }

        for _ in 0..UPLOADS_PER_FRAME {
            let Ok((coord, res)) = self.results.try_recv() else { break };
            self.pending.remove(&coord);
            if self.stale.remove(&coord) {
                // Asked for before the props changed again: load it again.
                self.pending.insert(coord);
                let _ = self.requests.send(coord);
                continue;
            }
            self.refresh.remove(&coord);
            match res {
                Ok(cpu) => {
                    // The one copy of a building moves here; the one it replaces goes.
                    if self.global {
                        self.tiles.retain(|c, _| *c == coord);
                    }
                    let gpu_tile = renderer.upload_tile(gpu, &cpu);
                    self.tiles.insert(coord, LoadedTile { gpu: gpu_tile, heights: cpu.heights, areas: cpu.areas, surfaces: cpu.surfaces, collision: cpu.collision, fires: cpu.fires, liquids: cpu.liquids, wmo_liquids: cpu.wmo_liquids, sounds: cpu.sounds, beds: cpu.beds });
                }
                Err(e) => {
                    log::warn!("tile {coord:?}: {e:#}");
                    self.failed.insert(coord);
                }
            }
        }

        self.tiles.retain(|t, _| (t.0 - cx).abs() <= r + 1 && (t.1 - cz).abs() <= r + 1);
        self.refresh.retain(|t| self.tiles.contains_key(t) || self.pending.contains(t));
        self.update_nav(x, z);
    }

    /// Make the tiles of the navigation mesh near `(x, z)` (render space) that are still missing, the nearest first, as long as the frame's time allows (at least one).
    /// A tile is made when the terrain under it is loaded.
    fn update_nav(&mut self, x: f32, z: f32) {
        const REACH: f32 = 160.0;
        const BUDGET: std::time::Duration = std::time::Duration::from_millis(3);
        let began = std::time::Instant::now();
        let [north, west, _] = crate::coords::render_to_wow(glam::Vec3::new(x, 0.0, z));
        let tile = TILE as f32;
        let (ny, nx) = (((north as f64 + HALF_WORLD) / TILE).floor() as i32, ((west as f64 + HALF_WORLD) / TILE).floor() as i32);
        let span = (REACH / tile).ceil() as i32;
        let mut wanted: Vec<(i32, i32)> = Vec::new();
        for dy in -span..=span {
            for dx in -span..=span {
                let t = (nx + dx, ny + dy);
                if (dx * dx + dy * dy) as f32 * tile * tile <= REACH * REACH && !self.nav_tried.contains(&t) {
                    wanted.push(t);
                }
            }
        }
        wanted.sort_by_key(|t| (t.0 - nx).pow(2) + (t.1 - ny).pow(2));
        for (tx, ty) in wanted {
            if began.elapsed() > BUDGET {
                break;
            }
            // Kept from before: no terrain needed. (The cache counts tiles from the corner of the world: north first.)
            if let Some(kept) = self.nav_cache.as_mut().and_then(|c| c.load(ty, tx)) {
                log::debug!("navigation tile {tx},{ty} read from the cache");
                self.nav_tried.insert((tx, ty));
                if let Some(tile) = kept {
                    self.nav.add_tile(tile);
                }
                continue;
            }
            // The terrain under the tile, its border and the objects that may reach over it (a few dozen yards) has to be there.
            const REACH_OF_OBJECTS: f64 = 80.0;
            let (lo_n, lo_w) = (ty as f64 * TILE - HALF_WORLD - BORDER_YARDS - REACH_OF_OBJECTS, tx as f64 * TILE - HALF_WORLD - BORDER_YARDS - REACH_OF_OBJECTS);
            let side = TILE + 2.0 * (BORDER_YARDS + REACH_OF_OBJECTS);
            let corners = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].map(|(a, b)| crate::coords::wow_to_render([(lo_n + a * side) as f32, (lo_w + b * side) as f32, 0.0]));
            let ready = if self.global {
                // A dungeon is one building, kept as one copy in the tile the camera is in: the tile is made while the copy is within a tile of it.
                let here = tile_of(corners[0].x, corners[0].z);
                self.tiles.keys().any(|c| (c.0 - here.0).abs() <= 1 && (c.1 - here.1).abs() <= 1)
            } else {
                // (A square of the map that has no terrain counts as there.)
                corners.iter().all(|c| {
                    let t = tile_of(c.x, c.z);
                    self.tiles.contains_key(&t) || self.failed.contains(&t) || t.0 < 0 || t.1 < 0 || !self.wdt.has_tile(t.0 as usize, t.1 as usize)
                })
            };
            if !ready {
                continue;
            }
            self.nav_tried.insert((tx, ty));
            // (The tile is counted from the corner of the world: north first.)
            log::debug!("navigation tile {tx},{ty} built");
            let built = crate::navmesh::build(&*self, ty, tx);
            if let Some(cache) = self.nav_cache.as_mut() {
                let tile = built.as_ref().map(crate::navmesh::MeshTile::from_built);
                if let Err(e) = cache.save(ty, tx, tile.as_ref()) {
                    log::warn!("the navigation cache cannot be written: {e}");
                    self.nav_cache = None;
                }
            }
            if let Some(built) = built {
                self.nav.add(&built);
            }
        }
    }

    /// Use these props from now on: the tiles whose props changed are loaded again, and shown as they were until the new ones are there.
    pub fn set_props(&mut self, props: Vec<Prop>) {
        let mut changed = HashSet::new();
        // (The tiles of the mesh made from now on, and kept from before, are of other props.)
        self.nav_cache = None;
        {
            let mut old = self.props.write().unwrap_or_else(|e| e.into_inner());
            // Most edits touch one prop among thousands: only what lies between the same beginning and the same end is compared.
            let start = old.iter().zip(&props).take_while(|(a, b)| a == b).count();
            let end = old[start..].iter().rev().zip(props[start..].iter().rev()).take_while(|(a, b)| a == b).count();
            let (a, b) = (&old[start..old.len() - end], &props[start..props.len() - end]);
            for p in a.iter().filter(|p| !b.contains(p)).chain(b.iter().filter(|p| !a.contains(p))) {
                changed.insert(tile_of_prop(p));
            }
            *old = props;
        }
        for t in changed {
            self.failed.remove(&t);
            if self.pending.contains(&t) {
                self.stale.insert(t);
                self.refresh.insert(t);
            } else if self.tiles.contains_key(&t) {
                self.pending.insert(t);
                self.refresh.insert(t);
                let _ = self.requests.send(t);
            }
        }
    }

    /// Whether a tile is being made again for new props (what the editor moves is shown by other means until then).
    pub fn refreshing(&self) -> bool {
        !self.refresh.is_empty()
    }

    /// Liquid animation frames for `Renderer::set_water_textures`.
    /// Every placed model with particle emitters in the loaded tiles, by its tile and its number there.
    pub fn fires(&self) -> impl Iterator<Item = (TileCoord, usize, &Fire)> {
        self.tiles.iter().flat_map(|(&tile, t)| t.fires.iter().enumerate().map(move |(i, f)| (tile, i, f)))
    }

    pub fn water_textures(&self) -> Vec<Arc<GroundTexture>> {
        load_water_textures(&self.mount, &self.cache)
    }

    pub fn tiles(&self) -> impl Iterator<Item = (TileCoord, &TileGpu)> {
        self.tiles.iter().map(|(c, t)| (*c, &t.gpu))
    }

    pub fn loaded_count(&self) -> usize {
        self.tiles.len()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Terrain height under `(x, z)` in render space; `None` if no terrain is loaded there.
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let coord = tile_of(x, z);
        let tile = self.tiles.get(&coord)?;
        let (ox, oz) = tile_origin(coord);
        tile.heights.height_at((x as f64 - ox) as f32, (z as f64 - oz) as f32)
    }

    /// Where a creature that walks to `(x, z)` with its feet at height `y` stands: the highest surface of any slope within a yard above them; none for one under the terrain with nothing below it (it has no height yet, 0: the terrain).
    pub fn walker_floor(&self, x: f32, z: f32, y: f32) -> Option<f32> {
        let max_y = y + 1.0;
        let mut best = self.height_at(x, z).filter(|&h| h <= max_y);
        for (mesh, ox, oz) in self.nearby_meshes(x, z) {
            if let Some(s) = mesh.surface(x - ox, z - oz, max_y) {
                if best.is_none_or(|b| s > b) {
                    best = Some(s);
                }
            }
        }
        // A creature that has no height yet (0) is put on the terrain; one that has stays where it is when nothing lies under it, and is not ported up to the terrain over it.
        best.or_else(|| self.height_at(x, z).filter(|_| y == 0.0))
    }

    /// Footstep surface (`TerrainType.dbc` id) of the terrain under `(x, z)`.
    pub fn terrain_surface_at(&self, x: f32, z: f32) -> Option<u8> {
        let coord = tile_of(x, z);
        let tile = self.tiles.get(&coord)?;
        let (ox, oz) = tile_origin(coord);
        let (lx, lz) = (((x as f64 - ox) as f32).max(0.0), ((z as f64 - oz) as f32).max(0.0));
        let (cx, cz) = (((lx / CHUNK_SIZE) as usize).min(15), ((lz / CHUNK_SIZE) as usize).min(15));
        let cell = |l: f32, c: usize| (((l - c as f32 * CHUNK_SIZE) / CHUNK_SIZE * 16.0) as usize).min(15);
        tile.surfaces.get((cz * 16 + cx) * 256 + cell(lz, cz) * 16 + cell(lx, cx)).copied()
    }

    /// Surface under a character standing at `feet`: stone on a building floor or bridge, otherwise the terrain's.
    pub fn surface_under(&self, feet: glam::Vec3) -> u8 {
        use crate::terrain::{SURFACE_DIRT, SURFACE_STONE};
        match (Collider::floor(self, feet.x, feet.z, feet.y + 1.0), self.height_at(feet.x, feet.z)) {
            (Some(floor), Some(ground)) if floor > ground + 0.3 => SURFACE_STONE,
            _ => self.terrain_surface_at(feet.x, feet.z).unwrap_or(SURFACE_DIRT),
        }
    }

    /// Whether a point (render space) is inside a building: the original lets no one ride there.
    pub fn indoors(&self, at: glam::Vec3) -> bool {
        self.tiles.values().any(|tile| tile.gpu.indoors(at))
    }

    /// The room of a building a point (render space) is in.
    pub fn room_at(&self, at: glam::Vec3) -> Option<Room> {
        self.place_at(at).1
    }

    /// The area at a point (render space): the room of a building when it is an area of its own, else the terrain's.
    pub fn area_at(&self, at: glam::Vec3) -> Option<u32> {
        self.place_at(at).0
    }

    /// The area and the room at a point, from one look for the room (which is the costly part: it tests the floors of the buildings).
    pub fn place_at(&self, at: glam::Vec3) -> (Option<u32>, Option<Room>) {
        let (x, z) = (at.x, at.z);
        let coord = tile_of(x, z);
        let Some(tile) = self.tiles.get(&coord) else { return (None, None) };
        let room = tile.gpu.room_at(at).and_then(|r| self.areas.room(r));
        let (ox, oz) = tile_origin(coord);
        let (lx, lz) = ((x as f64 - ox) as f32, (z as f64 - oz) as f32);
        let (cx, cz) = (((lx / CHUNK_SIZE) as usize).min(15), ((lz / CHUNK_SIZE) as usize).min(15));
        let terrain = tile.areas.get(cz * 16 + cx).copied();
        // A model used in several zones (a farm barn) names the area of one of them: it counts only in the zone of the ground it stands on.
        let same_zone = |a: u32| terrain.filter(|&t| t != 0).map_or(true, |t| self.areas.zone(a) == self.areas.zone(t));
        if let Some(r) = room.filter(|r| r.area != 0 && same_zone(r.area)) {
            return (Some(r.area), room);
        }
        (terrain, room)
    }
}

/// For each kind of liquid heard (water, ocean, magma, slime) and each speed it flows at (still, slow, fast), the nearest place it is.
pub type Waters = [[Option<glam::Vec3>; 3]; 4];

/// The liquid of a cell: its kind (an index into `SoundWaterType`'s liquid types), how fast it flows, and its surface at the four corners.
struct Cell {
    kind: usize,
    speed: usize,
    min: (f32, f32),
    height: f32,
    /// The surface at the corners (x, z), (x + 1, z), (x, z + 1) and (x + 1, z + 1) of the cell.
    corners: [f32; 4],
}

impl World {
    /// The cells with liquid in the chunks of the tiles round `(x, z)` that come within `reach` of it.
    fn liquid_cells(&self, x: f32, z: f32, reach: f32) -> Vec<Cell> {
        let mut out = Vec::new();
        let (cx, cz) = tile_of(x, z);
        for coord in (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (cx + dx, cz + dz))) {
            let Some(tile) = self.tiles.get(&coord) else { continue };
            let (ox, oz) = tile_origin(coord);
            for (slot, liquid) in tile.liquids.iter().enumerate().filter_map(|(i, l)| l.as_ref().map(|l| (i, l))) {
                let (chunk_x, chunk_z) = (ox as f32 + (slot % 16) as f32 * CHUNK_SIZE, oz as f32 + (slot / 16) as f32 * CHUNK_SIZE);
                if x < chunk_x - reach || x > chunk_x + CHUNK_SIZE + reach || z < chunk_z - reach || z > chunk_z + CHUNK_SIZE + reach {
                    continue;
                }
                let kind = if liquid.kind == 0 { 0 } else { liquid.kind.trailing_zeros().min(3) as usize };
                for cell in 0..64usize {
                    let flags = liquid.cell_flags[cell];
                    if flags & 0x0f == 0x0f {
                        continue;
                    }
                    let (cell_x, cell_z) = (cell % 8, cell / 8);
                    let h = |dx: usize, dz: usize| liquid.heights[(cell_z + dz) * 9 + cell_x + dx];
                    out.push(Cell {
                        kind,
                        // The low bits of a river cell's flags are how fast it flows: 4 slowly, 8 fast.
                        speed: (((flags & 0x0f) >> 2) & 3).min(2) as usize,
                        min: (chunk_x + cell_x as f32 * UNIT_SIZE, chunk_z + cell_z as f32 * UNIT_SIZE),
                        height: (h(0, 0) + h(1, 0) + h(0, 1) + h(1, 1)) / 4.0,
                        corners: [h(0, 0), h(1, 0), h(0, 1), h(1, 1)],
                    });
                }
            }
        }
        out
    }

    /// A few of the liquid cells of the buildings (for a look at what is loaded).
    pub fn sample_building_liquids(&self) -> Vec<glam::Vec3> {
        self.tiles.iter().flat_map(|(&c, t)| { let (ox, oz) = tile_origin(c); t.wmo_liquids.iter().step_by(97).map(move |(q, _)| q[0] + glam::Vec3::new(ox as f32, 0.0, oz as f32)) }).take(12).collect()
    }

    /// How many liquid cells the buildings of the loaded tiles have.
    pub fn building_liquid_cells(&self) -> usize {
        self.tiles.values().map(|t| t.wmo_liquids.len()).sum()
    }

    /// The liquid cells of the buildings round `(x, z)` that come within `reach` of it: the four corners (render space) and the kind.
    fn wmo_cells(&self, x: f32, z: f32, reach: f32) -> Vec<([glam::Vec3; 4], usize)> {
        let mut out = Vec::new();
        let (cx, cz) = tile_of(x, z);
        for coord in (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (cx + dx, cz + dz))) {
            let Some(tile) = self.tiles.get(&coord) else { continue };
            let (ox, oz) = tile_origin(coord);
            let shift = glam::Vec3::new(ox as f32, 0.0, oz as f32);
            for &(corners, kind) in &tile.wmo_liquids {
                let c = corners.map(|p| p + shift);
                let (lo, hi) = (c.iter().fold(glam::Vec3::MAX, |m, p| m.min(*p)), c.iter().fold(glam::Vec3::MIN, |m, p| m.max(*p)));
                if x >= lo.x - reach && x <= hi.x + reach && z >= lo.z - reach && z <= hi.z + reach {
                    out.push((c, kind as usize));
                }
            }
        }
        out
    }

    /// The surface of the liquid that is nearest the height `y` at `(x, z)`, if there is any: a lake or river of the terrain, or the pool of a
    /// building (not its depth: the terrain may be above it).
    pub fn liquid_at(&self, x: f32, z: f32, y: f32) -> Option<f32> {
        let terrain = self.liquid_cells(x, z, 0.0).into_iter().find(|c| x >= c.min.0 && x < c.min.0 + UNIT_SIZE && z >= c.min.1 && z < c.min.1 + UNIT_SIZE).map(|c| c.height);
        let rooms = self.wmo_cells(x, z, 0.0).into_iter().filter_map(|(q, _)| height_in_quad(x, z, &q));
        terrain.into_iter().chain(rooms).min_by(|a, b| (a - y).abs().total_cmp(&(b - y).abs()))
    }

    /// The highest surface of liquid at `(x, z)`.
    pub fn liquid_level(&self, x: f32, z: f32) -> Option<f32> {
        self.liquid_at(x, z, f32::MAX)
    }

    /// The liquid cells that touch the square of `half` yards round `(x, z)` and lie within three yards of the height `y`: the four corners of each
    /// (render space), what a ripple is laid on.
    pub fn liquid_quads(&self, x: f32, z: f32, y: f32, half: f32) -> Vec<[glam::Vec3; 4]> {
        let touches = |c: &Cell| c.min.0 < x + half && c.min.0 + UNIT_SIZE > x - half && c.min.1 < z + half && c.min.1 + UNIT_SIZE > z - half;
        let terrain = self.liquid_cells(x, z, half).into_iter().filter(touches).map(|c| {
            let (x0, z0, x1, z1) = (c.min.0, c.min.1, c.min.0 + UNIT_SIZE, c.min.1 + UNIT_SIZE);
            [glam::Vec3::new(x0, c.corners[0], z0), glam::Vec3::new(x1, c.corners[1], z0), glam::Vec3::new(x1, c.corners[3], z1), glam::Vec3::new(x0, c.corners[2], z1)]
        });
        let rooms = self.wmo_cells(x, z, half).into_iter().map(|(q, _)| q).filter(|q| {
            let (lo, hi) = (q.iter().fold(glam::Vec3::MAX, |m, p| m.min(*p)), q.iter().fold(glam::Vec3::MIN, |m, p| m.max(*p)));
            lo.x < x + half && hi.x > x - half && lo.z < z + half && hi.z > z - half
        });
        terrain.chain(rooms).filter(|q| (q.iter().map(|p| p.y).sum::<f32>() / 4.0 - y).abs() < 3.0).collect()
    }

    /// For each kind of liquid and flow, the nearest place of it within `radius` yards of `at` (what the original hears of water, FUN_00462b50).
    pub fn nearby_liquids(&self, at: glam::Vec3, radius: f32) -> Waters {
        let mut found: Waters = Default::default();
        for c in self.liquid_cells(at.x, at.z, radius) {
            let p = glam::Vec3::new(at.x.clamp(c.min.0, c.min.0 + UNIT_SIZE), c.height, at.z.clamp(c.min.1, c.min.1 + UNIT_SIZE));
            let slot = &mut found[c.kind][c.speed];
            if p.distance(at) <= radius && slot.is_none_or(|q| p.distance(at) < q.distance(at)) {
                *slot = Some(p);
            }
        }
        // The pools of buildings (the cells of their liquids flow nowhere).
        for (q, kind) in self.wmo_cells(at.x, at.z, radius) {
            let height = q.iter().map(|p| p.y).sum::<f32>() / 4.0;
            let (lo, hi) = (q.iter().fold(glam::Vec3::MAX, |m, p| m.min(*p)), q.iter().fold(glam::Vec3::MIN, |m, p| m.max(*p)));
            let p = glam::Vec3::new(at.x.clamp(lo.x, hi.x), height, at.z.clamp(lo.z, hi.z));
            let slot = &mut found[kind.min(3)][0];
            if p.distance(at) <= radius && slot.is_none_or(|q| p.distance(at) < q.distance(at)) {
                *slot = Some(p);
            }
        }
        found
    }

    /// The models near `at` that play a sound all the time (a waterfall, a fountain): where, and the number of the sound entry.
    pub fn nearby_sounds(&self, at: glam::Vec3, radius: f32) -> Vec<(glam::Vec3, u32)> {
        let (cx, cz) = tile_of(at.x, at.z);
        let mut out = Vec::new();
        for coord in (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (cx + dx, cz + dz))) {
            let Some(tile) = self.tiles.get(&coord) else { continue };
            let (ox, oz) = tile_origin(coord);
            out.extend(tile.sounds.iter().map(|&(p, id)| (glam::Vec3::new(p.x + ox as f32, p.y, p.z + oz as f32), id)).filter(|(p, _)| p.distance(at) <= radius));
        }
        out
    }

    /// Whether a bed of the map or of its buildings is within `range` yards of `at` (sideways) and `height` of it (up or down).
    pub fn bed_near(&self, at: glam::Vec3, range: f32, height: f32, seen: impl Fn(glam::Vec3) -> bool) -> bool {
        let (cx, cz) = tile_of(at.x, at.z);
        (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (cx + dx, cz + dz))).any(|coord| {
            let Some(tile) = self.tiles.get(&coord) else { return false };
            let (ox, oz) = tile_origin(coord);
            tile.beds.iter().any(|b| (b.x + ox as f32 - at.x).hypot(b.z + oz as f32 - at.z) <= range && (b.y - at.y).abs() < height && seen(glam::Vec3::new(b.x + ox as f32, b.y, b.z + oz as f32)))
        })
    }

    /// Loaded tiles around `(x, z)`: objects owned by a neighbour can reach into this tile.
    fn nearby_meshes(&self, x: f32, z: f32) -> impl Iterator<Item = (&CollisionMesh, f32, f32)> {
        let (cx, cz) = tile_of(x, z);
        (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| (cx + dx, cz + dz))).filter_map(move |coord| {
            let tile = self.tiles.get(&coord)?;
            let (ox, oz) = tile_origin(coord);
            (!tile.collision.is_empty()).then_some((&tile.collision, ox as f32, oz as f32))
        })
    }

    /// Render-space centroids of collision triangles with the given slope range within `radius` of `(x, z)`.
    pub fn ramp_centroids(&self, x: f32, z: f32, radius: f32, min_deg: f32, max_deg: f32) -> Vec<glam::Vec3> {
        self.nearby_meshes(x, z)
            .flat_map(|(mesh, ox, oz)| {
                mesh.centroids_with_slope(min_deg, max_deg)
                    .into_iter()
                    .map(move |c| glam::Vec3::new(c.x + ox, c.y, c.z + oz))
            })
            .filter(|c| (c.x - x).hypot(c.z - z) <= radius)
            .collect()
    }

    pub fn triangles_near(&self, x: f32, z: f32, radius: f32) -> Vec<[glam::Vec3; 3]> {
        self.nearby_meshes(x, z)
            .flat_map(|(mesh, ox, oz)| {
                let off = glam::Vec3::new(ox, 0.0, oz);
                mesh.triangles_near(x - ox, z - oz, radius).into_iter().map(move |t| [t[0] + off, t[1] + off, t[2] + off])
            })
            .collect()
    }

    /// How far from `from` towards `to` a camera can go before it touches the terrain or an object (a
    /// wall, a roof, a tree trunk), keeping a margin so the near plane does not cut into it.
    pub fn camera_distance(&self, from: glam::Vec3, to: glam::Vec3) -> f32 {
        let (dir, len) = (to - from, from.distance(to));
        let mut hit: Option<f32> = None;
        for (mesh, ox, oz) in self.nearby_meshes(from.x, from.z) {
            let off = glam::Vec3::new(ox, 0.0, oz);
            if let Some(t) = mesh.raycast(from - off, to - off) {
                hit = Some(hit.map_or(t, |h: f32| h.min(t)));
            }
        }
        // The terrain: march along the segment until it dips below the ground. Not from inside a mine or cave,
        // where the hill's surface is above the head.
        let above_ground = self.height_at(from.x, from.z).is_none_or(|g| from.y >= g);
        let steps = if above_ground { (len / 0.25).ceil().max(1.0) as usize } else { 0 };
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let p = from + dir * t;
            if self.height_at(p.x, p.z).is_some_and(|g| p.y < g + 0.15) {
                hit = Some(hit.map_or(t, |h| h.min(t)));
                break;
            }
        }
        hit.map_or(len, |t| (t * len - MARGIN).max(0.0))
    }

    /// Where the segment from `from` to `to` first touches the terrain or an object, if it does.
    pub fn ray_hit(&self, from: glam::Vec3, to: glam::Vec3) -> Option<glam::Vec3> {
        let len = from.distance(to);
        let at = self.camera_distance(from, to) + MARGIN;
        (at < len).then(|| from + (to - from) / len * at)
    }

    /// Whether a walker who goes from `from` to `to` (feet, render space) is stopped by a wall, a tree, a rock or a steep bank: something solid at his knee or his
    /// waist within half a yard of the way, or ground that climbs more than a yard and a fifth for every yard.
    pub fn blocked(&self, from: glam::Vec3, to: glam::Vec3) -> bool {
        const REACH: f32 = 0.5;
        const STEEPEST: f32 = 1.2;
        let flat = glam::Vec3::new(to.x - from.x, 0.0, to.z - from.z);
        let length = flat.length();
        if length < 1e-5 {
            return false;
        }
        let ahead = to + flat / length * REACH;
        // The rays run along a floor that climbs ahead (a ramp into a mine) instead of into it.
        let top = Collider::floor(self, ahead.x, ahead.z, from.y + 1.0).map_or(from.y, |y| y.max(from.y));
        for height in [0.5, 1.2] {
            let up = glam::Vec3::Y * height;
            for (mesh, ox, oz) in self.nearby_meshes(from.x, from.z) {
                let off = glam::Vec3::new(ox, 0.0, oz);
                if mesh.raycast(from + up - off, glam::Vec3::new(ahead.x, top, ahead.z) + up - off).is_some() {
                    return true;
                }
            }
        }
        // A bank of terrain, unless the walker stands on a floor above it.
        matches!((self.height_at(from.x, from.z), self.height_at(to.x, to.z)), (Some(a), Some(b)) if from.y <= a + 0.3 && (b - a) / length > STEEPEST)
    }

    pub fn collision_triangles(&self) -> usize {
        self.tiles.values().map(|t| t.collision.triangle_count()).sum()
    }
}

impl crate::weather::Terrain for World {
    fn ground(&self, x: f32, z: f32) -> Option<f32> {
        self.height_at(x, z)
    }

    fn solid_below(&self, x: f32, z: f32, from_y: f32) -> Option<f32> {
        const DEPTH: f32 = 150.0;
        let (from, to) = (glam::Vec3::new(x, from_y, z), glam::Vec3::new(x, from_y - DEPTH, z));
        self.nearby_meshes(x, z)
            .filter_map(|(mesh, ox, oz)| {
                let off = glam::Vec3::new(ox, 0.0, oz);
                mesh.raycast(from - off, to - off)
            })
            .min_by(f32::total_cmp)
            .map(|t| from_y - t * DEPTH)
    }
}

impl Collider for World {
    fn floor(&self, x: f32, z: f32, max_y: f32) -> Option<f32> {
        let mut best = self.height_at(x, z).filter(|&h| h <= max_y);
        for (mesh, ox, oz) in self.nearby_meshes(x, z) {
            if let Some(y) = mesh.floor(x - ox, z - oz, max_y) {
                if best.is_none_or(|b| y > b) {
                    best = Some(y);
                }
            }
        }
        best
    }
}

impl crate::movement::TriangleSource for World {
    /// Terrain cells and object collision meshes overlapping a box, in WoW coordinates
    /// (x north, y west, z up), relative to `origin`.
    fn gather(&self, origin: [f64; 3], min: glam::Vec3, max: glam::Vec3, out: &mut Vec<crate::movement::Tri>) {
        use crate::movement::Tri as MTri;
        // WoW (x, y, z) -> render (X = -y, Y = z, Z = -x).
        let (lo, hi) = (
            glam::DVec3::new(-(origin[1] + max.y as f64), origin[2] + min.z as f64, -(origin[0] + max.x as f64)),
            glam::DVec3::new(-(origin[1] + min.y as f64), origin[2] + max.z as f64, -(origin[0] + min.x as f64)),
        );
        let (t0, t1) = (tile_of(lo.x as f32, lo.z as f32), tile_of(hi.x as f32, hi.z as f32));
        let mut push = |ox: f64, oz: f64, v: [glam::Vec3; 3]| {
            let w = v.map(|p| {
                let (rx, ry, rz) = (p.x as f64 + ox, p.y as f64, p.z as f64 + oz);
                // render -> WoW, then relative to the origin
                glam::Vec3::new((-rz - origin[0]) as f32, (-rx - origin[1]) as f32, (ry - origin[2]) as f32)
            });
            if let Some(t) = MTri::new(w[0], w[1], w[2]) {
                out.push(t);
            }
        };
        // Objects can reach across tile borders, so look one tile further than the box.
        for tz in (t0.1 - 1)..=(t1.1 + 1) {
            for tx in (t0.0 - 1)..=(t1.0 + 1) {
                let coord = (tx, tz);
                let Some(tile) = self.tiles.get(&coord) else { continue };
                let (ox, oz) = tile_origin(coord);
                let local_min = glam::Vec3::new((lo.x - ox) as f32, lo.y as f32, (lo.z - oz) as f32);
                let local_max = glam::Vec3::new((hi.x - ox) as f32, hi.y as f32, (hi.z - oz) as f32);
                let inside = tx >= t0.0 && tx <= t1.0 && tz >= t0.1 && tz <= t1.1;
                if inside {
                    let mut cells = Vec::new();
                    tile.heights.cell_triangles(local_min.x, local_max.x, local_min.z, local_max.z, &mut cells);
                    for c in cells {
                        push(ox, oz, c);
                    }
                }
                tile.collision.for_each_in_box(local_min, local_max, |t| push(ox, oz, [t.a, t.b, t.c]));
            }
        }
    }

    fn gather_liquid(&self, origin: [f64; 3], min: glam::Vec3, max: glam::Vec3, out: &mut Vec<crate::movement::Wet>) {
        use crate::movement::Wet;
        let (lo, hi) = (
            glam::DVec3::new(-(origin[1] + max.y as f64), 0.0, -(origin[0] + max.x as f64)),
            glam::DVec3::new(-(origin[1] + min.y as f64), 0.0, -(origin[0] + min.x as f64)),
        );
        // The navigation area of a kind of liquid (0 water, 1 ocean, 2 magma, 3 slime).
        let area = |kind: usize| if kind >= 2 { 8 } else { 9 };
        // render -> WoW, then relative to the origin.
        let to_wow = |p: glam::Vec3| glam::Vec3::new((-(p.z as f64) - origin[0]) as f32, (-(p.x as f64) - origin[1]) as f32, (p.y as f64 - origin[2]) as f32);
        let mut quad = |q: [glam::Vec3; 4], kind: usize, cell: Option<(i32, i32)>| {
            // The corners go round the cell: two triangles.
            let w = q.map(to_wow);
            out.push(Wet { v: [w[0], w[1], w[2]], area: area(kind), cell });
            out.push(Wet { v: [w[0], w[2], w[3]], area: area(kind), cell });
        };
        let (cx, cz) = (((lo.x + hi.x) / 2.0) as f32, ((lo.z + hi.z) / 2.0) as f32);
        let reach = ((hi.x - lo.x).max(hi.z - lo.z) / 2.0) as f32 + UNIT_SIZE;
        for c in self.liquid_cells(cx, cz, reach) {
            if (c.min.0 as f64) > hi.x || (c.min.0 as f64 + UNIT_SIZE as f64) < lo.x || (c.min.1 as f64) > hi.z || (c.min.1 as f64 + UNIT_SIZE as f64) < lo.z {
                continue;
            }
            let (x0, z0, x1, z1) = (c.min.0, c.min.1, c.min.0 + UNIT_SIZE, c.min.1 + UNIT_SIZE);
            // (North and west of the cell's middle, counted in cells from the corner of the world.)
            let key = (((-(z0 + z1) / 2.0) as f64 + HALF_WORLD).div_euclid(UNIT_SIZE as f64) as i32, ((-(x0 + x1) / 2.0) as f64 + HALF_WORLD).div_euclid(UNIT_SIZE as f64) as i32);
            quad([glam::Vec3::new(x0, c.corners[0], z0), glam::Vec3::new(x1, c.corners[1], z0), glam::Vec3::new(x1, c.corners[3], z1), glam::Vec3::new(x0, c.corners[2], z1)], c.kind, Some(key));
        }
        for (q, kind) in self.wmo_cells(cx, cz, reach) {
            let (qlo, qhi) = (q.iter().fold(glam::Vec3::MAX, |m, p| m.min(*p)), q.iter().fold(glam::Vec3::MIN, |m, p| m.max(*p)));
            if (qlo.x as f64) <= hi.x && (qhi.x as f64) >= lo.x && (qlo.z as f64) <= hi.z && (qhi.z as f64) >= lo.z {
                quad(q, kind, None);
            }
        }
    }

    fn is_loaded(&self, origin: [f64; 3]) -> bool {
        let (x, z) = (-origin[1], -origin[0]);
        self.tiles.contains_key(&tile_of(x as f32, z as f32))
    }
}

/// The height of the surface of a liquid cell (the quad `q`, its corners in order round it) above `(x, z)`, if the point is over it.
fn height_in_quad(x: f32, z: f32, q: &[glam::Vec3; 4]) -> Option<f32> {
    let over = |a: glam::Vec3, b: glam::Vec3, c: glam::Vec3| {
        let d = (b.z - c.z) * (a.x - c.x) + (c.x - b.x) * (a.z - c.z);
        if d.abs() < 1e-9 {
            return None;
        }
        let u = ((b.z - c.z) * (x - c.x) + (c.x - b.x) * (z - c.z)) / d;
        let v = ((c.z - a.z) * (x - c.x) + (a.x - c.x) * (z - c.z)) / d;
        (u >= 0.0 && v >= 0.0 && u + v <= 1.0).then(|| u * a.y + v * b.y + (1.0 - u - v) * c.y)
    };
    over(q[0], q[1], q[2]).or_else(|| over(q[0], q[2], q[3]))
}

/// The file that keeps the navigation tiles of `map`, stamped with the props of the content (and the builder): none if it cannot be had.
fn open_nav_cache(map: &str) -> Option<NavCache> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", crate::content::get().props).hash(&mut hasher);
    let path = crate::save::base_dir().join("navmesh").join(format!("{map}.nav"));
    NavCache::open(&path, hasher.finish()).map_err(|e| log::warn!("{}: {e}", path.display())).ok()
}
