//! The world map (key M), in the style of `levelup.rs` round the original's pictures: the map of the zone the character stands in, with the areas it has
//! explored drawn in (the overlays of `WorldMapOverlay.dbc`), the places it has discovered and its own spot. As in the original, a click on a zone (or a city)
//! of a continent opens its map, a click on a continent of the world map opens that, and the right button goes one map out. The wheel zooms, dragging moves
//! the map and a click on a discovered place travels there.

use super::*;
use crate::coords::render_to_wow;
use std::collections::HashMap;
use wowfmt::dbc::Dbc;

/// The map's extent in WoW coordinates (`WorldMapArea.dbc`, row Elwynn): west at its left and right edges, north at its top and bottom.
const LEFT: f32 = 1535.4166;
const RIGHT: f32 = -1935.4166;
const TOP: f32 = -7939.583;
const BOTTOM: f32 = -10254.166;
/// The picture is 4 x 3 tiles of 256 texels, of which 1002 x 668 show (as in the original's `WorldMapDetailFrame`).
const PICTURE: (f32, f32) = (1002.0, 668.0);
const TILE: f32 = 256.0;
/// How much of the picture's size the window gets, in interface units.
const SHOWN: f32 = 0.78;
/// The smallest the picture goes when the window is resized (its share of the 1002 x 668 picture).
const MIN_SCALE: f32 = 0.55;
/// How close (interface units) the pointer must be to a place to pick it.
const REACH: f32 = 12.0;
const MAX_ZOOM: f32 = 8.0;
/// Pixels the pointer may move between pressing and releasing for it to be a click and not a drag.
const DRAG: f32 = 5.0;
const GOLD: [f32; 4] = [1.0, 0.82, 0.0, 1.0];

/// The zone grid of a continent (`Kalimdor.zmp`, `Azeroth.zmp`): 128 x 128 `AreaTable` ids over the world's 64 x 64 terrain tiles, so a cell is half a tile square.
const GRID: usize = 128;
const TILE_YARDS: f32 = 533.33331;

/// A picture of an area that shows once the character has been in it (`WorldMapOverlay.dbc`).
struct Overlay {
    /// The texture files are this name and a number.
    name: String,
    /// The `AreaTable` ids that explore it.
    areas: [u32; 4],
    /// Size and place in the picture, texels.
    size: (f32, f32),
    at: (f32, f32),
}

/// One map of `WorldMapArea.dbc`: a zone or a city with its picture's folder, its extent and its overlays.
struct Zone {
    /// The folder of its pictures under `Interface\WorldMap`.
    dir: String,
    /// The `AreaTable` id of the zone and its name.
    area: u32,
    title: String,
    /// The id of the map it lies on (which continent it belongs to).
    map: u32,
    /// Whether it is a capital (`AreaTable` flag 0x100): a city lies in a zone.
    city: bool,
    /// West at the left and right edges, north at the top and bottom (as the consts above).
    bounds: [f32; 4],
    overlays: Vec<Overlay>,
}

/// A continent: a row of `WorldMapArea.dbc` without an area, its place on the world map from `WorldMapContinent.dbc`, and its zone grid.
struct Continent {
    dir: String,
    /// Its name in `Map.dbc`.
    title: String,
    map: u32,
    bounds: [f32; 4],
    /// Where it lies on the world map (0 to 1 each way: left, top, right, bottom) and how much of its highlight picture that covers (the picture is square).
    rect: [f32; 4],
    share: (f32, f32),
    /// For each cell of the grid the zone it belongs to (an index into `zones`, and one more; 0 for none), and whether the cell is open water ("The Great Sea").
    grid: Vec<u16>,
    water: Vec<bool>,
}

/// Every map and the parent of each `AreaTable` id, read when the map is first opened.
struct Atlas {
    zones: Vec<Zone>,
    continents: Vec<Continent>,
    parent: HashMap<u32, u32>,
}

impl Atlas {
    /// The map (an index into `zones`) an `AreaTable` id belongs to on a map of `map`: the area's parent zone, or itself (a zone, a city).
    fn zone_of(&self, area: u32, map: Option<u32>) -> Option<usize> {
        let area = self.parent.get(&area).copied().filter(|p| *p != 0).unwrap_or(area);
        self.zones.iter().position(|z| z.area == area && map.is_none_or(|m| z.map == m))
    }

    /// The continent a zone lies on.
    fn continent_of(&self, zone: usize) -> Option<usize> {
        self.continents.iter().position(|c| c.map == self.zones[zone].map)
    }

    /// The zone a city stands in: the one that has most of the land cells of the grid round the city's own (the sea next to a city belongs to a zone too).
    fn home_of(&self, city: usize) -> Option<usize> {
        let [left, right, top, bottom] = self.zones[city].bounds;
        let continent = &self.continents[self.continent_of(city)?];
        let grid = &continent.grid;
        // The cells from the city's west to its east edge and from its north to its south edge, and two more all round.
        let cell = |yards: f32| 64.0 - yards / (TILE_YARDS / 2.0);
        let (cols, rows) = ((cell(left) as i32 - 2, cell(right) as i32 + 2), (cell(top) as i32 - 2, cell(bottom) as i32 + 2));
        let mut votes: HashMap<usize, usize> = HashMap::new();
        for (row, col) in (rows.0..=rows.1).flat_map(|r| (cols.0..=cols.1).map(move |c| (r, c))) {
            if (0..GRID as i32).contains(&row) && (0..GRID as i32).contains(&col) && !continent.water[row as usize * GRID + col as usize] {
                if let Some(zone) = usize::from(grid[row as usize * GRID + col as usize]).checked_sub(1).filter(|z| !self.zones[*z].city) {
                    *votes.entry(zone).or_default() += 1;
                }
            }
        }
        votes.into_iter().max_by_key(|&(zone, n)| (n, std::cmp::Reverse(zone))).map(|(zone, _)| zone)
    }
}

/// What the map shows: the world, a continent or a zone (an index into the atlas; none before it is read).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    World,
    Continent(usize),
    Zone(Option<usize>),
}

/// A button of the window's header.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Close,
    Lock,
}

/// What the pressed mouse button has hold of: the picture (a drag moves what is shown of it; how far it has gone says whether it was a click), the window (a drag moves
/// it), the window's corner (a drag resizes it) or a button of its header (a click on it).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Grab {
    Pan { last: (f32, f32), travelled: f32 },
    Move((f32, f32)),
    Resize,
    Press(Part),
}

