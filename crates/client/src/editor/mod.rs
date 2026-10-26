//! The in-game editor (F1): the lists of the content that people change — where creatures and chests stand and the places to discover,
//! the kinds of creatures (the NPCs and the enemies), the quests, the items and the loot tables — as forms over the game. What is changed is written, when it is saved and
//! the content with it would be valid, to `content/editor.ron` (an overlay on the content files, which keep their comments), and
//! Apply saves and takes the change into use while the game runs (the character is kept; the creatures and chests are made again).
//!
//! The widgets are the plainest immediate-mode ones, drawn with the interface's quads and the game's own font: a list, a text field
//! (click it, type, Enter keeps what was typed, Esc throws it away), buttons and arrows that step through choices.

use crate::character::{Race, Sex};
use crate::content::{self, Action as CreatureAction, GossipOption, Overlay, Prop, Raws, RawBody, RawCreature, RawGoal, RawItem, RawQuest, Spawn, Step, Waypoint};
use crate::coords::render_to_wow;
use crate::font::GlyphSet;
use crate::gallery::{Gallery, Pic, Subject};
use crate::propedit::{self, Drag, Eye, Gizmo, Handle, Mode, Models};
use crate::items::{UseEffect, Wear};
use crate::lockpick::Level;
use crate::preview::{self, Preview};
use crate::renderer::{Gpu, Renderer};
use crate::ui::{Quad, Ui, FULL_UV};
use anyhow::{Context, Result};
use glam::{Mat4, Vec2, Vec3};
use std::collections::HashSet;
use std::fmt::Display;
use std::sync::Arc;
use std::str::FromStr;

mod creature_tab;
mod gui;
mod item_tab;
mod lessons;
mod pickers;
mod placer;
mod places;
mod quest_tab;

use creature_tab::*;
use gui::*;
use item_tab::*;
use pickers::*;
use placer::*;
use places::*;
use quest_tab::*;

/// Width of the editor in interface units (the interface is 768 units high).
const WIDTH: f32 = 720.0;
const ROW: f32 = 18.0;
const LIST_WIDTH: f32 = 220.0;
/// The form beside a list: where it starts and how wide it is.
const PANE_X: f32 = LIST_WIDTH + 24.0;
const PANE_W: f32 = WIDTH - LIST_WIDTH - 32.0;
const UI_HEIGHT: f32 = 768.0;
/// Wide enough for the longest label of a form, so that none is cut short.
const LABEL_WIDTH: f32 = 128.0;

const PANEL: [f32; 4] = [0.04, 0.04, 0.07, 0.93];
const BOX: [f32; 4] = [0.12, 0.12, 0.17, 1.0];
const HOT: [f32; 4] = [0.22, 0.24, 0.34, 1.0];
/// A section's heading bar.
const PANEL_LINE: [f32; 4] = [0.09, 0.09, 0.14, 1.0];
const SELECTED: [f32; 4] = [0.25, 0.3, 0.5, 1.0];
const TEXT: [f32; 4] = [0.9, 0.9, 0.9, 1.0];
const DIM: [f32; 4] = [0.55, 0.55, 0.6, 1.0];
const GOLD: [f32; 4] = [1.0, 0.82, 0.0, 1.0];
const RED: [f32; 4] = [1.0, 0.35, 0.3, 1.0];
const GREEN: [f32; 4] = [0.4, 1.0, 0.4, 1.0];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    World,
    Creatures,
    Quests,
    Items,
    Loot,
}

const TABS: [(Tab, &str); 5] = [(Tab::World, "Places"), (Tab::Creatures, "Creatures"), (Tab::Quests, "Quests"), (Tab::Items, "Items"), (Tab::Loot, "Loot")];

/// What the editor asks of the game.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Stand at this place (north, west).
    Teleport(f32, f32),
    /// The changes are saved: take them into use.
    Apply,
    /// The props changed: show these in the world now (not saved by this).
    Props(Vec<Prop>),
}

/// Where the player stands: north, west, the ground under the feet and heading in degrees.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub north: f32,
    pub west: f32,
    pub height: f32,
    pub yaw: f32,
    /// The map being played ("Azeroth", a dungeon).
    pub map: &'static str,
}

/// A map name that lives as long as the program (there are few of them), for a `Place`.
pub fn interned(name: &str) -> &'static str {
    static NAMES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    match names.iter().find(|n| **n == name) {
        Some(n) => n,
        None => {
            let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
            names.push(leaked);
            leaked
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Backspace,
    Enter,
    Escape,
}

/// What the player did since the last frame.
#[derive(Default)]
struct Input {
    /// Pixels.
    mouse: (f32, f32),
    click: bool,
    wheel: f32,
    typed: String,
    keys: Vec<Key>,
}

/// The text field being edited.
#[derive(Default)]
struct Edit {
    focus: Option<u64>,
    buffer: String,
    /// A field that was clicked this frame, to take the focus at its end.
    want: Option<(u64, String)>,
    /// The field that has the focus was drawn this frame (else it is gone, and the keys must not go to it any more).
    seen: bool,
    /// The button that asked "sure?" (a delete, a revert) and when: a second click soon after does it, anything else does not.
    armed: Option<(u64, std::time::Instant)>,
}

/// What lists and forms keep between frames.
struct Pick {
    selected: usize,
    scroll: usize,
    form_scroll: f32,
    /// What the list is narrowed by, and a number that tells its field from those of the other lists.
    find: String,
    salt: usize,
    /// The list is to scroll to the selected line when it is next drawn (a click in the world chose it).
    reveal: bool,
    /// The lesson selected in the form of a teacher.
    lesson: usize,
}

struct Data {
    /// What the content files hold, what is saved on top of them, and what is being edited.
    base: Raws,
    saved: Raws,
    cur: Raws,
}

/// What the gizmo works on: the entry selected in the list of the tab (props, creatures or chests) and, when the pointer clicks in the world, the one
/// it is over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sel {
    Prop(usize),
    Creature(usize),
    Chest(usize),
    Node(usize),
}

/// Where the ground is (render space x, z -> height), as the world has it.
type Ground<'a> = &'a dyn Fn(f32, f32) -> Option<f32>;

/// What the gizmo moves, as a prop: a creature or a chest is put on the ground, and its facing (degrees clockwise) is a prop's heading
/// turned the other way round.
fn stand(d: &Data, sel: Sel, ground: Ground) -> Option<Prop> {
    let (name, at, facing) = match sel {
        Sel::Prop(i) => return d.cur.props.get(i).cloned(),
        Sel::Creature(i) => d.cur.spawns.get(i).map(|s| (s.creature.clone(), s.at, s.facing))?,
        Sel::Chest(i) => d.cur.chests.get(i).map(|c| (c.id.clone(), c.at, c.facing))?,
        Sel::Node(i) => return places::node_prop(&d.cur.node_kinds, d.cur.nodes.get(i)?, ground),
    };
    Some(Prop { model: name, map: None, zone: None, pickup: None, sleep: false, physics: false, station: 0, at: (at.0, at.1, ground(-at.1, -at.0)?), rotation: [0.0, -facing, 0.0], scale: 1.0 })
}

