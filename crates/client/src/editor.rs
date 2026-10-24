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
use std::fmt::Display;
use std::sync::Arc;
use std::str::FromStr;

/// Width of the editor in interface units (the interface is 768 units high).
const WIDTH: f32 = 720.0;
const ROW: f32 = 18.0;
const LIST_WIDTH: f32 = 220.0;
const UI_HEIGHT: f32 = 768.0;
const LABEL_WIDTH: f32 = 96.0;

const PANEL: [f32; 4] = [0.04, 0.04, 0.07, 0.93];
const BOX: [f32; 4] = [0.12, 0.12, 0.17, 1.0];
const HOT: [f32; 4] = [0.22, 0.24, 0.34, 1.0];
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
    };
    Some(Prop { model: name, map: None, zone: None, pickup: None, sleep: false, physics: false, at: (at.0, at.1, ground(-at.1, -at.0)?), rotation: [0.0, -facing, 0.0], scale: 1.0 })
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
    }
}

pub struct Editor {
    pub open: bool,
    font: GlyphSet,
    white: usize,
    /// The client's picture of the places of the map (`POIIcons`), for choosing the icon of a place.
    poi_icons: Option<usize>,
    tab: Tab,
    /// What the places tab lists: 0 the creatures, 1 the chests, 2 the places to discover, 3 the props.
    sub: usize,
    pick: [Pick; 8],
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
    input: Input,
    edit: Edit,
    status: (String, [f32; 4]),
    /// The kind of creature that "new here" puts down.
    kind: usize,
    /// The picture of the creature being edited.
    preview: Preview,
    /// The chooser that is open, and what was chosen in it.
    modal: Modal,
}

struct Gui<'a> {
    ui: &'a mut Ui,
    font: &'a GlyphSet,
    white: usize,
    poi_icons: Option<usize>,
    s: f32,
    input: &'a mut Input,
    edit: &'a mut Edit,
    status: &'a mut (String, [f32; 4]),
    /// Where the mouse may act now, in units (the form's area).
    clip: Option<[f32; 4]>,
    modal: &'a mut Modal,
    /// What the references can be (ids of what the content has).
    ids: &'a Ids,
    /// Nothing under a chooser that is open may be clicked.
    blocked: bool,
    /// The colour of the text of the next field, when it is not the usual.
    tint: Option<[f32; 4]>,
}

/// A number for a field that stays put when the form changes under it.
fn fid(tag: &str, n: usize) -> u64 {
    tag.bytes().fold(0xcbf29ce484222325u64 ^ n as u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

impl Gui<'_> {
    fn px(&self, v: f32) -> f32 {
        v * self.s
    }

    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.picture(self.white, x, y, w, h, color);
    }

    /// A texture over the rectangle, tinted.
    fn picture(&mut self, texture: usize, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        let r = [self.px(x), self.px(y), self.px(w), self.px(h)];
        self.ui.quad(Quad { texture, rect: r, uv: FULL_UV, color, additive: false });
    }

    /// A part of a texture (`uv`: left, top, right, bottom) over the rectangle.
    fn picture_part(&mut self, texture: usize, x: f32, y: f32, w: f32, h: f32, uv: [f32; 4]) {
        let r = [self.px(x), self.px(y), self.px(w), self.px(h)];
        self.ui.quad(Quad { texture, rect: r, uv, color: [1.0; 4], additive: false });
    }

    /// The area the quads are cut to now: that of the form, if there is one.
    fn restore_clip(&mut self) {
        let clip = self.clip.map(|[x, y, w, h]| [self.px(x), self.px(y), self.px(w), self.px(h)]);
        self.ui.set_clip(clip);
    }

    /// Text with the top of its line at `y`; clipped to `w` units (and to the form's area).
    fn text(&mut self, x: f32, y: f32, w: f32, text: &str, color: [f32; 4]) {
        let mut clip = [x, y - 2.0, w, ROW];
        if let Some([cx, cy, cw, ch]) = self.clip {
            let (x0, y0, x1, y1) = (clip[0].max(cx), clip[1].max(cy), (clip[0] + clip[2]).min(cx + cw), (clip[1] + clip[3]).min(cy + ch));
            clip = [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)];
        }
        self.ui.set_clip(Some([self.px(clip[0]), self.px(clip[1]), self.px(clip[2]), self.px(clip[3])]));
        self.font.draw(self.ui, text, self.px(x), self.px(y), self.s, color);
        self.restore_clip();
    }

    fn mouse(&self) -> (f32, f32) {
        (self.input.mouse.0 / self.s, self.input.mouse.1 / self.s)
    }

    fn inside(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let (mx, my) = self.mouse();
        let in_clip = !self.blocked && self.clip.is_none_or(|[cx, cy, cw, ch]| mx >= cx && mx < cx + cw && my >= cy && my < cy + ch);
        in_clip && mx >= x && mx < x + w && my >= y && my < y + h
    }

    /// A click inside the rectangle, taken.
    fn clicked(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let hit = self.input.click && self.inside(x, y, w, h);
        if hit {
            self.input.click = false;
        }
        hit
    }

    fn button(&mut self, x: f32, y: f32, w: f32, label: &str) -> bool {
        let hot = self.inside(x, y, w, ROW);
        self.fill(x, y, w, ROW, if hot { HOT } else { BOX });
        let width = self.font.width(label);
        self.text(x + (w - width) / 2.0, y + 3.0, w, label, if hot { GOLD } else { TEXT });
        self.clicked(x, y, w, ROW)
    }

    /// A label and a text field after it; true when what was typed is kept.
    fn field(&mut self, tag: &str, n: usize, x: f32, y: f32, w: f32, value: &mut String) -> bool {
        let id = fid(tag, n);
        let color = self.tint.take().unwrap_or(TEXT);
        let editing = self.edit.focus == Some(id);
        let hot = self.inside(x, y, w, ROW);
        self.fill(x, y, w, ROW, if editing { SELECTED } else if hot { HOT } else { BOX });
        let mut changed = false;
        if editing {
            let typed = std::mem::take(&mut self.input.typed);
            self.edit.buffer.extend(typed.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()));
            for key in std::mem::take(&mut self.input.keys) {
                match key {
                    Key::Backspace => {
                        self.edit.buffer.pop();
                    }
                    Key::Enter => {
                        *value = self.edit.buffer.clone();
                        (self.edit.focus, changed) = (None, true);
                    }
                    Key::Escape => self.edit.focus = None,
                }
            }
            // A click elsewhere keeps it too.
            if self.input.click && !self.inside(x, y, w, ROW) {
                *value = self.edit.buffer.clone();
                (self.edit.focus, changed) = (None, true);
            }
        }
        if self.edit.focus == Some(id) {
            self.edit.seen = true;
            let shown = format!("{}_", self.edit.buffer);
            let overflow = (self.font.width(&shown) - (w - 10.0)).max(0.0);
            self.text(x + 4.0 - overflow, y + 3.0, w - 6.0 + overflow, &shown, TEXT);
        } else {
            self.text(x + 4.0, y + 3.0, w - 8.0, value, color);
        }
        if self.input.click && self.inside(x, y, w, ROW) && self.edit.focus != Some(id) {
            self.input.click = false;
            self.edit.want = Some((id, value.clone()));
        }
        changed
    }

    /// Arrows that step through choices: -1 for the left one, 1 for the right one.
    fn arrows(&mut self, x: f32, y: f32, w: f32, shown: &str) -> i32 {
        let mut step = 0;
        if self.button(x, y, 20.0, "<") {
            step = -1;
        }
        if self.button(x + w - 20.0, y, 20.0, ">") {
            step = 1;
        }
        let width = self.font.width(shown);
        self.text(x + 24.0 + (w - 48.0 - width).max(0.0) / 2.0, y + 3.0, w - 48.0, shown, TEXT);
        step
    }

    fn say(&mut self, text: impl Into<String>, color: [f32; 4]) {
        *self.status = (text.into(), color);
    }
}

/// The next (or previous) of `options` after `current`, wrapping round.
fn step<'a>(options: &'a [String], current: &str, by: i32) -> &'a str {
    let at = options.iter().position(|o| o == current).unwrap_or(0) as i32;
    &options[(at + by).rem_euclid(options.len() as i32) as usize]
}

/// One line of a form: its label, and a text field for a string.
fn text_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut String) -> bool {
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    let changed = g.field(label, n, x + LABEL_WIDTH, *y, w - LABEL_WIDTH, value);
    *y += ROW + 3.0;
    changed
}

/// A line for a value that is written and read as text: changed when it was typed and read.
fn value_row<T: Display + FromStr>(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut T) -> bool {
    let mut text = value.to_string();
    if !text_row(g, y, x, w, label, n, &mut text) {
        return false;
    }
    match text.trim().parse::<T>() {
        Ok(v) => {
            *value = v;
            true
        }
        Err(_) => {
            g.say(format!("{label}: \"{text}\" is not a valid value"), RED);
            false
        }
    }
}

/// A line for an optional string: empty is none.
fn option_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut Option<String>) -> bool {
    let mut text = value.clone().unwrap_or_default();
    if !text_row(g, y, x, w, label, n, &mut text) {
        return false;
    }
    *value = Some(text).filter(|t| !t.trim().is_empty());
    true
}

/// A line for an optional number: empty is none.
fn option_value_row<T: Display + FromStr>(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut Option<T>) -> bool {
    let mut text = value.as_ref().map_or_else(String::new, T::to_string);
    if !text_row(g, y, x, w, label, n, &mut text) {
        return false;
    }
    if text.trim().is_empty() {
        *value = None;
        return true;
    }
    match text.trim().parse::<T>() {
        Ok(v) => {
            *value = Some(v);
            true
        }
        Err(_) => {
            g.say(format!("{label}: \"{text}\" is not a valid value (leave it empty for none)"), RED);
            false
        }
    }
}

/// A grey line under a row, saying what it means.
fn hint(g: &mut Gui, y: &mut f32, x: f32, text: &str) {
    let w = WIDTH - x - LABEL_WIDTH - 8.0;
    for row in wrapped(g, w, text) {
        g.text(x + LABEL_WIDTH, *y - 2.0, w, &row, DIM);
        *y += ROW - 2.0;
    }
}

/// A line for a list of strings, written with commas.
fn list_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut Vec<String>, kind: Option<Ref>) -> bool {
    let mut text = value.join(", ");
    let changed = match kind {
        Some(kind) => text_row_ref(g, y, x, w, label, n, &mut text, kind, Some("")),
        None => text_row(g, y, x, w, label, n, &mut text),
    };
    if !changed {
        return false;
    }
    *value = text.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    true
}

/// A line for a list of `id: count` pairs.
fn stacks_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut Vec<(String, u32)>) -> bool {
    let mut text = value.iter().map(|(id, count)| format!("{id}:{count}")).collect::<Vec<_>>().join(", ");
    if !text_row_ref(g, y, x, w, label, n, &mut text, Ref::Item, Some(":1")) {
        return false;
    }
    let parsed: Option<Vec<(String, u32)>> = text
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| {
            let (id, count) = t.split_once(':').unwrap_or((t, "1"));
            Some((id.trim().to_string(), count.trim().parse().ok()?))
        })
        .collect();
    match parsed {
        Some(p) => {
            *value = p;
            true
        }
        None => {
            g.say(format!("{label}: write item:count, item:count"), RED);
            false
        }
    }
}

/// A line of numbers written with commas (`count` of them).
fn numbers_row<T: Display + FromStr + Copy>(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut [T]) -> bool {
    let mut text = value.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ");
    if !text_row(g, y, x, w, label, n, &mut text) {
        return false;
    }
    let parsed: Vec<T> = text.split(',').filter_map(|t| t.trim().parse().ok()).collect();
    if parsed.len() != value.len() {
        g.say(format!("{label}: write {} numbers with commas", value.len()), RED);
        return false;
    }
    value.copy_from_slice(&parsed);
    true
}

/// A line of arrows over a few choices.
fn choice_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, options: &[String], current: &mut String) -> bool {
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    let by = g.arrows(x + LABEL_WIDTH, *y, (w - LABEL_WIDTH).min(260.0), current);
    *y += ROW + 3.0;
    if by != 0 {
        *current = step(options, current, by).to_string();
    }
    by != 0
}

/// A line with a switch.
fn switch_row(g: &mut Gui, y: &mut f32, x: f32, label: &str, value: &mut bool) -> bool {
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    let clicked = g.button(x + LABEL_WIDTH, *y, 60.0, if *value { "yes" } else { "no" });
    *y += ROW + 3.0;
    if clicked {
        *value = !*value;
    }
    clicked
}

/// A list of lines to choose one of, scrolled by the wheel, with a field above it that narrows it to the lines that have all the words
/// typed (the first two of its `rows` are for the field); true when the choice changed. `pick.selected` is a line of `lines`.
/// How far from the editor's eye the creatures are that it lists and can pick in the world, yards.
const LISTED_RANGE: f32 = 300.0;

fn near_here(at: (f32, f32), me: Place) -> bool {
    (at.0 - me.north).hypot(at.1 - me.west) <= LISTED_RANGE
}

fn list(g: &mut Gui, x: f32, y: f32, w: f32, rows: usize, lines: &[String], pick: &mut Pick) -> bool {
    let find_id = fid("listfind", pick.salt);
    g.field("listfind", pick.salt, x, y, w - 6.0, &mut pick.find);
    let typed = if g.edit.focus == Some(find_id) { g.edit.buffer.clone() } else { pick.find.clone() };
    if typed.is_empty() && g.edit.focus != Some(find_id) {
        g.text(x + 4.0, y + 3.0, w - 14.0, "Find...", DIM);
    }
    let (y, rows) = (y + ROW + 3.0, rows.saturating_sub(2));
    let words: Vec<String> = typed.split_whitespace().map(str::to_ascii_lowercase).collect();
    let shown: Vec<usize> = (0..lines.len()).filter(|&i| !lines[i].is_empty() && words.iter().all(|word| lines[i].to_ascii_lowercase().contains(word))).collect();
    let h = rows as f32 * ROW;
    g.fill(x, y, w, h, [0.0, 0.0, 0.0, 0.4]);
    if g.inside(x, y, w, h) {
        let wheel = std::mem::take(&mut g.input.wheel);
        pick.scroll = (pick.scroll as f32 - wheel * 3.0).clamp(0.0, shown.len().saturating_sub(rows) as f32) as usize;
    }
    if std::mem::take(&mut pick.reveal) {
        if let Some(at) = shown.iter().position(|&i| i == pick.selected) {
            pick.scroll = at.saturating_sub(rows / 2);
        }
    }
    pick.scroll = pick.scroll.min(shown.len().saturating_sub(rows));
    let mut changed = false;
    for (row, &i) in shown.iter().skip(pick.scroll).take(rows).enumerate() {
        let ry = y + row as f32 * ROW;
        let hot = g.inside(x, ry, w - 6.0, ROW);
        if i == pick.selected {
            g.fill(x, ry, w - 6.0, ROW, SELECTED);
        } else if hot {
            g.fill(x, ry, w - 6.0, ROW, HOT);
        }
        g.text(x + 4.0, ry + 3.0, w - 10.0, &lines[i], if i == pick.selected { GOLD } else { TEXT });
        if g.clicked(x, ry, w - 6.0, ROW) && i != pick.selected {
            pick.selected = i;
            changed = true;
            g.edit.focus = None;
        }
    }
    // The bar: where the visible part is.
    if shown.len() > rows {
        let (top, size) = (pick.scroll as f32 / shown.len() as f32, rows as f32 / shown.len() as f32);
        g.fill(x + w - 4.0, y + top * h, 3.0, size * h, DIM);
    }
    if shown.is_empty() {
        g.text(x + 4.0, y + 3.0, w - 10.0, "Nothing matches.", DIM);
    }
    changed
}

/// An id that no one of `ids` has, from `base`.
fn fresh(ids: &[&str], base: &str) -> String {
    (1..).map(|n| format!("{base}_{n}")).find(|id| !ids.contains(&id.as_str())).expect("there is a number left")
}