/// Where the parts of the window are on a screen (pixels): the window, the map's picture in it, its buttons, its corner for resizing and its header.
struct Frame {
    panel: [f32; 4],
    rect: [f32; 4],
    close: [f32; 4],
    lock: [f32; 4],
    grip: [f32; 4],
    header: [f32; 4],
    /// Pixels to a unit of the interface.
    unit: f32,
}

/// What the open map keeps: the middle of the part shown (0 to 1 over the picture) and the zoom, what the mouse has hold of, the area the character stands in, what is
/// shown, the atlas itself, and the window: the size of its picture (a share of the original's), where its top left corner is (in units; none while it is where it opens,
/// in the middle) and whether it is locked in place.
pub struct MapState {
    centre: (f32, f32),
    zoom: f32,
    grab: Option<Grab>,
    here: Option<u32>,
    view: View,
    atlas: Option<Atlas>,
    scale: f32,
    origin: Option<(f32, f32)>,
    locked: bool,
}

impl Default for MapState {
    fn default() -> Self {
        MapState { centre: (0.5, 0.5), zoom: 1.0, grab: None, here: None, view: View::Zone(None), atlas: None, scale: SHOWN, origin: None, locked: true }
    }
}

impl MapState {
    /// Keep the shown part inside the picture.
    fn clamp(&mut self) {
        self.zoom = self.zoom.clamp(1.0, MAX_ZOOM);
        let half = 0.5 / self.zoom;
        self.centre = (self.centre.0.clamp(half, 1.0 - half), self.centre.1.clamp(half, 1.0 - half));
    }

    /// Where the parts of the window are: the window is as big as its picture (kept to what the screen has room for) and its chrome, and stays on the screen.
    fn frame(&self, screen: (u32, u32)) -> Frame {
        let unit = screen.1 as f32 / ui_height();
        let (width, height) = (screen.0 as f32 / unit, ui_height());
        let fit = ((width - 32.0) / PICTURE.0).min((height - 120.0) / PICTURE.1);
        let scale = self.scale.min(fit).max(MIN_SCALE.min(fit));
        let (w, h) = (PICTURE.0 * scale + 32.0, PICTURE.1 * scale + 120.0);
        // Opened in the middle, a little up, clear of the action bar.
        let (x, y) = self.origin.unwrap_or(((width - w) / 2.0, (height - h) / 2.0 - 46.0));
        let (x, y) = (x.clamp(0.0, (width - w).max(0.0)), y.clamp(0.0, (height - h).max(0.0)));
        let at = |dx: f32, dy: f32, w: f32, h: f32| [(x + dx) * unit, (y + dy) * unit, w * unit, h * unit];
        Frame { panel: at(0.0, 0.0, w, h), rect: at(16.0, 72.0, PICTURE.0 * scale, PICTURE.1 * scale), close: at(w - 42.0, 16.0, 26.0, 26.0), lock: at(w - 76.0, 16.0, 26.0, 26.0), grip: at(w - 30.0, h - 30.0, 22.0, 22.0), header: at(0.0, 0.0, w, 72.0), unit }
    }

    /// The window is where and as big as it is now, so a drag can change it from there.
    fn hold_window(&mut self, f: &Frame) {
        self.origin = Some((f.panel[0] / f.unit, f.panel[1] / f.unit));
        self.scale = f.rect[2] / f.unit / PICTURE.0;
    }

    /// The pointer went down on the window: it takes hold of the corner (to resize), a button of the header, the picture (to move what it shows) or, unless the
    /// window is locked, the header (to move the window).
    fn press(&mut self, pos: (f32, f32), screen: (u32, u32)) {
        let f = self.frame(screen);
        let over = |r: [f32; 4]| levelup::inside(r, pos);
        self.grab = if over(f.grip) {
            self.hold_window(&f);
            Some(Grab::Resize)
        } else if over(f.close) {
            Some(Grab::Press(Part::Close))
        } else if over(f.lock) {
            Some(Grab::Press(Part::Lock))
        } else if over(f.rect) {
            Some(Grab::Pan { last: pos, travelled: 0.0 })
        } else if over(f.header) && !self.locked {
            self.hold_window(&f);
            Some(Grab::Move(pos))
        } else {
            None
        };
    }

    /// The pointer moved: while the button is down the picture, the window or its size follows it.
    fn drag(&mut self, pos: (f32, f32), screen: (u32, u32)) {
        let f = self.frame(screen);
        match self.grab {
            Some(Grab::Pan { last, travelled }) => {
                let (dx, dy) = (pos.0 - last.0, pos.1 - last.1);
                self.grab = Some(Grab::Pan { last: pos, travelled: travelled + dx.abs() + dy.abs() });
                self.centre = (self.centre.0 - dx / f.rect[2] / self.zoom, self.centre.1 - dy / f.rect[3] / self.zoom);
                self.clamp();
            }
            Some(Grab::Move(last)) => {
                let (x, y) = self.origin.unwrap_or((f.panel[0] / f.unit, f.panel[1] / f.unit));
                self.origin = Some((x + (pos.0 - last.0) / f.unit, y + (pos.1 - last.1) / f.unit));
                self.grab = Some(Grab::Move(pos));
                // Kept on the screen as it goes, so the window does not wait for the pointer to come back.
                let g = self.frame(screen);
                self.origin = Some((g.panel[0] / g.unit, g.panel[1] / g.unit));
            }
            Some(Grab::Resize) => {
                // The corner follows the pointer, along whichever way it was pulled further; the top left stays, and the picture keeps its proportions.
                let (x, y) = (f.panel[0] / f.unit, f.panel[1] / f.unit);
                let (across, down) = ((pos.0 / f.unit - x - 32.0) / PICTURE.0, (pos.1 / f.unit - y - 120.0) / PICTURE.1);
                let now = self.scale;
                let want = if (across - now).abs() >= (down - now).abs() { across } else { down };
                let room = ((screen.0 as f32 / f.unit - x - 32.0) / PICTURE.0).min((ui_height() - y - 120.0) / PICTURE.1);
                self.scale = want.clamp(MIN_SCALE, room.max(MIN_SCALE));
            }
            _ => {}
        }
    }

    /// Where a point of the picture (0 to 1 each way) is in `rect`.
    fn to_screen(&self, rect: [f32; 4], (fx, fy): (f32, f32)) -> (f32, f32) {
        let half = 0.5 / self.zoom;
        (rect[0] + (fx - (self.centre.0 - half)) * self.zoom * rect[2], rect[1] + (fy - (self.centre.1 - half)) * self.zoom * rect[3])
    }