/// Put what the gizmo moved back.
fn store(d: &mut Data, sel: Sel, p: &Prop) {
    let facing = (-p.rotation[1]).rem_euclid(360.0);
    match sel {
        Sel::Prop(i) => d.cur.props[i] = p.clone(),
        Sel::Creature(i) => {
            let s = &mut d.cur.spawns[i];
            if s.at != (p.at.0, p.at.1) {
                s.height = None;
            }
            (s.at, s.facing) = ((p.at.0, p.at.1), facing);
        }
        Sel::Chest(i) => (d.cur.chests[i].at, d.cur.chests[i].facing) = ((p.at.0, p.at.1), facing),
        // A node's heading is a prop's (not turned the other way round).
        Sel::Node(i) => (d.cur.nodes[i].at, d.cur.nodes[i].facing) = (p.at, p.rotation[1].rem_euclid(360.0)),
    }
}

pub struct Editor {
    pub open: bool,
    font: GlyphSet,
    white: usize,
    /// The client's picture of the places of the map (`POIIcons`), for choosing the icon of a place.
    poi_icons: Option<usize>,
    tab: Tab,
    /// What the places tab lists: 0 the creatures, 1 the chests, 2 the places to discover, 3 the props, 4 the nodes (veins and herbs), 5 their kinds.
    sub: usize,
    pick: [Pick; 10],
    /// The map being played, as of the last picture: what the creatures put down are made for.
    map: &'static str,
    /// Where the editor's eye was at the last picture.
    around: Place,
    data: Option<Data>,
    placer: Placer,
    /// The props the world shows now.
    sent: Vec<Prop>,
    /// How the last frame was seen.
    eye: Option<Eye>,
    /// Ctrl + S was pressed: the next frame saves and applies, as the buttons do.
    save_apply: bool,
    /// The ground under each creature and chest (as the last frame saw it), to pick them with the pointer.
    spawn_ground: Vec<Option<f32>>,
    chest_ground: Vec<Option<f32>>,
    node_ground: Vec<Option<f32>>,
    input: Input,
    edit: Edit,
    status: (String, [f32; 4]),
    /// The kind of creature that "new here" puts down.
    kind: usize,
    /// The sections of the forms the person opened or closed from how they start.
    folds: HashSet<u64>,
    /// The picture of the creature being edited.
    preview: Preview,
    /// The chooser that is open, and what was chosen in it.
    modal: Modal,
}

impl Editor {
    pub fn new(renderer: &mut Renderer, gpu: &Gpu, font: &fontdue::Font, mount: Arc<mpq::Mount>) -> Result<Editor> {
        let white = renderer.ui.add_texture(gpu, 1, 1, &[255; 4]);
        let poi_icons = mount.read(&format!("{}.blp", crate::compass::POI_ICONS)).ok().and_then(|d| wowfmt::blp::Blp::parse(&d).ok()).map(|blp| renderer.ui.add_texture(gpu, blp.width, blp.height, &blp.mips[0]));
        let mut salt = 0;
        let mut pick = || {
            salt += 1;
            Pick { selected: 0, scroll: 0, form_scroll: 0.0, find: String::new(), salt, reveal: false, lesson: 0 }
        };
        Ok(Editor {
            open: false,
            font: GlyphSet::new(&mut renderer.ui, gpu, font, 12.0, 1).context("editor font")?,
            preview: Preview::new(renderer, gpu, mount.clone()),
            modal: Modal::default(),
            white,
            poi_icons,
            tab: Tab::World,
            sub: 0,
            pick: [pick(), pick(), pick(), pick(), pick(), pick(), pick(), pick(), pick(), pick()],
            map: "Azeroth",
            around: Place { north: 0.0, west: 0.0, height: 0.0, yaw: 0.0, map: "Azeroth" },
            data: None,
            placer: {
                let models = Models::new(mount.clone());
                Placer { gallery: Gallery::new(mount.clone(), models.cache.clone()), models, mount, files: None, model: String::new(), placing: false, view: None, memory: None, tool: Mode::Move, drag: None, drag_actor: false, hot: None, hover: None, hold: false, out: Vec::new(), aim: Aim::Prop, creature: String::new(), chest: String::new(), node: String::new(), ghosted: false, turn: 0.0, scale: 1.0, lift: 0.0 }
            },
            sent: content::get().props.clone(),
            eye: None,
            save_apply: false,
            spawn_ground: Vec::new(),
            chest_ground: Vec::new(),
            node_ground: Vec::new(),
            input: Input::default(),
            edit: Edit::default(),
            status: ("F1 closes the editor. Click a field to type in it: Enter keeps what you typed, Esc throws it away. Save and apply puts the changes into the running game.".into(), DIM),
            kind: 0,
            folds: HashSet::new(),
        })
    }