impl Editor {
    pub fn new(renderer: &mut Renderer, gpu: &Gpu, font: &fontdue::Font, mount: Arc<mpq::Mount>) -> Result<Editor> {
        let white = renderer.ui.add_texture(gpu, 1, 1, &[255; 4]);
        let poi_icons = mount.read(&format!("{}.blp", crate::compass::POI_ICONS)).ok().and_then(|d| wowfmt::blp::Blp::parse(&d).ok()).map(|blp| renderer.ui.add_texture(gpu, blp.width, blp.height, &blp.mips[0]));
        let mut salt = 0;
        let mut pick = || {
            salt += 1;
            Pick { selected: 0, scroll: 0, form_scroll: 0.0, find: String::new(), salt, reveal: false }
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
            pick: [pick(), pick(), pick(), pick(), pick(), pick(), pick(), pick()],
            map: "Azeroth",
            around: Place { north: 0.0, west: 0.0, height: 0.0, yaw: 0.0, map: "Azeroth" },
            data: None,
            placer: {
                let models = Models::new(mount.clone());
                Placer { gallery: Gallery::new(mount.clone(), models.cache.clone()), models, mount, files: None, model: String::new(), placing: false, view: None, memory: None, tool: Mode::Move, drag: None, drag_actor: false, hot: None, hover: None, hold: false, out: Vec::new(), aim: Aim::Prop, creature: String::new(), chest: String::new(), ghosted: false, turn: 0.0, scale: 1.0, lift: 0.0 }
            },
            sent: content::get().props.clone(),
            eye: None,
            save_apply: false,
            spawn_ground: Vec::new(),
            chest_ground: Vec::new(),
            input: Input::default(),
            edit: Edit::default(),
            status: ("F1 closes the editor. Click a field to type in it; Enter keeps it.".into(), DIM),
            kind: 0,
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
            _ => 0,
        };
        let i = match self.tab {
            Tab::World => [0, 1, 6, 7][self.sub],
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
                self.status = (format!("Saved to content/editor.ron{}. Apply uses it in the game now.", if props { " and content/props.ron" } else { "" }), GREEN);
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
            _ => return None,
        };
        let at = self.pick[[0, 1, 0, 7][self.sub]].selected;
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
        let Some((_, found)) = best else { return false };
        let (sub, pick, i) = match found {
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
                None => None,
            },
            // Put down: what the next click would make, where the pointer touches the world.
            (_, p) if self.open && p.placing => p.hover.and_then(|at| match p.aim {
                Aim::Prop if placeable(&p.model) && p.mount.exists(&p.model) => {
                    let [north, west, height] = render_to_wow(at + Vec3::Y * p.lift);
                    Some(Want::Prop(Prop { model: p.model.clone(), map: None, zone: None, pickup: None, sleep: false, physics: false, at: (north, west, height), rotation: [0.0, p.turn, 0.0], scale: p.scale }))
                }
                Aim::Prop => None,
                Aim::Chest => Some(Want::Chest(p.chest.clone(), at, p.turn.to_radians())),
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
                d.cur.props.push(Prop { model: self.placer.model.clone(), map: None, zone: None, pickup: None, sleep: false, physics: false, at: (north, west, height + lift), rotation: [0.0, turn, 0.0], scale });
                let p = &mut self.pick[7];
                p.selected = d.cur.props.len() - 1;
                p.scroll = d.cur.props.len().saturating_sub(10);
            }
            Aim::Creature => {
                d.cur.spawns.push(Spawn { creature: self.placer.creature.clone(), map: Some(self.map.to_string()), at: (north, west), facing: turn, wander: crate::content::DEFAULT_WANDER, height: None, respawn: None, path: Vec::new() });
                let p = &mut self.pick[0];
                p.selected = d.cur.spawns.len() - 1;
                p.reveal = true;
            }
            Aim::Stop => {
                if let Some(s) = d.cur.spawns.get_mut(self.pick[0].selected) {
                    s.path.push(Waypoint { at: (north, west), wait: 0.0, facing: None, steps: Vec::new() });
                }
            }
            Aim::Chest => {
                // Like the chest selected (its lock and what it holds), else a plain one that holds something.
                let mut chest = d.cur.chests.get(self.pick[1].selected).or(d.cur.chests.first()).cloned().unwrap_or_else(|| content::ChestDef { id: String::new(), model: String::new(), at: (0.0, 0.0), facing: 0.0, lock: Some(Level::Novice), loot: "chest_poor".into(), copper: 0, items: Vec::new() });
                let ids: Vec<&str> = d.cur.chests.iter().map(|c| c.id.as_str()).collect();
                chest.id = fresh(&ids, "chest");
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
        }
        self.gizmo(ui, eye, size.1, ground);
        // A creature or a chest that has no ghost to show gets a mark on the ground with an arrow for the way it will face.
        if let (true, Some(at), false) = (self.placer.placing, self.placer.hover, self.placer.ghosted || self.placer.aim == Aim::Prop) {
            let color = match self.placer.aim {
                Aim::Chest => [1.0, 0.6, 0.2, 1.0],
                Aim::Stop => [0.4, 0.7, 1.0, 1.0],
                _ => [0.4, 1.0, 0.4, 1.0],
            };
            propedit::draw_marker(ui, self.white, &eye, at, self.placer.turn, color);
        }
        self.patrol_overlay(ui, size, eye, ground);
        let (save, apply, revert) = {
            let s = size.1 as f32 / UI_HEIGHT;
            let Editor { font, white, poi_icons, tab, sub, pick, data, input, edit, status, kind, preview, modal, placer, .. } = self;
            let data = data.as_mut().expect("checked above");
            let ids = Ids::of(&data.cur);
            let mut g = Gui { ui, font, white: *white, poi_icons: *poi_icons, s, input, edit, status, clip: None, blocked: modal.open.is_some() || modal.icons.is_some() || placer.view.is_some(), modal, ids: &ids, tint: None };
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
            let (top, bottom) = (36.0, UI_HEIGHT - 52.0);
            match *tab {
                Tab::World => world(&mut g, data, pick, sub, kind, placer, player, around, &mut actions, top, bottom),
                Tab::Creatures => creatures(&mut g, data, &mut pick[2], preview, top, bottom),
                Tab::Quests => quests(&mut g, data, &mut pick[3], player, top, bottom),
                Tab::Items => items(&mut g, data, &mut pick[4], preview, top, bottom),
                Tab::Loot => loot(&mut g, data, &mut pick[5], top, bottom),
            }
            // The bottom: save, apply, revert, and what happened.
            let y = UI_HEIGHT - 44.0;
            let dirty = data.cur != data.saved;
            let save = g.button(8.0, y, 110.0, if dirty { "Save *" } else { "Save" });
            let apply = g.button(124.0, y, 150.0, "Apply (live)");
            let revert = g.button(280.0, y, 110.0, "Revert");
            let (text, color) = g.status.clone();
            g.text(8.0, y + 24.0, WIDTH - 16.0, &text, color);
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
                Ok(()) => self.status = ("Back to what is saved.".into(), DIM),
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

/// The places tab: where creatures and chests stand, and the places the compass shows.
#[allow(clippy::too_many_arguments)]
fn world(g: &mut Gui, d: &mut Data, pick: &mut [Pick; 8], sub: &mut usize, kind: &mut usize, placer: &mut Placer, me: Place, around: Place, actions: &mut Vec<Action>, top: f32, bottom: f32) {
    let rows = ((bottom - top - 28.0) / ROW) as usize;
    for (i, name) in ["Creatures", "Chests", "Discoveries", "Props"].into_iter().enumerate() {
        if g.button(8.0 + i as f32 * 114.0, top, 108.0, name) {
            *sub = i;
        }
    }
    g.fill(8.0 + *sub as f32 * 114.0, top + ROW, 108.0, 2.0, GOLD);
    let (x, w) = (LIST_WIDTH + 24.0, WIDTH - LIST_WIDTH - 32.0);
    let near = |n: f32, west: f32| ((n - me.north).powi(2) + (west - me.west).powi(2)).sqrt();
    if *sub == 0 {
        // Of the world's tens of thousands only those near are listed (a line left empty is not shown).
        let lines: Vec<String> = d.cur.spawns.iter().map(|s| if near_here(s.at, around) { format!("{} {:.0}yd  ({:.0}, {:.0})", s.creature, near(s.at.0, s.at.1), s.at.0, s.at.1) } else { String::new() }).collect();
        let p = &mut pick[0];
        list(g, 8.0, top + 28.0, LIST_WIDTH, rows, &lines, p);
        let kinds: Vec<String> = d.cur.creatures.iter().map(|c| c.id.clone()).collect();
        let selected = p.selected;
        let (mut delete, mut added) = (false, false);
        // The form scrolls: the patrol can be long.
        form(g, p, x, top + 28.0, w, bottom, |g, y, _| {
            if let Some(s) = d.cur.spawns.get_mut(selected) {
                g.text(x, *y, w, "The selected creature", GOLD);
                *y += ROW + 4.0;
                text_row_ref(g, y, x, w, "Kind", 4, &mut s.creature, Ref::Creature, None);
                let before = s.at;
                value_row(g, y, x, w, "North", 0, &mut s.at.0);
                value_row(g, y, x, w, "West", 1, &mut s.at.1);
                if s.at != before {
                    s.height = None;
                }
                value_row(g, y, x, w, "Facing", 2, &mut s.facing);
                // How far from here it strolls, in yards (0 stands still).
                value_row(g, y, x, w, "Wander", 3, &mut s.wander);
                s.wander = s.wander.max(0.0);
                option_value_row(g, y, x, w, "Back after (s)", 5, &mut s.respawn);
                hint(g, y, x, "Seconds from its death until it is back; empty: as long as its kind says.");
                option_row(g, y, x, w, "Map", 6, &mut s.map);
                hint(g, y, x, "The map it stands in (Azeroth, Kalimdor, a dungeon's name); empty: in every map.");
                if g.button(x, *y, 150.0, "Here (where I stand)") {
                    (s.at, s.facing, s.height, s.map) = ((me.north, me.west), me.yaw, None, Some(me.map.to_string()));
                }
                if g.button(x + 156.0, *y, 90.0, "Go to it") {
                    actions.push(Action::Teleport(s.at.0, s.at.1));
                }
                *y += ROW + 8.0;
                patrol_form(g, y, x, w, s, me, actions, placer);
                if g.button(x, *y, 100.0, "Delete") {
                    delete = true;
                }
                *y += ROW + 8.0;
                gizmo_row(g, y, x, w, placer, false);
            }
            *y += 12.0;
            g.text(x, *y, w, "Put a new creature down", GOLD);
            *y += ROW + 4.0;
            *kind = (*kind).min(kinds.len().saturating_sub(1));
            let mut current = kinds.get(*kind).cloned().unwrap_or_default();
            // What is chosen in the window starts the putting down at once, as for props.
            let chosen = matches!(&g.modal.picked, Some((t, _)) if *t == fid("pick", 5));
            if text_row_ref(g, y, x, w, "Kind", 5, &mut current, Ref::Creature, None) {
                *kind = kinds.iter().position(|k| *k == current).unwrap_or(0);
            }
            if chosen {
                placer.start(Aim::Creature, &current, me.yaw);
            }
            if placer.placing && placer.aim == Aim::Creature {
                if g.button(x, *y, 260.0, "Stop putting down (Esc)") {
                    placer.placing = false;
                }
            } else if g.button(x, *y, 260.0, "Click in the world to put it down") {
                placer.start(Aim::Creature, &current, me.yaw);
            }
            *y += ROW + 6.0;
            if g.button(x, *y, 220.0, "New here, facing as I do") {
                d.cur.spawns.push(Spawn { creature: current, map: Some(me.map.to_string()), at: (me.north, me.west), facing: me.yaw, wander: crate::content::DEFAULT_WANDER, height: None, respawn: None, path: Vec::new() });
                added = true;
            }
            *y += ROW + 12.0;
        });
        if delete {
            d.cur.spawns.remove(selected);
            p.selected = selected.saturating_sub(1);
        }
        if added {
            p.selected = d.cur.spawns.len() - 1;
            p.reveal = true;
        }
    } else if *sub == 1 {
        let lines: Vec<String> = d.cur.chests.iter().map(|c| format!("{} {:.0}yd  ({:.0}, {:.0})", c.id, near(c.at.0, c.at.1), c.at.0, c.at.1)).collect();
        let p = &mut pick[1];
        list(g, 8.0, top + 28.0, LIST_WIDTH, rows, &lines, p);
        let locks: Vec<String> = ["None", "Novice", "Apprentice", "Adept", "Expert", "Master"].iter().map(|s| s.to_string()).collect();
        let mut y = top + 28.0;
        let ids: Vec<String> = d.cur.chests.iter().map(|c| c.id.clone()).collect();
        let selected = p.selected;
        if let Some(c) = d.cur.chests.get_mut(selected) {
            g.text(x, y, w, "The selected chest", GOLD);
            y += ROW + 4.0;
            text_row(g, &mut y, x, w, "Id", 10, &mut c.id);
            text_row(g, &mut y, x, w, "Model", 11, &mut c.model);
            let mut lock = c.lock.map_or("None".to_string(), |l| l.name().to_string());
            if choice_row(g, &mut y, x, w, "Lock", &locks, &mut lock) {
                c.lock = [Level::Novice, Level::Apprentice, Level::Adept, Level::Expert, Level::Master].into_iter().find(|l| l.name() == lock);
            }
            text_row_ref(g, &mut y, x, w, "Loot table", 12, &mut c.loot, Ref::Loot, None);
            g.text(x + LABEL_WIDTH, y - 2.0, w, "(by chance; may be left empty)", DIM);
            y += ROW - 2.0;
            value_row(g, &mut y, x, w, "Always copper", 16, &mut c.copper);
            stacks_row(g, &mut y, x, w, "Always holds", 17, &mut c.items);
            value_row(g, &mut y, x, w, "North", 13, &mut c.at.0);
            value_row(g, &mut y, x, w, "West", 14, &mut c.at.1);
            value_row(g, &mut y, x, w, "Facing", 15, &mut c.facing);
            if g.button(x, y, 150.0, "Here (where I stand)") {
                (c.at, c.facing) = ((me.north, me.west), me.yaw);
            }
            if g.button(x + 156.0, y, 90.0, "Go to it") {
                actions.push(Action::Teleport(c.at.0, c.at.1));
            }
            y += ROW + 8.0;
            if g.button(x, y, 100.0, "Delete") {
                d.cur.chests.remove(selected);
                p.selected = selected.saturating_sub(1);
            } else if g.button(x + 106.0, y, 220.0, "Copy it here, facing as I do") {
                let mut copy = d.cur.chests[selected].clone();
                copy.id = fresh(&ids.iter().map(String::as_str).collect::<Vec<_>>(), "chest");
                (copy.at, copy.facing) = ((me.north, me.west), me.yaw);
                d.cur.chests.push(copy);
                p.selected = d.cur.chests.len() - 1;
            }
            y += ROW + 8.0;
            gizmo_row(g, &mut y, x, w, placer, false);
        }
        let mut y = top + 28.0 + 15.0 * (ROW + 3.0) + 70.0;
        g.text(x, y, w, "Put new chests down (as the selected one: its lock and what it holds)", GOLD);
        y += ROW + 4.0;
        if placer.chest.is_empty() {
            placer.chest = CHEST_MODELS[0].0.to_string();
        }
        g.text(x, y + 3.0, LABEL_WIDTH, "Model", DIM);
        let tag = fid("pick", 60);
        g.field("Chest model", 60, x + LABEL_WIDTH, y, w - LABEL_WIDTH - BROWSE - 4.0, &mut placer.chest);
        if let Some(model) = g.picked(tag) {
            placer.chest = model.clone();
            placer.start(Aim::Chest, &model, me.yaw);
        }
        if g.button(x + w - BROWSE, y, BROWSE, "...") {
            g.modal.want = Some((tag, Pickable::Chests, placer.chest.clone()));
        }
        y += ROW + 6.0;
        if placer.placing && placer.aim == Aim::Chest {
            if g.button(x, y, 260.0, "Stop putting down (Esc)") {
                placer.placing = false;
            }
        } else if g.button(x, y, 260.0, "Click in the world to put it down") {
            let model = placer.chest.clone();
            placer.start(Aim::Chest, &model, me.yaw);
        }
    } else if *sub == 2 {
        pois(g, d, &mut pick[6], me, actions, top, rows, x, w);
    } else {
        props(g, d, &mut pick[7], placer, me, actions, top, rows, x, w);
    }
    // Esc ends the putting down, and so does leaving the tab of what is put down.
    if placer.placing && !g.blocked && g.input.keys.contains(&Key::Escape) {
        placer.placing = false;
    }
    placer.placing &= matches!((placer.aim, *sub), (Aim::Creature | Aim::Stop, 0) | (Aim::Chest, 1) | (Aim::Prop, 3));
}

/// What the mouse and keys do in the world while the editor is open, over the world beside the window (the lines for what is going on
/// come first).
fn hud(g: &mut Gui, placer: &Placer, props: bool, width: f32) {
    let mut lines: Vec<(String, [f32; 4])> = Vec::new();
    if placer.placing && placer.aim == Aim::Stop {
        lines.push(("Adding stops to the patrol: every click in the world adds one stop there (a blue mark shows where). Esc stops.".to_string(), GOLD));
    } else if placer.placing && placer.aim != Aim::Prop {
        let what = if placer.aim == Aim::Creature { placer.creature.as_str() } else { file_stem(&placer.chest) };
        lines.push((format!("Putting down {what}: click stands it there, facing the way the arrow points. Hold the right button and move the mouse to turn it ({:.0} deg). Esc stops.", placer.turn), GOLD));
    } else if placer.placing {
        lines.push((format!("Putting down {}: click puts it there. Hold the right button and move the mouse to turn it ({:.0} deg), the wheel sizes it (x{:.2}), Shift + wheel raises it ({:+.2}). Esc stops.", file_stem(&placer.model), placer.turn, placer.scale, placer.lift), GOLD));
    } else if props {
        lines.push(("Click a model to select it, then drag the arrows, the ring or the knob of the gizmo (Move, Turn, Scale in the window).".to_string(), GOLD));
    }
    lines.push(("Camera: hold the middle or right button and move the mouse to look, W A S D fly, Space up, Ctrl down, Shift faster, the wheel sets the speed. G: drop to the ground. Ctrl + S: save and apply.".to_string(), TEXT));
    let x = WIDTH + 12.0;
    let w = (width - x - 12.0).max(100.0);
    let rows: Vec<(String, [f32; 4])> = lines.iter().flat_map(|(text, color)| wrapped(g, w, text).into_iter().map(move |row| (row, *color))).collect();
    let top = UI_HEIGHT - 18.0 - rows.len() as f32 * ROW;
    g.fill(x - 6.0, top - 4.0, w + 12.0, rows.len() as f32 * ROW + 10.0, [0.0, 0.0, 0.0, 0.55]);
    for (i, (row, color)) in rows.iter().enumerate() {
        g.text(x, top + i as f32 * ROW, w, row, *color);
    }
}

/// `text` broken at spaces into rows that are at most `w` units wide.
fn wrapped(g: &Gui, w: f32, text: &str) -> Vec<String> {
    let mut rows = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let longer = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if g.font.width(&longer) > w && !line.is_empty() {
            rows.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = longer;
        }
    }
    rows.push(line);
    rows
}

/// The doodads and buildings of the game that can be put into the world, and the one that clicks in the world put down.
struct Placer {
    mount: Arc<mpq::Mount>,
    /// The files, listed once when the gallery is first opened.
    files: Option<Vec<String>>,
    model: String,
    placing: bool,
    gallery: Gallery,
    /// The gallery window, while it is open, and what it showed when it was last closed.
    view: Option<View>,
    memory: Option<Memory>,
    models: Models,
    /// What the gizmo does, the part of it being dragged, and the part under the pointer.
    tool: Mode,
    drag: Option<Drag>,
    /// What is dragged is a creature or a chest: it is applied (saved) when it is let go.
    drag_actor: bool,
    hot: Option<Handle>,
    /// Where the pointer touches the world while models are put down, and whether the ghost of a dragged prop stays until the world shows it.
    hover: Option<Vec3>,
    hold: bool,
    /// What the editor asks of the game besides what a frame's buttons do.
    out: Vec<Action>,
    /// What clicks in the world put down: a prop (`model`), a creature (`creature`) or a chest (`chest`, its model).
    aim: Aim,
    creature: String,
    chest: String,
    /// Whether the ghost of what is put down is shown (else a mark on the ground shows where it will stand).
    ghosted: bool,
    /// How the model that is put down is turned (degrees), how big it is and how far above the ground it stands.
    turn: f32,
    scale: f32,
    lift: f32,
}

impl Placer {
    fn files(&mut self) -> &[String] {
        let mount = &self.mount;
        self.files.get_or_insert_with(|| mount.list().into_iter().filter(|f| placeable(f)).collect())
    }

    /// Start putting a creature or a chest down: the next clicks in the world stand it there, facing as `facing`.
    fn start(&mut self, aim: Aim, what: &str, facing: f32) {
        match aim {
            Aim::Creature => self.creature = what.to_string(),
            Aim::Chest => self.chest = what.to_string(),
            Aim::Prop | Aim::Stop => {}
        }
        (self.aim, self.placing, self.turn) = (aim, true, facing);
    }

    /// Open the gallery on the model `current`.
    fn open_gallery(&mut self, g: &mut Gui, current: &str) {
        let files = self.files().to_vec();
        let mut view = View::new(&files, current);
        if let Some(m) = &self.memory {
            (view.find, view.folders.selected, view.folders.scroll, view.folders.find, view.scroll) = (m.find.clone(), m.folder, m.folder_scroll, m.folder_find.clone(), m.scroll);
        }
        // What is typed goes to its find field at once.
        g.edit.want = Some((fid("gallery", 0), view.find.clone()));
        self.view = Some(view);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Aim {
    Prop,
    Creature,
    Chest,
    /// A stop of the patrol of the creature selected.
    Stop,
}

/// The chest models the game can open and show (the picks and the opening animation are those of these two).
const CHEST_MODELS: [(&str, &str); 2] = [(r"World\Generic\ActiveDoodads\Chest01\Chest01.m2", "Chest01"), (r"World\Generic\ActiveDoodads\Chest03\Chest03.m2", "Chest03")];

/// The gallery window: the folders of the models on the left, the pictures of the models of the folder on the right.
struct View {
    find: String,
    /// The folders as lines for `list` (the first is all models); the one chosen is `folders.selected`.
    lines: Vec<String>,
    folders: Pick,
    /// For each file: its lower-case name to search in, and its folder (an index of `keys`).
    index: Vec<(String, usize)>,
    /// The first row of pictures on show.
    scroll: usize,
    selected: Option<String>,
    /// For the creatures and the chests: the window has their pictures in a grid and no folders.
    entries: Option<Entries>,
}

/// What a field chooses from in the gallery: its tag (the answer goes to it), the title and the entries.
struct Entries {
    tag: u64,
    title: &'static str,
    list: Vec<GalleryEntry>,
    /// The window stays when one is chosen (the field is for what is put down: more can be chosen, and put down).
    keep_open: bool,
}

struct GalleryEntry {
    /// What is given back, what is shown under the picture, the lower-case words that are searched, the key of the picture and what it
    /// is made of (none: a model file, which is the key).
    value: String,
    label: String,
    search: String,
    key: String,
    subject: Option<Subject>,
}

impl View {
    fn of_entries(tag: u64, title: &'static str, list: Vec<GalleryEntry>, keep_open: bool, current: &str) -> View {
        let selected = Some(current.to_string()).filter(|c| list.iter().any(|e| e.value == *c));
        View { find: String::new(), lines: Vec::new(), folders: Pick { selected: 0, scroll: 0, form_scroll: 0.0, find: String::new(), salt: 91, reveal: false }, index: Vec::new(), scroll: 0, selected, entries: Some(Entries { tag, title, list, keep_open }) }
    }

    fn new(files: &[String], current: &str) -> View {
        // The folders by their lower-case name (the archives spell some of them in capitals): how they are written, and how many models.
        let mut counts = std::collections::BTreeMap::<String, (String, usize)>::new();
        for f in files {
            let name = category_of(f);
            let entry = counts.entry(name.to_ascii_lowercase()).or_insert_with(|| (name.clone(), 0));
            entry.1 += 1;
            let lower = |s: &str| s.bytes().filter(u8::is_ascii_lowercase).count();
            if lower(&name) > lower(&entry.0) {
                entry.0 = name;
            }
        }
        let all = String::new();
        let keys: Vec<&String> = std::iter::once(&all).chain(counts.keys()).collect();
        let lines = std::iter::once(format!("All models ({})", files.len())).chain(counts.values().map(|(name, n)| format!("({n}) {name}"))).collect();
        let position = |file: &str| keys.iter().position(|k| **k == category_of(file).to_ascii_lowercase()).expect("every folder is listed");
        let index = files.iter().map(|f| (f.to_ascii_lowercase(), position(f))).collect();
        let selected = Some(current.to_string()).filter(|c| files.iter().any(|f| f.eq_ignore_ascii_case(c)));
        // The folder of the model that is chosen now, if there is one.
        let at = selected.as_deref().map_or(0, position);
        View { find: String::new(), lines, folders: Pick { selected: at, scroll: at.saturating_sub(8), form_scroll: 0.0, find: String::new(), salt: 90, reveal: false }, index, scroll: 0, selected, entries: None, }
    }
}

/// What the model gallery showed when it was closed (the words, the folder and how far it was scrolled), so that it opens on the same again.
struct Memory {
    find: String,
    folder: usize,
    folder_scroll: usize,
    folder_find: String,
    scroll: usize,
}

impl Memory {
    fn of(v: &View) -> Memory {
        Memory { find: v.find.clone(), folder: v.folders.selected, folder_scroll: v.folders.scroll, folder_find: v.folders.find.clone(), scroll: v.scroll }
    }
}

/// The folder of a model in the gallery: its place under `World`, three levels at most, without the folders that every doodad has.
fn category_of(file: &str) -> String {
    let dirs: Vec<&str> = file.split('\\').skip(1).collect();
    dirs[..dirs.len().saturating_sub(1)].iter().filter(|d| !d.eq_ignore_ascii_case("passivedoodads")).take(3).copied().collect::<Vec<_>>().join("\\")
}

/// Pictures of the gallery in a row and in a column.
const GALLERY_COLUMNS: usize = 4;
const GALLERY_ROWS: usize = 4;

/// The gallery window (drawn over everything else of the editor): the models as pictures in a grid, narrowed by a folder and by the words
/// typed. A second click on a picture or Enter chooses it, Esc or "Cancel" does not.
fn gallery(g: &mut Gui, placer: &mut Placer) {
    let Some(mut v) = placer.view.take() else { return };
    if v.entries.is_some() {
        return entries_window(g, placer, v);
    }
    let (px, py, pw, ph) = (30.0, 40.0, 660.0, 680.0);
    g.fill(0.0, 0.0, WIDTH, UI_HEIGHT, [0.0, 0.0, 0.0, 0.6]);
    g.fill(px, py, pw, ph, [0.07, 0.07, 0.1, 1.0]);
    g.text(px + 16.0, py + 12.0, pw - 32.0, "Choose a model (doodads and buildings)", GOLD);
    let (enter, esc) = (g.input.keys.contains(&Key::Enter), g.input.keys.contains(&Key::Escape));
    g.input.keys.retain(|k| !matches!(k, Key::Enter | Key::Escape));
    let find_id = fid("gallery", 0);
    g.text(px + 16.0, py + 43.0, 40.0, "Find", DIM);
    g.field("gallery", 0, px + 56.0, py + 40.0, 300.0, &mut v.find);
    let typed = if g.edit.focus == Some(find_id) { g.edit.buffer.clone() } else { v.find.clone() };
    let words: Vec<String> = typed.split_whitespace().map(str::to_ascii_lowercase).collect();
    // The folders on the left.
    let before = v.folders.selected;
    list(g, px + 16.0, py + 72.0, 190.0, ((ph - 120.0) / ROW) as usize, &v.lines, &mut v.folders);
    if v.folders.selected != before {
        v.scroll = 0;
    }
    let folder = v.folders.selected;
    let files = placer.files.as_deref().unwrap_or(&[]);
    let shown: Vec<&String> = v.index.iter().enumerate().filter(|(_, (name, k))| (folder == 0 || *k == folder) && words.iter().all(|w| name.contains(w))).map(|(i, _)| &files[i]).collect();
    // The pictures on the right.
    let (gx, gy, cw, ch) = (px + 16.0 + 190.0 + 14.0, py + 72.0, 106.0, 124.0);
    let rows = shown.len().div_ceil(GALLERY_COLUMNS);
    if g.inside(gx, gy, cw * GALLERY_COLUMNS as f32, ch * GALLERY_ROWS as f32) {
        let wheel = std::mem::take(&mut g.input.wheel);
        v.scroll = (v.scroll as f32 - wheel).max(0.0) as usize;
    }
    v.scroll = v.scroll.min(rows.saturating_sub(GALLERY_ROWS));
    let mut confirm = None;
    let mut hovered: Option<&String> = None;
    for (n, file) in shown.iter().skip(v.scroll * GALLERY_COLUMNS).take(GALLERY_COLUMNS * GALLERY_ROWS).enumerate() {
        let (cx, cy) = (gx + (n % GALLERY_COLUMNS) as f32 * cw, gy + (n / GALLERY_COLUMNS) as f32 * ch);
        let chosen = v.selected.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(file));
        let hot = g.inside(cx, cy, cw - 6.0, ch - 6.0);
        g.fill(cx, cy, cw - 6.0, ch - 6.0, if chosen { SELECTED } else if hot { HOT } else { BOX });
        match placer.gallery.picture(file) {
            Pic::Ready(texture) => g.picture(texture, cx + 2.0, cy + 2.0, 100.0, 100.0, [1.0; 4]),
            Pic::Loading => g.text(cx + 34.0, cy + 44.0, 60.0, "...", DIM),
            Pic::Empty => g.text(cx + 14.0, cy + 44.0, 80.0, "no picture", DIM),
        }
        g.text(cx + 4.0, cy + 105.0, cw - 14.0, file_stem(file), if chosen { GOLD } else { TEXT });
        if hot {
            hovered = Some(*file);
        }
        if g.clicked(cx, cy, cw - 6.0, ch - 6.0) {
            // A click on a picture chooses it, and the model is put down at once.
            confirm = Some((*file).clone());
            v.selected = Some((*file).clone());
        }
    }
    if rows > GALLERY_ROWS {
        let (area, top, size) = (ch * GALLERY_ROWS as f32, v.scroll as f32 / rows as f32, GALLERY_ROWS as f32 / rows as f32);
        g.fill(gx + cw * GALLERY_COLUMNS as f32 - 2.0, gy + top * area, 3.0, size * area, DIM);
    }
    if shown.is_empty() {
        g.text(gx + 4.0, gy + 3.0, 400.0, "Nothing matches.", DIM);
    }
    let below = gy + ch * GALLERY_ROWS as f32 + 4.0;
    g.text(gx, below, 430.0, &format!("{} models  (click one, then click in the world to put it down; the wheel scrolls)", shown.len()), DIM);
    g.text(gx, below + ROW, 430.0, hovered.map(|h| h.as_str()).or(v.selected.as_deref()).unwrap_or(""), TEXT);
    let closed = g.button(px + 16.0, py + ph - 34.0, 110.0, "Close");
    if esc || closed {
        g.edit.focus = None;
        g.edit.want = None;
        placer.memory = Some(Memory::of(&v));
        return;
    }
    // Choosing a model starts putting it down: the next click in the world, beside this window, stands it there. The window stays, to choose
    // another one.
    if let Some(file) = confirm.or(if enter { v.selected.clone() } else { None }) {
        (placer.model, placer.aim, placer.placing) = (file, Aim::Prop, true);
        g.edit.focus = None;
        g.edit.want = None;
    }
    placer.view = Some(v);
}

/// The gallery of creatures or chests: their pictures in a grid, narrowed by the words typed. Choosing one gives its value back to the field
/// that asked (as a chooser does).
fn entries_window(g: &mut Gui, placer: &mut Placer, mut v: View) {
    let Some(entries) = v.entries.take() else { return };
    const COLUMNS: usize = 6;
    let (px, py, pw, ph) = (30.0, 40.0, 660.0, 680.0);
    g.fill(0.0, 0.0, WIDTH, UI_HEIGHT, [0.0, 0.0, 0.0, 0.6]);
    g.fill(px, py, pw, ph, [0.07, 0.07, 0.1, 1.0]);
    g.text(px + 16.0, py + 12.0, pw - 32.0, entries.title, GOLD);
    let (enter, esc) = (g.input.keys.contains(&Key::Enter), g.input.keys.contains(&Key::Escape));
    g.input.keys.retain(|k| !matches!(k, Key::Enter | Key::Escape));
    let find_id = fid("gallery", 0);
    g.text(px + 16.0, py + 43.0, 40.0, "Find", DIM);
    g.field("gallery", 0, px + 56.0, py + 40.0, 300.0, &mut v.find);
    let typed = if g.edit.focus == Some(find_id) { g.edit.buffer.clone() } else { v.find.clone() };
    // A window that closes keeps the typing going; one that stays lets the keys go to the game when the field is left.
    if !entries.keep_open && g.edit.focus != Some(find_id) && g.edit.want.is_none() {
        g.edit.want = Some((find_id, v.find.clone()));
    }
    let words: Vec<String> = typed.split_whitespace().map(str::to_ascii_lowercase).collect();
    let shown: Vec<&GalleryEntry> = entries.list.iter().filter(|e| words.iter().all(|w| e.search.contains(w))).collect();
    let (gx, gy, cw, ch) = (px + 16.0, py + 72.0, 106.0, 124.0);
    let rows = shown.len().div_ceil(COLUMNS);
    if g.inside(gx, gy, cw * COLUMNS as f32, ch * GALLERY_ROWS as f32) {
        let wheel = std::mem::take(&mut g.input.wheel);
        v.scroll = (v.scroll as f32 - wheel).max(0.0) as usize;
    }
    v.scroll = v.scroll.min(rows.saturating_sub(GALLERY_ROWS));
    let mut confirm = None;
    let mut hovered: Option<&GalleryEntry> = None;
    for (n, entry) in shown.iter().skip(v.scroll * COLUMNS).take(COLUMNS * GALLERY_ROWS).enumerate() {
        let (cx, cy) = (gx + (n % COLUMNS) as f32 * cw, gy + (n / COLUMNS) as f32 * ch);
        let chosen = v.selected.as_deref() == Some(entry.value.as_str());
        let hot = g.inside(cx, cy, cw - 6.0, ch - 6.0);
        g.fill(cx, cy, cw - 6.0, ch - 6.0, if chosen { SELECTED } else if hot { HOT } else { BOX });
        let pic = match &entry.subject {
            Some(subject) => placer.gallery.picture_subject(&entry.key, subject),
            None => placer.gallery.picture(&entry.key),
        };
        match pic {
            Pic::Ready(texture) => g.picture(texture, cx + 2.0, cy + 2.0, 100.0, 100.0, [1.0; 4]),
            Pic::Loading => g.text(cx + 34.0, cy + 44.0, 60.0, "...", DIM),
            Pic::Empty => g.text(cx + 14.0, cy + 44.0, 80.0, "no picture", DIM),
        }
        g.text(cx + 4.0, cy + 105.0, cw - 14.0, &entry.label, if chosen { GOLD } else { TEXT });
        if hot {
            hovered = Some(*entry);
        }
        if g.clicked(cx, cy, cw - 6.0, ch - 6.0) {
            confirm = Some(entry.value.clone());
            v.selected = Some(entry.value.clone());
        }
    }
    if rows > GALLERY_ROWS {
        let (area, top, size) = (ch * GALLERY_ROWS as f32, v.scroll as f32 / rows as f32, GALLERY_ROWS as f32 / rows as f32);
        g.fill(gx + cw * COLUMNS as f32 - 2.0, gy + top * area, 3.0, size * area, DIM);
    }
    if shown.is_empty() {
        g.text(gx + 4.0, gy + 3.0, 400.0, "Nothing matches.", DIM);
    }
    let below = gy + ch * GALLERY_ROWS as f32 + 4.0;
    g.text(gx, below, 600.0, &format!("{} of {}  (click one to choose it{}; the wheel scrolls)", shown.len(), entries.list.len(), if entries.keep_open { ", then click in the world to put it down" } else { "" }), DIM);
    g.text(gx, below + ROW, 600.0, hovered.map(|h| h.value.as_str()).or(v.selected.as_deref()).unwrap_or(""), TEXT);
    let (chosen_now, cancelled) = (g.button(px + 16.0, py + ph - 34.0, 110.0, if entries.keep_open { "Close" } else { "Choose" }), g.button(px + 132.0, py + ph - 34.0, 110.0, "Cancel"));
    if esc || cancelled || (entries.keep_open && chosen_now) {
        g.edit.focus = None;
        g.edit.want = None;
        return;
    }
    if let Some(value) = confirm.or(if chosen_now || enter { v.selected.clone() } else { None }) {
        g.modal.picked = Some((entries.tag, value));
        g.edit.focus = None;
        g.edit.want = None;
        if !entries.keep_open {
            return;
        }
    }
    v.entries = Some(entries);
    placer.view = Some(v);
}

impl Placer {
    /// Open the gallery of creatures or chests for the field `tag`, on the one called `current`.
    fn open_entries(&mut self, g: &mut Gui, raws: &Raws, tag: u64, what: Pickable, current: &str) {
        let (title, list): (&'static str, Vec<GalleryEntry>) = match what {
            Pickable::Creatures => (
                "Choose a creature",
                raws.creatures
                    .iter()
                    .map(|c| {
                        let subject = match &c.body {
                            RawBody::Beast { model, skins, .. } => Subject::Beast { model: model.clone(), skins: skins.clone() },
                            RawBody::Humanoid { outfit, look } => Subject::Person { outfit: outfit.iter().filter_map(|id| crate::items::all().iter().position(|i| i.id == *id)).collect(), look: look.clone() },
                        };
                        GalleryEntry { value: c.id.clone(), label: c.name.clone(), search: format!("{} {}", c.id, c.name).to_ascii_lowercase(), key: format!("c|{}|{:?}", c.id, c.body), subject: Some(subject) }
                    })
                    .collect(),
            ),
            Pickable::Chests => ("Choose a chest", CHEST_MODELS.iter().map(|(model, name)| GalleryEntry { value: model.to_string(), label: name.to_string(), search: name.to_ascii_lowercase(), key: model.to_string(), subject: None }).collect()),
        };
        self.view = Some(View::of_entries(tag, title, list, tag == fid("pick", 5) || tag == fid("pick", 60), current));
        // What is typed goes to its find field at once.
        g.edit.want = Some((fid("gallery", 0), String::new()));
    }
}

/// Whether a file of the archives is a model that can stand in the world: a doodad, or a building without its groups (`name_000.wmo`).
fn placeable(file: &str) -> bool {
    let f = file.to_ascii_lowercase();
    let group = f.strip_suffix(".wmo").is_some_and(|s| s.len() > 4 && s.as_bytes()[s.len() - 4] == b'_' && s[s.len() - 3..].bytes().all(|b| b.is_ascii_digit()));
    f.starts_with(r"world\") && !f.starts_with(r"world\maps\") && (f.ends_with(".m2") || (f.ends_with(".wmo") && !group))
}

/// The props: models put into the world. The form changes the one selected; clicks in the world put the chosen model down.
#[allow(clippy::too_many_arguments)]
/// The buttons that choose what the gizmo does (a creature or a chest is moved and turned, a prop also scaled), and what to do with it.
fn gizmo_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, placer: &mut Placer, scale: bool) {
    g.text(x, *y + 3.0, 40.0, "Gizmo", DIM);
    for (i, (mode, name)) in [(Mode::Move, "Move"), (Mode::Turn, "Turn"), (Mode::Scale, "Scale")].into_iter().filter(|(m, _)| scale || *m != Mode::Scale).enumerate() {
        let bx = x + 48.0 + i as f32 * 66.0;
        if placer.tool == mode || (!scale && placer.tool == Mode::Scale && mode == Mode::Move) {
            g.fill(bx - 2.0, *y - 2.0, 64.0, ROW + 4.0, SELECTED);
        }
        if g.button(bx, *y, 60.0, name) {
            placer.tool = mode;
        }
    }
    g.text(x, *y + ROW + 4.0, w, "Click it in the world to select it, then drag the gizmo over it.", DIM);
    *y += 2.0 * ROW + 8.0;
}

/// The patrol of a creature: the places it walks to one after the other, round and round, and for each the seconds it stands there, the way it
/// turns and what it does there (the original's waypoint script). Stops are added where one stands or by clicking in the world, where the
/// patrol is drawn.
#[allow(clippy::too_many_arguments)]
fn patrol_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, s: &mut Spawn, me: Place, actions: &mut Vec<Action>, placer: &mut Placer) {
    g.text(x, *y, w, if s.path.is_empty() { "Patrol (none: it stays where it is or strolls)" } else { "Patrol (stop 1, 2, 3 ... and round again)" }, GOLD);
    *y += ROW + 4.0;
    let count = s.path.len();
    let (mut remove, mut swap) = (None, None);
    for (i, stop) in s.path.iter_mut().enumerate() {
        let base = 800 + i * 8;
        g.text(x, *y + 3.0, LABEL_WIDTH, &format!("Stop {}", i + 1), GOLD);
        let bx = x + LABEL_WIDTH;
        if g.button(bx, *y, 54.0, "Here") {
            stop.at = (me.north, me.west);
        }
        if g.button(bx + 58.0, *y, 40.0, "Go") {
            actions.push(Action::Teleport(stop.at.0, stop.at.1));
        }
        if g.button(bx + 102.0, *y, 40.0, "Up") && i > 0 {
            swap = Some((i - 1, i));
        }
        if g.button(bx + 146.0, *y, 50.0, "Down") && i + 1 < count {
            swap = Some((i, i + 1));
        }
        if g.button(bx + 200.0, *y, 64.0, "Remove") {
            remove = Some(i);
        }
        *y += ROW + 3.0;
        let mut place = [stop.at.0, stop.at.1];
        if numbers_row(g, y, x, w, "  north, west", base, &mut place) {
            stop.at = (place[0], place[1]);
        }
        value_row(g, y, x, w, "  waits (s)", base + 1, &mut stop.wait);
        stop.wait = stop.wait.max(0.0);
        option_value_row(g, y, x, w, "  turns to", base + 2, &mut stop.facing);
        steps_form(g, y, x, w, &mut stop.steps, 5000 + i * 100);
        *y += 4.0;
    }
    if let Some((a, b)) = swap {
        s.path.swap(a, b);
    }
    if let Some(i) = remove {
        s.path.remove(i);
    }
    if count > 0 {
        hint(g, y, x, "\"waits\": how long it stands there; \"turns to\": the heading while it does (empty: as it walks).");
    }
    if g.button(x + LABEL_WIDTH, *y, 220.0, "Add a stop here (where I stand)") {
        s.path.push(Waypoint { at: (me.north, me.west), wait: 0.0, facing: None, steps: Vec::new() });
    }
    *y += ROW + 3.0;
    if placer.placing && placer.aim == Aim::Stop {
        if g.button(x + LABEL_WIDTH, *y, 220.0, "Stop adding (Esc)") {
            placer.placing = false;
        }
    } else if g.button(x + LABEL_WIDTH, *y, 220.0, "Click in the world to add stops") {
        placer.start(Aim::Stop, "", me.yaw);
    }
    *y += ROW + 3.0;
    if count > 0 {
        if g.button(x + LABEL_WIDTH, *y, 220.0, "Clear the patrol") {
            s.path.clear();
        }
        *y += ROW + 3.0;
    }
    *y += 8.0;
}

/// What a spell line of an item does, one effect to a line: its kind chosen with the arrows, and its numbers under names. `base` keeps the
/// fields of this list apart from those of the others.
fn effects_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, effects: &mut Vec<UseEffect>, base: usize) {
    use UseEffect as E;
    const KINDS: [&str; 8] = ["Heals", "Restores mana", "Heals over time", "Restores mana over time", "Raises a stat", "Returns to the hearth", "Gives a mount", "Gives a bonus while worn"];
    let stats: Vec<String> = ["Strength", "Agility", "Stamina", "Intellect", "Spirit", "Armor", "Health"].iter().map(|s| s.to_string()).collect();
    let mut remove = None;
    for (j, e) in effects.iter_mut().enumerate().take(10) {
        let n = base + j * 10;
        let now = match e {
            E::Heal(..) => 0,
            E::Mana(..) => 1,
            E::HealOver(..) => 2,
            E::ManaOver(..) => 3,
            E::Boost(..) => 4,
            E::Hearth => 5,
            E::Mount(..) => 6,
            E::Equip(..) => 7,
        };
        g.text(x, *y + 3.0, LABEL_WIDTH, &format!("  does {}", j + 1), DIM);
        let by = g.arrows(x + LABEL_WIDTH, *y, 220.0, KINDS[now]);
        if g.button(x + LABEL_WIDTH + 230.0, *y, 70.0, "Remove") {
            remove = Some(j);
        }
        *y += ROW + 3.0;
        if by != 0 {
            *e = match (now as i32 + by).rem_euclid(KINDS.len() as i32) {
                0 => E::Heal(20, 30),
                1 => E::Mana(20, 30),
                2 => E::HealOver(100, 10.0),
                3 => E::ManaOver(100, 10.0),
                4 => E::Boost(2, 10, 60.0),
                5 => E::Hearth,
                6 => E::Mount(0, 60),
                _ => E::Equip(0, 0, 1),
            };
        }
        match e {
            E::Heal(low, high) | E::Mana(low, high) => {
                value_row(g, y, x, w, "    at least", n, low);
                value_row(g, y, x, w, "    at most", n + 1, high);
            }
            E::HealOver(total, seconds) | E::ManaOver(total, seconds) => {
                value_row(g, y, x, w, "    in all", n, total);
                value_row(g, y, x, w, "    over (s)", n + 1, seconds);
            }
            E::Boost(stat, amount, seconds) => {
                let mut name = stats[(*stat as usize).min(stats.len() - 1)].clone();
                if choice_row(g, y, x, w, "    stat", &stats, &mut name) {
                    *stat = stats.iter().position(|s| *s == name).unwrap_or(0) as u8;
                }
                value_row(g, y, x, w, "    by", n + 1, amount);
                value_row(g, y, x, w, "    for (s)", n + 2, seconds);
                hint(g, y, x, "0 seconds: for as long as it is worn.");
            }
            E::Hearth => {}
            E::Mount(display, percent) => {
                value_row(g, y, x, w, "    mount look", n, display);
                hint(g, y, x, "A row of CreatureDisplayInfo.dbc.");
                value_row(g, y, x, w, "    speed added %", n + 1, percent);
            }
            E::Equip(aura, misc, amount) => {
                value_row(g, y, x, w, "    aura number", n, aura);
                value_row(g, y, x, w, "    misc", n + 1, misc);
                value_row(g, y, x, w, "    amount", n + 2, amount);
                hint(g, y, x, "The aura of the original's spell (what a worn item gives); misc: for a spell bonus, its schools.");
            }
        }
    }
    if let Some(j) = remove {
        effects.remove(j);
    }
    if effects.len() < 10 && g.button(x + LABEL_WIDTH, *y, 150.0, "Add an effect") {
        effects.push(E::Heal(20, 30));
    }
    *y += ROW + 3.0;
}

/// What a stop of a patrol makes the creature do (the original's waypoint script), one thing to a line, each some seconds after it arrives.
fn steps_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, steps: &mut Vec<(f32, Step)>, base: usize) {
    const KINDS: [&str; 5] = ["Runs or walks on", "Strolls around", "Stays for good", "Plays an animation", "Looks like another kind"];
    let mut remove = None;
    for (j, (after, what)) in steps.iter_mut().enumerate().take(10) {
        let n = base + j * 10;
        let now = match what {
            Step::Run(_) => 0,
            Step::Roam(..) => 1,
            Step::Idle => 2,
            Step::Emote(_) => 3,
            Step::Morph(_) => 4,
        };
        g.text(x, *y + 3.0, LABEL_WIDTH, &format!("  does {}", j + 1), DIM);
        let by = g.arrows(x + LABEL_WIDTH, *y, 220.0, KINDS[now]);
        if g.button(x + LABEL_WIDTH + 230.0, *y, 70.0, "Remove") {
            remove = Some(j);
        }
        *y += ROW + 3.0;
        if by != 0 {
            *what = match (now as i32 + by).rem_euclid(KINDS.len() as i32) {
                0 => Step::Run(true),
                1 => Step::Roam(5.0, 10.0),
                2 => Step::Idle,
                3 => Step::Emote(1),
                _ => Step::Morph(String::new()),
            };
        }
        value_row(g, y, x, w, "    after (s)", n, after);
        *after = after.max(0.0);
        match what {
            Step::Run(run) => {
                switch_row(g, y, x, "    runs", run);
                hint(g, y, x, "Yes: it runs from here on; no: it walks.");
            }
            Step::Roam(yards, seconds) => {
                value_row(g, y, x, w, "    within (yd)", n + 1, yards);
                value_row(g, y, x, w, "    for (s)", n + 2, seconds);
                hint(g, y, x, "Then it goes on with the patrol.");
            }
            Step::Idle => hint(g, y, x, "No more patrol until it is back from a fight or has died and come again."),
            Step::Emote(animation) => {
                value_row(g, y, x, w, "    animation", n + 1, animation);
                hint(g, y, x, "A number of the model's animations, held while it stands; 0 stops it.");
            }
            Step::Morph(kind) => {
                text_row_ref(g, y, x, w, "    looks like", n + 1, kind, Ref::Creature, None);
                hint(g, y, x, "It stays what it is; only its look changes.");
            }
        }
    }
    if let Some(j) = remove {
        steps.remove(j);
    }
    if steps.len() < 10 && g.button(x + LABEL_WIDTH, *y, 150.0, "Add something to do") {
        steps.push((0.0, Step::Run(true)));
    }
    *y += ROW + 3.0;
}

fn props(g: &mut Gui, d: &mut Data, p: &mut Pick, placer: &mut Placer, me: Place, actions: &mut Vec<Action>, top: f32, rows: usize, x: f32, w: f32) {
    let near = |n: f32, west: f32| ((n - me.north).powi(2) + (west - me.west).powi(2)).sqrt();
    let lines: Vec<String> = d.cur.props.iter().map(|q| format!("{} {:.0}yd {}", file_stem(&q.model), near(q.at.0, q.at.1), q.zone.as_deref().unwrap_or(""))).collect();
    list(g, 8.0, top + 28.0, LIST_WIDTH, rows, &lines, p);
    let selected = p.selected;
    let mut y = top + 28.0;
    if let Some(q) = d.cur.props.get_mut(selected) {
        g.text(x, y, w, &q.zone.as_ref().map_or("The selected prop".to_string(), |z| format!("The selected prop ({z})")), GOLD);
        y += ROW + 4.0;
        text_row(g, &mut y, x, w, "Model", 40, &mut q.model);
        value_row(g, &mut y, x, w, "North", 41, &mut q.at.0);
        value_row(g, &mut y, x, w, "West", 42, &mut q.at.1);
        value_row(g, &mut y, x, w, "Height", 43, &mut q.at.2);
        value_row(g, &mut y, x, w, "Turn", 44, &mut q.rotation[1]);
        value_row(g, &mut y, x, w, "Tilt", 45, &mut q.rotation[0]);
        value_row(g, &mut y, x, w, "Roll", 46, &mut q.rotation[2]);
        value_row(g, &mut y, x, w, "Scale", 47, &mut q.scale);
        // What it gives when it is picked up (a quest item; only while a quest wants it): none when no item is named.
        let mut item = q.pickup.as_ref().map_or_else(String::new, |k| k.item.clone());
        text_row_ref(g, &mut y, x, w, "Gives item", 49, &mut item, Ref::Item, None);
        if item.trim().is_empty() {
            q.pickup = None;
        } else {
            let k = q.pickup.get_or_insert_with(|| content::Pickup { item: String::new(), respawn: content::Pickup::respawn_default(), vanish: true });
            k.item = item.trim().to_string();
            g.text(x, y + 3.0, LABEL_WIDTH, "Disappears", DIM);
            if g.button(x + LABEL_WIDTH, y, 120.0, if k.vanish { "Yes, then back" } else { "No, it stays" }) {
                k.vanish = !k.vanish;
            }
            y += ROW + 3.0;
            if k.vanish {
                value_row(g, &mut y, x, w, "Back after (s)", 50, &mut k.respawn);
            }
        }
        g.text(x, y + 3.0, LABEL_WIDTH, "Bed", DIM);
        if g.button(x + LABEL_WIDTH, y, 120.0, if q.sleep { "Yes, sleep here" } else { "No" }) {
            q.sleep = !q.sleep;
        }
        y += ROW + 3.0;
        g.text(x, y + 3.0, LABEL_WIDTH, "Physics", DIM);
        if g.button(x + LABEL_WIDTH, y, 120.0, if q.physics { "Yes, loose" } else { "No" }) {
            q.physics = !q.physics;
        }
        y += ROW + 3.0;
        if g.button(x, y, 150.0, "Here (where I stand)") {
            q.at = (me.north, me.west, me.height);
        }
        if g.button(x + 156.0, y, 90.0, "Go to it") {
            actions.push(Action::Teleport(q.at.0, q.at.1));
        }
        y += ROW + 8.0;
        if g.button(x, y, 100.0, "Delete") {
            d.cur.props.remove(selected);
            p.selected = selected.saturating_sub(1);
        } else if g.button(x + 106.0, y, 220.0, "Copy it here") {
            let mut copy = d.cur.props[selected].clone();
            copy.at = (me.north, me.west, me.height);
            (copy.map, copy.zone) = (None, None);
            d.cur.props.push(copy);
            p.selected = d.cur.props.len() - 1;
        }
        y += ROW + 8.0;
        gizmo_row(g, &mut y, x, w, placer, true);
    }
    let mut y = top + 28.0 + 15.0 * (ROW + 3.0) + 70.0;
    g.text(x, y, w, "Put new props down", GOLD);
    y += ROW + 4.0;
    g.text(x, y + 3.0, LABEL_WIDTH, "Model", DIM);
    let ok = placeable(&placer.model) && placer.mount.exists(&placer.model);
    if !placer.model.is_empty() && !ok {
        g.tint = Some(RED);
    }
    g.field("Model", 48, x + LABEL_WIDTH, y, w - LABEL_WIDTH - BROWSE - 4.0, &mut placer.model);
    if g.button(x + w - BROWSE, y, BROWSE, "...") {
        let current = placer.model.clone();
        placer.open_gallery(g, &current);
    }
    y += ROW + 6.0;
    if placer.placing {
        if (!g.blocked && g.input.keys.contains(&Key::Escape)) || g.button(x, y, 260.0, "Stop putting down (Esc)") {
            placer.placing = false;
        }
        g.text(x, y + ROW + 4.0, w, "Every click in the world, beside this window, stands the model there.", DIM);
    } else if ok && g.button(x, y, 260.0, "Click in the world to put it down") {
        (placer.aim, placer.placing) = (Aim::Prop, true);
    }
}

/// The places to discover: the question marks of the compass until the player comes near.
#[allow(clippy::too_many_arguments)]
fn pois(g: &mut Gui, d: &mut Data, p: &mut Pick, me: Place, actions: &mut Vec<Action>, top: f32, rows: usize, x: f32, w: f32) {
    let near = |n: f32, west: f32| ((n - me.north).powi(2) + (west - me.west).powi(2)).sqrt();
    let lines: Vec<String> = d.cur.pois.iter().map(|q| format!("{} {:.0}yd", q.name, near(q.at.0, q.at.1))).collect();
    list(g, 8.0, top + 28.0, LIST_WIDTH, rows, &lines, p);
    let ids: Vec<String> = d.cur.pois.iter().map(|q| q.id.clone()).collect();
    let mut y = top + 28.0;
    let selected = p.selected;
    if let Some(q) = d.cur.pois.get_mut(selected) {
        g.text(x, y, w, "The selected place", GOLD);
        y += ROW + 4.0;
        if let Some(icon) = g.modal.icon_picked.take() {
            q.icon = icon;
        }
        text_row(g, &mut y, x, w, "Id", 20, &mut q.id);
        text_row(g, &mut y, x, w, "Name", 21, &mut q.name);
        value_row(g, &mut y, x, w, "North", 22, &mut q.at.0);
        value_row(g, &mut y, x, w, "West", 23, &mut q.at.1);
        value_row(g, &mut y, x, w, "Radius (yd)", 24, &mut q.radius);
        option_value_row(g, &mut y, x, w, "Feet height", 25, &mut q.height);
        hint(g, &mut y, x, "A floor of a building or a bridge; empty: the terrain.");
        if g.button(x, y, 150.0, "Here (where I stand)") {
            q.at = (me.north, me.west);
        }
        if g.button(x + 156.0, y, 90.0, "Go to it") {
            actions.push(Action::Teleport(q.at.0, q.at.1));
        }
        y += ROW + 8.0;
        g.text(x, y + 9.0, 60.0, "Icon", DIM);
        if let Some(texture) = g.poi_icons {
            g.fill(x + 60.0, y, 34.0, 34.0, BOX);
            g.picture_part(texture, x + 63.0, y + 3.0, 28.0, 28.0, crate::compass::poi_uv(q.icon));
        }
        if g.button(x + 100.0, y + 8.0, 110.0, "Choose...") {
            g.modal.icons = Some(q.icon);
        }
        y += 34.0 + 8.0;
        if g.button(x, y, 100.0, "Delete") {
            d.cur.pois.remove(selected);
            p.selected = selected.saturating_sub(1);
        }
    }
    let y = top + 28.0 + 12.0 * (ROW + 3.0) + 70.0;
    if g.button(x, y, 220.0, "New here") {
        d.cur.pois.push(content::Poi { id: fresh(&ids.iter().map(String::as_str).collect::<Vec<_>>(), "place"), name: "New place".into(), at: (me.north, me.west), radius: 30.0, icon: 4, height: None });
        p.selected = d.cur.pois.len() - 1;
        p.scroll = d.cur.pois.len().saturating_sub(rows);
    }
}

/// The creatures tab: the kinds of creatures, the NPCs and the enemies.
fn creatures(g: &mut Gui, d: &mut Data, p: &mut Pick, pv: &mut Preview, top: f32, bottom: f32) {
    let rows = ((bottom - top - 26.0) / ROW) as usize;
    let lines: Vec<String> = d.cur.creatures.iter().map(|c| format!("{}{}", c.id, if c.hostile { "  (enemy)" } else if c.neutral { "  (neutral)" } else { "" })).collect();
    let ids: Vec<String> = d.cur.creatures.iter().map(|c| c.id.clone()).collect();
    if g.button(8.0, top, 106.0, "New (copy)") && !d.cur.creatures.is_empty() {
        let mut copy = d.cur.creatures[p.selected.min(ids.len() - 1)].clone();
        copy.id = fresh(&ids.iter().map(String::as_str).collect::<Vec<_>>(), "creature");
        copy.name = "New creature".into();
        d.cur.creatures.push(copy);
        p.selected = d.cur.creatures.len() - 1;
        p.scroll = d.cur.creatures.len().saturating_sub(rows);
    }
    if g.button(120.0, top, 100.0, "Delete") && !d.cur.creatures.is_empty() {
        d.cur.creatures.remove(p.selected.min(ids.len() - 1));
        p.selected = p.selected.saturating_sub(1);
    }
    list(g, 8.0, top + 26.0, LIST_WIDTH, rows, &lines, p);
    let Some(c) = d.cur.creatures.get_mut(p.selected) else { return };
    let (x, w) = (LIST_WIDTH + 24.0, WIDTH - LIST_WIDTH - 32.0);
    let items = &d.cur.items;
    form(g, p, x, top, w, bottom, |g, y, _| creature_form(g, y, x, w, c, pv, items));
}

/// The picture of the creature, and beside it what changes how it looks. A beast: the model, from all those of the game, and its
/// skins, from those the game has for the model. A person: his skin, face, hair and beard, and what he wears.
fn look_panel(g: &mut Gui, y: &mut f32, x: f32, w: f32, c: &mut RawCreature, pv: &mut Preview, items: &[RawItem]) {
    const PICTURE: (f32, f32) = (180.0, 216.0);
    pv.show(&c.body);
    g.fill(x, *y, PICTURE.0, PICTURE.1, BOX);
    g.picture(pv.texture(), x, *y, PICTURE.0, PICTURE.1, [1.0; 4]);
    if let Some(e) = &pv.error {
        g.text(x + 4.0, *y + 4.0, PICTURE.0 - 8.0, e, RED);
    }
    let (rx, rw) = (x + PICTURE.0 + 10.0, w - PICTURE.0 - 10.0);
    let mut ry = *y;
    const TAG: f32 = 64.0;
    match &mut c.body {
        RawBody::Beast { model, skins, .. } => {
            pv.load_catalogue();
            let (model_tag, skin_tag) = (fid("model", 0), fid("skin", 0));
            match &pv.catalogue {
                Some(Ok(catalogue)) => {
                    // The model: any of those of the game, chosen in a window with a search and a picture.
                    if let Some(chosen) = g.picked(model_tag) {
                        *skins = catalogue.skins_of(&chosen).first().cloned().unwrap_or_default();
                        *model = chosen;
                    }
                    g.text(rx, ry + 3.0, TAG, "Model", DIM);
                    g.text(rx + TAG, ry + 3.0, rw - TAG - BROWSE - 4.0, file_stem(model), TEXT);
                    if g.button(rx + rw - BROWSE, ry, BROWSE, "...") {
                        let entries = catalogue.models.iter().map(|m| Entry { value: m.clone(), label: model_label(m) }).collect();
                        g.open_chooser(model_tag, &format!("Choose a model ({} in the game)", catalogue.models.len()), Shows::Model, entries, model);
                    }
                    ry += ROW + 3.0;
                    // The skin: the sets the game has for the model, one at a time with arrows or all in a window.
                    let sets = catalogue.skins_of(model);
                    if let Some(n) = g.picked(skin_tag).and_then(|v| v.parse::<usize>().ok()) {
                        *skins = sets.get(n).cloned().unwrap_or_default();
                    }
                    let at = sets.iter().position(|s| preview::same_skins(s, skins));
                    g.text(rx, ry + 3.0, TAG, "Skin", DIM);
                    let name = match (at, sets.len()) {
                        (_, 0) => "(the model has none)".to_string(),
                        (Some(i), n) => format!("{} {}/{}", file_stem(&sets[i][0].1), i + 1, n),
                        (None, n) => format!("(not one of {n})"),
                    };
                    let by = g.arrows(rx + TAG, ry, rw - TAG - BROWSE - 4.0, &name);
                    if g.button(rx + rw - BROWSE, ry, BROWSE, "...") && !sets.is_empty() {
                        let entries = sets.iter().enumerate().map(|(i, s)| Entry { value: i.to_string(), label: s.iter().map(|(_, f)| file_stem(f)).collect::<Vec<_>>().join(" + ") }).collect();
                        g.open_chooser(skin_tag, "Choose the skins", Shows::Skin(model.clone()), entries, &at.map_or(String::new(), |i| i.to_string()));
                    }
                    ry += ROW + 3.0;
                    if by != 0 && !sets.is_empty() {
                        *skins = sets[(at.map_or(if by > 0 { -1 } else { 0 }, |i| i as i32) + by).rem_euclid(sets.len() as i32) as usize].clone();
                    }
                }
                Some(Err(e)) => g.text(rx, ry + 3.0, rw, e, RED),
                None => {}
            }
        }
        RawBody::Humanoid { outfit, look } => {
            // The race and sex; what the tables have for a race and sex limits the rest, so a change takes the rest back into them.
            let all: Vec<Race> = Race::ALL.iter().copied().chain([Race::Goblin]).collect();
            let races: Vec<String> = all.iter().map(|r| r.name().to_string()).collect();
            let mut race = look.race.name().to_string();
            g.text(rx, ry + 3.0, TAG + 20.0, "Race", DIM);
            let by = g.arrows(rx + TAG + 20.0, ry, rw - TAG - 20.0, &race);
            if by != 0 {
                race = step(&races, &race, by).to_string();
                look.race = all[races.iter().position(|r| *r == race).unwrap_or(0)];
            }
            ry += ROW + 3.0;
            g.text(rx, ry + 3.0, TAG + 20.0, "Sex", DIM);
            if g.arrows(rx + TAG + 20.0, ry, rw - TAG - 20.0, look.sex.name()) != 0 {
                look.sex = if look.sex == Sex::Male { Sex::Female } else { Sex::Male };
            }
            ry += ROW + 3.0;
            let max = pv.ranges(look.race, look.sex, look.skin, look.hair_style);
            (look.skin, look.face, look.hair_style, look.hair_color, look.beard) = (look.skin.min(max.skin), look.face.min(max.face), look.hair_style.min(max.hair_style), look.hair_color.min(max.hair_color), look.beard.min(max.beard));
            let mut set = |g: &mut Gui, label: &str, value: &mut u32, max: u32| {
                g.text(rx, ry + 3.0, TAG + 20.0, label, DIM);
                let by = g.arrows(rx + TAG + 20.0, ry, rw - TAG - 20.0, &value.to_string());
                *value = (*value as i32 + by).rem_euclid(max as i32 + 1) as u32;
                ry += ROW + 3.0;
            };
            set(g, "Skin", &mut look.skin, max.skin);
            set(g, "Face", &mut look.face, max.face);
            set(g, "Hair style", &mut look.hair_style, max.hair_style);
            set(g, "Hair colour", &mut look.hair_color, max.hair_color);
            set(g, "Beard", &mut look.beard, max.beard);
            ry += 4.0;
            // What he wears: chosen from every item that can be worn, in a window with a search; it takes the place of what he wears in the same place.
            let tag = fid("outfit", 0);
            if let Some(id) = g.picked(tag) {
                if let Some(new) = items.iter().find(|i| i.id == id) {
                    outfit.retain(|o| items.iter().find(|i| i.id == *o).is_none_or(|i| i.wear != new.wear));
                    outfit.push(id);
                }
            }
            g.text(rx, ry + 3.0, TAG, "Item", DIM);
            if g.button(rx + TAG, ry, rw - TAG, "Put on...") {
                let entries = items.iter().filter(|i| i.wear != Wear::None).map(|i| Entry { value: i.id.clone(), label: format!("{}  ({}, level {})", i.name, i.wear.name(), i.level) }).collect();
                g.open_chooser(tag, "Choose what he puts on", Shows::Nothing, entries, "");
            }
            ry += ROW + 3.0;
            if g.button(rx + TAG, ry, rw - TAG, "Take all off") {
                outfit.clear();
            }
            ry += ROW + 3.0;
        }
    }
    g.text(rx, ry + 3.0, TAG, "Turn", DIM);
    let by = g.arrows(rx + TAG, ry, rw - TAG, "round");
    pv.turn += by as f32 * 30f32.to_radians();
    *y += PICTURE.1 + 6.0;
}

/// A model as the list of models shows it: the folder and the name, or the name alone when the folder is called the same.
fn model_label(path: &str) -> String {
    let short = path.trim_start_matches("Creature\\").trim_end_matches(".m2");
    match short.split_once('\\') {
        Some((folder, name)) if folder.eq_ignore_ascii_case(name) => name.to_string(),
        _ => short.to_string(),
    }
}

/// The name of a file without its folders and ending.
fn file_stem(path: &str) -> &str {
    let name = path.rsplit('\\').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// How a creature fights beyond the bite: its mana, the time between its swings, its shot, whom it calls, and the events of its script (each written as the
/// content file has it, with a sentence over it that says what it means).
fn fighting_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, c: &mut RawCreature) {
    let b = &mut c.brain;
    value_row(g, y, x, w, "Mana", 200, &mut b.mana);
    value_row(g, y, x, w, "Swing (s, 0 = 2)", 201, &mut b.swing);
    value_row(g, y, x, w, "Shot damage", 202, &mut b.shot);
    value_row(g, y, x, w, "Calls help (yd)", 203, &mut b.assist);
    value_row(g, y, x, w, "Faction", 204, &mut b.faction);
    switch_row(g, y, x, "Fights back only", &mut b.calm);
    let mut remove = None;
    for (i, e) in b.events.iter_mut().enumerate() {
        g.text(x + LABEL_WIDTH, *y, w - LABEL_WIDTH, &crate::brain::describe(e), DIM);
        *y += ROW;
        let mut text = ron::to_string(&*e).unwrap_or_default();
        if text_row(g, y, x, w, &format!("Event {}", i + 1), 300 + i, &mut text) {
            match ron::from_str::<content::BrainEvent>(&text) {
                Ok(parsed) => *e = parsed,
                Err(err) => g.say(format!("Event {}: {err}", i + 1), RED),
            }
        }
        if g.button(x + LABEL_WIDTH, *y, 70.0, "Remove") {
            remove = Some(i);
        }
        *y += ROW + 3.0;
    }
    if let Some(i) = remove {
        b.events.remove(i);
    }
    if g.button(x + LABEL_WIDTH, *y, 120.0, "Add an event") {
        // Casts a fireball (spell 133) at the player every few seconds of a fight.
        b.events.push(content::BrainEvent { on: crate::brain::event::TIMER, p: vec![3000, 6000, 8000, 12000], flags: 0x401, acts: vec![(crate::brain::act::CAST, 133, 1, 0)], ..Default::default() });
    }
    *y += ROW + 3.0;
}

fn creature_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, c: &mut RawCreature, pv: &mut Preview, items: &[RawItem]) {
    look_panel(g, y, x, w, c, pv, items);
    text_row(g, y, x, w, "Id", 100, &mut c.id);
    text_row(g, y, x, w, "Name", 101, &mut c.name);
    option_row(g, y, x, w, "Title", 102, &mut c.title);
    value_row(g, y, x, w, "Level", 103, &mut c.level);
    let types: Vec<String> = ["None", "Beast", "Dragonkin", "Demon", "Elemental", "Giant", "Undead", "Humanoid", "Critter", "Mechanical"].iter().map(|s| s.to_string()).collect();
    let mut kind = types[(c.kind as usize).min(types.len() - 1)].clone();
    if choice_row(g, y, x, w, "Type", &types, &mut kind) {
        c.kind = types.iter().position(|t| *t == kind).unwrap_or(0) as u32;
    }
    hint(g, y, x, "What the bonuses of gear \"versus\" name.");
    switch_row(g, y, x, "Enemy", &mut c.hostile);
    switch_row(g, y, x, "Neutral", &mut c.neutral);
    // How a player of the Horde finds it, when that is not how the Alliance does.
    let mut differs = c.horde.is_some();
    if switch_row(g, y, x, "Horde differs", &mut differs) {
        c.horde = differs.then_some((c.hostile, c.neutral));
    }
    if let Some((hostile, neutral)) = &mut c.horde {
        switch_row(g, y, x, "  Enemy to Horde", hostile);
        switch_row(g, y, x, "  Neutral to Horde", neutral);
    }
    hint(g, y, x, "Enemy and Neutral above are what the Alliance finds; the Horde's own view is kept only when it differs.");
    value_row(g, y, x, w, "Health", 104, &mut c.health);
    value_row(g, y, x, w, "Bite damage", 105, &mut c.bite_damage);
    value_row(g, y, x, w, "Respawn (s)", 106, &mut c.respawn);
    fighting_form(g, y, x, w, c);
    option_ref_row(g, y, x, w, "Loot table", 107, &mut c.loot, Ref::Loot);
    list_row(g, y, x, w, "Sells", 108, &mut c.vendor, Some(Ref::Item));
    match &mut c.body {
        RawBody::Beast { model, skins, scale } => {
            text_row(g, y, x, w, "Model", 110, model);
            value_row(g, y, x, w, "Scale", 111, scale);
            let mut text = skins.iter().map(|(slot, file)| format!("{slot}={file}")).collect::<Vec<_>>().join("; ");
            if text_row(g, y, x, w, "Skins", 112, &mut text) {
                let parsed: Option<Vec<(u32, String)>> = text.split(';').map(str::trim).filter(|t| !t.is_empty()).map(|t| Some((t.split_once('=')?.0.trim().parse().ok()?, t.split_once('=')?.1.trim().to_string()))).collect();
                match parsed {
                    Some(p) => *skins = p,
                    None => g.say("Skins: write slot=file; slot=file", RED),
                }
            }
        }
        RawBody::Humanoid { outfit, look } => {
            list_row(g, y, x, w, "Wears", 113, outfit, Some(Ref::Item));
            let mut v = [look.skin, look.face, look.hair_style, look.hair_color, look.beard];
            if numbers_row(g, y, x, w, "Look", 114, &mut v) {
                (look.skin, look.face, look.hair_style, look.hair_color, look.beard) = (v[0], v[1], v[2], v[3], v[4]);
            }
            g.text(x + LABEL_WIDTH, *y, w, "(skin, face, hair style, hair colour, beard)", DIM);
            *y += ROW;
        }
    }
    *y += 6.0;
    // What it says.
    let mut talks = c.gossip.is_some();
    if switch_row(g, y, x, "Talks", &mut talks) {
        c.gossip = talks.then(|| content::Gossip { greeting: "Hello, $N.".into(), options: Vec::new() });
    }
    if let Some(gossip) = &mut c.gossip {
        text_row(g, y, x, w, "Greeting", 120, &mut gossip.greeting);
        let mut remove = None;
        for (i, o) in gossip.options.iter_mut().enumerate() {
            text_row(g, y, x, w, &format!("Line {}", i + 1), 130 + i, &mut o.text);
            let kinds: Vec<String> = ["Say", "Vendor", "Train"].iter().map(|s| s.to_string()).collect();
            let now = match o.action {
                CreatureAction::Say(_) => "Say",
                CreatureAction::Vendor => "Vendor",
                CreatureAction::Train { .. } => "Train",
            };
            g.text(x, *y + 3.0, LABEL_WIDTH, "  does", DIM);
            let by = g.arrows(x + LABEL_WIDTH, *y, 140.0, now);
            if by != 0 {
                o.action = match step(&kinds, now, by) {
                    "Say" => CreatureAction::Say("...".into()),
                    "Vendor" => CreatureAction::Vendor,
                    _ => CreatureAction::Train { skill: crate::skills::Skill::Riding, rank: 75, needs: 0, level: 40, cost: 900_000 },
                };
            }
            if g.button(x + LABEL_WIDTH + 150.0, *y, 70.0, "Remove") {
                remove = Some(i);
            }
            *y += ROW + 3.0;
            match &mut o.action {
                CreatureAction::Say(text) => {
                    text_row(g, y, x, w, "  says", 160 + i, text);
                }
                CreatureAction::Vendor => {}
                CreatureAction::Train { skill, rank, needs, level, cost } => {
                    let names: Vec<String> = crate::skills::ALL.iter().map(|s| s.name().to_string()).collect();
                    let mut taught = skill.name().to_string();
                    if choice_row(g, y, x, w, "  teaches", &names, &mut taught) {
                        *skill = crate::skills::ALL.into_iter().find(|s| s.name() == taught).unwrap_or(*skill);
                    }
                    value_row(g, y, x, w, "  up to rank", 1000 + i * 4, rank);
                    value_row(g, y, x, w, "  needs rank", 1001 + i * 4, needs);
                    value_row(g, y, x, w, "  min. level", 1002 + i * 4, level);
                    value_row(g, y, x, w, "  cost (copper)", 1003 + i * 4, cost);
                    hint(g, y, x, &format!("= {}g {}s {}c, for a character who has rank {} and is level {} or more.", *cost / 10_000, *cost / 100 % 100, *cost % 100, needs, level));
                }
            }
        }
        if let Some(i) = remove {
            gossip.options.remove(i);
        }
        if g.button(x + LABEL_WIDTH, *y, 120.0, "Add a line") {
            gossip.options.push(GossipOption { text: "Tell me more.".into(), action: CreatureAction::Say("...".into()) });
        }
        *y += ROW + 3.0;
    }
}