    /// The point of the picture under a screen position.
    fn from_screen(&self, rect: [f32; 4], pos: (f32, f32)) -> (f32, f32) {
        let half = 0.5 / self.zoom;
        (self.centre.0 - half + (pos.0 - rect[0]) / rect[2] / self.zoom, self.centre.1 - half + (pos.1 - rect[1]) / rect[3] / self.zoom)
    }

    /// Where a place (north, west) is in `rect`.
    fn place(&self, rect: [f32; 4], north: f32, west: f32) -> (f32, f32) {
        let [left, right, top, bottom] = self.bounds();
        self.to_screen(rect, ((left - west) / (left - right), (top - north) / (top - bottom)))
    }

    /// The zone being shown, if it is one.
    fn zone(&self) -> Option<&Zone> {
        match self.view {
            View::Zone(Some(i)) => self.atlas.as_ref()?.zones.get(i),
            _ => None,
        }
    }

    fn continent(&self) -> Option<&Continent> {
        match self.view {
            View::Continent(i) => self.atlas.as_ref()?.continents.get(i),
            _ => None,
        }
    }

    /// The extent in the world of the map shown.
    fn bounds(&self) -> [f32; 4] {
        self.zone().map(|z| z.bounds).or_else(|| self.continent().map(|c| c.bounds)).unwrap_or([LEFT, RIGHT, TOP, BOTTOM])
    }

    /// The folder of the pictures shown and the title.
    fn names(&self) -> (String, String) {
        match (self.view, self.zone(), self.continent()) {
            (_, Some(z), _) => (z.dir.clone(), z.title.clone()),
            (_, _, Some(c)) => (c.dir.clone(), c.title.clone()),
            (View::World, _, _) => ("World".to_string(), "Azeroth".to_string()),
            _ => ("Elwynn".to_string(), "Elwynn Forest".to_string()),
        }
    }

    /// The way down to what is shown: the world, its continent, its zone.
    fn trail(&self) -> String {
        let continent = match (self.view, self.atlas.as_ref()) {
            (View::Continent(i), Some(a)) => a.continents.get(i),
            (View::Zone(Some(z)), Some(a)) => a.continent_of(z).map(|c| &a.continents[c]),
            _ => None,
        };
        let mut parts = vec!["Azeroth".to_string()];
        parts.extend(continent.map(|c| c.title.clone()));
        parts.extend(self.zone().map(|z| z.title.clone()));
        parts.join("  >  ")
    }

    /// Show another map from its middle, unzoomed.
    fn set_view(&mut self, view: View) {
        (self.view, self.centre, self.zoom, self.grab) = (view, (0.5, 0.5), 1.0, None);
    }

    /// The zone at a point of the map shown (0 to 1 each way), as the client finds it (`FUN_004a6ec0`): the point is taken to the world through the extent of the
    /// map shown, and the continent's grid names the zone in the cell there.
    fn zone_at(&self, (x, y): (f32, f32)) -> Option<usize> {
        let atlas = self.atlas.as_ref()?;
        let continent = match self.view {
            View::Continent(c) => c,
            View::Zone(Some(z)) => atlas.continent_of(z)?,
            _ => return None,
        };
        let [left, right, top, bottom] = self.bounds();
        let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
        let (west, north) = (left - x * (left - right), top - y * (top - bottom));
        // Half way across the grid is the world's middle, and a cell is half a tile.
        let (col, row) = ((0.5 - west / (TILE_YARDS * 64.0)) * GRID as f32, (0.5 - north / (TILE_YARDS * 64.0)) * GRID as f32);
        if !(0.0..GRID as f32).contains(&col) || !(0.0..GRID as f32).contains(&row) {
            return None;
        }
        usize::from(atlas.continents[continent].grid[row as usize * GRID + col as usize]).checked_sub(1)
    }

    /// What the pointer is over that a click would open (`UpdateMapHighlight`, `ProcessMapClick`): on the world map a continent (by its rectangle), on a continent
    /// or a zone a zone other than the one shown.
    fn target_at(&self, (x, y): (f32, f32)) -> Option<View> {
        let atlas = self.atlas.as_ref()?;
        match self.view {
            View::World => atlas.continents.iter().position(|c| x >= c.rect[0] && x < c.rect[2] && y >= c.rect[1] && y < c.rect[3]).map(View::Continent),
            _ => self.zone_at((x, y)).filter(|z| self.view != View::Zone(Some(*z))).map(|z| View::Zone(Some(z))),
        }
    }

    /// The right button (`WorldMapZoomOutButton_OnClick`): a zone goes back to its continent, a continent to the world.
    fn zoom_out(&mut self) {
        let next = match self.view {
            View::World => return,
            View::Continent(_) => View::World,
            // A city goes back to the zone it stands in (the client's own button goes to the continent), a zone to its continent.
            View::Zone(Some(z)) if self.atlas.as_ref().is_some_and(|a| a.zones[z].city) => match self.atlas.as_ref().and_then(|a| a.home_of(z)) {
                Some(home) => View::Zone(Some(home)),
                None => View::World,
            },
            View::Zone(zone) => zone.and_then(|z| self.atlas.as_ref()?.continent_of(z)).map_or(View::World, View::Continent),
        };
        self.set_view(next);
    }

    /// Show the map of the zone the character stands in (its city's own map, else its zone's), or Elwynn's when that has none.
    fn show_here(&mut self) {
        let Some(atlas) = &self.atlas else { return };
        let pick = atlas.zone_of(self.here.unwrap_or(0), None).or_else(|| atlas.zones.iter().position(|z| z.dir == "Elwynn"));
        if View::Zone(pick) != self.view {
            self.set_view(View::Zone(pick));
        }
    }

