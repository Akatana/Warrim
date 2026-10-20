//! The world map (key M): the original's Elwynn Forest map with the areas the character has explored drawn in (the overlays of
//! `WorldMapOverlay.dbc`), the places it has discovered marked on it and its own spot. The wheel zooms, dragging moves the map and a
//! click on a discovered place travels there.

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
const SHOWN: f32 = 0.85;
/// How close (interface units) the pointer must be to a place to pick it.
const REACH: f32 = 12.0;
const MAX_ZOOM: f32 = 8.0;
/// Pixels the pointer may move between pressing and releasing for it to be a click and not a drag.
const DRAG: f32 = 5.0;
const GOLD: [f32; 4] = [1.0, 0.82, 0.0, 1.0];

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
    /// West at the left and right edges, north at the top and bottom (as the consts above).
    bounds: [f32; 4],
    overlays: Vec<Overlay>,
}

/// Every map and the parent of each `AreaTable` id, read when the map is first opened.
struct Atlas {
    zones: Vec<Zone>,
    parent: HashMap<u32, u32>,
}

/// What the open map keeps: the middle of the part shown (0 to 1 over the picture) and the zoom, a drag in progress (the pointer when
/// last seen and how far it has gone in all), the area the character stands in, the map shown of the atlas and the atlas itself.
pub struct MapState {
    centre: (f32, f32),
    zoom: f32,
    drag: Option<((f32, f32), f32)>,
    here: Option<u32>,
    zone: Option<usize>,
    atlas: Option<Atlas>,
}

impl Default for MapState {
    fn default() -> Self {
        MapState { centre: (0.5, 0.5), zoom: 1.0, drag: None, here: None, zone: None, atlas: None }
    }
}