/// A line of switches for the races a quest is for (none marked: all of them).
fn races_row(g: &mut Gui, y: &mut f32, x: f32, mask: &mut u32) {
    g.text(x, *y + 3.0, LABEL_WIDTH, "Races", DIM);
    for (i, race) in Race::ALL.iter().enumerate() {
        let on = *mask & race.mask() != 0;
        let short = ["Hu", "Or", "Dw", "Ni", "Un", "Ta", "Gn", "Tr"][i].to_string();
        if g.button(x + LABEL_WIDTH + i as f32 * 42.0, *y, 38.0, &if on { format!("[{short}]") } else { short }) {
            *mask ^= race.mask();
        }
    }
    *y += ROW + 3.0;
    hint(g, y, x, "Hu Or Dw Ni Un Ta Gn Tr; none marked: every race may take it.");
}

/// The quests tab.
fn quests(g: &mut Gui, d: &mut Data, p: &mut Pick, me: Place, top: f32, bottom: f32) {
    let rows = ((bottom - top - 26.0) / ROW) as usize;
    let lines: Vec<String> = d.cur.quests.iter().map(|q| q.title.clone()).collect();
    let ids: Vec<String> = d.cur.quests.iter().map(|q| q.id.clone()).collect();
    if g.button(8.0, top, 106.0, "New") {
        let id = fresh(&ids.iter().map(String::as_str).collect::<Vec<_>>(), "quest");
        let giver = d.cur.creatures.iter().find(|c| c.gossip.is_some() && !c.hostile).map_or(String::new(), |c| c.id.clone());
        let text = format!(r#"(id: "{id}", title: "New quest", giver: "{giver}", turn_in: "{giver}", details: "What is asked.", objective: "What to do.", progress: "Not yet.", completion: "Well done.")"#);
        if let Ok(q) = ron::from_str::<RawQuest>(&text) {
            d.cur.quests.push(q);
            p.selected = d.cur.quests.len() - 1;
            p.scroll = d.cur.quests.len().saturating_sub(rows);
        }
    }
    if g.button(120.0, top, 100.0, "Delete") && !d.cur.quests.is_empty() {
        d.cur.quests.remove(p.selected.min(ids.len() - 1));
        p.selected = p.selected.saturating_sub(1);
    }
    list(g, 8.0, top + 26.0, LIST_WIDTH, rows, &lines, p);
    let Some(q) = d.cur.quests.get_mut(p.selected) else { return };
    let (x, w) = (LIST_WIDTH + 24.0, WIDTH - LIST_WIDTH - 32.0);
    form(g, p, x, top, w, bottom, |g, y, _| {
        text_row(g, y, x, w, "Id", 200, &mut q.id);
        text_row(g, y, x, w, "Title", 201, &mut q.title);
        text_row_ref(g, y, x, w, "Given by", 202, &mut q.giver, Ref::Creature, None);
        text_row_ref(g, y, x, w, "Turned in to", 203, &mut q.turn_in, Ref::Creature, None);
        option_ref_row(g, y, x, w, "Needs quest", 204, &mut q.requires, Ref::Quest);
        value_row(g, y, x, w, "Level", 213, &mut q.level);
        hint(g, y, x, "0 for none; the experience falls as the character outgrows the quest.");
        value_row(g, y, x, w, "Lowest level", 215, &mut q.min_level);
        races_row(g, y, x, &mut q.races);
        option_ref_row(g, y, x, w, "Begun by item", 216, &mut q.begins_with, Ref::Item);
        hint(g, y, x, "Using the item (a letter, a note) opens the quest, and nobody offers it; empty: the one who gives it offers it.");
        text_row(g, y, x, w, "Offer says", 205, &mut q.details);
        text_row(g, y, x, w, "Objective", 206, &mut q.objective);
        text_row(g, y, x, w, "Waiting says", 207, &mut q.progress);
        text_row(g, y, x, w, "Done says", 208, &mut q.completion);
        value_row(g, y, x, w, "Copper", 209, &mut q.reward.copper);
        value_row(g, y, x, w, "Experience", 210, &mut q.reward.xp);
        stacks_row(g, y, x, w, "Gives", 211, &mut q.reward.items);
        stacks_row(g, y, x, w, "Choose one of", 212, &mut q.reward.choice);
        stacks_row(g, y, x, w, "Given at start", 214, &mut q.start_items);
        hint(g, y, x, "Handed over when the quest is taken (a letter to deliver); taken back if it is given up.");
        *y += 4.0;
        g.text(x, *y, w, "Goals", GOLD);
        *y += ROW + 2.0;
        let mut remove = None;
        for (i, goal) in q.goals.iter_mut().enumerate() {
            let (kill, target, count) = match goal {
                RawGoal::Kill(t, c) => (true, t.clone(), *c),
                RawGoal::Collect(t, c) => (false, t.clone(), *c),
                RawGoal::Reach(map, at, radius) => {
                    // A walk to a place: the map, north, west and how close, yards.
                    g.text(x, *y + 3.0, 56.0, "Walk to", DIM);
                    g.field("walkmap", 340 + i, x + 60.0, *y, 130.0, map);
                    let mut numbers = [at.0, at.1, *radius].map(|v| format!("{v}"));
                    for (k, (label, width)) in [("walkn", 80.0), ("walkw", 80.0), ("walkr", 50.0)].into_iter().enumerate() {
                        let at_x = x + 196.0 + [0.0, 86.0, 172.0][k];
                        if g.field(label, 360 + i, at_x, *y, width, &mut numbers[k]) {
                            if let Ok(v) = numbers[k].trim().parse::<f32>() {
                                match k {
                                    0 => at.0 = v,
                                    1 => at.1 = v,
                                    _ => *radius = v.max(1.0),
                                }
                            }
                        }
                    }
                    if g.button(x + w - 78.0, *y, 70.0, "Remove") {
                        remove = Some(i);
                    }
                    *y += ROW + 3.0;
                    continue;
                }
            };
            let mut kind = if kill { "Kill".to_string() } else { "Collect".to_string() };
            let by = g.arrows(x, *y, 110.0, &kind);
            let (mut target, mut count) = (target, count.to_string());
            let (tag, target_kind) = (fid("goalpick", i), if kill { Ref::Creature } else { Ref::Item });
            if !target.trim().is_empty() && !g.ids.has(target_kind, target.trim()) {
                g.tint = Some(RED);
            }
            g.field("goal", 300 + i, x + 116.0, *y, w - 116.0 - 130.0 - 32.0, &mut target);
            if let Some(chosen) = g.picked(tag) {
                target = chosen;
            }
            g.browse_ids(tag, x + w - 158.0, *y, target_kind, target.trim());
            g.field("count", 320 + i, x + w - 124.0, *y, 40.0, &mut count);
            if g.button(x + w - 78.0, *y, 70.0, "Remove") {
                remove = Some(i);
            }
            if by != 0 {
                kind = step(&["Kill".into(), "Collect".into()], &kind, by).to_string();
            }
            let count = count.trim().parse().unwrap_or(1);
            *goal = if kind == "Kill" { RawGoal::Kill(target, count) } else { RawGoal::Collect(target, count) };
            *y += ROW + 3.0;
        }
        if let Some(i) = remove {
            q.goals.remove(i);
        }
        if g.button(x, *y, 120.0, "Add a goal") {
            q.goals.push(RawGoal::Kill(String::new(), 1));
        }
        if g.button(x + 126.0, *y, 190.0, "Add a walk to here") {
            q.goals.push(RawGoal::Reach(me.map.to_string(), (me.north, me.west), 10.0));
        }
        *y += ROW + 3.0;
    });
}

/// The loot tables: what creatures and chests drop.
fn loot(g: &mut Gui, d: &mut Data, p: &mut Pick, top: f32, bottom: f32) {
    let rows = ((bottom - top - 26.0) / ROW) as usize;
    let keys: Vec<String> = d.cur.loot.keys().cloned().collect();
    if g.button(8.0, top, 106.0, "New") {
        let id = fresh(&keys.iter().map(String::as_str).collect::<Vec<_>>(), "loot");
        d.cur.loot.insert(id.clone(), content::RawTable { copper: (1, 5), items: Vec::new(), also: Vec::new() });
        p.selected = d.cur.loot.keys().position(|k| *k == id).unwrap_or(0);
        p.scroll = p.selected.saturating_sub(rows / 2);
    }
    if g.button(120.0, top, 100.0, "Delete") {
        if let Some(key) = keys.get(p.selected) {
            d.cur.loot.remove(key);
            p.selected = p.selected.saturating_sub(1);
        }
    }
    list(g, 8.0, top + 26.0, LIST_WIDTH, rows, &keys, p);
    let Some(key) = keys.get(p.selected).cloned() else { return };
    let (x, w) = (LIST_WIDTH + 24.0, WIDTH - LIST_WIDTH - 32.0);
    let mut rename = None;
    if let Some(table) = d.cur.loot.get_mut(&key) {
        form(g, p, x, top, w, bottom, |g, y, _| {
            let mut id = key.clone();
            if text_row(g, y, x, w, "Id", 500, &mut id) && !id.trim().is_empty() {
                rename = Some(id.trim().to_string());
            }
            let mut copper = [table.copper.0, table.copper.1];
            if numbers_row(g, y, x, w, "Copper", 501, &mut copper) {
                table.copper = (copper[0], copper[1]);
            }
            list_row(g, y, x, w, "Also rolls", 502, &mut table.also, Some(Ref::Loot));
            hint(g, y, x, "Other tables rolled as well: their copper is added and their items are among these (one step deep).");
            *y += 4.0;
            g.text(x, *y, 170.0, "Item", GOLD);
            g.text(x + 176.0, *y, 60.0, "Count", GOLD);
            g.text(x + 242.0, *y, 60.0, "Chance %", GOLD);
            *y += ROW + 2.0;
            let mut remove = None;
            for (i, drop) in table.items.iter_mut().enumerate() {
                let (mut id, mut count, mut chance) = (drop.0.clone(), if drop.1.0 == drop.1.1 { drop.1.0.to_string() } else { format!("{}-{}", drop.1.0, drop.1.1) }, drop.2.to_string());
                let tag = fid("lootpick", i);
                if !id.trim().is_empty() && !g.ids.has(Ref::Item, id.trim()) {
                    g.tint = Some(RED);
                }
                if g.field("lootitem", 510 + i, x, *y, 140.0, &mut id) {
                    drop.0 = id.clone();
                }
                if let Some(chosen) = g.picked(tag) {
                    drop.0 = chosen.clone();
                    id = chosen;
                }
                g.browse_ids(tag, x + 144.0, *y, Ref::Item, id.trim());
                if g.field("lootcount", 600 + i, x + 176.0, *y, 60.0, &mut count) {
                    let (low, high) = count.split_once('-').unwrap_or((&count, &count));
                    match (low.trim().parse(), high.trim().parse()) {
                        (Ok(l), Ok(h)) => drop.1 = (l, h),
                        _ => g.say("Count: write 2 or 1-3", RED),
                    }
                }
                if g.field("lootchance", 700 + i, x + 242.0, *y, 44.0, &mut chance) {
                    match chance.trim().parse() {
                        Ok(c) => drop.2 = c,
                        Err(_) => g.say("Chance: a number from 0 to 100", RED),
                    }
                }
                if g.button(x + 292.0, *y, 70.0, "Remove") {
                    remove = Some(i);
                }
                *y += ROW + 3.0;
            }
            if let Some(i) = remove {
                table.items.remove(i);
            }
            if g.button(x, *y, 120.0, "Add a drop") {
                table.items.push((String::new(), (1, 1), 50.0));
            }
            *y += ROW + 3.0;
        });
    }
    // A new id: the table moves to it.
    if let Some(id) = rename.filter(|id| *id != key && !d.cur.loot.contains_key(id)) {
        if let Some(table) = d.cur.loot.remove(&key) {
            d.cur.loot.insert(id.clone(), table);
            p.selected = d.cur.loot.keys().position(|k| *k == id).unwrap_or(0);
        }
    }
}

/// The items tab.
fn items(g: &mut Gui, d: &mut Data, p: &mut Pick, pv: &mut Preview, top: f32, bottom: f32) {
    let rows = ((bottom - top - 26.0) / ROW) as usize;
    let lines: Vec<String> = d.cur.items.iter().map(|i| i.name.clone()).collect();
    let ids: Vec<String> = d.cur.items.iter().map(|i| i.id.clone()).collect();
    if g.button(8.0, top, 106.0, "New (copy)") && !d.cur.items.is_empty() {
        let mut copy = d.cur.items[p.selected.min(ids.len() - 1)].clone();
        copy.id = fresh(&ids.iter().map(String::as_str).collect::<Vec<_>>(), "item");
        copy.name = "New item".into();
        d.cur.items.push(copy);
        p.selected = d.cur.items.len() - 1;
        p.scroll = d.cur.items.len().saturating_sub(rows);
    }
    if g.button(120.0, top, 100.0, "Delete") && !d.cur.items.is_empty() {
        d.cur.items.remove(p.selected.min(ids.len() - 1));
        p.selected = p.selected.saturating_sub(1);
    }
    list(g, 8.0, top + 26.0, LIST_WIDTH, rows, &lines, p);
    let Some(it) = d.cur.items.get_mut(p.selected) else { return };
    let (x, w) = (LIST_WIDTH + 24.0, WIDTH - LIST_WIDTH - 32.0);
    form(g, p, x, top, w, bottom, |g, y, _| item_form(g, y, x, w, it, pv));
}

/// Everywhere an item can be worn.
const WEARS: [Wear; 24] = [
    Wear::None,
    Wear::Head,
    Wear::Neck,
    Wear::Shoulder,
    Wear::Shirt,
    Wear::Chest,
    Wear::Waist,
    Wear::Legs,
    Wear::Feet,
    Wear::Wrist,
    Wear::Hands,
    Wear::Finger,
    Wear::Trinket,
    Wear::Back,
    Wear::OneHand,
    Wear::MainHand,
    Wear::TwoHand,
    Wear::Shield,
    Wear::OffHand,
    Wear::Held,
    Wear::Ranged,
    Wear::Thrown,
    Wear::Ammo,
    Wear::Tabard,
];

/// The picture of what an item looks like worn or held, and beside it its look (a row of `ItemDisplayInfo.dbc`, chosen in a window
/// with a search and the picture of each), its quality and where it is worn.
fn item_panel(g: &mut Gui, y: &mut f32, x: f32, w: f32, it: &mut RawItem, pv: &mut Preview) {
    const PICTURE: (f32, f32) = (180.0, 216.0);
    const TAG: f32 = 64.0;
    pv.show_display(it.display, it.wear);
    g.fill(x, *y, PICTURE.0, PICTURE.1, BOX);
    g.picture(pv.texture(), x, *y, PICTURE.0, PICTURE.1, [1.0; 4]);
    if let Some(e) = &pv.error {
        g.text(x + 4.0, *y + 4.0, PICTURE.0 - 8.0, e, RED);
    }
    let (rx, rw) = (x + PICTURE.0 + 10.0, w - PICTURE.0 - 10.0);
    let mut ry = *y;
    let tag = fid("display", 0);
    if let Some(v) = g.picked(tag).and_then(|v| v.parse().ok()) {
        it.display = v;
    }
    g.text(rx, ry + 3.0, TAG, "Look", DIM);
    let mut text = it.display.to_string();
    if g.field("display", 402, rx + TAG, ry, rw - TAG - BROWSE - 4.0, &mut text) {
        match text.trim().parse() {
            Ok(v) => it.display = v,
            Err(_) => g.say(format!("Look: \"{text}\" is not a number"), RED),
        }
    }
    if g.button(rx + rw - BROWSE, ry, BROWSE, "...") {
        let entries: Vec<Entry> = pv.displays(it.wear).into_iter().map(|(id, label)| Entry { value: id.to_string(), label }).collect();
        let title = format!("Choose a look: {} in the game that fit \"{:?}\"", entries.len(), it.wear);
        g.open_chooser(tag, &title, Shows::Display(it.wear), entries, &it.display.to_string());
    }
    ry += ROW + 3.0;
    g.text(rx, ry, rw, &pv.note, DIM);
    ry += ROW + 4.0;
    let qualities: Vec<String> = ["0 Poor", "1 Common", "2 Uncommon", "3 Rare", "4 Epic", "5 Legendary", "6 Artifact"].iter().map(|s| s.to_string()).collect();
    let mut quality = qualities[it.quality.min(6)].clone();
    if choice_row(g, &mut ry, rx, rw, "Quality", &qualities, &mut quality) {
        it.quality = qualities.iter().position(|q| *q == quality).unwrap_or(1);
    }
    let wears: Vec<String> = WEARS.iter().map(|w| format!("{w:?}")).collect();
    let mut wear = format!("{:?}", it.wear);
    if choice_row(g, &mut ry, rx, rw, "Worn as", &wears, &mut wear) {
        it.wear = WEARS.into_iter().find(|v| format!("{v:?}") == wear).unwrap_or(Wear::None);
    }
    g.text(rx, ry + 3.0, TAG, "Turn", DIM);
    let by = g.arrows(rx + TAG, ry, rw - TAG, "round");
    pv.turn += by as f32 * 30f32.to_radians();
    *y += PICTURE.1 + 6.0;
}

fn item_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, it: &mut RawItem, pv: &mut Preview) {
    text_row(g, y, x, w, "Id", 400, &mut it.id);
    text_row(g, y, x, w, "Name", 401, &mut it.name);
    item_panel(g, y, x, w, it, pv);
    text_row(g, y, x, w, "Type", 403, &mut it.kind.0);
    text_row(g, y, x, w, "Kind", 404, &mut it.kind.1);
    value_row(g, y, x, w, "Armor", 405, &mut it.armor);
    value_row(g, y, x, w, "Block", 406, &mut it.block);
    let mut stats = [it.stats.0, it.stats.1, it.stats.2, it.stats.3, it.stats.4];
    if numbers_row(g, y, x, w, "Stats", 407, &mut stats) {
        it.stats = (stats[0], stats[1], stats[2], stats[3], stats[4]);
    }
    g.text(x + LABEL_WIDTH, *y - 2.0, w, "(strength, agility, stamina, intellect, spirit)", DIM);
    *y += ROW - 2.0;
    let mut weapon = it.weapon.is_some();
    if switch_row(g, y, x, "Weapon", &mut weapon) {
        it.weapon = weapon.then_some(crate::items::Weapon { min: 5.0, max: 8.0, speed: 2.0 });
    }
    if let Some(wp) = &mut it.weapon {
        let mut v = [wp.min, wp.max, wp.speed];
        if numbers_row(g, y, x, w, "  min, max, speed", 408, &mut v) {
            (wp.min, wp.max, wp.speed) = (v[0], v[1], v[2]);
        }
    }
    value_row(g, y, x, w, "Stack", 409, &mut it.stack);
    text_row(g, y, x, w, "Sounds", 410, &mut it.handling);
    option_ref_row(g, y, x, w, "Teaches spell", 411, &mut it.tome, Ref::Spell);
    value_row(g, y, x, w, "Buy price", 412, &mut it.buy);
    value_row(g, y, x, w, "Sell price", 413, &mut it.sell);
    switch_row(g, y, x, "Quest item", &mut it.quest);
    g.text(x + LABEL_WIDTH + 70.0, *y - ROW - 1.0, w, "(drops only for a quest that wants it; not sold)", DIM);
    // What the original's tables say besides.
    *y += 6.0;
    g.text(x, *y, w, "As in the original", GOLD);
    *y += ROW + 2.0;
    let mut kind = [it.class as u32, it.sub as u32];
    if numbers_row(g, y, x, w, "Class, subclass", 420, &mut kind) {
        (it.class, it.sub) = (kind[0].min(255) as u8, kind[1].min(255) as u8);
    }
    g.text(x + LABEL_WIDTH, *y - 2.0, w, "(2 weapon, 4 armor; the subclass is the kind: 7 sword, 4 mace, 1 cloth ...)", DIM);
    *y += ROW - 2.0;
    value_row(g, y, x, w, "Level needed", 421, &mut it.level);
    let binds: Vec<String> = ["Never", "Pickup", "Equip", "Use", "Quest"].iter().map(|s| s.to_string()).collect();
    let mut bind = format!("{:?}", it.bind);
    if choice_row(g, y, x, w, "Binds", &binds, &mut bind) {
        it.bind = [crate::items::Bind::Never, crate::items::Bind::Pickup, crate::items::Bind::Equip, crate::items::Bind::Use, crate::items::Bind::Quest].into_iter().find(|b| format!("{b:?}") == bind).unwrap_or_default();
    }
    switch_row(g, y, x, "Unique", &mut it.unique);
    value_row(g, y, x, w, "Durability", 422, &mut it.durability);
    let mut vitals = [it.health, it.mana];
    if numbers_row(g, y, x, w, "Health, mana", 423, &mut vitals) {
        (it.health, it.mana) = (vitals[0], vitals[1]);
    }
    numbers_row(g, y, x, w, "Resist", 424, &mut it.resist);
    g.text(x + LABEL_WIDTH, *y - 2.0, w, "(holy, fire, nature, frost, shadow, arcane)", DIM);
    *y += ROW - 2.0;
    text_row(g, y, x, w, "Lore", 425, &mut it.text);
    let mut skill = [it.skill.0, it.skill.1];
    if numbers_row(g, y, x, w, "Skill, rank", 426, &mut skill) {
        it.skill = (skill[0], skill[1]);
    }
    let mut masks = [it.classes, it.races];
    if numbers_row(g, y, x, w, "Classes, races", 427, &mut masks) {
        (it.classes, it.races) = (masks[0], masks[1]);
    }
    g.text(x + LABEL_WIDTH, *y - 2.0, w, "(bit masks; 0 for everyone)", DIM);
    *y += ROW - 2.0;
    value_row(g, y, x, w, "Sold in", 428, &mut it.buy_count);
    option_value_row(g, y, x, w, "Weight", 450, &mut it.weight);
    hint(g, y, x, "What one of it weighs in the pack; empty: the usual for its kind.");
    let mut conjured = it.flags & 2 != 0;
    if switch_row(g, y, x, "Conjured", &mut conjured) {
        it.flags = if conjured { it.flags | 2 } else { it.flags & !2 };
    }
    value_row(g, y, x, w, "Flags", 452, &mut it.flags);
    hint(g, y, x, "The original's item flags as one number; the game uses 2 (conjured) and keeps the rest as they are.");
    value_row(g, y, x, w, "Bag slots", 451, &mut it.slots);
    hint(g, y, x, "For a bag or a quiver: how many slots it adds (0 for none).");
    // More damage of a weapon, of other schools than the blow itself.
    let mut remove = None;
    for (i, m) in it.more.iter_mut().enumerate() {
        let mut v = [m.0, m.1, m.2 as f32];
        if numbers_row(g, y, x, w, &format!("More damage {}", i + 1), 460 + i, &mut v) {
            *m = (v[0], v[1], v[2].clamp(0.0, 255.0) as u8);
        }
        if g.button(x + LABEL_WIDTH, *y, 90.0, "Remove") {
            remove = Some(i);
        }
        *y += ROW + 3.0;
    }
    if let Some(i) = remove {
        it.more.remove(i);
    }
    if g.button(x, *y, 150.0, "Add more damage") {
        it.more.push((1.0, 2.0, 2));
    }
    *y += ROW + 3.0;
    hint(g, y, x, "lowest, highest, school (1 holy, 2 fire, 3 nature, 4 frost, 5 shadow, 6 arcane).");
    // What it casts when used (0), worn (1) or on a blow (2), in the words of the original's spell.
    let mut remove = None;
    for (i, s) in it.spells.iter_mut().enumerate() {
        let mut v = [s.spell, s.trigger as u32];
        if numbers_row(g, y, x, w, &format!("Spell {}", i + 1), 430 + i, &mut v) {
            (s.spell, s.trigger) = (v[0], v[1].min(255) as u8);
        }
        text_row(g, y, x, w, "  says", 440 + i, &mut s.text);
        value_row(g, y, x, w, "  charges", 1100 + i * 4, &mut s.charges);
        hint(g, y, x, "Negative: one use uses the item up.");
        value_row(g, y, x, w, "  cooldown (ms)", 1101 + i * 4, &mut s.cooldown);
        hint(g, y, x, "-1: the spell's own.");
        value_row(g, y, x, w, "  procs a minute", 1102 + i * 4, &mut s.ppm);
        hint(g, y, x, "For a spell on a blow; 0: the spell's own chance.");
        effects_form(g, y, x, w, &mut s.effects, 3000 + i * 100);
        if g.button(x + LABEL_WIDTH, *y, 90.0, "Remove") {
            remove = Some(i);
        }
        *y += ROW + 3.0;
    }
    if let Some(i) = remove {
        it.spells.remove(i);
    }
    if g.button(x, *y, 150.0, "Add a spell line") {
        it.spells.push(crate::items::ItemSpell { spell: 0, trigger: 0, charges: 0, cooldown: 0, ppm: 0.0, effects: Vec::new(), text: String::new() });
    }
    *y += ROW + 3.0;
}