    /// The picture that lights up under the pointer (`UpdateMapHighlight`), and where it goes on the map shown (0 to 1 each way: left, top, width, height) with the
    /// part of the square picture that is drawn on (the pictures are made in that part): on the world map a continent's highlight over its rectangle, on a
    /// continent a zone's over the zone's extent. On a zone's own map the neighbours are only named.
    fn highlight(&self, target: View) -> Option<(String, [f32; 4], (f32, f32))> {
        let atlas = self.atlas.as_ref()?;
        match (self.view, target) {
            (View::World, View::Continent(i)) => {
                let c = &atlas.continents[i];
                Some((format!(r"Interface\WorldMap\{0}\{0}Highlight", c.dir), [c.rect[0], c.rect[1], c.rect[2] - c.rect[0], c.rect[3] - c.rect[1]], c.share))
            }
            (View::Continent(c), View::Zone(Some(z))) => {
                let ([cl, cr, ct, cb], zone) = (atlas.continents[c].bounds, &atlas.zones[z]);
                let [zl, zr, zt, zb] = zone.bounds;
                let (w, h) = (zl - zr, zt - zb);
                let share = if w <= h { (w / h, 1.0) } else { (1.0, h / w) };
                Some((format!(r"Interface\WorldMap\{0}\{0}Highlight", zone.dir), [(cl - zl) / (cl - cr), (ct - zt) / (ct - cb), w / (cl - cr), h / (ct - cb)], share))
            }
            _ => None,
        }
    }
}

/// Where a continent lies on the world map (0 to 1 each way: left, top, right, bottom), from the tiles of the terrain it covers (`WorldMapContinent.dbc`): the
/// picture is 16 pixels to a tile of the grid, the tile 32 is at its middle, moved by `off`, and `scale` tiles of the picture are one of the terrain's.
fn world_rect(cols: (f32, f32), rows: (f32, f32), off: (f32, f32), scale: f32) -> [f32; 4] {
    let (ox, oy) = (PICTURE.0 / 32.0 - 32.0 * scale + off.0, PICTURE.1 / 32.0 - 32.0 * scale + off.1);
    let (kx, ky) = (16.0 / PICTURE.0, 16.0 / PICTURE.1);
    [(cols.0 * scale + ox) * kx, (rows.0 * scale + oy) * ky, ((cols.1 + 1.0) * scale + ox) * kx, ((rows.1 + 1.0) * scale + oy) * ky]
}

fn atlas(mount: &Mount) -> Atlas {
    let table = |name: &str| Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc")).ok()?).ok();
    let (Some(areas), Some(all), Some(names), Some(maps), Some(tiles)) = (table("WorldMapArea"), table("WorldMapOverlay"), table("AreaTable"), table("Map"), table("WorldMapContinent")) else {
        return Atlas { zones: Vec::new(), continents: Vec::new(), parent: HashMap::new() };
    };
    // AreaTable: 0 id, 2 parent zone, 11 name (enUS).
    let parent = names.iter().map(|r| (r.u32(0), r.u32(2))).collect();
    let zones: Vec<Zone> = areas
        .iter()
        .filter(|r| r.u32(2) != 0 && !r.string(3).is_empty())
        .map(|r| Zone {
            dir: r.string(3).to_string(),
            area: r.u32(2),
            title: names.find_by_id(r.u32(2)).map_or_else(|| r.string(3).to_string(), |n| n.string(11).to_string()),
            map: r.u32(1),
            city: names.find_by_id(r.u32(2)).is_some_and(|n| n.u32(4) & 0x100 != 0),
            bounds: [r.f32(4), r.f32(5), r.f32(6), r.f32(7)],
            overlays: all
                .iter()
                .filter(|o| o.u32(1) == r.u32(0))
                .map(|o| Overlay { name: o.string(8).to_string(), areas: [o.u32(2), o.u32(3), o.u32(4), o.u32(5)], size: (o.u32(9) as f32, o.u32(10) as f32), at: (o.u32(11) as f32, o.u32(12) as f32) })
                .collect(),
        })
        .collect();
    let seas: std::collections::HashSet<u32> = names.iter().filter(|r| r.string(11).ends_with("Sea") || r.string(11).ends_with("Seas")).map(|r| r.u32(0)).collect();
    let mut atlas = Atlas { zones, continents: Vec::new(), parent };
    // The continents are the rows without an area. Each cell of the grid file names an area; the zone of the cell is that area's parent (or the area) if the
    // continent has a map for it, else none.
    for row in areas.iter().filter(|r| r.u32(2) == 0 && !r.string(3).is_empty()) {
        let (map, dir) = (row.u32(1), row.string(3).to_string());
        let Some(tile) = tiles.iter().find(|t| t.u32(1) == map) else { continue };
        let Ok(file) = mount.read(&format!(r"Interface\WorldMap\{dir}.zmp")) else { continue };
        let cells: Vec<u32> = file.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        if cells.len() != GRID * GRID {
            continue;
        }
        let grid = cells.iter().map(|&a| if atlas.parent.contains_key(&a) { atlas.zone_of(a, Some(map)).map_or(0, |z| z as u16 + 1) } else { 0 }).collect();
        let (cols, rows) = ((tile.u32(3) - tile.u32(2) + 1) as f32, (tile.u32(5) - tile.u32(4) + 1) as f32);
        atlas.continents.push(Continent {
            dir,
            title: maps.find_by_id(map).map_or_else(|| row.string(3).to_string(), |m| m.string(4).to_string()),
            map,
            bounds: [row.f32(4), row.f32(5), row.f32(6), row.f32(7)],
            rect: world_rect((tile.u32(2) as f32, tile.u32(3) as f32), (tile.u32(4) as f32, tile.u32(5) as f32), (tile.f32(6), tile.f32(7)), tile.f32(8)),
            share: if cols <= rows { (cols / rows, 1.0) } else { (1.0, rows / cols) },
            water: cells.iter().map(|a| seas.contains(a)).collect(),
            grid,
        });
    }
    atlas
}

/// A padlock in a chip: shut (the shackle down in the body) or open (the shackle up and free on one side).
fn padlock(ui: &mut Ui, art: &levelup::Art, chip: [f32; 4], s: f32, shut: bool, color: [f32; 4]) {
    let (cx, cy) = (chip[0] + chip[2] / 2.0, chip[1] + chip[3] / 2.0);
    let bar = |ui: &mut Ui, x: f32, y: f32, w: f32, h: f32| levelup::slice(ui, art.rounded, [cx + x * s, cy + y * s, w * s, h * s], 0.8 * s, color);
    levelup::slice(ui, art.rounded, [cx - 6.0 * s, cy - 1.0 * s, 12.0 * s, 9.0 * s], 2.0 * s, color);
    let lift = if shut { 0.0 } else { 3.0 };
    bar(ui, -4.0, -6.0 - lift, 8.0, 1.8);
    bar(ui, -4.0, -6.0 - lift, 1.8, 6.0 + lift);
    if shut {
        bar(ui, 2.2, -6.0, 1.8, 6.0);
    }
}