impl MapState {
    /// Keep the shown part inside the picture.
    fn clamp(&mut self) {
        self.zoom = self.zoom.clamp(1.0, MAX_ZOOM);
        let half = 0.5 / self.zoom;
        self.centre = (self.centre.0.clamp(half, 1.0 - half), self.centre.1.clamp(half, 1.0 - half));
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

    fn zone(&self) -> Option<&Zone> {
        self.atlas.as_ref()?.zones.get(self.zone?)
    }

    fn bounds(&self) -> [f32; 4] {
        self.zone().map_or([LEFT, RIGHT, TOP, BOTTOM], |z| z.bounds)
    }

    /// Show the map of the zone the character stands in (its city's own map, else its zone's), or Elwynn's when that has none.
    fn show_here(&mut self) {
        let Some(atlas) = &self.atlas else { return };
        let area = self.here.unwrap_or(0);
        let zone = atlas.parent.get(&area).copied().filter(|p| *p != 0).unwrap_or(area);
        let pick = [area, zone].iter().find_map(|a| atlas.zones.iter().position(|z| z.area == *a)).or_else(|| atlas.zones.iter().position(|z| z.dir == "Elwynn"));
        if pick != self.zone {
            (self.zone, self.centre, self.zoom) = (pick, (0.5, 0.5), 1.0);
        }
    }
}

fn atlas(mount: &Mount) -> Atlas {
    let table = |name: &str| Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc")).ok()?).ok();
    let (Some(areas), Some(all), Some(names)) = (table("WorldMapArea"), table("WorldMapOverlay"), table("AreaTable")) else { return Atlas { zones: Vec::new(), parent: HashMap::new() } };
    // AreaTable: 0 id, 2 parent zone, 11 name (enUS).
    let parent = names.iter().map(|r| (r.u32(0), r.u32(2))).collect();
    let zones = areas
        .iter()
        .filter(|r| r.u32(2) != 0 && !r.string(3).is_empty())
        .map(|r| Zone {
            dir: r.string(3).to_string(),
            area: r.u32(2),
            title: names.find_by_id(r.u32(2)).map_or_else(|| r.string(3).to_string(), |n| n.string(11).to_string()),
            bounds: [r.f32(4), r.f32(5), r.f32(6), r.f32(7)],
            overlays: all
                .iter()
                .filter(|o| o.u32(1) == r.u32(0))
                .map(|o| Overlay { name: o.string(8).to_string(), areas: [o.u32(2), o.u32(3), o.u32(4), o.u32(5)], size: (o.u32(9) as f32, o.u32(10) as f32), at: (o.u32(11) as f32, o.u32(12) as f32) })
                .collect(),
        })
        .collect();
    Atlas { zones, parent }
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
        self.map.drag = None;
        self.after_close();
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

    /// The picture on the screen: x, y, width, height in pixels.
    fn map_rect(screen: (u32, u32)) -> [f32; 4] {
        let s = screen.1 as f32 / ui_height() * SHOWN;
        let (w, h) = (PICTURE.0 * s, PICTURE.1 * s);
        [(screen.0 as f32 - w) / 2.0, (screen.1 as f32 - h) / 2.0, w, h]
    }

    /// The discovered place under `pos`, if any.
    fn map_place_at(&self, pos: (f32, f32), screen: (u32, u32)) -> Option<usize> {
        let rect = Self::map_rect(screen);
        let reach = REACH * screen.1 as f32 / ui_height();
        content::get().pois.iter().enumerate().filter(|(i, _)| self.found[*i]).find_map(|(i, p)| {
            let at = self.map.place(rect, p.at.0, p.at.1);
            let on_map = at.0 >= rect[0] && at.0 <= rect[0] + rect[2] && at.1 >= rect[1] && at.1 <= rect[1] + rect[3];
            (on_map && (at.0 - pos.0).powi(2) + (at.1 - pos.1).powi(2) <= reach * reach).then_some(i)
        })
    }

    /// The wheel zooms towards the pointer: the point of the map under it stays under it.
    pub fn map_zoom(&mut self, pos: (f32, f32), screen: (u32, u32), lines: f32) {
        let rect = Self::map_rect(screen);
        let under = self.map.from_screen(rect, pos);
        self.map.zoom = (self.map.zoom * 1.25f32.powf(lines)).clamp(1.0, MAX_ZOOM);
        let (rx, ry) = ((pos.0 - rect[0]) / rect[2], (pos.1 - rect[1]) / rect[3]);
        self.map.centre = (under.0 - (rx - 0.5) / self.map.zoom, under.1 - (ry - 0.5) / self.map.zoom);
        self.map.clamp();
    }

    /// The pointer went down on the map: a drag may begin.
    pub(super) fn map_press(&mut self, pos: (f32, f32)) {
        self.map.drag = Some((pos, 0.0));
    }

    /// The pointer moved: while the button is down the map follows it.
    pub(super) fn map_drag(&mut self, pos: (f32, f32), screen: (u32, u32)) {
        let rect = Self::map_rect(screen);
        if let Some((last, travelled)) = &mut self.map.drag {
            let (dx, dy) = (pos.0 - last.0, pos.1 - last.1);
            *travelled += dx.abs() + dy.abs();
            *last = pos;
            self.map.centre = (self.map.centre.0 - dx / rect[2] / self.map.zoom, self.map.centre.1 - dy / rect[3] / self.map.zoom);
            self.map.clamp();
        }
    }

    /// The button went up: a press that did not move on a discovered place travels there and the map closes.
    pub(super) fn map_release(&mut self, pos: (f32, f32), screen: (u32, u32)) {
        let moved = self.map.drag.take().map_or(f32::MAX, |(_, travelled)| travelled);
        if moved > DRAG {
            return;
        }
        if let Some(i) = self.map_place_at(pos, screen) {
            let poi = &content::get().pois[i];
            self.travel = Some((poi.at.0, poi.at.1, poi.height));
            self.map_open = false;
            self.after_close();
        }
    }

    /// The map over the game, with the character at `pos` (render space) facing `yaw`.
    pub fn draw_map(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32), pos: Vec3, yaw: f32) {
        if !self.map_open {
            return;
        }
        let mount = self.mount.clone();
        let s = screen.1 as f32 / ui_height();
        let rect = Self::map_rect(screen);
        let (dir, title) = self.map.zone().map_or(("Elwynn".to_string(), "Elwynn Forest".to_string()), |z| (z.dir.clone(), z.title.clone()));
        let quad = |ui: &mut Ui, rect: [f32; 4], color: [f32; 4]| ui.quad(Quad { texture: crate::ui::WHITE, rect, uv: FULL_UV, color, additive: false });
        quad(ui, [rect[0] - 6.0 * s, rect[1] - 30.0 * s, rect[2] + 12.0 * s, rect[3] + 58.0 * s], [0.0, 0.0, 0.0, 0.85]);
        // A texel rectangle of the picture on the screen (the picture is cut to the window while it is zoomed).
        let view = |map: &MapState, (x, y, w, h): (f32, f32, f32, f32)| {
            let (x0, y0) = map.to_screen(rect, (x / PICTURE.0, y / PICTURE.1));
            [x0, y0, w / PICTURE.0 * map.zoom * rect[2], h / PICTURE.1 * map.zoom * rect[3]]
        };
        ui.set_clip(Some(rect));
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
        let hover = self.cursor.and_then(|p| self.map_place_at(p, screen));
        let inside = |p: (f32, f32), margin: f32| p.0 >= rect[0] + margin && p.0 <= rect[0] + rect[2] - margin && p.1 >= rect[1] + margin && p.1 <= rect[1] + rect[3] - margin;
        for (i, poi) in content::get().pois.iter().enumerate().filter(|(i, _)| self.found[*i]) {
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
                self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontNormal", &poi.name, (at.0, at.1 + 10.0 * s), 0.5, WHITE);
            }
        }
        // Where the tracked quest leads, each place with the picture of what is there, the nearest one bigger.
        let [north, west, _] = render_to_wow(pos);
        let targets: Vec<([f32; 3], crate::quests::Icon)> = self.quest_targets.iter().map(|&(p, icon)| (render_to_wow(p), icon)).collect();
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
        // The character: the arrow of the original (`Interface\Minimap\MinimapArrow.m2` is this picture on one square), turned to face where it does.
        let here = self.map.place(rect, north, west);
        if inside(here, 0.0) {
            if let Some(texture) = self.tree.texture(ui, gpu, &mount, r"Interface\Minimap\MinimapArrow") {
                let size = 44.0 * s;
                ui.set_turn(Some((yaw, here.0, here.1)));
                ui.quad(Quad { texture, rect: [here.0 - size / 2.0, here.1 - size / 2.0, size, size], uv: FULL_UV, color: WHITE, additive: false });
                ui.set_turn(None);
            }
        }
        ui.set_clip(None);
        let found = self.found.iter().filter(|&&f| f).count();
        self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontNormalLarge", &title, (screen.0 as f32 / 2.0, rect[1] - 26.0 * s), 0.5, WHITE);
        let footer = format!("Discovered {found} of {}. Wheel zooms, drag moves the map, click a discovered place to travel there. M or Esc closes it.", self.found.len());
        self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontHighlightSmall", &footer, (screen.0 as f32 / 2.0, rect[1] + rect[3] + 8.0 * s), 0.5, WHITE);
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
}