/// What a chooser shows of the entry it is on, in the picture beside its list.
#[derive(Clone, PartialEq, Debug)]
enum Shows {
    Nothing,
    /// The entry is the file of a creature model.
    Model,
    /// The entry is the number of a set of skins of this model.
    Skin(String),
    /// The entry is a row of `ItemDisplayInfo.dbc`, to be worn or held in this way.
    Display(Wear),
}

/// A line of a chooser: what it gives back, and what it says.
struct Entry {
    value: String,
    label: String,
}

/// A window over the editor with a list to choose one of, narrowed by what is typed.
struct Chooser {
    /// The field that asked, to take the answer.
    tag: u64,
    title: String,
    entries: Vec<Entry>,
    find: String,
    selected: Option<usize>,
    scroll: usize,
    shows: Shows,
}

#[derive(Default)]
struct Modal {
    open: Option<Chooser>,
    picked: Option<(u64, String)>,
    /// A field asked for the gallery of creatures or chests (opened by the editor, which has the pictures).
    want: Option<(u64, Pickable, String)>,
    /// The picker of the icon of a place is open (on this icon), and the one that was clicked in it.
    icons: Option<usize>,
    icon_picked: Option<usize>,
}

/// What the gallery over a field lets one choose from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pickable {
    Creatures,
    Chests,
}