impl Panels {
    pub fn toggle_map(&mut self) {
        let on = !self.map_open;
        if on {
            self.close_all();
            if self.map.atlas.is_none() {
                self.map.atlas = Some(atlas(&self.mount));
            }
            self.map.show_here();
        }
        self.map_open = on;
        self.map.grab = None;
        self.after_close();
    }

    /// A spell tracks the creatures of a type: they show on the map until the same spell is cast again (or another one is).
    pub fn track(&mut self, kind: u32) {
        self.tracking = if self.tracking == Some(kind) { None } else { Some(kind) };
        self.notice(if self.tracking.is_some() { "You can sense them now." } else { "You sense nothing now." });
    }

    /// Where the creatures that are tracked stand now (every frame).
    pub fn set_tracking(&mut self, creatures: Option<&crate::creature::Creatures>) {
        self.tracked = match (self.tracking, creatures) {
            (Some(kind), Some(c)) => c.of_type(kind),
            _ => Vec::new(),
        };
    }

    /// Where the player asked to travel (north, west, feet height if known) since the last call.
    pub fn take_travel(&mut self) -> Option<(f32, f32, Option<f32>)> {
        self.travel.take()
    }

    /// The character stands in `area`: its overlay is drawn on the map from now on. Returns whether the area is new.
    pub fn explore(&mut self, area: Option<u32>) -> bool {
        self.map.here = area;
        match area {
            Some(a) if a != 0 && !self.explored.contains(&a) => {
                self.explored.push(a);
                true
            }
            _ => false,
        }
    }

    /// The discovered place under `pos`, if any (only on the map of a zone).
    fn map_place_at(&self, pos: (f32, f32), screen: (u32, u32)) -> Option<usize> {
        if !matches!(self.map.view, View::Zone(_)) {
            return None;
        }
        let rect = self.map.frame(screen).rect;
        let reach = REACH * screen.1 as f32 / ui_height();
        content::get().pois.iter().enumerate().filter(|(i, _)| self.found[*i]).find_map(|(i, p)| {
            let at = self.map.place(rect, p.at.0, p.at.1);
            let on_map = at.0 >= rect[0] && at.0 <= rect[0] + rect[2] && at.1 >= rect[1] && at.1 <= rect[1] + rect[3];
            (on_map && (at.0 - pos.0).powi(2) + (at.1 - pos.1).powi(2) <= reach * reach).then_some(i)
        })
    }

    /// The wheel zooms towards the pointer: the point of the map under it stays under it.
    pub fn map_zoom(&mut self, pos: (f32, f32), screen: (u32, u32), lines: f32) {
        let rect = self.map.frame(screen).rect;
        let under = self.map.from_screen(rect, pos);
        self.map.zoom = (self.map.zoom * 1.25f32.powf(lines)).clamp(1.0, MAX_ZOOM);
        let (rx, ry) = ((pos.0 - rect[0]) / rect[2], (pos.1 - rect[1]) / rect[3]);
        self.map.centre = (under.0 - (rx - 0.5) / self.map.zoom, under.1 - (ry - 0.5) / self.map.zoom);
        self.map.clamp();
    }

    /// The pointer went down on the window (see `MapState::press`).
    pub(super) fn map_press(&mut self, pos: (f32, f32), screen: (u32, u32)) {
        self.map.press(pos, screen);
    }

    /// The pointer moved: while the button is down the picture, the window or its size follows it.
    pub(super) fn map_drag(&mut self, pos: (f32, f32), screen: (u32, u32)) {
        self.map.drag(pos, screen);
    }

    /// The left button went up: on a button of the header that is a click on it; a press on the picture that did not move travels to a discovered place (and closes
    /// the map), or opens the continent or the zone under it (`ProcessMapClick`).
    pub(super) fn map_release(&mut self, pos: (f32, f32), screen: (u32, u32)) {
        let f = self.map.frame(screen);
        match self.map.grab.take() {
            Some(Grab::Press(Part::Close)) if levelup::inside(f.close, pos) => self.toggle_map(),
            Some(Grab::Press(Part::Lock)) if levelup::inside(f.lock, pos) => {
                self.map.locked = !self.map.locked;
                self.events.push(Sfx::Named("igMainMenuOptionCheckBoxOn"));
            }
            Some(Grab::Pan { travelled, .. }) if travelled <= DRAG => {
                if let Some(i) = self.map_place_at(pos, screen) {
                    let poi = &content::get().pois[i];
                    self.travel = Some((poi.at.0, poi.at.1, poi.height));
                    self.map_open = false;
                    self.after_close();
                } else if let Some(view) = levelup::inside(f.rect, pos).then(|| self.map.from_screen(f.rect, pos)).and_then(|at| self.map.target_at(at)) {
                    self.map.set_view(view);
                }
            }
            _ => {}
        }
    }

    /// The right button went up (`WorldMapButton_OnClick`): one map out, from the zone to its continent, then to the world.
    pub(super) fn map_zoom_out(&mut self) {
        self.map.grab = None;
        self.map.zoom_out();
    }