    /// F1: open, with what is saved and what the files hold, or close.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.placer.drag = None;
        self.edit = Edit::default();
        // What was changed and not saved stays for when it is opened again.
        if self.open && self.data.is_none() {
            if let Err(e) = self.reload() {
                self.status = (format!("{e:#}"), RED);
            }
        }
    }

    fn reload(&mut self) -> Result<()> {
        let base = Raws::files()?;
        let saved = base.clone().with(&Overlay::read()?);
        self.data = Some(Data { base, cur: saved.clone(), saved });
        Ok(())
    }

    /// Open it at a tab (`places`, `creatures`, `quests`, `items`) with the entry `selected` (for previews).
    pub fn show(&mut self, tab: &str, selected: usize) {
        self.toggle();
        self.tab = match tab {
            "creatures" => Tab::Creatures,
            "quests" => Tab::Quests,
            "items" => Tab::Items,
            "loot" => Tab::Loot,
            _ => Tab::World,
        };
        self.sub = match tab {
            "chests" => 1,
            "discoveries" => 2,
            "props" => 3,
            "nodes" => 4,
            "nodekinds" => 5,
            _ => 0,
        };
        let i = match self.tab {
            Tab::World => [0, 1, 6, 7, 8, 9][self.sub],
            Tab::Creatures => 2,
            Tab::Quests => 3,
            Tab::Items => 4,
            Tab::Loot => 5,
        };
        self.pick[i].selected = selected;
        self.pick[i].scroll = selected.saturating_sub(10);
    }

    /// Whether the keys go to a field (and not to the game).
    pub fn typing(&self) -> bool {
        self.open && self.edit.focus.is_some()
    }

    /// Whether a point (pixels) is over the editor.
    pub fn over(&self, pos: (f32, f32), height: u32) -> bool {
        self.open && pos.0 < WIDTH * height as f32 / UI_HEIGHT
    }

    pub fn mouse_moved(&mut self, pos: (f32, f32)) {
        self.input.mouse = pos;
    }

    pub fn click(&mut self) {
        self.input.click = true;
    }

    pub fn wheel(&mut self, lines: f32) {
        self.input.wheel += lines;
    }

    pub fn typed(&mut self, text: &str) {
        self.input.typed.push_str(text);
    }

    pub fn key(&mut self, key: Key) {
        self.input.keys.push(key);
    }

    /// Save what was changed, if the content with it is valid.
    fn save(&mut self) -> bool {
        let Some(d) = &mut self.data else { return false };
        if let Err(e) = content::check(&d.cur) {
            self.status = (format!("Not saved: {e:#}"), RED);
            return false;
        }
        // The props are the world's own content: they go to content/props.ron whole, and only the rest to the overlay (what an older overlay
        // held of them moves out of it with this).
        // Those of the original's objects that are still there as they were stay in their own file (and the overlay names those taken away).
        let classic = match content::classic_props() {
            Ok(c) => c,
            Err(e) => {
                self.status = (format!("Not saved: {e:#}"), RED);
                return false;
            }
        };
        let own = content::difference(&classic, &d.cur.props).1;
        let props = own != content::difference(&classic, &d.base.props).1;
        match props.then(|| content::write_props(&own)).unwrap_or(Ok(())).and_then(|()| {
            d.base.props = own.iter().chain(&classic).cloned().collect();
            Overlay::between(&d.base, &d.cur).write()
        }) {
            Ok(()) => {
                d.saved = d.cur.clone();
                self.status = (format!("Saved to content/editor.ron{}. \"Save and apply\" puts it into the running game.", if props { " and content/props.ron" } else { "" }), GREEN);
                true
            }
            Err(e) => {
                self.status = (format!("Not saved: {e:#}"), RED);
                false
            }
        }
    }

    /// The game took what was saved into use (`live`), or starts again with it.
    pub fn applied(&mut self, renderer: &mut Renderer, live: bool) {
        self.preview.forget(renderer);
        self.sent = content::get().props.clone();
        self.status = if live { ("Applied: the game uses the changes now.".into(), GREEN) } else { ("Starting the game again with the changes (they could not be applied live).".into(), DIM) };
    }

    /// What the list of the tab has selected, if it has a prop, a creature or a chest selected.
    fn selection(&self) -> Option<Sel> {
        let d = self.data.as_ref()?;
        let (n, make): (usize, fn(usize) -> Sel) = match (self.tab, self.sub) {
            (Tab::World, 0) => (d.cur.spawns.len(), Sel::Creature),
            (Tab::World, 1) => (d.cur.chests.len(), Sel::Chest),
            (Tab::World, 3) => (d.cur.props.len(), Sel::Prop),
            (Tab::World, 4) => (d.cur.nodes.len(), Sel::Node),
            _ => return None,
        };
        let at = self.pick[[0, 1, 0, 7, 8][self.sub]].selected;
        (at < n).then(|| make(at))
    }

    /// The gizmo over what is selected: the part held is dragged, else the one under the pointer is lit. It is drawn before the window,
    /// which covers it.
    fn gizmo(&mut self, ui: &mut Ui, eye: Eye, height: u32, ground: Ground) {
        self.placer.hot = None;
        let sel = self.selection().filter(|_| !self.placer.placing && self.placer.view.is_none());
        let (Some(sel), Some(d)) = (sel, self.data.as_mut()) else {
            self.placer.drag = None;
            return;
        };
        let Some(mut thing) = stand(d, sel, ground) else {
            self.placer.drag = None;
            return;
        };
        let mouse = Vec2::new(self.input.mouse.0, self.input.mouse.1);
        if let Some(moved) = self.placer.drag.as_ref().and_then(|drag| drag.update(&eye, mouse)) {
            store(d, sel, &moved);
            thing = stand(d, sel, ground).unwrap_or(moved);
        }
        // Only a prop is scaled (and lifted).
        let prop = matches!(sel, Sel::Prop(_));
        let mode = if !prop && self.placer.tool == Mode::Scale { Mode::Move } else { self.placer.tool };
        let gizmo = Gizmo::new(&eye, propedit::position(&thing), mode, prop);
        let over_window = self.input.mouse.0 < WIDTH * height as f32 / UI_HEIGHT;
        let hot = self.placer.drag.as_ref().map(|drag| drag.handle).or_else(|| if over_window { None } else { gizmo.grab(mouse) });
        self.placer.hot = hot;
        gizmo.draw(ui, self.white, hot);
    }

    /// The patrol of the creature selected, drawn over the world: its stops numbered and joined by a line along the ground, the last one back to the first.
    fn patrol_overlay(&self, ui: &mut Ui, size: (u32, u32), eye: Eye, ground: Ground) {
        let Some(Sel::Creature(i)) = self.selection().filter(|_| self.tab == Tab::World && self.sub == 0) else { return };
        let Some(s) = self.data.as_ref().and_then(|d| d.cur.spawns.get(i)).filter(|s| !s.path.is_empty()) else { return };
        let scale = size.1 as f32 / UI_HEIGHT;
        let at = |north: f32, west: f32| ground(-west, -north).map(|y| Vec3::new(-west, y + 0.3, -north));
        let white = self.white;
        let dot = |ui: &mut Ui, p: Vec3, half: f32, color: [f32; 4]| {
            if let Some(px) = eye.project(p) {
                ui.quad(Quad { texture: white, rect: [px.x - half, px.y - half, 2.0 * half, 2.0 * half], uv: FULL_UV, color, additive: false });
            }
        };
        let stops: Vec<Option<Vec3>> = s.path.iter().map(|w| at(w.at.0, w.at.1)).collect();
        let line = [0.4, 0.7, 1.0, 0.9];
        for (k, from) in stops.iter().enumerate() {
            let (Some(a), Some(b)) = (from, stops[(k + 1) % stops.len()].as_ref()) else { continue };
            let steps = (a.distance(*b) as usize).max(1);
            for n in 0..steps {
                let p = a.lerp(*b, n as f32 / steps as f32);
                let y = ground(p.x, p.z).map_or(p.y, |h| h + 0.3);
                dot(ui, Vec3::new(p.x, y, p.z), 1.5, line);
            }
        }
        for (k, p) in stops.iter().enumerate() {
            let Some(p) = p else { continue };
            dot(ui, *p, 7.0, [0.0, 0.0, 0.0, 0.8]);
            dot(ui, *p, 5.0, if k == 0 { GREEN } else { GOLD });
            if let Some(px) = eye.project(*p + Vec3::Y * 0.8) {
                let text = (k + 1).to_string();
                let big = scale * 1.3;
                let (tx, ty) = (px.x - self.font.width(&text) * big / 2.0, px.y - 14.0 * big);
                ui.quad(Quad { texture: white, rect: [tx - 3.0, ty - 1.0, self.font.width(&text) * big + 6.0, 16.0 * big], uv: FULL_UV, color: [0.0, 0.0, 0.0, 0.7], additive: false });
                self.font.draw(ui, &text, tx, ty, big, TEXT);
            }
        }
    }

    /// The left button went down in the world, outside the window: it takes hold of the part of the gizmo under the pointer, or else
    /// selects the prop, creature or chest that the pointer is over (and shows its tab and its place in the list). True when it did
    /// something.
    /// `standing` is where the creatures of the world are now: (kind id, north and west of the place each stands at, where it is, render space); one that has
    /// strolled off is picked where it is, not where its place is.
    pub fn world_press(&mut self, ground: Ground, standing: &[(String, f32, f32, Vec3)]) -> bool {
        if !self.open || self.placer.placing || self.placer.view.is_some() {
            return false;
        }
        let Some(eye) = self.eye else { return false };
        let mouse = Vec2::new(self.input.mouse.0, self.input.mouse.1);
        let sel = self.selection();
        let Some(d) = &mut self.data else { return false };
        if let (Some(handle), Some(sel)) = (self.placer.hot, sel) {
            if let Some(thing) = stand(d, sel, ground) {
                if let Some(drag) = Drag::begin(handle, &thing, &eye, mouse) {
                    if let Sel::Prop(i) = sel {
                        // The world leaves the prop out while it is dragged (a ghost shows it) and has it again where it is let go.
                        let mut without = d.cur.props.clone();
                        without.remove(i);
                        self.sent = without.clone();
                        self.placer.out.push(Action::Props(without));
                    }
                    self.placer.drag_actor = !matches!(sel, Sel::Prop(_));
                    self.placer.drag = Some(drag);
                    return true;
                }
            }
        }
        // What the pointer is over: the nearest of the props, the creatures and the chests.
        let (origin, dir) = eye.ray(mouse);
        let mut best: Option<(f32, Sel)> = None;
        let mut consider = |t: f32, sel: Sel| {
            if best.is_none_or(|(b, _)| t < b) {
                best = Some((t, sel));
            }
        };
        let models = &mut self.placer.models;
        if let Some((i, t)) = propedit::pick_hit(&d.cur.props, origin, dir, |file| models.bounds(file)) {
            consider(t, Sel::Prop(i));
        }
        for (i, s) in d.cur.spawns.iter().enumerate() {
            if let Some(&Some(y)) = self.spawn_ground.get(i) {
                // The size of the creature: an animal by its scale, a person as tall as a man.
                let (height, radius) = match d.cur.creatures.iter().find(|c| c.id == s.creature).map(|c| &c.body) {
                    Some(RawBody::Beast { scale, .. }) => (1.9 * scale, 0.85 * scale),
                    _ => (2.2, 0.6),
                };
                let home = Vec3::new(-s.at.1, y, -s.at.0);
                let here = standing.iter().find(|(id, n, w, _)| *id == s.creature && (n - s.at.0).abs() < 0.6 && (w - s.at.1).abs() < 0.6).map_or(home, |(.., pos)| *pos);
                for at in [home, here] {
                    if let Some(t) = propedit::hit_standing(origin, dir, at - Vec3::new(radius, 0.0, radius), at + Vec3::new(radius, height, radius)) {
                        consider(t, Sel::Creature(i));
                    }
                }
            }
        }
        for (i, c) in d.cur.chests.iter().enumerate() {
            if let Some(&Some(y)) = self.chest_ground.get(i) {
                let at = Vec3::new(-c.at.1, y, -c.at.0);
                if let Some(t) = propedit::hit_standing(origin, dir, at - Vec3::new(0.8, 0.0, 0.8), at + Vec3::new(0.8, 1.0, 0.8)) {
                    consider(t, Sel::Chest(i));
                }
            }
        }
        for (i, n) in d.cur.nodes.iter().enumerate() {
            if let Some(&Some(y)) = self.node_ground.get(i) {
                let at = Vec3::new(-n.at.1, y, -n.at.0);
                if let Some(t) = propedit::hit_standing(origin, dir, at - Vec3::new(0.6, 0.0, 0.6), at + Vec3::new(0.6, 1.0, 0.6)) {
                    consider(t, Sel::Node(i));
                }
            }
        }
        let Some((_, found)) = best else { return false };
        let (sub, pick, i) = match found {
            Sel::Node(i) => (4, 8, i),
            Sel::Creature(i) => (0, 0, i),
            Sel::Chest(i) => (1, 1, i),
            Sel::Prop(i) => (3, 7, i),
        };
        (self.tab, self.sub) = (Tab::World, sub);
        self.pick[pick].selected = i;
        self.pick[pick].find.clear();
        self.pick[pick].reveal = true;
        true
    }

    /// The left button went up: what was dragged stays where it was let go (a creature or a chest is applied, so that it moves in the game).
    pub fn release(&mut self) {
        if self.placer.drag.take().is_some() && self.placer.drag_actor {
            self.save_apply = true;
        }
    }

    /// Ctrl + S: save and apply, as the two buttons at the bottom do together.
    pub fn request_save(&mut self) {
        self.save_apply = self.open;
    }

    /// G: what is selected on the props tab drops to the ground (`ground` gives the height of the terrain at a place, render space), and
    /// what is put down goes back to stand on it.
    pub fn snap_to_ground(&mut self, ground: impl Fn(f32, f32) -> Option<f32>) {
        if self.placer.placing {
            self.placer.lift = 0.0;
            self.status = ("Back on the ground.".into(), DIM);
            return;
        }
        let (on_props, selected) = (self.tab == Tab::World && self.sub == 3, self.pick[7].selected);
        let Some(prop) = self.data.as_mut().filter(|_| on_props).and_then(|d| d.cur.props.get_mut(selected)) else { return };
        let at = propedit::position(prop);
        self.status = match ground(at.x, at.z) {
            Some(height) => {
                prop.at.2 = height;
                ("Dropped to the ground.".into(), DIM)
            }
            None => ("There is no ground loaded under it.".into(), RED),
        };
    }

    /// The pointer moved by `dx` pixels with the right button held while a model is put down: it turns.
    pub fn turn_by(&mut self, dx: f32) {
        self.placer.turn = (self.placer.turn + dx * 0.5).rem_euclid(360.0);
    }

    /// The wheel over the world while a model is put down: it makes it bigger or smaller, with Shift it raises or lowers it. True if
    /// it was taken.
    pub fn scroll_world(&mut self, lines: f32, shift: bool) -> bool {
        if !self.placing() || self.placer.aim != Aim::Prop {
            return false;
        }
        let p = &mut self.placer;
        if shift {
            p.lift = (p.lift + lines * 0.25).clamp(-20.0, 200.0);
        } else {
            p.scale = (p.scale * 1.1f32.powf(lines)).clamp(0.05, 50.0);
        }
        true
    }

    /// Where the pointer touches the world (for the ghost of the model that is put down).
    pub fn set_hover(&mut self, hit: Option<Vec3>) {
        self.placer.hover = hit;
    }

    /// Show the ghost of what is put down or dragged (call after `draw`): the model, an animal, a person, as it will stand and face when it is
    /// put down. `refreshing`: the world is still making the tile of a dragged prop.
    pub fn update_ghosts(&mut self, renderer: &mut Renderer, gpu: &Gpu, refreshing: bool, dt: f32) {
        /// What to show: a model (a prop or a chest) or a creature, where, and facing how (radians, 0 north, clockwise).
        enum Want {
            Prop(Prop),
            Chest(String, Vec3, f32),
            Creature(String, Vec3, f32),
        }
        let sel = self.selection();
        let want = match (&self.data, &self.placer) {
            // Dragged: the thing selected, where the gizmo has taken it.
            (Some(d), p) if p.drag.is_some() => match sel {
                Some(Sel::Prop(i)) => d.cur.props.get(i).cloned().map(Want::Prop),
                Some(Sel::Creature(i)) => d.cur.spawns.get(i).zip(self.spawn_ground.get(i).copied().flatten()).map(|(s, y)| Want::Creature(s.creature.clone(), Vec3::new(-s.at.1, y, -s.at.0), s.facing.to_radians())),
                Some(Sel::Chest(i)) => d.cur.chests.get(i).zip(self.chest_ground.get(i).copied().flatten()).map(|(c, y)| Want::Chest(c.model.clone(), Vec3::new(-c.at.1, y, -c.at.0), c.facing.to_radians())),
                Some(Sel::Node(i)) => d.cur.nodes.get(i).zip(self.node_ground.get(i).copied().flatten()).and_then(|(n, y)| places::node_prop(&d.cur.node_kinds, n, &|_, _| Some(y))).map(Want::Prop),
                None => None,
            },
            // Put down: what the next click would make, where the pointer touches the world.
            (d, p) if self.open && p.placing => p.hover.and_then(|at| match p.aim {
                Aim::Prop if placeable(&p.model) && p.mount.exists(&p.model) => {
                    let [north, west, height] = render_to_wow(at + Vec3::Y * p.lift);
                    Some(Want::Prop(Prop { model: p.model.clone(), map: None, zone: None, pickup: None, sleep: false, physics: false, station: 0, at: (north, west, height), rotation: [0.0, p.turn, 0.0], scale: p.scale }))
                }
                Aim::Prop => None,
                Aim::Chest => Some(Want::Chest(p.chest.clone(), at, p.turn.to_radians())),
                Aim::Node => {
                    let [north, west, height] = render_to_wow(at);
                    let node = content::Node { kind: p.node.clone(), map: None, at: (north, west, height), facing: p.turn, respawn: None };
                    d.as_ref().and_then(|d| places::node_prop(&d.cur.node_kinds, &node, &|_, _| Some(height))).map(Want::Prop)
                }
                Aim::Creature => Some(Want::Creature(p.creature.clone(), at, p.turn.to_radians())),
                Aim::Stop => None,
            }),
            _ => None,
        };
        // The body of the creature that is shown.
        let body = match (&want, &self.data) {
            (Some(Want::Creature(kind, ..)), Some(d)) => d.cur.creatures.iter().find(|c| c.id == *kind).map(|c| c.body.clone()),
            _ => None,
        };
        let p = &mut self.placer;
        p.ghosted = false;
        let dragging_prop = p.drag.is_some() && matches!(sel, Some(Sel::Prop(_)));
        match want {
            Some(want) => {
                p.ghosted = match (want, body) {
                    (Want::Prop(prop), _) => p.models.show(renderer, gpu, &prop.model, propedit::position(&prop), propedit::orient(&prop)),
                    // As the world places them: a creature or a chest is turned by `Mat4::from_rotation_y(FRAC_PI_2 - facing)`.
                    (Want::Chest(model, at, yaw), _) => p.models.show(renderer, gpu, &model, at, Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - yaw)),
                    (Want::Creature(_, at, yaw), Some(RawBody::Beast { model, skins, scale })) => p.models.show_beast(renderer, gpu, &model, &skins, at, yaw, scale),
                    (Want::Creature(_, at, yaw), Some(RawBody::Humanoid { outfit, look })) => {
                        let worn: Vec<usize> = outfit.iter().filter_map(|id| crate::items::all().iter().position(|i| i.id == *id)).collect();
                        p.models.show_person(renderer, gpu, &worn, &look, dt, at, yaw)
                    }
                    (Want::Creature(..), None) => false,
                };
                if !p.ghosted {
                    p.models.hide(renderer);
                }
                p.hold = dragging_prop;
            }
            None if !(p.hold && refreshing) => {
                p.models.hide(renderer);
                p.hold = false;
            }
            None => {}
        }
    }

    /// Whether a click in the world puts a prop down now.
    pub fn placing(&self) -> bool {
        self.open && self.placer.placing && self.data.is_some()
    }

    /// A click in the world while props are put down: the chosen model stands at `hit` (render space), and stays selected for the form.
    pub fn place(&mut self, hit: Vec3) {
        let Some(d) = &mut self.data else { return };
        let [north, west, height] = render_to_wow(hit);
        let (turn, scale, lift) = (self.placer.turn, self.placer.scale, self.placer.lift);
        match self.placer.aim {
            Aim::Prop => {
                d.cur.props.push(Prop { model: self.placer.model.clone(), map: None, zone: None, pickup: None, sleep: false, physics: false, station: 0, at: (north, west, height + lift), rotation: [0.0, turn, 0.0], scale });
                let p = &mut self.pick[7];
                p.selected = d.cur.props.len() - 1;
                p.scroll = d.cur.props.len().saturating_sub(10);
            }
            Aim::Creature => {
                d.cur.spawns.push(new_spawn(self.placer.creature.clone(), self.map, (north, west), turn));
                let p = &mut self.pick[0];
                p.selected = d.cur.spawns.len() - 1;
                p.reveal = true;
            }
            Aim::Stop => {
                if let Some(s) = d.cur.spawns.get_mut(self.pick[0].selected) {
                    s.path.push(Waypoint { at: (north, west), wait: 0.0, facing: None, steps: Vec::new() });
                }
            }
            Aim::Node => {
                d.cur.nodes.push(content::Node { kind: self.placer.node.clone(), map: Some(self.map.to_string()), at: (north, west, height), facing: turn, respawn: None });
                let p = &mut self.pick[8];
                p.selected = d.cur.nodes.len() - 1;
                p.reveal = true;
            }
            Aim::Chest => {
                let mut chest = chest_like(d, self.pick[1].selected);
                (chest.model, chest.at, chest.facing) = (self.placer.chest.clone(), (north, west), turn);
                d.cur.chests.push(chest);
                let p = &mut self.pick[1];
                p.selected = d.cur.chests.len() - 1;
                p.scroll = d.cur.chests.len().saturating_sub(10);
            }
        }
    }

    /// Load and draw the pictures the model gallery shows (call after `draw`, before the frame is drawn); with `wait` until all are there.
    pub fn render_gallery(&mut self, renderer: &mut Renderer, gpu: &Gpu, world: &crate::world::World, base: Vec3, wait: bool) {
        if !(self.open && self.placer.view.is_some()) {
            return;
        }
        for _ in 0..if wait { 2000 } else { 1 } {
            let p = &mut self.placer;
            p.gallery.update(renderer, gpu, world, base, &mut p.models);
            if !wait || !self.placer.gallery.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Draw what the form of the creature asked to be seen into its picture (call before the frame is drawn); `base` is anywhere on a
    /// loaded tile of the world.
    pub fn render_preview(&mut self, renderer: &mut Renderer, gpu: &Gpu, world: &crate::world::World, base: Vec3, dt: f32) {
        if self.open {
            self.preview.render(renderer, gpu, world, base, dt);
        }
    }

    /// Draw the editor and take the actions of what was clicked this frame.
    pub fn draw(&mut self, ui: &mut Ui, size: (u32, u32), player: Place, eye: Eye, ground: Ground) -> Vec<Action> {
        let mut actions = Vec::new();
        self.preview.clear();
        self.placer.gallery.begin();
        if !self.open || self.data.is_none() {
            return actions;
        }
        self.eye = Some(eye);
        self.map = player.map;
        // Near the editor's eye (it flies free of the player), not the player, is what is listed and can be picked.
        let around = Place { north: -eye.position().z, west: -eye.position().x, ..player };
        self.around = around;
        if let Some(d) = &self.data {
            // The ground under the creatures near the editor's eye (the world has tens of thousands; the others cannot be picked in the world).
            self.spawn_ground = d.cur.spawns.iter().map(|s| if near_here(s.at, around) { ground(-s.at.1, -s.at.0) } else { None }).collect();
            self.chest_ground = d.cur.chests.iter().map(|c| ground(-c.at.1, -c.at.0)).collect();
            self.node_ground = d.cur.nodes.iter().map(|n| if near_here((n.at.0, n.at.1), around) { ground(-n.at.1, -n.at.0) } else { None }).collect();
        }
        self.gizmo(ui, eye, size.1, ground);
        // A creature or a chest that has no ghost to show gets a mark on the ground with an arrow for the way it will face.
        if let (true, Some(at), false) = (self.placer.placing, self.placer.hover, self.placer.ghosted || self.placer.aim == Aim::Prop) {
            let color = match self.placer.aim {
                Aim::Chest => [1.0, 0.6, 0.2, 1.0],
                Aim::Node => [0.9, 0.9, 0.3, 1.0],
                Aim::Stop => [0.4, 0.7, 1.0, 1.0],
                _ => [0.4, 1.0, 0.4, 1.0],
            };
            propedit::draw_marker(ui, self.white, &eye, at, self.placer.turn, color);
        }
        self.patrol_overlay(ui, size, eye, ground);
        let (save, apply, revert) = {
            let s = size.1 as f32 / UI_HEIGHT;
            let Editor { font, white, poi_icons, tab, sub, pick, data, input, edit, status, kind, preview, modal, placer, folds, .. } = self;
            let data = data.as_mut().expect("checked above");
            let ids = Ids::of(&data.cur);
            let mut g = Gui { ui, font, white: *white, poi_icons: *poi_icons, s, input, edit, status, clip: None, blocked: modal.open.is_some() || modal.icons.is_some() || placer.view.is_some(), modal, ids: &ids, tint: None, tall: None, folds };
            g.fill(0.0, 0.0, WIDTH, UI_HEIGHT, PANEL);
            // The tabs.
            let mut x = 8.0;
            for (t, name) in TABS {
                let w = 100.0;
                if *tab == t {
                    g.fill(x, 6.0, w, ROW + 2.0, SELECTED);
                }
                if g.button(x, 6.0, w, name) {
                    *tab = t;
                }
                x += w + 6.0;
            }
            g.text(WIDTH - 150.0, 10.0, 140.0, "Editor (F1 closes)", DIM);
            let (top, bottom) = (36.0, UI_HEIGHT - 64.0);
            match *tab {
                Tab::World => world(&mut g, data, pick, sub, kind, placer, player, around, &mut actions, top, bottom),
                Tab::Creatures => creatures(&mut g, data, &mut pick[2], preview, top, bottom),
                Tab::Quests => quests(&mut g, data, &mut pick[3], player, top, bottom),
                Tab::Items => items(&mut g, data, &mut pick[4], preview, top, bottom),
                Tab::Loot => loot(&mut g, data, &mut pick[5], top, bottom),
            }
            // The bottom: save, apply, discard, and what happened (two lines: an error can be long).
            let y = UI_HEIGHT - 58.0;
            let dirty = data.cur != data.saved;
            let save = g.button_in(8.0, y, 110.0, if dirty { "Save *" } else { "Save" }, if dirty { GOLD } else { DIM });
            let apply = g.button(124.0, y, 150.0, "Save and apply");
            let revert = dirty && g.danger("discard", 280.0, y, 150.0, "Discard changes");
            g.text(440.0, y + 3.0, WIDTH - 448.0, if dirty { "* not saved yet" } else { "everything is saved" }, DIM);
            let (text, color) = g.status.clone();
            for (i, row) in wrapped(&g, WIDTH - 16.0, &text).into_iter().take(2).enumerate() {
                g.text(8.0, y + 24.0 + i as f32 * (ROW - 2.0), WIDTH - 16.0, &row, color);
            }
            g.blocked = false;
            hud(&mut g, placer, *tab == Tab::World && *sub == 3, size.0 as f32 / s);
            if let Some((tag, what, current)) = g.modal.want.take() {
                placer.open_entries(&mut g, &data.cur, tag, what, &current);
            }
            chooser(&mut g, preview);
            icon_chooser(&mut g);
            gallery(&mut g, placer);
            (save, apply, revert)
        };
        // A field that has the focus but was not on screen (its window or tab was left) lets go of it: it would swallow the keys, F1 too.
        if !std::mem::take(&mut self.edit.seen) {
            self.edit.focus = None;
        }
        // A field that was clicked takes the focus now, with what it holds.
        if let Some((id, text)) = self.edit.want.take() {
            self.edit.focus = Some(id);
            self.edit.buffer = text;
        }
        self.input.click = false;
        self.input.wheel = 0.0;
        self.input.typed.clear();
        self.input.keys.clear();
        if revert {
            match self.reload() {
                Ok(()) => self.status = ("Changes discarded: back to what is saved.".into(), DIM),
                Err(e) => self.status = (format!("{e:#}"), RED),
            }
        }
        let apply = apply || std::mem::take(&mut self.save_apply);
        if (save || apply) && self.save() && apply {
            actions.push(Action::Apply);
        }
        // Props that were put down or changed are shown at once (once a typed field is kept), saved or not.
        actions.append(&mut self.placer.out);
        match &self.data {
            Some(d) if self.edit.focus.is_none() && self.placer.drag.is_none() && d.cur.props != self.sent && !actions.contains(&Action::Apply) => {
                self.sent = d.cur.props.clone();
                actions.push(Action::Props(self.sent.clone()));
            }
            _ => {}
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_are_listed_by_name_and_references_are_known_by_their_ids() {
        assert_eq!((model_label(r"Creature\Wolf\Wolf.m2").as_str(), model_label(r"Creature\Boar\BoarSkin.m2").as_str()), ("Wolf", r"Boar\BoarSkin"));
        let ids = Ids::of(&Raws::files().unwrap());
        assert!(ids.has(Ref::Creature, "wolf") && ids.has(Ref::Item, "longsword") && ids.has(Ref::Loot, "wolf") && !ids.has(Ref::Quest, "nothing"));
    }

    #[test]
    fn ids_are_numbered_until_one_is_free() {
        assert_eq!(fresh(&["a_1", "a_2"], "a"), "a_3");
        assert_eq!(fresh(&[], "a"), "a_1");
    }

    #[test]
    fn money_is_written_in_gold_silver_and_copper_and_numbers_are_asked_for_in_words() {
        assert_eq!((coins(0).as_str(), coins(5).as_str(), coins(120).as_str(), coins(90_205).as_str()), ("0c", "5c", "1s 20c", "9g 2s 5c"));
        assert_eq!((number_words::<u32>(), number_words::<i32>(), number_words::<f32>()), ("a whole number, 0 or more", "a whole number", "a number (a point for decimals)"));
    }

    #[test]
    fn the_arrows_step_round_a_list_of_choices() {
        let o: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        assert_eq!((step(&o, "c", 1), step(&o, "a", -1), step(&o, "b", 1)), ("a", "c", "c"));
    }

    #[test]
    fn an_overlay_is_written_beside_the_content_files_and_read_back() {
        let dir = std::env::temp_dir().join(format!("rsworld-content-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Nothing else reads the folder while testing (the tests are of the files built in).
        std::env::set_var("RSWORLD_CONTENT", &dir);
        assert_eq!(Overlay::read().unwrap(), Overlay::default(), "no file is no change");
        let base = Raws::files().unwrap();
        let mut cur = base.clone();
        cur.quests[1].reward.copper += 1;
        let overlay = Overlay::between(&base, &cur);
        overlay.write().unwrap();
        assert_eq!(Overlay::read().unwrap(), overlay);
        assert!(std::fs::read_to_string(dir.join("editor.ron")).unwrap().contains("Written by the in-game editor"));
        // The props go to their own file, which reads back as they were written.
        let barn = Prop { model: r"World\wmo\Azeroth\Buildings\Human_Barn_Silo\barn.wmo".into(), map: None, zone: None, pickup: None, sleep: false, physics: false, station: 0, at: (-8915.0, -150.0, 85.0), rotation: [0.0, 90.0, 0.0], scale: 1.5 };
        content::write_props(std::slice::from_ref(&barn)).unwrap();
        let text = std::fs::read_to_string(dir.join("props.ron")).unwrap();
        assert_eq!(ron::from_str::<Vec<Prop>>(&text).unwrap(), vec![barn]);
        assert!(!dir.join("props.ron.tmp").exists());
        std::env::remove_var("RSWORLD_CONTENT");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_teacher_with_lessons_is_kept_in_the_overlay_and_a_bad_lesson_is_caught() {
        use crate::content::Lesson;
        use crate::skills::Skill;
        let base = Raws::files().unwrap();
        let mut cur = base.clone();
        let mut teacher = cur.creatures.iter().find(|c| c.id == "trader_aldric").unwrap().clone();
        teacher.id = "miner".into();
        cur.creatures.push(teacher);
        cur.lessons.insert("miner".into(), vec![Lesson { spell: 2581, cost: 10, skill: None, level: 0, step: Some((Skill::Mining, 1)) }]);
        // Another teacher is made to teach nothing.
        let silent = cur.lessons.keys().next().unwrap().clone();
        cur.lessons.remove(&silent);
        let overlay = Overlay::between(&base, &cur);
        assert_eq!((overlay.lessons.len(), overlay.lessons_removed.len()), (1, 1));
        let back: Overlay = ron::from_str(&ron::ser::to_string_pretty(&overlay, ron::ser::PrettyConfig::default()).unwrap()).unwrap();
        let again = base.clone().with(&back);
        assert_eq!(again.lessons, cur.lessons);
        content::check(&again).unwrap();
        // A step that no spell of the game names, and a recipe that is none, are caught before they are saved.
        cur.lessons.get_mut("miner").unwrap()[0].spell = 4_000_000;
        assert!(content::check(&cur).is_err());
    }

    #[test]
    fn what_the_editor_changes_comes_back_as_it_was_and_is_still_valid() {
        let base = Raws::files().unwrap();
        let mut cur = base.clone();
        // A new enemy, a changed quest, a removed chest, a creature moved and another put down.
        let mut wolf = cur.creatures.iter().find(|c| c.id == "wolf").unwrap().clone();
        wolf.id = "wolf_pup".into();
        wolf.name = "Wolf Pup".into();
        cur.creatures.push(wolf);
        cur.quests[0].reward.xp += 5;
        cur.chests.pop();
        cur.spawns[0].at.0 += 3.0;
        cur.spawns.push(Spawn { creature: "wolf_pup".into(), map: None, at: (-8900.0, -100.0), facing: 12.0, wander: 0.0, height: None, respawn: None, path: Vec::new() });
        // A loot table made and one changed, a place moved and a place made.
        cur.loot.insert("pup".into(), content::RawTable { copper: (1, 2), items: vec![("linen_cloth".into(), (1, 1), 30.0)], also: Vec::new() });
        cur.loot.get_mut("wolf").unwrap().copper.1 += 4;
        cur.loot.remove("chest_fine");
        cur.pois[0].radius += 1.0;
        // A chest that also holds what is fixed, and a trader who is an orc woman.
        (cur.chests[0].copper, cur.chests[0].items) = (3, vec![("hearthstone".into(), 1)]);
        if let RawBody::Humanoid { look, .. } = &mut cur.creatures.iter_mut().find(|c| c.id == "trader_aldric").unwrap().body {
            (look.race, look.sex) = (crate::character::Race::Orc, crate::character::Sex::Female);
        }
        cur.pois.push(content::Poi { id: "pup_den".into(), name: "Pup Den".into(), at: (-8900.0, -100.0), radius: 20.0, icon: 4, height: None });
        // Two props, one of them the same as the other (a copy stays a copy).
        let barn = Prop { model: r"World\wmo\Azeroth\Buildings\Human_Barn_Silo\barn.wmo".into(), map: None, zone: None, pickup: None, sleep: false, physics: false, station: 0, at: (-8915.0, -150.0, 85.0), rotation: [0.0, 90.0, 0.0], scale: 1.0 };
        cur.props.extend([barn.clone(), barn]);
        // A node kind made (like the first, another name and size), a node of it put down and another one taken away.
        let mut kind = cur.node_kinds[0].clone();
        (kind.id, kind.name, kind.scale) = ("big_vein".into(), "Big Vein".into(), 2.0);
        cur.node_kinds.push(kind);
        cur.nodes.push(content::Node { kind: "big_vein".into(), map: Some("Azeroth".into()), at: (-8900.0, -100.0, 85.0), facing: 30.0, respawn: Some(60.0) });
        cur.nodes.remove(0);
        let overlay = Overlay::between(&base, &cur);
        assert_eq!((overlay.props_removed.len(), overlay.props_added.len()), (0, 2));
        assert_eq!((overlay.creatures.entries.len(), overlay.quests.entries.len(), overlay.chests.removed.len(), overlay.spawns_removed.len(), overlay.spawns_added.len()), (2, 1, 1, 1, 2));
        assert_eq!((overlay.loot.len(), overlay.loot_removed.len(), overlay.pois.entries.len()), (2, 1, 2));
        // Written and read back, put on the files, it is what was edited.
        let text = ron::ser::to_string_pretty(&overlay, ron::ser::PrettyConfig::default()).unwrap();
        let back: Overlay = ron::from_str(&text).unwrap();
        assert_eq!(back, overlay);
        let again = base.clone().with(&back);
        assert_eq!((again.creatures.len(), again.quests.clone(), again.chests.len()), (cur.creatures.len(), cur.quests.clone(), cur.chests.len()));
        assert!(again.loot == cur.loot && again.pois == cur.pois && again.chests == cur.chests && again.creatures == cur.creatures);
        assert!(again.spawns.len() == cur.spawns.len() && cur.spawns.iter().all(|s| again.spawns.contains(s)));
        assert_eq!(again.props, cur.props);
        assert!(again.node_kinds == cur.node_kinds && again.nodes.len() == cur.nodes.len() && cur.nodes.iter().all(|n| again.nodes.contains(n)));
        assert_eq!((overlay.node_kinds.entries.len(), overlay.nodes_removed.len(), overlay.nodes_added.len()), (1, 1, 1));
        content::check(&again).unwrap();
        // And a change that breaks a reference is caught before it is saved.
        let mut broken = cur.clone();
        broken.spawns.push(Spawn { creature: "nobody".into(), map: None, at: (0.0, 0.0), facing: 0.0, wander: 5.0, height: None, respawn: None, path: Vec::new() });
        assert!(content::check(&broken).is_err());
        let mut broken = cur.clone();
        broken.props[0].model = r"World\Generic\Thing.blp".into();
        assert!(content::check(&broken).is_err(), "only models can be placed");
        // A prop that is picked up names an item of the game; what it leaves out is a time of two minutes, and that it vanishes.
        let mut picked = cur.clone();
        picked.props[0].pickup = Some(ron::from_str(r#"(item: "tough_wolf_meat")"#).unwrap());
        assert_eq!(picked.props[0].pickup, Some(content::Pickup { item: "tough_wolf_meat".into(), respawn: 120.0, vanish: true }));
        content::check(&picked).unwrap();
        picked.props[0].pickup.as_mut().unwrap().item = "nothing".into();
        assert!(content::check(&picked).is_err(), "an unknown item cannot be picked up");
        // A bed is a flag that is only written when set.
        let mut bed = Prop { model: "x.m2".into(), map: None, zone: None, pickup: None, sleep: false, physics: false, station: 0, at: (0.0, 0.0, 0.0), rotation: [0.0; 3], scale: 1.0 };
        assert!(!ron::to_string(&bed).unwrap().contains("sleep"));
        bed.sleep = true;
        let text = ron::to_string(&bed).unwrap();
        assert!(text.contains("sleep:true"));
        assert!(ron::from_str::<content::Prop>(&text).unwrap().sleep);
    }

    #[test]
    fn a_patrol_a_trainer_and_the_other_new_fields_are_kept_and_still_valid() {
        let base = Raws::files().unwrap();
        let mut cur = base.clone();
        let stop = |north: f32, steps: Vec<(f32, Step)>| Waypoint { at: (north, -100.0), wait: 2.0, facing: Some(90.0), steps };
        let steps = vec![(0.0, Step::Run(true)), (1.5, Step::Emote(1)), (2.0, Step::Roam(4.0, 10.0))];
        // What the "does" field shows is read back as it was.
        assert_eq!(ron::from_str::<Vec<(f32, Step)>>(&ron::to_string(&steps).unwrap()).unwrap(), steps);
        (cur.spawns[0].respawn, cur.spawns[0].path) = (Some(60.0), vec![stop(-8900.0, steps), stop(-8910.0, Vec::new())]);
        let talker = cur.creatures.iter_mut().find(|c| c.gossip.is_some()).unwrap();
        talker.kind = 7;
        talker.gossip.as_mut().unwrap().options.push(GossipOption { text: "Teach me.".into(), action: CreatureAction::Train { skill: crate::skills::Skill::Riding, rank: 75, needs: 0, level: 40, cost: 900_000 } });
        (cur.quests[0].level, cur.quests[0].start_items) = (5, vec![("hearthstone".into(), 1)]);
        let plain = cur.loot.iter().find(|(_, t)| t.also.is_empty()).map(|(k, _)| k.clone()).unwrap();
        cur.loot.insert("both".into(), content::RawTable { copper: (1, 2), items: Vec::new(), also: vec![plain] });
        let item = &mut cur.items[0];
        (item.weight, item.slots, item.more) = (Some(1.5), 4, vec![(1.0, 2.0, 2)]);
        let effects = vec![crate::items::UseEffect::Heal(30, 40), crate::items::UseEffect::Boost(2, 10, 60.0)];
        assert_eq!(ron::from_str::<Vec<crate::items::UseEffect>>(&ron::to_string(&effects).unwrap()).unwrap(), effects);
        item.flags |= 2;
        item.spells.push(crate::items::ItemSpell { spell: 133, trigger: 2, charges: -1, cooldown: 5000, ppm: 3.5, effects, text: "Test".into() });
        cur.pois[0].height = Some(12.0);
        let overlay = Overlay::between(&base, &cur);
        let back: Overlay = ron::from_str(&ron::ser::to_string_pretty(&overlay, ron::ser::PrettyConfig::default()).unwrap()).unwrap();
        let again = base.clone().with(&back);
        assert!(again.creatures == cur.creatures && again.quests == cur.quests && again.items == cur.items && again.pois == cur.pois && again.loot == cur.loot);
        assert!(cur.spawns.iter().all(|s| again.spawns.contains(s)));
        content::check(&again).unwrap();
        // A stop of a patrol that turns into a kind that does not exist is caught before it is saved.
        let mut broken = cur.clone();
        broken.spawns[0].path[0].steps.push((0.0, Step::Morph("nobody".into())));
        assert!(content::check(&broken).is_err());
    }

    #[test]
    fn the_originals_objects_are_edited_and_only_what_changed_of_them_is_kept() {
        let classic = content::classic_props().unwrap();
        let base = Raws::files().unwrap();
        assert!(base.props.len() >= classic.len() && classic.iter().all(|c| base.props.contains(c)));
        // One of them taken away, one moved: the moved one belongs to props.ron, the overlay names both of the original's entries.
        let mut cur = base.clone();
        let gone = cur.props.remove(cur.props.len() - 1);
        let mut moved = cur.props[cur.props.len() - 1].clone();
        cur.props.truncate(cur.props.len() - 1);
        let original = moved.clone();
        moved.at.2 += 5.0;
        cur.props.push(moved.clone());
        let own = content::difference(&classic, &cur.props).1;
        // (Props of the game's own, such as beds put down in the editor, are among them too.)
        assert!(own.contains(&moved));
        let mut saved = cur.clone();
        saved.props = own.iter().chain(&classic).cloned().collect();
        let overlay = Overlay::between(&saved, &cur);
        assert_eq!((overlay.props_removed.len(), overlay.props_added.len()), (2, 0));
        let again = saved.with(&overlay);
        assert!(again.props.contains(&moved) && !again.props.contains(&gone) && !again.props.contains(&original) && again.props.len() == cur.props.len());
    }

    #[test]
    fn the_gallery_sorts_models_into_folders_of_at_most_three_levels() {
        assert_eq!(category_of(r"World\Azeroth\Elwynn\PassiveDoodads\Fences\ElwynnFence01.m2"), r"Azeroth\Elwynn\Fences");
        assert_eq!(category_of(r"World\Generic\PassiveDoodads\Lights\Candelabra01.m2"), r"Generic\Lights");
        assert_eq!(category_of(r"World\wmo\Azeroth\Buildings\Human_Barn_Silo\barn.wmo"), r"wmo\Azeroth\Buildings");
        let files: Vec<String> = [r"World\AZEROTH\Westfall\Rocks\a.m2", r"World\Azeroth\Westfall\Rocks\b.m2", r"World\Generic\x.m2"].map(String::from).into();
        let view = View::new(&files, r"World\Generic\x.m2");
        // The two spellings are one folder, named as the more readable one; the folder of the model that is chosen is on.
        assert_eq!(view.lines, ["All models (3)", "(2) Azeroth\\Westfall\\Rocks", "(1) Generic"]);
        assert_eq!(view.folders.selected, 2);
    }

    #[test]
    fn the_models_to_put_down_are_the_doodads_and_the_buildings_but_not_their_parts() {
        for (file, ok) in [
            (r"World\Generic\PassiveDoodads\Lights\Candelabra01.m2", true),
            (r"World\wmo\Azeroth\Buildings\Human_Barn_Silo\barn.wmo", true),
            (r"World\wmo\Azeroth\Buildings\Human_Barn_Silo\barn_000.wmo", false),
            (r"World\Maps\Azeroth\Azeroth_32_48.adt", false),
            (r"World\Generic\PassiveDoodads\Lights\Candel_Black.blp", false),
            (r"Creature\Wolf\Wolf.m2", false),
        ] {
            assert_eq!(placeable(file), ok, "{file}");
        }
    }
}