/// What a field can refer to by its id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ref {
    Creature,
    Item,
    Loot,
    Quest,
    Spell,
}

impl Ref {
    fn title(self) -> &'static str {
        match self {
            Ref::Creature => "Choose a creature",
            Ref::Item => "Choose an item",
            Ref::Loot => "Choose a loot table",
            Ref::Quest => "Choose a quest",
            Ref::Spell => "Choose a spell",
        }
    }
}

/// The ids (and what they are called) of everything a field can refer to.
#[derive(Default)]
struct Ids {
    creatures: Vec<(String, String)>,
    items: Vec<(String, String)>,
    loot: Vec<(String, String)>,
    quests: Vec<(String, String)>,
    spells: Vec<(String, String)>,
}

impl Ids {
    fn of(raws: &Raws) -> Ids {
        Ids {
            creatures: raws.creatures.iter().map(|c| (c.id.clone(), c.name.clone())).collect(),
            items: raws.items.iter().map(|i| (i.id.clone(), i.name.clone())).collect(),
            loot: raws.loot.keys().map(|k| (k.clone(), String::new())).collect(),
            quests: raws.quests.iter().map(|q| (q.id.clone(), q.title.clone())).collect(),
            spells: content::get().spells.iter().map(|s| (s.id.clone(), s.name.clone())).collect(),
        }
    }