    /// The map over the game, with the character at `pos` (render space) facing `yaw`.
    pub fn draw_map(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32), pos: Vec3, yaw: f32) {
        if !self.map_open {
            return;
        }
        let Some(art) = self.take_art(ui, gpu) else { return };
        let mount = self.mount.clone();
        let s = screen.1 as f32 / ui_height();
        let f = self.map.frame(screen);
        let (rect, panel, close) = (f.rect, f.panel, f.close);
        let (dir, mut title) = self.map.names();
        let target = self.cursor.filter(|p| levelup::inside(rect, *p)).and_then(|p| self.map.target_at(self.map.from_screen(rect, p)));
        if let (Some(view), Some(atlas)) = (target, &self.map.atlas) {
            title = match view {
                View::Continent(i) => atlas.continents[i].title.to_string(),
                View::Zone(Some(z)) => atlas.zones[z].title.clone(),
                _ => title,
            };
        }
        // The window: the world dims behind it, it floats on a soft shadow, with the title and the way down to the map in its header.
        levelup::quad(ui, crate::ui::WHITE, [0.0, 0.0, screen.0 as f32, screen.1 as f32], [0.0, 0.0, 0.0, 0.38], false);
        let spread = 40.0 * s;
        levelup::quad(ui, art.glow, [panel[0] - spread, panel[1] - spread + 10.0 * s, panel[2] + 2.0 * spread, panel[3] + 2.0 * spread], [0.0, 0.0, 0.0, 0.55], false);
        levelup::slice(ui, art.rounded, panel, 18.0 * s, [0.07, 0.07, 0.08, 0.95]);
        levelup::slice(ui, art.ring, panel, 18.0 * s, [1.0, 1.0, 1.0, 0.11]);
        art.title.draw(ui, &title, panel[0] + 24.0 * s, panel[1] + 14.0 * s, s, [1.0; 4]);
        art.body.draw(ui, &self.map.trail(), panel[0] + 24.0 * s, panel[1] + 46.0 * s, s, levelup::GREY);
        let over_close = self.cursor.is_some_and(|c| levelup::inside(close, c));
        levelup::quad(ui, art.disc, close, [1.0, 1.0, 1.0, if over_close { 0.16 } else { 0.07 }], false);
        levelup::plus(ui, &art, close, 10.0 * s, if over_close { [1.0; 4] } else { levelup::GREY }, std::f32::consts::FRAC_PI_4);
        // The lock: gold and shut while the window is held in place, grey and open while it can be moved.
        let over_lock = self.cursor.is_some_and(|c| levelup::inside(f.lock, c));
        levelup::quad(ui, art.disc, f.lock, [1.0, 1.0, 1.0, if over_lock { 0.16 } else { 0.07 }], false);
        padlock(ui, &art, f.lock, s, self.map.locked, if self.map.locked { levelup::ACCENT } else if over_lock { [1.0; 4] } else { levelup::GREY });
        // The corner to resize by: three rows of dots.
        let over_grip = self.cursor.is_some_and(|c| levelup::inside(f.grip, c)) || self.map.grab == Some(Grab::Resize);
        for (i, j) in [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (0, 2)] {
            let d = 3.0 * s;
            let (x, y) = (f.grip[0] + f.grip[2] - 5.0 * s - (i as f32 * 5.0 + 1.5) * s, f.grip[1] + f.grip[3] - 5.0 * s - (j as f32 * 5.0 + 1.5) * s);
            levelup::quad(ui, art.disc, [x - d / 2.0, y - d / 2.0, d, d], [1.0, 1.0, 1.0, if over_grip { 0.7 } else { 0.32 }], false);
        }

        // A texel rectangle of the picture on the screen (the picture is cut to the window while it is zoomed).
        let view = |map: &MapState, (x, y, w, h): (f32, f32, f32, f32)| {
            let (x0, y0) = map.to_screen(rect, (x / PICTURE.0, y / PICTURE.1));
            [x0, y0, w / PICTURE.0 * map.zoom * rect[2], h / PICTURE.1 * map.zoom * rect[3]]
        };
        ui.set_clip(Some(rect));
        levelup::quad(ui, crate::ui::WHITE, rect, [0.0, 0.0, 0.0, 1.0], false);
        // The 4 x 3 tiles; the last column and row are cut where the picture ends.
        for i in 0..12 {
            let (col, row) = ((i % 4) as f32, (i / 4) as f32);
            let (w, h) = ((PICTURE.0 - col * TILE).min(TILE), (PICTURE.1 - row * TILE).min(TILE));
            if let Some(texture) = self.tree.texture(ui, gpu, &mount, &format!(r"Interface\WorldMap\{dir}\{dir}{}", i + 1)) {
                ui.quad(Quad { texture, rect: view(&self.map, (col * TILE, row * TILE, w, h)), uv: [0.0, 0.0, w / TILE, h / TILE], color: WHITE, additive: false });
            }
        }
        // The explored areas over it, each cut into tiles of up to 256 texels whose files are the next power of two in size.
        let shown: Vec<(String, (f32, f32), (f32, f32))> = self.map.zone().iter().flat_map(|z| &z.overlays).filter(|o| o.areas.iter().any(|a| *a != 0 && self.explored.contains(a))).map(|o| (o.name.clone(), o.size, o.at)).collect();
        for (name, size, at) in shown {
            let (cols, rows) = ((size.0 / TILE).ceil() as usize, (size.1 / TILE).ceil() as usize);
            for row in 0..rows {
                for col in 0..cols {
                    let (w, h) = ((size.0 - col as f32 * TILE).min(TILE), (size.1 - row as f32 * TILE).min(TILE));
                    let (file_w, file_h) = ((w as u32).next_power_of_two() as f32, (h as u32).next_power_of_two() as f32);
                    if let Some(texture) = self.tree.texture(ui, gpu, &mount, &format!(r"Interface\WorldMap\{dir}\{name}{}", row * cols + col + 1)) {
                        ui.quad(Quad { texture, rect: view(&self.map, (at.0 + col as f32 * TILE, at.1 + row as f32 * TILE, w, h)), uv: [0.0, 0.0, w / file_w, h / file_h], color: WHITE, additive: false });
                    }
                }
            }
        }
        // The continent or the zone under the pointer lights up (the original's highlight pictures, added to the map): the map it would open.
        if let Some((path, at, share)) = target.and_then(|t| self.map.highlight(t)) {
            if let Some(texture) = self.tree.texture(ui, gpu, &mount, &path) {
                ui.quad(Quad { texture, rect: view(&self.map, (at[0] * PICTURE.0, at[1] * PICTURE.1, at[2] * PICTURE.0, at[3] * PICTURE.1)), uv: [0.0, 0.0, share.0, share.1], color: WHITE, additive: true });
            }
        }
        let hover = self.cursor.and_then(|p| self.map_place_at(p, screen));
        let inside = |p: (f32, f32), margin: f32| p.0 >= rect[0] + margin && p.0 <= rect[0] + rect[2] - margin && p.1 >= rect[1] + margin && p.1 <= rect[1] + rect[3] - margin;
        let on_zone = matches!(self.map.view, View::Zone(_));
        let mut label = None;
        for (i, poi) in content::get().pois.iter().enumerate().filter(|(i, _)| on_zone && self.found[*i]) {
            let at = self.map.place(rect, poi.at.0, poi.at.1);
            if !inside(at, 6.0 * s) {
                continue;
            }
            let size = if hover == Some(i) { 26.0 } else { 20.0 } * s;
            if let Some(texture) = self.tree.texture(ui, gpu, &mount, crate::compass::POI_ICONS) {
                ui.quad(Quad { texture, rect: [at.0 - size / 2.0, at.1 - size / 2.0, size, size], uv: crate::compass::poi_uv(poi.icon), color: WHITE, additive: false });
            }
            // The places lie close together at this scale, so only the one under the pointer is named.
            if hover == Some(i) {
                label = Some((poi.name.clone(), at));
            }
        }
        // Where the tracked quest leads, each place with the picture of what is there, the nearest one bigger.
        let [north, west, _] = render_to_wow(pos);
        let targets: Vec<([f32; 3], crate::quests::Icon)> = if on_zone { self.quest_targets.iter().map(|&(p, icon)| (render_to_wow(p), icon)).collect() } else { Vec::new() };
        let distance = |t: &[f32; 3]| (t[0] - north).hypot(t[1] - west);
        let nearest = targets.iter().map(|t| distance(&t.0)).min_by(f32::total_cmp);
        for (t, icon) in targets {
            use crate::quests::Icon;
            let (path, uv) = match icon {
                Icon::Skull => (r"Interface\TargetingFrame\UI-RaidTargetingIcons", [0.75, 0.25, 1.0, 0.5]),
                Icon::Chest => (r"Interface\Icons\INV_Box_01", [0.07, 0.07, 0.93, 0.93]),
                Icon::Vendor => (r"Interface\GossipFrame\VendorGossipIcon", FULL_UV),
                Icon::TurnIn => (r"Interface\GossipFrame\ActiveQuestIcon", FULL_UV),
            };
            let at = self.map.place(rect, t[0], t[1]);
            if !inside(at, 4.0 * s) {
                continue;
            }
            if let Some(texture) = self.tree.texture(ui, gpu, &mount, path) {
                let size = if nearest == Some(distance(&t)) { 24.0 } else { 15.0 } * s;
                ui.quad(Quad { texture, rect: [at.0 - size / 2.0, at.1 - size / 2.0, size, size], uv, color: WHITE, additive: false });
            }
        }
        // What is tracked: a small round mark on each.
        if on_zone {
            if let Some(texture) = self.tree.texture(ui, gpu, &mount, r"Interface\TargetingFrame\UI-RaidTargetingIcons") {
                for &p in &self.tracked {
                    let [n, w, _] = render_to_wow(p);
                    let at = self.map.place(rect, n, w);
                    if inside(at, 4.0 * s) {
                        let size = 11.0 * s;
                        ui.quad(Quad { texture, rect: [at.0 - size / 2.0, at.1 - size / 2.0, size, size], uv: [0.25, 0.0, 0.5, 0.25], color: [1.0, 0.3, 0.3, 1.0], additive: false });
                    }
                }
            }
        }
        // The character: the arrow of the original (`Interface\Minimap\MinimapArrow.m2` is this picture on one square), turned to face where it does; the
        // world map has none.
        let here = (self.map.view != View::World).then(|| self.map.place(rect, north, west));
        if let Some(here) = here.filter(|h| inside(*h, 0.0)) {
            if let Some(texture) = self.tree.texture(ui, gpu, &mount, r"Interface\Minimap\MinimapArrow") {
                let size = 44.0 * s;
                ui.set_turn(Some((yaw, here.0, here.1)));
                ui.quad(Quad { texture, rect: [here.0 - size / 2.0, here.1 - size / 2.0, size, size], uv: FULL_UV, color: WHITE, additive: false });
                ui.set_turn(None);
            }
        }
        ui.set_clip(None);
        levelup::slice(ui, art.ring, [rect[0] - 2.0 * s, rect[1] - 2.0 * s, rect[2] + 4.0 * s, rect[3] + 4.0 * s], 4.0 * s, [1.0, 1.0, 1.0, 0.16]);
        if let Some((name, at)) = label {
            let width = art.body.width(&name) * s + 18.0 * s;
            let pill = [(at.0 - width / 2.0).clamp(rect[0], rect[0] + rect[2] - width), at.1 + 14.0 * s, width, 24.0 * s];
            levelup::slice(ui, art.rounded, pill, 12.0 * s, [0.06, 0.06, 0.07, 0.92]);
            levelup::slice(ui, art.ring, pill, 12.0 * s, levelup::tint(levelup::ACCENT, 0.5));
            art.body.draw(ui, &name, pill[0] + 9.0 * s, pill[1] + 4.0 * s, s, [1.0; 4]);
        }
        let hints = if on_zone { "Click a discovered place to travel there   Right click: zoom out   Wheel: zoom   Drag: pan   M or Esc: close" } else { "Click a continent or a zone to open it   Right click: zoom out   Wheel: zoom   Drag: pan   M or Esc: close" };
        let foot = panel[1] + panel[3] - 30.0 * s;
        art.small.draw(ui, hints, panel[0] + 24.0 * s, foot, s, levelup::GREY);
        if on_zone {
            let found = format!("Discovered {} of {}", self.found.iter().filter(|&&f| f).count(), self.found.len());
            let (width, hints_width) = (art.small.width(&found) * s, art.small.width(hints) * s);
            let x = panel[0] + panel[2] - 40.0 * s - width;
            // Only while it does not run into the hints (a small window).
            if x > panel[0] + 24.0 * s + hints_width + 16.0 * s {
                art.small.draw(ui, &found, x, foot, s, levelup::ACCENT);
            }
        }
        // What the lock does, when the pointer rests on it.
        if over_lock {
            let text = if self.map.locked { "Unlock the window to move it" } else { "Lock the window in place" };
            let width = art.body.width(text) * s + 18.0 * s;
            let tip = [(f.lock[0] + f.lock[2] / 2.0 - width / 2.0).clamp(panel[0], panel[0] + panel[2] - width), f.lock[1] + f.lock[3] + 6.0 * s, width, 24.0 * s];
            levelup::slice(ui, art.rounded, tip, 12.0 * s, [0.06, 0.06, 0.07, 0.96]);
            levelup::slice(ui, art.ring, tip, 12.0 * s, [1.0, 1.0, 1.0, 0.14]);
            art.body.draw(ui, text, tip[0] + 9.0 * s, tip[1] + 4.0 * s, s, [1.0; 4]);
        }
        self.level_art = Some(art);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_corners_are_the_areas_corners_and_places_of_the_zone_are_on_it() {
        let rect = [100.0, 50.0, 800.0, 600.0];
        let map = MapState::default();
        assert_eq!(map.place(rect, TOP, LEFT), (100.0, 50.0));
        let end = map.place(rect, BOTTOM, RIGHT);
        assert!((end.0 - 900.0).abs() < 0.01 && (end.1 - 650.0).abs() < 0.01);
        // Places of other zones are off this map and are skipped when it is drawn; the Elwynn ones are on it.
        let on = |p: &&_| { let p: &crate::content::Poi = p; let at = map.place(rect, p.at.0, p.at.1); (100.0..=900.0).contains(&at.0) && (50.0..=650.0).contains(&at.1) };
        assert!(content::get().pois.iter().filter(on).count() > 0);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer_and_stays_inside_the_picture() {
        let rect = [100.0, 50.0, 800.0, 600.0];
        let mut map = MapState::default();
        let pointer = (300.0, 200.0);
        let under = map.from_screen(rect, pointer);
        map.zoom = 4.0;
        let (rx, ry) = ((pointer.0 - rect[0]) / rect[2], (pointer.1 - rect[1]) / rect[3]);
        map.centre = (under.0 - (rx - 0.5) / map.zoom, under.1 - (ry - 0.5) / map.zoom);
        map.clamp();
        let after = map.from_screen(rect, pointer);
        assert!((after.0 - under.0).abs() < 1e-5 && (after.1 - under.1).abs() < 1e-5);
        map.centre = (-3.0, 9.0);
        map.clamp();
        assert_eq!(map.centre, (0.125, 0.875));
    }

    #[test]
    fn clicks_open_what_the_zone_grid_names_and_the_right_button_goes_one_map_out() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let mut map = MapState { atlas: Some(atlas(&mount)), ..Default::default() };
        let atlas = map.atlas.as_ref().unwrap();
        let find = |title: &str| atlas.zones.iter().position(|z| z.title == title).unwrap_or_else(|| panic!("no map of {title}"));
        let (kalimdor, kingdoms) = (atlas.continents.iter().position(|c| c.map == 1).unwrap(), atlas.continents.iter().position(|c| c.map == 0).unwrap());
        let (elwynn, stormwind, westfall) = (find("Elwynn Forest"), find("Stormwind City"), find("Westfall"));
        let cities = [("Stormwind City", "Elwynn Forest"), ("Ironforge", "Dun Morogh"), ("Orgrimmar", "Durotar"), ("Undercity", "Tirisfal Glades"), ("Thunder Bluff", "Mulgore"), ("Darnassus", "Teldrassil")].map(|(city, home)| (city, find(city), find(home)));
        // On the continent's map a place (north, west) is where the continent's extent puts it, and the grid's cell there names the zone or the city.
        map.set_view(View::Continent(kingdoms));
        let [left, right, top, bottom] = map.bounds();
        let at = |north: f32, west: f32| ((left - west) / (left - right), (top - north) / (top - bottom));
        assert_eq!(map.target_at(at(-9097.0, -200.0)), Some(View::Zone(Some(elwynn))));
        assert_eq!(map.target_at(at(-8727.0, 709.0)), Some(View::Zone(Some(stormwind))));
        assert_eq!(map.target_at(at(-10600.0, 1100.0)), Some(View::Zone(Some(westfall))));
        // The zone's own map names its neighbours, and not itself.
        map.set_view(View::Zone(Some(elwynn)));
        let [left, right, top, bottom] = map.bounds();
        assert_eq!(map.target_at(((left - 1000.0) / (left - right), (top + 10500.0) / (top - bottom))), Some(View::Zone(Some(westfall))));
        assert_eq!(map.target_at(((left + 200.0) / (left - right), (top + 9097.0) / (top - bottom))), None);
        // A city goes out to the zone it stands in.
        for (city, from, home) in cities {
            map.set_view(View::Zone(Some(from)));
            map.zoom_out();
            assert_eq!(map.view, View::Zone(Some(home)), "out of {city}");
        }
        map.set_view(View::Zone(Some(elwynn)));
        // Right button: a zone to its continent, the continent to the world, and there it stays.
        map.zoom_out();
        assert_eq!(map.view, View::Continent(kingdoms));
        map.zoom_out();
        assert_eq!(map.view, View::World);
        map.zoom_out();
        assert_eq!(map.view, View::World);
        // On the world map each continent is a box of its own, and the sea between is nothing.
        assert_eq!(map.target_at((0.25, 0.5)), Some(View::Continent(kalimdor)));
        assert_eq!(map.target_at((0.8, 0.5)), Some(View::Continent(kingdoms)));
        assert_eq!(map.target_at((0.5, 0.5)), None);
    }

    #[test]
    fn the_window_moves_by_its_header_unless_locked_and_resizes_by_its_corner_with_its_proportions() {
        let screen = (1280, 720);
        let mut map = MapState::default();
        let f = map.frame(screen);
        // The header only moves a window that is unlocked; a press on the picture pans it.
        let header = (f.header[0] + 100.0, f.header[1] + 20.0);
        map.press(header, screen);
        assert_eq!(map.grab, None);
        map.press((f.rect[0] + 50.0, f.rect[1] + 50.0), screen);
        assert!(matches!(map.grab, Some(Grab::Pan { .. })));
        map.grab = None;
        map.locked = false;
        map.press(header, screen);
        map.drag((header.0 + 40.0, header.1 + 30.0), screen);
        let moved = map.frame(screen);
        assert!((moved.panel[0] - f.panel[0] - 40.0).abs() < 0.01 && (moved.panel[1] - f.panel[1] - 30.0).abs() < 0.01);
        // However far it is pulled it stays on the screen.
        map.drag((header.0 + 5000.0, header.1 + 5000.0), screen);
        let edge = map.frame(screen);
        assert!(edge.panel[0] + edge.panel[2] <= screen.0 as f32 + 0.01 && edge.panel[1] + edge.panel[3] <= screen.1 as f32 + 0.01);
        map.grab = None;
        // The corner: the top left stays, the picture keeps its proportions and its size has bounds.
        map.origin = None;
        let start = map.frame(screen);
        let corner = (start.grip[0] + 10.0, start.grip[1] + 10.0);
        map.press(corner, screen);
        assert_eq!(map.grab, Some(Grab::Resize));
        map.drag((corner.0 - 100.0, corner.1 - 75.0), screen);
        let small = map.frame(screen);
        assert!(small.rect[2] < start.rect[2] && (small.panel[0] - start.panel[0]).abs() < 0.01 && (small.panel[1] - start.panel[1]).abs() < 0.01);
        assert!((small.rect[2] / small.rect[3] - PICTURE.0 / PICTURE.1).abs() < 0.001);
        map.drag((0.0, 0.0), screen);
        assert!((map.frame(screen).rect[2] / start.unit / PICTURE.0 - MIN_SCALE).abs() < 0.001);
        map.drag((5000.0, 5000.0), screen);
        let big = map.frame(screen);
        assert!(big.panel[0] + big.panel[2] <= screen.0 as f32 + 0.01 && big.panel[1] + big.panel[3] <= screen.1 as f32 + 0.01);
    }
}