    fn of_kind(&self, kind: Ref) -> &[(String, String)] {
        match kind {
            Ref::Creature => &self.creatures,
            Ref::Item => &self.items,
            Ref::Loot => &self.loot,
            Ref::Quest => &self.quests,
            Ref::Spell => &self.spells,
        }
    }

    fn has(&self, kind: Ref, id: &str) -> bool {
        self.of_kind(kind).iter().any(|(i, _)| i == id)
    }
}

/// The width of the button that opens a chooser.
const BROWSE: f32 = 28.0;

impl Gui<'_> {
    /// What was chosen for the field `tag`, once.
    fn picked(&mut self, tag: u64) -> Option<String> {
        if self.modal.picked.as_ref().is_some_and(|(t, _)| *t == tag) {
            return self.modal.picked.take().map(|(_, v)| v);
        }
        None
    }

    /// Open a chooser for the field `tag`, on the entry with the value `current`.
    fn open_chooser(&mut self, tag: u64, title: &str, shows: Shows, entries: Vec<Entry>, current: &str) {
        let selected = entries.iter().position(|e| e.value == current);
        self.modal.open = Some(Chooser { tag, title: title.to_string(), entries, find: String::new(), selected, scroll: selected.map_or(0, |s| s.saturating_sub(10)), shows });
        // What is typed goes to its find field at once.
        self.edit.want = Some((fid("chooser", 0), String::new()));
    }

    /// The `...` button that opens a chooser over the ids of `kind` for the field `tag`.
    fn browse_ids(&mut self, tag: u64, x: f32, y: f32, kind: Ref, current: &str) {
        if self.button(x, y, BROWSE, "...") {
            if kind == Ref::Creature {
                // The creatures are chosen by their pictures.
                self.modal.want = Some((tag, Pickable::Creatures, current.to_string()));
            } else {
                let entries = self.ids.of_kind(kind).iter().map(|(id, name)| Entry { value: id.clone(), label: if name.is_empty() { id.clone() } else { format!("{id}  ({name})") } }).collect();
                self.open_chooser(tag, kind.title(), Shows::Nothing, entries, current);
            }
        }
    }
}

/// A line for a reference: a text field and the button that chooses from the ids of `kind`; unknown ids are red. With `append` the
/// text is a list, and what is chosen is added to it (followed by the text of `append`, such as `:1`).
#[allow(clippy::too_many_arguments)]
fn text_row_ref(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut String, kind: Ref, append: Option<&str>) -> bool {
    let tag = fid("pick", n);
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    if append.is_none() && !value.trim().is_empty() && !g.ids.has(kind, value.trim()) {
        g.tint = Some(RED);
    }
    let mut changed = g.field(label, n, x + LABEL_WIDTH, *y, w - LABEL_WIDTH - BROWSE - 4.0, value);
    if let Some(chosen) = g.picked(tag) {
        match append {
            None => *value = chosen,
            Some(after) => {
                if !value.trim().is_empty() {
                    value.push_str(", ");
                }
                value.push_str(&chosen);
                value.push_str(after);
            }
        }
        changed = true;
    }
    g.browse_ids(tag, x + w - BROWSE, *y, kind, value.trim());
    *y += ROW + 3.0;
    changed
}

/// `text_row_ref` for an id that may be left out.
fn option_ref_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut Option<String>, kind: Ref) -> bool {
    let mut text = value.clone().unwrap_or_default();
    if !text_row_ref(g, y, x, w, label, n, &mut text, kind, None) {
        return false;
    }
    *value = Some(text.trim().to_string()).filter(|t| !t.is_empty());
    true
}

/// The window that chooses (drawn over everything else of the editor, which can not be clicked meanwhile): a find field, the list it
/// narrows, and for some lists a picture of the entry the list is on. Enter or "Choose" gives the entry back, Esc or "Cancel" does not.
fn chooser(g: &mut Gui, pv: &mut Preview) {
    let Some(mut c) = g.modal.open.take() else { return };
    let (px, py, pw, ph) = (30.0, 40.0, 660.0, 680.0);
    g.fill(0.0, 0.0, WIDTH, UI_HEIGHT, [0.0, 0.0, 0.0, 0.6]);
    g.fill(px, py, pw, ph, [0.07, 0.07, 0.1, 1.0]);
    g.text(px + 16.0, py + 12.0, pw - 32.0, &c.title, GOLD);
    let (enter, esc) = (g.input.keys.contains(&Key::Enter), g.input.keys.contains(&Key::Escape));
    g.input.keys.retain(|k| !matches!(k, Key::Enter | Key::Escape));
    let find_id = fid("chooser", 0);
    g.text(px + 16.0, py + 43.0, 40.0, "Find", DIM);
    g.field("chooser", 0, px + 56.0, py + 40.0, 300.0, &mut c.find);
    let typed = if g.edit.focus == Some(find_id) { g.edit.buffer.clone() } else { c.find.clone() };
    if g.edit.focus != Some(find_id) && g.edit.want.is_none() {
        g.edit.want = Some((find_id, c.find.clone()));
    }
    let words: Vec<String> = typed.split_whitespace().map(str::to_ascii_lowercase).collect();
    let shown: Vec<usize> = (0..c.entries.len()).filter(|&i| words.iter().all(|w| c.entries[i].label.to_ascii_lowercase().contains(w))).collect();
    // The entry the list is on: the chosen one if it is still there, else the first.
    let at = c.selected.filter(|s| shown.contains(s)).or(shown.first().copied());
    let (lx, ly, lw) = (px + 16.0, py + 72.0, 340.0);
    let rows = ((ph - 132.0) / ROW) as usize;
    let h = rows as f32 * ROW;
    g.fill(lx, ly, lw, h, [0.0, 0.0, 0.0, 0.4]);
    if g.inside(lx, ly, lw, h) {
        let wheel = std::mem::take(&mut g.input.wheel);
        c.scroll = (c.scroll as f32 - wheel * 3.0).max(0.0) as usize;
    }
    c.scroll = c.scroll.min(shown.len().saturating_sub(rows));
    let mut confirm = None;
    for (row, &i) in shown.iter().skip(c.scroll).take(rows).enumerate() {
        let ry = ly + row as f32 * ROW;
        if Some(i) == at {
            g.fill(lx, ry, lw - 6.0, ROW, SELECTED);
        } else if g.inside(lx, ry, lw - 6.0, ROW) {
            g.fill(lx, ry, lw - 6.0, ROW, HOT);
        }
        g.text(lx + 4.0, ry + 3.0, lw - 10.0, &c.entries[i].label, if Some(i) == at { GOLD } else { TEXT });
        // A second click on the entry chooses it.
        if g.clicked(lx, ry, lw - 6.0, ROW) {
            if Some(i) == at && c.selected == Some(i) {
                confirm = Some(i);
            }
            c.selected = Some(i);
        }
    }
    if shown.len() > rows {
        let (top, size) = (c.scroll as f32 / shown.len() as f32, rows as f32 / shown.len() as f32);
        g.fill(lx + lw - 4.0, ly + top * h, 3.0, size * h, DIM);
    }
    if shown.is_empty() {
        g.text(lx + 4.0, ly + 3.0, lw - 10.0, "Nothing matches.", DIM);
    }
    g.text(lx, ly + h + 4.0, lw, &format!("{} of {}  (click twice or press Enter to choose)", shown.len(), c.entries.len()), DIM);
    // The picture of the entry.
    if let (Some(i), true) = (at, c.shows != Shows::Nothing) {
        let value = c.entries[i].value.clone();
        pv.load_catalogue();
        let beast = |model: &str, skins: Vec<(u32, String)>| RawBody::Beast { model: model.to_string(), skins, scale: 1.0 };
        let body = match (&c.shows, &pv.catalogue) {
            (Shows::Model, Some(Ok(cat))) => Some(beast(&value, cat.skins_of(&value).first().cloned().unwrap_or_default())),
            (Shows::Skin(model), Some(Ok(cat))) => Some(beast(model, value.parse::<usize>().ok().and_then(|n| cat.skins_of(model).get(n).cloned()).unwrap_or_default())),
            _ => None,
        };
        match (&c.shows, body) {
            (_, Some(body)) => pv.show(&body),
            (Shows::Display(wear), None) => {
                if let Ok(id) = value.parse() {
                    pv.show_display(id, *wear);
                }
            }
            _ => {}
        }
        let (rx, ry) = (lx + lw + 16.0, ly);
        g.fill(rx, ry, 250.0, 300.0, BOX);
        g.picture(pv.texture(), rx, ry, 250.0, 300.0, [1.0; 4]);
        if let Some(e) = &pv.error {
            g.text(rx + 4.0, ry + 4.0, 242.0, e, RED);
        }
        g.text(rx, ry + 306.0, 260.0, &c.entries[i].label, TEXT);
        if matches!(c.shows, Shows::Display(_)) {
            g.text(rx, ry + 324.0, 260.0, &pv.note, DIM);
        }
        g.text(rx, ry + 346.0, 56.0, "Turn", DIM);
        let by = g.arrows(rx + 56.0, ry + 343.0, 194.0, "round");
        pv.turn += by as f32 * 30f32.to_radians();
    }
    let chosen = g.button(px + 16.0, py + ph - 34.0, 110.0, "Choose");
    let cancelled = g.button(px + 132.0, py + ph - 34.0, 110.0, "Cancel");
    if esc || cancelled {
        g.edit.focus = None;
        g.edit.want = None;
        return;
    }
    if let Some(i) = confirm.or(if chosen || enter { at } else { None }) {
        g.modal.picked = Some((c.tag, c.entries[i].value.clone()));
        g.edit.focus = None;
        g.edit.want = None;
        return;
    }
    g.modal.open = Some(c);
}

/// The window that chooses the icon of a place from the client's picture of them (over everything else, like `chooser`): a click on a
/// cell gives it back, Esc or "Cancel" does not.
fn icon_chooser(g: &mut Gui) {
    let Some(current) = g.modal.icons else { return };
    let (cell, pw, ph) = (44.0, 8.0 * 44.0 + 32.0, 8.0 * 44.0 + 100.0);
    let (px, py) = ((WIDTH - pw) / 2.0, 120.0);
    g.fill(0.0, 0.0, WIDTH, UI_HEIGHT, [0.0, 0.0, 0.0, 0.6]);
    g.fill(px, py, pw, ph, [0.07, 0.07, 0.1, 1.0]);
    g.text(px + 16.0, py + 12.0, pw - 32.0, "Icon of the place", GOLD);
    let esc = g.input.keys.contains(&Key::Escape);
    g.input.keys.retain(|k| *k != Key::Escape);
    if let Some(texture) = g.poi_icons {
        for icon in 0..64 {
            let (cx, cy) = (px + 16.0 + (icon % 8) as f32 * cell, py + 40.0 + (icon / 8) as f32 * cell);
            let hot = g.inside(cx, cy, cell - 2.0, cell - 2.0);
            g.fill(cx, cy, cell - 2.0, cell - 2.0, if current == icon { SELECTED } else if hot { HOT } else { BOX });
            g.picture_part(texture, cx + 3.0, cy + 3.0, cell - 8.0, cell - 8.0, crate::compass::poi_uv(icon));
            if g.clicked(cx, cy, cell - 2.0, cell - 2.0) {
                g.modal.icon_picked = Some(icon);
                g.modal.icons = None;
                return;
            }
        }
    }
    if g.button(px + 16.0, py + ph - 34.0, 110.0, "Cancel") || esc {
        g.modal.icons = None;
    }
}

/// The right side of a tab: a form in a scrolled area. `fill` lays out the rows from the `y` it is given.
fn form(g: &mut Gui, p: &mut Pick, x: f32, top: f32, w: f32, bottom: f32, fill: impl FnOnce(&mut Gui, &mut f32, &mut Pick)) {
    let area = [x - 4.0, top, w + 8.0, bottom - top];
    if g.inside(area[0], area[1], area[2], area[3]) {
        let wheel = std::mem::take(&mut g.input.wheel);
        p.form_scroll = (p.form_scroll - wheel * 40.0).max(0.0);
    }
    g.clip = Some(area);
    g.restore_clip();
    let mut y = top - p.form_scroll;
    fill(g, &mut y, p);
    g.clip = None;
    g.restore_clip();
    // Not scrolled past the end.
    p.form_scroll = p.form_scroll.min((y + p.form_scroll - bottom).max(0.0));
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
        let barn = Prop { model: r"World\wmo\Azeroth\Buildings\Human_Barn_Silo\barn.wmo".into(), map: None, zone: None, pickup: None, sleep: false, physics: false, at: (-8915.0, -150.0, 85.0), rotation: [0.0, 90.0, 0.0], scale: 1.5 };
        content::write_props(std::slice::from_ref(&barn)).unwrap();
        let text = std::fs::read_to_string(dir.join("props.ron")).unwrap();
        assert_eq!(ron::from_str::<Vec<Prop>>(&text).unwrap(), vec![barn]);
        assert!(!dir.join("props.ron.tmp").exists());
        std::env::remove_var("RSWORLD_CONTENT");
        std::fs::remove_dir_all(&dir).unwrap();
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
        let barn = Prop { model: r"World\wmo\Azeroth\Buildings\Human_Barn_Silo\barn.wmo".into(), map: None, zone: None, pickup: None, sleep: false, physics: false, at: (-8915.0, -150.0, 85.0), rotation: [0.0, 90.0, 0.0], scale: 1.0 };
        cur.props.extend([barn.clone(), barn]);
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
        let mut bed = Prop { model: "x.m2".into(), map: None, zone: None, pickup: None, sleep: false, physics: false, at: (0.0, 0.0, 0.0), rotation: [0.0; 3], scale: 1.0 };
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
