//! The original's interface, loaded from its own XML (`Interface\FrameXML\*.xml`, listed by `FrameXML.toc`): frame
//! definitions with their templates and inheritance, anchors, textures with texture coordinates, font strings with the
//! fonts of `Fonts.xml`, and button states. There is no Lua: the scripts are ignored and the game fills in what they
//! would (texts, item icons) and reacts to clicks itself, by frame name.
//!
//! Coordinates are the original's interface units: the screen is 768 units high, `x` runs right and `y` up from the
//! bottom, as in `SetPoint` offsets.

use crate::font::GlyphSet;
use crate::renderer::Gpu;
use crate::ui::{Quad, Ui, FULL_UV, WHITE};
use anyhow::{ensure, Context, Result};
use mpq::Mount;
use roxmltree::{Document, Node as Xml};
use std::collections::HashMap;
use std::rc::Rc;
use wowfmt::blp::Blp;

pub const UI_HEIGHT: f32 = 768.0;

/// How big the interface is drawn (the video options' UI scale; the game sets it, 1 elsewhere).
static UI_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

pub fn set_ui_scale(scale: f32) {
    UI_SCALE.store(scale.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// The height of the screen in interface units: 768, more when the interface is scaled down.
pub fn ui_height() -> f32 {
    UI_HEIGHT / f32::from_bits(UI_SCALE.load(std::sync::atomic::Ordering::Relaxed))
}

/// One line of a tooltip: text on the left, optionally more on the right, in a font object.
#[derive(Clone, Debug)]
pub struct TipLine {
    pub left: String,
    pub right: Option<String>,
    pub color: [f32; 4],
    pub right_color: [f32; 4],
    pub font: &'static str,
}

impl TipLine {
    pub fn new(text: impl Into<String>, color: [f32; 4]) -> TipLine {
        TipLine { left: text.into(), right: None, color, right_color: color, font: "GameTooltipText" }
    }

    pub fn double(left: impl Into<String>, right: impl Into<String>, color: [f32; 4], right_color: [f32; 4]) -> TipLine {
        TipLine { left: left.into(), right: Some(right.into()), color, right_color, font: "GameTooltipText" }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Point {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Point {
    fn parse(s: &str) -> Option<Point> {
        Some(match s {
            "TOPLEFT" => Point::TopLeft,
            "TOP" => Point::Top,
            "TOPRIGHT" => Point::TopRight,
            "LEFT" => Point::Left,
            "CENTER" => Point::Center,
            "RIGHT" => Point::Right,
            "BOTTOMLEFT" => Point::BottomLeft,
            "BOTTOM" => Point::Bottom,
            "BOTTOMRIGHT" => Point::BottomRight,
            _ => return None,
        })
    }

    /// How far along the rectangle the point is: (0 left .. 1 right, 0 bottom .. 1 top).
    fn frac(self) -> (f32, f32) {
        match self {
            Point::TopLeft => (0.0, 1.0),
            Point::Top => (0.5, 1.0),
            Point::TopRight => (1.0, 1.0),
            Point::Left => (0.0, 0.5),
            Point::Center => (0.5, 0.5),
            Point::Right => (1.0, 0.5),
            Point::BottomLeft => (0.0, 0.0),
            Point::Bottom => (0.5, 0.0),
            Point::BottomRight => (1.0, 0.0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn at(&self, p: Point) -> (f32, f32) {
        let (fx, fy) = p.frac();
        (self.x + self.w * fx, self.y + self.h * fy)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Clone, Debug)]
struct AnchorDef {
    point: Point,
    relative_to: Option<String>,
    relative_point: Option<Point>,
    x: f32,
    y: f32,
}

#[derive(Clone, Default, Debug)]
struct TextureDef {
    name: Option<String>,
    /// Virtual textures this one takes what it does not say itself from.
    inherits: Vec<String>,
    file: Option<String>,
    size: Option<(f32, f32)>,
    anchors: Vec<AnchorDef>,
    set_all_points: bool,
    /// left, right, top, bottom
    tex_coords: Option<[f32; 4]>,
    color: Option<[f32; 4]>,
    additive: bool,
    hidden: bool,
}

#[derive(Clone, Default, Debug)]
struct StringDef {
    name: Option<String>,
    font: Option<String>,
    size: Option<(f32, f32)>,
    anchors: Vec<AnchorDef>,
    set_all_points: bool,
    text: Option<String>,
    justify_h: Option<String>,
    justify_v: Option<String>,
    color: Option<[f32; 4]>,
    hidden: bool,
}

#[derive(Clone, Debug)]
enum RegionDef {
    Tex(TextureDef),
    Str(StringDef),
}

impl RegionDef {
    fn name(&self) -> Option<&str> {
        match self {
            RegionDef::Tex(t) => t.name.as_deref(),
            RegionDef::Str(s) => s.name.as_deref(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Level {
    Background,
    Border,
    Artwork,
    Overlay,
    Highlight,
}

#[derive(Clone, Debug)]
struct LayerDef {
    level: Level,
    regions: Vec<RegionDef>,
}

/// Button textures: normal, pushed, disabled, highlight, checked.
const BUTTON_TEXTURES: [&str; 5] = ["NormalTexture", "PushedTexture", "DisabledTexture", "HighlightTexture", "CheckedTexture"];

/// A frame's border and background (`<Backdrop>`): a tiled or stretched background inside the insets and an edge strip.
#[derive(Clone, Default, Debug)]
struct BackdropDef {
    bg_file: Option<String>,
    edge_file: Option<String>,
    tile: bool,
    tile_size: f32,
    edge_size: f32,
    /// left, right, top, bottom
    insets: [f32; 4],
    color: Option<[f32; 4]>,
    border_color: Option<[f32; 4]>,
}

#[derive(Clone, Default, Debug)]
pub struct FrameDef {
    tag: String,
    name: Option<String>,
    inherits: Vec<String>,
    is_virtual: bool,
    hidden: Option<bool>,
    parent: Option<String>,
    id: Option<i32>,
    text: Option<String>,
    size: Option<(f32, f32)>,
    anchors: Vec<AnchorDef>,
    set_all_points: Option<bool>,
    /// left, right, top, bottom: how far the clickable area is inside the frame.
    hit_insets: Option<[f32; 4]>,
    enable_mouse: Option<bool>,
    layers: Vec<LayerDef>,
    children: Vec<FrameDef>,
    button: [Option<TextureDef>; 5],
    /// The font object of the label of a button.
    button_font: Option<String>,
    /// `justifyH` of the normal font: how the button's text is aligned.
    button_justify: Option<String>,
    button_text: Option<StringDef>,
    backdrop: Option<BackdropDef>,
    /// A status bar: the picture it is filled with and its colour.
    bar_file: Option<String>,
    bar_color: Option<[f32; 4]>,
    /// How much above its siblings the frame is drawn (`SetFrameLevel(this:GetFrameLevel() + n)` in its `OnLoad`).
    level: Option<i32>,
    /// `this:SetWidth(n)` / `this:SetHeight(n)` in its `OnLoad`.
    set_width: Option<f32>,
    set_height: Option<f32>,
}

#[derive(Clone, Default, Debug)]
struct FontDef {
    inherits: Option<String>,
    file: Option<String>,
    height: Option<f32>,
    color: Option<[f32; 4]>,
    outline: Option<usize>,
    shadow: Option<((f32, f32), [f32; 4])>,
}

/// A font object resolved through its inheritance.
#[derive(Clone, Debug)]
struct FontStyle {
    file: String,
    height: f32,
    color: [f32; 4],
    outline: usize,
    shadow: Option<((f32, f32), [f32; 4])>,
}

// ---------------------------------------------------------------------------------------------------------------
// Parsing

fn attr<'a>(n: &Xml<'a, '_>, name: &str) -> Option<&'a str> {
    n.attribute(name)
}

fn flag(n: &Xml, name: &str) -> Option<bool> {
    attr(n, name).map(|v| v.eq_ignore_ascii_case("true"))
}

fn num(n: &Xml, name: &str) -> Option<f32> {
    attr(n, name).and_then(|v| v.trim().parse().ok())
}

fn child<'a, 'b>(n: &Xml<'a, 'b>, tag: &str) -> Option<Xml<'a, 'b>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == tag)
}

/// A pair given as `x`/`y` attributes of the node or of its `AbsDimension` child.
fn pair(n: &Xml) -> Option<(f32, f32)> {
    let src = child(n, "AbsDimension").unwrap_or(*n);
    Some((num(&src, "x")?, num(&src, "y")?))
}

fn rgba(n: &Xml) -> [f32; 4] {
    [num(n, "r").unwrap_or(1.0), num(n, "g").unwrap_or(1.0), num(n, "b").unwrap_or(1.0), num(n, "a").unwrap_or(1.0)]
}

fn anchors(n: &Xml) -> Vec<AnchorDef> {
    let Some(list) = child(n, "Anchors") else { return Vec::new() };
    list.children()
        .filter(|c| c.is_element() && c.tag_name().name() == "Anchor")
        .filter_map(|a| {
            let (x, y) = child(&a, "Offset").and_then(|o| pair(&o)).unwrap_or((0.0, 0.0));
            Some(AnchorDef {
                point: Point::parse(attr(&a, "point")?)?,
                relative_to: attr(&a, "relativeTo").map(String::from),
                relative_point: attr(&a, "relativePoint").and_then(Point::parse),
                x,
                y,
            })
        })
        .collect()
}

fn texture_def(n: &Xml) -> TextureDef {
    TextureDef {
        name: attr(n, "name").map(String::from),
        inherits: attr(n, "inherits").map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default(),
        file: attr(n, "file").map(String::from),
        size: child(n, "Size").and_then(|s| pair(&s)),
        anchors: anchors(n),
        set_all_points: flag(n, "setAllPoints").unwrap_or(false),
        tex_coords: child(n, "TexCoords").map(|t| [num(&t, "left").unwrap_or(0.0), num(&t, "right").unwrap_or(1.0), num(&t, "top").unwrap_or(0.0), num(&t, "bottom").unwrap_or(1.0)]),
        color: child(n, "Color").map(|c| rgba(&c)),
        additive: attr(n, "alphaMode") == Some("ADD"),
        hidden: flag(n, "hidden").unwrap_or(false),
    }
}

fn string_def(n: &Xml) -> StringDef {
    StringDef {
        name: attr(n, "name").map(String::from),
        font: attr(n, "inherits").map(String::from),
        size: child(n, "Size").and_then(|s| pair(&s)),
        anchors: anchors(n),
        set_all_points: flag(n, "setAllPoints").unwrap_or(false),
        text: attr(n, "text").map(String::from),
        justify_h: attr(n, "justifyH").map(String::from),
        justify_v: attr(n, "justifyV").map(String::from),
        color: child(n, "Color").map(|c| rgba(&c)),
        hidden: flag(n, "hidden").unwrap_or(false),
    }
}

fn frame_def(n: &Xml) -> FrameDef {
    let mut f = FrameDef {
        tag: n.tag_name().name().to_string(),
        name: attr(n, "name").map(String::from),
        inherits: attr(n, "inherits").map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default(),
        is_virtual: flag(n, "virtual").unwrap_or(false),
        hidden: flag(n, "hidden"),
        parent: attr(n, "parent").map(String::from),
        id: attr(n, "id").and_then(|v| v.parse().ok()),
        text: attr(n, "text").map(String::from),
        size: child(n, "Size").and_then(|s| pair(&s)),
        anchors: anchors(n),
        set_all_points: flag(n, "setAllPoints"),
        hit_insets: child(n, "HitRectInsets").and_then(|h| child(&h, "AbsInset")).map(|i| [num(&i, "left").unwrap_or(0.0), num(&i, "right").unwrap_or(0.0), num(&i, "top").unwrap_or(0.0), num(&i, "bottom").unwrap_or(0.0)]),
        enable_mouse: flag(n, "enableMouse"),
        ..Default::default()
    };
    for c in n.children().filter(|c| c.is_element()) {
        match c.tag_name().name() {
            "Layers" => {
                for layer in c.children().filter(|l| l.is_element() && l.tag_name().name() == "Layer") {
                    let level = match attr(&layer, "level") {
                        Some("BORDER") => Level::Border,
                        Some("ARTWORK") => Level::Artwork,
                        Some("OVERLAY") => Level::Overlay,
                        Some("HIGHLIGHT") => Level::Highlight,
                        _ => Level::Background,
                    };
                    let regions = layer
                        .children()
                        .filter(|r| r.is_element())
                        .filter_map(|r| match r.tag_name().name() {
                            "Texture" => Some(RegionDef::Tex(texture_def(&r))),
                            "FontString" => Some(RegionDef::Str(string_def(&r))),
                            _ => None,
                        })
                        .collect();
                    f.layers.push(LayerDef { level, regions });
                }
            }
            "Frames" => f.children.extend(c.children().filter(|k| k.is_element()).map(|k| frame_def(&k))),
            // The frame a scroll frame scrolls: here it sits at the scroll frame's top left corner, unscrolled.
            "ScrollChild" => f.children.extend(c.children().filter(|k| k.is_element()).map(|k| {
                let mut def = frame_def(&k);
                if def.anchors.is_empty() {
                    def.anchors.push(AnchorDef { point: Point::TopLeft, relative_to: None, relative_point: None, x: 0.0, y: 0.0 });
                }
                def
            })),
            "ButtonText" => {
                let mut label = string_def(&c);
                label.name.get_or_insert_with(|| "$parentText".to_string());
                f.button_text = Some(label);
            }
            "Scripts" => {
                // Only one thing a script does matters for drawing: raising the frame above its siblings.
                let on_load = child(&c, "OnLoad").and_then(|l| l.text()).unwrap_or("");
                let number = |call: &str| on_load.split(call).nth(1).and_then(|t| t.trim().split(|ch: char| !(ch.is_ascii_digit() || ch == '.')).next()?.parse::<f32>().ok());
                f.level = number("SetFrameLevel(this:GetFrameLevel() +").map(|n| n as i32);
                (f.set_width, f.set_height) = (number("this:SetWidth("), number("this:SetHeight("));
            }
            "Backdrop" => {
                let val = |tag: &str| child(&c, tag).and_then(|e| child(&e, "AbsValue")).and_then(|v| num(&v, "val"));
                let insets = child(&c, "BackgroundInsets").and_then(|h| child(&h, "AbsInset")).map(|i| [num(&i, "left").unwrap_or(0.0), num(&i, "right").unwrap_or(0.0), num(&i, "top").unwrap_or(0.0), num(&i, "bottom").unwrap_or(0.0)]);
                f.backdrop = Some(BackdropDef {
                    bg_file: attr(&c, "bgFile").map(String::from),
                    edge_file: attr(&c, "edgeFile").map(String::from),
                    tile: flag(&c, "tile").unwrap_or(false),
                    tile_size: val("TileSize").unwrap_or(0.0),
                    edge_size: val("EdgeSize").unwrap_or(0.0),
                    insets: insets.unwrap_or([0.0; 4]),
                    color: child(&c, "Color").map(|e| rgba(&e)),
                    border_color: child(&c, "BorderColor").map(|e| rgba(&e)),
                });
            }
            // The knob of a slider: a picture on top of the frame.
            "ThumbTexture" => f.layers.push(LayerDef { level: Level::Overlay, regions: vec![RegionDef::Tex(texture_def(&c))] }),
            "BarTexture" => f.bar_file = attr(&c, "file").map(String::from),
            "BarColor" => f.bar_color = Some(rgba(&c)),
            "NormalFont" => {
                f.button_font = attr(&c, "inherits").or(attr(&c, "style")).map(String::from);
                f.button_justify = attr(&c, "justifyH").map(String::from);
            }
            tag => {
                if let Some(i) = BUTTON_TEXTURES.iter().position(|t| *t == tag) {
                    f.button[i] = Some(texture_def(&c));
                }
            }
        }
    }
    f
}

fn font_def(n: &Xml) -> FontDef {
    FontDef {
        inherits: attr(n, "inherits").map(String::from),
        file: attr(n, "font").map(String::from),
        height: child(n, "FontHeight").and_then(|h| child(&h, "AbsValue")).and_then(|v| num(&v, "val")),
        color: child(n, "Color").map(|c| rgba(&c)),
        outline: attr(n, "outline").map(|o| match o {
            "THICK" => 2,
            "NORMAL" => 1,
            _ => 0,
        }),
        shadow: child(n, "Shadow").map(|s| (child(&s, "Offset").and_then(|o| pair(&o)).unwrap_or((1.0, -1.0)), child(&s, "Color").map(|c| rgba(&c)).unwrap_or([0.0, 0.0, 0.0, 1.0]))),
    }
}

/// Every frame definition, font object and global string of the original's interface files.
#[derive(Default)]
pub struct Registry {
    frames: HashMap<String, FrameDef>,
    /// The non-virtual top-level frames, in load order.
    order: Vec<String>,
    fonts: HashMap<String, FontDef>,
    strings: HashMap<String, String>,
    /// The named textures (the virtual ones are templates for the pictures of buttons and the like).
    textures: HashMap<String, TextureDef>,
}

impl Registry {
    /// Read the files `FrameXML.toc` lists (the Lua ones only for `GlobalStrings.lua`).
    pub fn load(mount: &Mount) -> Result<Registry> {
        let mut reg = Registry::load_toc(mount, r"Interface\FrameXML\", "FrameXML.toc", "GlobalStrings.lua")?;
        // The talent window is an addon that loads on demand in the original (`Blizzard_TalentUI.toc`).
        reg.load_xml(mount, r"Interface\AddOns\Blizzard_TalentUI\Blizzard_TalentUI.xml")?;
        Ok(reg)
    }

    /// The screens before the game (character selection and creation): the files `GlueXML.toc` lists.
    pub fn load_glue(mount: &Mount) -> Result<Registry> {
        Registry::load_toc(mount, r"Interface\GlueXML\", "GlueXML.toc", "GlueStrings.lua")
    }

    fn load_toc(mount: &Mount, base: &str, toc: &str, strings: &str) -> Result<Registry> {
        let toc = String::from_utf8_lossy(&mount.read(&format!("{base}{toc}")).with_context(|| toc.to_string())?).into_owned();
        let mut reg = Registry::default();
        for line in toc.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if line.eq_ignore_ascii_case(strings) {
                if let Ok(bytes) = mount.read(&format!("{base}{line}")) {
                    // The texts of the screens before the game are in Windows-1252 (a curly apostrophe is the byte 0x92).
                    let text = String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| if b == 0x92 { '\'' } else { b as char }).collect());
                    reg.load_strings(&text.replace('\u{2019}', "'"));
                }
            } else if line.to_ascii_lowercase().ends_with(".xml") {
                if let Err(e) = reg.load_xml(mount, &format!("{base}{line}")) {
                    log::debug!("{line}: {e:#}");
                }
            }
        }
        Ok(reg)
    }

    /// `NAME = "text";` lines of `GlobalStrings.lua`.
    fn load_strings(&mut self, lua: &str) {
        for line in lua.lines() {
            let Some((key, rest)) = line.split_once('=') else { continue };
            let (key, rest) = (key.trim(), rest.trim());
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || !rest.starts_with('"') {
                continue;
            }
            // The string ends at the last unescaped quote.
            let body = &rest[1..];
            let mut out = String::new();
            let mut chars = body.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => match chars.next() {
                        Some('n') => out.push('\n'),
                        Some(o) => out.push(o),
                        None => break,
                    },
                    '"' => break,
                    c => out.push(c),
                }
            }
            self.strings.insert(key.to_string(), out);
        }
    }

    fn load_xml(&mut self, mount: &Mount, path: &str) -> Result<()> {
        let text = String::from_utf8_lossy(&mount.read(path)?).into_owned();
        let doc = Document::parse(&text)?;
        for n in doc.root_element().children().filter(|c| c.is_element()) {
            match n.tag_name().name() {
                "Script" => {}
                "Include" => {
                    if let Some(f) = attr(&n, "file") {
                        let dir = path.rsplit_once('\\').map_or("", |(d, _)| d);
                        let _ = self.load_xml(mount, &format!("{dir}\\{f}"));
                    }
                }
                "Font" => {
                    if let Some(name) = attr(&n, "name") {
                        self.fonts.insert(name.to_string(), font_def(&n));
                    }
                }
                "Texture" => {
                    if let Some(name) = attr(&n, "name") {
                        self.textures.insert(name.to_string(), texture_def(&n));
                    }
                }
                "FontString" => {}
                _ => {
                    let def = frame_def(&n);
                    if let Some(name) = def.name.clone() {
                        if !def.is_virtual {
                            self.order.push(name.clone());
                        }
                        self.frames.insert(name, def);
                    }
                }
            }
        }
        Ok(())
    }

    /// A texture with what it inherits filled in: its own file, size, anchors and so on win over those of its templates.
    fn resolve_texture(&self, def: &TextureDef, depth: usize) -> TextureDef {
        let mut out = TextureDef::default();
        if depth < 8 {
            for parent in def.inherits.iter().filter_map(|n| self.textures.get(n)) {
                let p = self.resolve_texture(parent, depth + 1);
                out = TextureDef {
                    file: p.file.or(out.file),
                    size: p.size.or(out.size),
                    anchors: if p.anchors.is_empty() { out.anchors } else { p.anchors },
                    set_all_points: p.set_all_points || out.set_all_points,
                    tex_coords: p.tex_coords.or(out.tex_coords),
                    color: p.color.or(out.color),
                    additive: p.additive || out.additive,
                    hidden: p.hidden || out.hidden,
                    ..out
                };
            }
        }
        TextureDef {
            name: def.name.clone(),
            inherits: Vec::new(),
            file: def.file.clone().or(out.file),
            size: def.size.or(out.size),
            anchors: if def.anchors.is_empty() { out.anchors } else { def.anchors.clone() },
            set_all_points: def.set_all_points || out.set_all_points,
            tex_coords: def.tex_coords.or(out.tex_coords),
            color: def.color.or(out.color),
            additive: def.additive || out.additive,
            hidden: def.hidden || out.hidden,
        }
    }

    /// A global string, or the key itself.
    pub fn string(&self, key: &str) -> String {
        self.strings.get(key).cloned().unwrap_or_else(|| key.to_string())
    }

    fn font_style(&self, name: &str) -> Option<FontStyle> {
        // Collect the chain from the object up to its roots, then apply from the root down.
        let mut chain = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            let Some(def) = self.fonts.get(&n) else { break };
            chain.push(def);
            cur = def.inherits.clone();
            if chain.len() > 16 {
                break;
            }
        }
        let mut style = FontStyle { file: String::new(), height: 12.0, color: [1.0; 4], outline: 0, shadow: None };
        for def in chain.iter().rev() {
            if let Some(f) = &def.file {
                style.file = f.clone();
            }
            style.height = def.height.unwrap_or(style.height);
            style.color = def.color.unwrap_or(style.color);
            style.outline = def.outline.unwrap_or(style.outline);
            style.shadow = def.shadow.or(style.shadow);
        }
        (!style.file.is_empty()).then_some(style)
    }

    /// The definition with its templates merged in (the template first, the frame's own settings on top).
    fn resolve(&self, def: &FrameDef, depth: usize) -> FrameDef {
        let mut out = FrameDef::default();
        if depth < 16 {
            for t in &def.inherits {
                if let Some(template) = self.frames.get(t) {
                    out = merge(out, self.resolve(template, depth + 1));
                }
            }
        }
        merge(out, def.clone())
    }
}

/// `over` applied on top of `base`.
fn merge(base: FrameDef, over: FrameDef) -> FrameDef {
    let mut out = base;
    if !over.tag.is_empty() {
        out.tag = over.tag;
    }
    out.name = over.name.or(out.name);
    out.is_virtual = over.is_virtual;
    out.hidden = over.hidden.or(out.hidden);
    out.parent = over.parent.or(out.parent);
    out.id = over.id.or(out.id);
    out.text = over.text.or(out.text);
    out.size = over.size.or(out.size);
    if !over.anchors.is_empty() {
        out.anchors = over.anchors;
    }
    out.set_all_points = over.set_all_points.or(out.set_all_points);
    out.hit_insets = over.hit_insets.or(out.hit_insets);
    out.enable_mouse = over.enable_mouse.or(out.enable_mouse);
    // A region or child with the name of a template's replaces it.
    let overriding: Vec<&str> = over.layers.iter().flat_map(|l| l.regions.iter().filter_map(|r| r.name())).collect();
    for layer in &mut out.layers {
        layer.regions.retain(|r| r.name().is_none_or(|n| !overriding.contains(&n)));
    }
    out.layers.extend(over.layers);
    let child_names: Vec<String> = over.children.iter().filter_map(|c| c.name.clone()).collect();
    out.children.retain(|c| c.name.as_ref().is_none_or(|n| !child_names.contains(n)));
    out.children.extend(over.children);
    for (slot, t) in out.button.iter_mut().zip(over.button) {
        *slot = t.or(slot.take());
    }
    out.button_font = over.button_font.or(out.button_font);
    out.button_justify = over.button_justify.or(out.button_justify);
    out.button_text = over.button_text.or(out.button_text);
    out.backdrop = over.backdrop.or(out.backdrop);
    out.bar_file = over.bar_file.or(out.bar_file);
    out.bar_color = over.bar_color.or(out.bar_color);
    out.level = over.level.or(out.level);
    out.set_width = over.set_width.or(out.set_width);
    out.set_height = over.set_height.or(out.set_height);
    out
}

// ---------------------------------------------------------------------------------------------------------------
// The instantiated interface

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Layer(Level),
    Normal,
    Pushed,
    Disabled,
    Highlight,
    Checked,
}

#[derive(Clone, Debug)]
enum Kind {
    Texture { file: Option<String>, id: Option<usize>, coords: [f32; 4], color: Option<[f32; 4]>, additive: bool },
    Text { font: Option<String>, text: String, justify_h: String, justify_v: String, color: Option<[f32; 4]> },
}

#[derive(Clone, Debug)]
struct Region {
    name: String,
    kind: Kind,
    role: Role,
    size: Option<(f32, f32)>,
    anchors: Vec<AnchorDef>,
    set_all: bool,
    hidden: bool,
}

pub struct Node {
    pub name: String,
    parent: Option<usize>,
    children: Vec<usize>,
    regions: Vec<Region>,
    shown: bool,
    size: Option<(f32, f32)>,
    anchors: Vec<AnchorDef>,
    set_all: bool,
    hit_insets: [f32; 4],
    mouse: bool,
    backdrop: Option<BackdropDef>,
    /// `SetScale`: this frame and everything in it is drawn this much larger.
    scale: f32,
    /// How opaque the frame and everything in it is drawn (`SetAlpha`).
    alpha: f32,
    model_texture: Option<usize>,
    level: i32,
    /// A scroll frame: what is in it is cut to its rectangle and can be moved up (`scrolled` units) by the wheel.
    scroll: bool,
    scrolled: f32,
    /// A status bar: the picture, the colour and how much of the frame it fills (0 to 1).
    bar: Option<Bar>,
}

struct Bar {
    file: String,
    color: [f32; 4],
    value: f32,
}

pub struct Tree {
    registry: Rc<Registry>,
    pub nodes: Vec<Node>,
    by_name: HashMap<String, usize>,
    regions_by_name: HashMap<String, (usize, usize)>,
    /// Frame indices in the order they were drawn last, for hit testing.
    drawn: Vec<usize>,
    pub hover: Option<usize>,
    pub pressed: Option<usize>,
    pub checked: Vec<usize>,
    textures: HashMap<String, Option<usize>>,
    font_files: HashMap<String, Option<fontdue::Font>>,
    glyphs: HashMap<(String, u32, usize), Option<GlyphSet>>,
    screen: (u32, u32),
    /// Multiplies the size of drawn text (floating unit names shrink with distance).
    text_scale: f32,
    /// The size of the text of each named font string whose size comes from its text (set by `measure_texts` before
    /// drawing), so that regions anchored to it, like the buttons under a greeting, land below its last line.
    text_dims: HashMap<String, (f32, f32)>,
    /// Buttons that are disabled (`set_enabled`).
    disabled: std::collections::HashSet<usize>,
    /// The money frames and what they show: copper, whether the coins are the small ones, and whether it is the
    /// player's purse (which also shows the smaller coins under the biggest one).
    moneys: HashMap<String, (u32, bool, bool)>,
}

/// Interface units a wheel notch scrolls.
const SCROLL_STEP: f32 = 24.0;

/// What a text is aligned to inside its rectangle.
fn justify(h: &str) -> f32 {
    match h {
        "LEFT" => 0.0,
        "RIGHT" => 1.0,
        _ => 0.5,
    }
}

impl Tree {
    pub fn new(registry: Rc<Registry>) -> Tree {
        let root = Node { name: "UIParent".into(), parent: None, children: Vec::new(), regions: Vec::new(), shown: true, size: None, anchors: Vec::new(), set_all: false, hit_insets: [0.0; 4], mouse: false, backdrop: None, scale: 1.0, alpha: 1.0, model_texture: None, level: 0, scroll: false, scrolled: 0.0, bar: None };
        Tree {
            registry,
            nodes: vec![root],
            by_name: HashMap::from([("UIParent".to_string(), 0)]),
            regions_by_name: HashMap::new(),
            drawn: Vec::new(),
            hover: None,
            pressed: None,
            checked: Vec::new(),
            textures: HashMap::new(),
            font_files: HashMap::new(),
            glyphs: HashMap::new(),
            text_dims: HashMap::new(),
            disabled: Default::default(),
            moneys: HashMap::new(),
            screen: (1280, 720),
            text_scale: 1.0,
        }
    }

    /// A global string of the original (`BLOCK` is "Block").
    pub fn string(&self, key: &str) -> String {
        self.registry.string(key)
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    pub fn name(&self, node: usize) -> &str {
        &self.nodes[node].name
    }

    /// Instantiate a named frame of the files under the screen, and every top-level frame that names it, or one of
    /// the frames made so far, as its parent.
    pub fn instantiate(&mut self, name: &str) -> Result<usize> {
        let def = self.registry.frames.get(name).cloned().with_context(|| format!("no frame {name}"))?;
        let idx = self.build(&def, 0);
        loop {
            let pending: Vec<String> = self
                .registry
                .order
                .iter()
                .filter(|n| !self.by_name.contains_key(*n))
                .filter(|n| self.registry.frames[*n].parent.as_ref().is_some_and(|p| p != "UIParent" && self.by_name.contains_key(p)))
                .cloned()
                .collect();
            if pending.is_empty() {
                break;
            }
            for n in pending {
                let def = self.registry.frames[&n].clone();
                let parent = self.by_name[def.parent.as_ref().unwrap()];
                self.build(&def, parent);
            }
        }
        Ok(idx)
    }

    /// Make a frame named `name` from the virtual template `template` (e.g. `OptionsSliderTemplate`), a child of the frame `parent`
    /// that was made already; its parts are named after it, as they are when the files make one.
    pub fn instantiate_template(&mut self, template: &str, name: &str, parent: &str) -> Result<usize> {
        let parent = *self.by_name.get(parent).with_context(|| format!("no frame {parent}"))?;
        ensure!(self.registry.frames.contains_key(template), "no template {template}");
        let def = FrameDef { name: Some(name.to_string()), inherits: vec![template.to_string()], ..Default::default() };
        Ok(self.build(&def, parent))
    }

    fn build(&mut self, def: &FrameDef, parent: usize) -> usize {
        let def = self.registry.resolve(def, 0);
        let parent_name = self.nodes[parent].name.clone();
        let name = def.name.clone().map_or_else(|| format!("{parent_name}#{}", self.nodes.len()), |n| n.replace("$parent", &parent_name));
        let button = def.tag.contains("Button");
        let idx = self.nodes.len();
        let node = Node {
            name: name.clone(),
            parent: Some(parent),
            children: Vec::new(),
            regions: Vec::new(),
            shown: !def.hidden.unwrap_or(false),
            size: match (def.size, def.set_width, def.set_height) {
                (size, None, None) => size,
                (size, w, h) => {
                    let (cw, ch) = size.unwrap_or((0.0, 0.0));
                    Some((w.unwrap_or(cw), h.unwrap_or(ch)))
                }
            },
            // `$parent` in an anchor of a frame is the frame's parent.
            anchors: def.anchors.iter().map(|a| AnchorDef { relative_to: a.relative_to.as_deref().map(|r| r.replace("$parent", &parent_name)), ..a.clone() }).collect(),
            set_all: def.set_all_points.unwrap_or(false),
            hit_insets: def.hit_insets.unwrap_or([0.0; 4]),
            mouse: def.enable_mouse.unwrap_or(button),
            backdrop: def.backdrop.clone(),
            scale: 1.0,
            alpha: 1.0,
            model_texture: None,
            scroll: def.tag == "ScrollFrame",
            scrolled: 0.0,
            bar: def.bar_file.clone().map(|file| Bar { file, color: def.bar_color.unwrap_or([1.0; 4]), value: 1.0 }),
            level: def.level.unwrap_or(0),
        };
        self.nodes.push(node);
        self.nodes[parent].children.push(idx);
        self.by_name.insert(name.clone(), idx);

        let sub = |s: &str| s.replace("$parent", &name);
        let mut regions = Vec::new();
        for layer in &def.layers {
            for r in &layer.regions {
                regions.push(self.region(r, Role::Layer(layer.level), &sub));
            }
        }
        for (i, role) in [Role::Normal, Role::Pushed, Role::Disabled, Role::Highlight, Role::Checked].into_iter().enumerate() {
            if let Some(t) = &def.button[i] {
                regions.push(self.region(&RegionDef::Tex(t.clone()), role, &sub));
            }
        }
        // A button that has a font has a text, named `$parentText`, even when its XML does not declare it.
        let implicit = (button && def.button_text.is_none() && def.button_font.is_some()).then(|| StringDef { name: Some("$parentText".to_string()), ..Default::default() });
        if let Some(label) = def.button_text.as_ref().or(implicit.as_ref()) {
            let mut s = label.clone();
            if s.font.is_none() {
                s.font = def.button_font.clone();
            }
            s.justify_h = s.justify_h.or(def.button_justify.clone());
            s.text = def.text.clone().or(s.text);
            if s.anchors.is_empty() && !s.set_all_points {
                s.set_all_points = true;
            }
            regions.push(self.region(&RegionDef::Str(s), Role::Layer(Level::Artwork), &sub));
        }
        for (i, r) in regions.iter().enumerate() {
            self.regions_by_name.insert(r.name.clone(), (idx, i));
        }
        self.nodes[idx].regions = regions;
        for c in &def.children {
            self.build(c, idx);
        }
        idx
    }

    fn region(&self, def: &RegionDef, role: Role, sub: &dyn Fn(&str) -> String) -> Region {
        let fix = |anchors: &[AnchorDef]| -> Vec<AnchorDef> {
            anchors.iter().map(|a| AnchorDef { relative_to: a.relative_to.as_deref().map(sub), ..a.clone() }).collect()
        };
        match def {
            RegionDef::Tex(t) => {
                let t = &self.registry.resolve_texture(t, 0);
                Region {
                name: t.name.as_deref().map(sub).unwrap_or_default(),
                kind: Kind::Texture { file: t.file.clone(), id: None, coords: t.tex_coords.unwrap_or([0.0, 1.0, 0.0, 1.0]), color: t.color, additive: t.additive },
                role,
                size: t.size,
                anchors: fix(&t.anchors),
                set_all: t.set_all_points,
                hidden: t.hidden,
                }
            }
            RegionDef::Str(s) => Region {
                name: s.name.as_deref().map(sub).unwrap_or_default(),
                kind: Kind::Text {
                    font: s.font.clone(),
                    text: s.text.as_deref().map(|t| self.registry.string(t)).unwrap_or_default(),
                    justify_h: s.justify_h.clone().unwrap_or_default(),
                    justify_v: s.justify_v.clone().unwrap_or_default(),
                    color: s.color,
                },
                role,
                size: s.size,
                anchors: fix(&s.anchors),
                set_all: s.set_all_points,
                hidden: s.hidden,
            },
        }
    }

    // ---- changing what is shown

    fn region_mut(&mut self, name: &str) -> Option<&mut Region> {
        let (n, r) = *self.regions_by_name.get(name)?;
        self.nodes[n].regions.get_mut(r)
    }

    pub fn set_text(&mut self, region: &str, text: &str) {
        if let Some(Region { kind: Kind::Text { text: t, .. }, .. }) = self.region_mut(region) {
            *t = text.to_string();
        }
    }

    /// Put a texture file on a texture region (`None` clears it); its coordinates become the whole image.
    pub fn set_texture(&mut self, region: &str, file: Option<&str>) {
        if let Some(Region { kind: Kind::Texture { file: f, id, coords, .. }, .. }) = self.region_mut(region) {
            *f = file.map(String::from);
            *id = None;
            *coords = [0.0, 1.0, 0.0, 1.0];
        }
    }

    /// Show an interface texture (an offscreen picture) on a texture region.
    pub fn set_region_texture(&mut self, region: &str, texture: Option<usize>) {
        if let Some(Region { kind: Kind::Texture { id, .. }, .. }) = self.region_mut(region) {
            *id = texture;
        }
    }

    /// Show an interface texture over a whole model frame (the character's paper doll).
    pub fn set_model_texture(&mut self, name: &str, texture: Option<usize>) {
        if let Some(&i) = self.by_name.get(name) {
            self.nodes[i].model_texture = texture;
        }
    }

    pub fn set_region_hidden(&mut self, region: &str, hidden: bool) {
        if let Some(r) = self.region_mut(region) {
            r.hidden = hidden;
        }
    }

    /// The font of a font string whose template is not a font (`BuffButtonDurationTemplate`, a font string itself) is set by hand.
    pub fn set_region_font(&mut self, region: &str, font: &str) {
        if let Some(Region { kind: Kind::Text { font: f, .. }, .. }) = self.region_mut(region) {
            *f = Some(font.to_string());
        }
    }

    pub fn set_text_color(&mut self, region: &str, color: [f32; 4]) {
        if let Some(Region { kind: Kind::Text { color: c, .. }, .. }) = self.region_mut(region) {
            *c = Some(color);
        }
    }

    pub fn set_scale(&mut self, name: &str, scale: f32) {
        if let Some(&i) = self.by_name.get(name) {
            self.nodes[i].scale = scale;
        }
    }

    /// `SetWidth` / `SetHeight` of a frame.
    pub fn set_size(&mut self, name: &str, w: Option<f32>, h: Option<f32>) {
        if let Some(&i) = self.by_name.get(name) {
            let (cw, ch) = self.nodes[i].size.unwrap_or((0.0, 0.0));
            self.nodes[i].size = Some((w.unwrap_or(cw), h.unwrap_or(ch)));
        }
    }

    /// `SetPoint` of a frame: its only anchor from now on.
    pub fn set_point(&mut self, name: &str, point: Point, relative_to: Option<&str>, relative_point: Option<Point>, x: f32, y: f32) {
        if let Some(&i) = self.by_name.get(name) {
            self.nodes[i].anchors = vec![AnchorDef { point, relative_to: relative_to.map(String::from), relative_point, x, y }];
            self.nodes[i].set_all = false;
        }
    }

    /// `SetPoint` of a texture or font string: its only anchor from now on.
    pub fn set_region_point(&mut self, region: &str, point: Point, relative_to: Option<&str>, relative_point: Option<Point>, x: f32, y: f32) {
        if let Some(r) = self.region_mut(region) {
            r.anchors = vec![AnchorDef { point, relative_to: relative_to.map(String::from), relative_point, x, y }];
            r.set_all = false;
        }
    }

    /// Show `copper` in a money frame (`MoneyFrame_Update`); the frame lays itself out when it is drawn.
    pub fn set_money(&mut self, frame: &str, copper: u32, small: bool, player: bool) {
        self.moneys.insert(frame.to_string(), (copper, small, player));
    }

    /// `MoneyFrame_Update`: the coin buttons are as wide as their numbers, those that are zero are left out (unless the
    /// purse shows the smaller coins under a bigger one), and the frame is as wide as what shows, with the coins at its left.
    fn layout_moneys(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount) {
        for (frame, (money, small, player)) in self.moneys.clone() {
            let (gold, silver, copper) = (money / 10000, money / 100 % 100, money % 100);
            let (icon, spacing) = (if small { 13.0 } else { 19.0 }, -4.0);
            let Some(style) = self.registry.font_style(if small { "NumberFontNormal" } else { "NumberFontNormalLarge" }) else { continue };
            let (g, sl, c) = (format!("{frame}GoldButton"), format!("{frame}SilverButton"), format!("{frame}CopperButton"));
            let mut widths = [0.0; 3];
            for (i, (button, value)) in [(&g, gold), (&sl, silver), (&c, copper)].into_iter().enumerate() {
                let text = value.to_string();
                widths[i] = self.text_width(ui, gpu, mount, &style, &text) + icon;
                self.set_text(&format!("{button}Text"), &text);
                self.set_size(button, Some(widths[i]), None);
            }
            let mut width = icon;
            let (mut lower, show_gold) = (false, gold > 0);
            if show_gold {
                width += widths[0];
                lower = player;
            }
            let show_silver = silver > 0 || lower;
            if show_silver {
                width += widths[1];
                self.set_point(&g, Point::Right, Some(&sl), Some(Point::Left), spacing, 0.0);
                if show_gold {
                    width -= spacing;
                }
                lower |= player;
            } else {
                self.set_point(&g, Point::Right, Some(&sl), Some(Point::Right), 0.0, 0.0);
            }
            let show_copper = copper > 0 || lower || player;
            if show_copper {
                width += widths[2];
                self.set_point(&sl, Point::Right, Some(&c), Some(Point::Left), spacing, 0.0);
                if show_silver {
                    width -= spacing;
                }
            } else {
                self.set_point(&sl, Point::Right, Some(&c), Some(Point::Right), 0.0, 0.0);
            }
            self.show(&g, show_gold);
            self.show(&sl, show_silver);
            self.show(&c, show_copper);
            self.set_size(&frame, Some(width), None);
        }
    }

    /// Fill a status bar to `value` (0 to 1), in `color` when one is given.
    pub fn set_bar(&mut self, name: &str, value: f32, color: Option<[f32; 4]>) {
        if let Some(&i) = self.by_name.get(name) {
            if let Some(bar) = &mut self.nodes[i].bar {
                bar.value = value.clamp(0.0, 1.0);
                bar.color = color.unwrap_or(bar.color);
            }
        }
    }

    /// `SetAlpha`: how opaque a frame and its contents are, 0 to 1.
    pub fn set_alpha(&mut self, name: &str, alpha: f32) {
        if let Some(&i) = self.by_name.get(name) {
            self.nodes[i].alpha = alpha.clamp(0.0, 1.0);
        }
    }

    /// `SetFrameLevel` relative to what the frame had: a lower one is drawn before its siblings.
    pub fn set_level(&mut self, name: &str, level: i32) {
        if let Some(&i) = self.by_name.get(name) {
            self.nodes[i].level = level;
        }
    }

    /// Tint a texture (`SetVertexColor`).
    pub fn set_region_tint(&mut self, region: &str, tint: [f32; 4]) {
        if let Some(Region { kind: Kind::Texture { color, .. }, .. }) = self.region_mut(region) {
            *color = Some(tint);
        }
    }

    /// Hide the picture of a button's normal state (its text and highlight stay).
    pub fn hide_button_art(&mut self, name: &str) {
        if let Some(&i) = self.by_name.get(name) {
            for r in &mut self.nodes[i].regions {
                if r.role == Role::Normal {
                    r.hidden = true;
                }
            }
        }
    }

    /// `SetBackdropColor` / `SetBackdropBorderColor`: tint the background and/or the border of a frame.
    pub fn set_backdrop_colors(&mut self, name: &str, background: Option<[f32; 4]>, border: Option<[f32; 4]>) {
        if let Some(b) = self.by_name.get(name).and_then(|&i| self.nodes[i].backdrop.as_mut()) {
            b.color = background.or(b.color);
            b.border_color = border.or(b.border_color);
        }
    }

    /// Check or uncheck a check button.
    pub fn set_checked(&mut self, name: &str, on: bool) {
        if let Some(&i) = self.by_name.get(name) {
            self.checked.retain(|&c| c != i);
            if on {
                self.checked.push(i);
            }
        }
    }

    /// Enable or disable a button: a disabled one shows its disabled picture and takes no clicks.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        if let Some(&i) = self.by_name.get(name) {
            if enabled {
                self.disabled.remove(&i);
            } else {
                self.disabled.insert(i);
            }
        }
    }

    /// Hide the pictures of a frame that are of a texture file (for those that have no name to be hidden by).
    pub fn hide_textures_of(&mut self, frame: &str, file: &str) {
        let Some(&i) = self.by_name.get(frame) else { return };
        for r in &mut self.nodes[i].regions {
            if matches!(&r.kind, Kind::Texture { file: Some(f), .. } if f.contains(file)) {
                r.hidden = true;
            }
        }
    }

    pub fn set_region_size(&mut self, region: &str, w: Option<f32>, h: Option<f32>) {
        if let Some(r) = self.region_mut(region) {
            let (cw, ch) = r.size.unwrap_or((0.0, 0.0));
            r.size = Some((w.unwrap_or(cw), h.unwrap_or(ch)));
        }
    }

    /// `SetTexCoord`: left, right, top, bottom.
    pub fn set_region_coords(&mut self, region: &str, uv: [f32; 4]) {
        if let Some(Region { kind: Kind::Texture { coords, .. }, .. }) = self.region_mut(region) {
            *coords = uv;
        }
    }

    pub fn show(&mut self, name: &str, on: bool) {
        if let Some(&i) = self.by_name.get(name) {
            self.nodes[i].shown = on;
        }
    }

    /// Whether the frame and all its parents are shown.
    pub fn is_shown(&self, node: usize) -> bool {
        let mut cur = Some(node);
        while let Some(i) = cur {
            if !self.nodes[i].shown {
                return false;
            }
            cur = self.nodes[i].parent;
        }
        true
    }

    pub fn is_name_shown(&self, name: &str) -> bool {
        self.index(name).is_some_and(|i| self.is_shown(i))
    }

    // ---- layout

    /// The product of the scales of a frame and its parents.
    fn eff_scale(&self, mut node: usize) -> f32 {
        let mut k = 1.0;
        loop {
            k *= self.nodes[node].scale;
            match self.nodes[node].parent {
                Some(p) => node = p,
                None => return k,
            }
        }
    }

    fn root_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: ui_height() * self.screen.0 as f32 / self.screen.1.max(1) as f32, h: ui_height() }
    }

    /// Where the anchors put something of `size`, inside `base` (its parent), with `lookup` finding what they name.
    fn solve(&self, anchors: &[AnchorDef], size: Option<(f32, f32)>, set_all: bool, base: Rect, k: f32, lookup: &mut dyn FnMut(&str) -> Option<Rect>) -> Rect {
        let size = size.map(|(w, h)| (w * k, h * k));
        if set_all && anchors.is_empty() {
            return base;
        }
        if anchors.is_empty() {
            let (w, h) = size.unwrap_or((0.0, 0.0));
            return Rect { x: base.x + (base.w - w) / 2.0, y: base.y + (base.h - h) / 2.0, w, h };
        }
        // One constraint per anchor and axis: the target coordinate for a fraction of the rectangle.
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        for a in anchors {
            let rel = a.relative_to.as_deref().and_then(|n| lookup(n)).unwrap_or(base);
            let (tx, ty) = rel.at(a.relative_point.unwrap_or(a.point));
            let (fx, fy) = a.point.frac();
            xs.push((fx, tx + a.x * k));
            ys.push((fy, ty + a.y * k));
        }
        let axis = |cs: &[(f32, f32)], len: f32| -> (f32, f32) {
            match cs.iter().find(|c| (c.0 - cs[0].0).abs() > 1e-3) {
                Some(&(f2, v2)) => {
                    let (f1, v1) = cs[0];
                    let len = (v2 - v1) / (f2 - f1);
                    (v1 - f1 * len, len)
                }
                None => (cs[0].1 - cs[0].0 * len, len),
            }
        };
        let (x, w) = axis(&xs, size.map_or(0.0, |s| s.0));
        let (y, h) = axis(&ys, size.map_or(0.0, |s| s.1));
        Rect { x, y, w, h }
    }

    fn frame_rect(&self, node: usize, cache: &mut Vec<Option<Rect>>, depth: usize) -> Rect {
        if node == 0 {
            return self.root_rect();
        }
        if let Some(r) = cache[node] {
            return r;
        }
        let n = &self.nodes[node];
        let parent = n.parent.unwrap_or(0);
        let base = if depth > 40 { self.root_rect() } else { self.frame_rect(parent, cache, depth + 1) };
        let mut lookup = |name: &str| -> Option<Rect> {
            if let Some(&i) = self.by_name.get(name) {
                return (i != node && depth < 40).then(|| self.frame_rect(i, cache, depth + 1));
            }
            // A frame may also hang off a texture or font string (the buttons under a greeting).
            let &(n, r) = self.regions_by_name.get(name)?;
            let target = self.nodes[n].regions.get(r)?;
            (n != node && depth < 24).then(|| self.region_rect_at(n, target, cache, None, depth + 1))
        };
        let k = self.eff_scale(node);
        let mut rect = self.solve(&n.anchors, n.size, n.set_all, base, k, &mut lookup);
        if n.anchors.is_empty() && !n.set_all {
            // A frame without anchors sits at the bottom left of its parent.
            let (w, h) = n.size.map_or((0.0, 0.0), |(w, h)| (w * k, h * k));
            rect = Rect { x: base.x, y: base.y, w, h };
        }
        // The child of a scroll frame is moved up as far as the frame is scrolled (its scroll bar stands beside the content and stays).
        if self.nodes[parent].scroll && !n.name.ends_with("ScrollBar") {
            rect.y += self.nodes[parent].scrolled * k;
        }
        cache[node] = Some(rect);
        rect
    }

    /// The rectangle of the nearest scroll frame around a frame, which cuts it, if there is one.
    fn clip_of(&self, node: usize, cache: &mut Vec<Option<Rect>>) -> Option<Rect> {
        let (mut at, mut below) = (self.nodes[node].parent, node);
        while let Some(p) = at {
            if self.nodes[p].scroll {
                // The scroll bar stands beside the content, not in it.
                return (!self.nodes[below].name.ends_with("ScrollBar")).then(|| self.frame_rect(p, cache, 0));
            }
            (at, below) = (self.nodes[p].parent, p);
        }
        None
    }

    /// `text` is the size of the region's text, which a font string without a size of its own has.
    fn region_rect(&self, node: usize, region: &Region, cache: &mut Vec<Option<Rect>>, text: Option<(f32, f32)>) -> Rect {
        self.region_rect_at(node, region, cache, text, 0)
    }

    /// Anchors may name other regions, which are laid out the same way (`depth` stops circular anchoring).
    fn region_rect_at(&self, node: usize, region: &Region, cache: &mut Vec<Option<Rect>>, text: Option<(f32, f32)>, depth: usize) -> Rect {
        let base = self.frame_rect(node, cache, 0);
        let mut lookup = |name: &str| -> Option<Rect> {
            if let Some(&i) = self.by_name.get(name) {
                return Some(self.frame_rect(i, cache, 1));
            }
            let &(n, r) = self.regions_by_name.get(name)?;
            let target = self.nodes[n].regions.get(r)?;
            (depth < 24 && (n != node || target.name != region.name)).then(|| self.region_rect_at(n, target, cache, None, depth + 1))
        };
        // A font string with a width and no height is as tall as its text wraps to.
        let dims = text.or_else(|| self.text_dims.get(&region.name).copied());
        let size = match region.size {
            Some((w, h)) if h == 0.0 => Some((w, dims.map_or(0.0, |d| d.1))),
            size => size.or(dims),
        };
        self.solve(&region.anchors, size, region.set_all || (region.anchors.is_empty() && size.is_none()), base, self.eff_scale(node), &mut lookup)
    }

    /// The rectangle of a named frame on a `screen` sized screen, in interface units.
    pub fn rect_of(&mut self, name: &str, screen: (u32, u32)) -> Option<Rect> {
        self.screen = screen;
        let i = self.index(name)?;
        let mut cache = vec![None; self.nodes.len()];
        Some(self.frame_rect(i, &mut cache, 0))
    }

    /// The rectangle of a named texture or font string region.
    #[cfg(test)]
    pub fn region_rect_of(&mut self, region: &str, screen: (u32, u32)) -> Option<Rect> {
        self.screen = screen;
        let &(n, r) = self.regions_by_name.get(region)?;
        let mut cache = vec![None; self.nodes.len()];
        let region = self.nodes[n].regions.get(r)?.clone();
        Some(self.region_rect(n, &region, &mut cache, None))
    }

    // ---- drawing

    pub(crate) fn texture(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, path: &str) -> Option<usize> {
        if let Some(t) = self.textures.get(&path.to_ascii_lowercase()) {
            return *t;
        }
        let full = if path.to_ascii_lowercase().ends_with(".blp") { path.to_string() } else { format!("{path}.blp") };
        let tex = mount.read(&full).ok().and_then(|d| Blp::parse(&d).ok()).map(|b| ui.add_texture(gpu, b.width, b.height, &b.mips[0]));
        if tex.is_none() {
            log::debug!("interface texture {path} not found");
        }
        self.textures.insert(path.to_ascii_lowercase(), tex);
        tex
    }

    fn glyphs(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, style: &FontStyle) -> Option<&GlyphSet> {
        let key = (style.file.to_ascii_lowercase(), (style.height * 10.0) as u32, style.outline);
        if !self.glyphs.contains_key(&key) {
            let font = self
                .font_files
                .entry(key.0.clone())
                .or_insert_with(|| mount.read(&style.file).ok().and_then(|b| fontdue::Font::from_bytes(b, fontdue::FontSettings::default()).ok()))
                .as_ref();
            let set = font.and_then(|f| GlyphSet::new(ui, gpu, f, style.height, style.outline).ok());
            self.glyphs.insert(key.clone(), set);
        }
        self.glyphs[&key].as_ref()
    }

    /// Queue everything shown for drawing on a `screen` sized screen.
    pub fn draw(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, screen: (u32, u32)) {
        self.screen = screen;
        let mut cache = vec![None; self.nodes.len()];
        self.drawn.clear();
        let mut order = Vec::new();
        self.collect(0, &mut order);
        self.layout_moneys(ui, gpu, mount);
        for &node in &order {
            self.measure_texts(ui, gpu, mount, node);
        }
        let (px, h) = (screen_scale(screen), screen.1 as f32);
        for &node in &order {
            let clip = self.clip_of(node, &mut cache).map(|r| [r.x * px, h - (r.y + r.h) * px, r.w * px, r.h * px]);
            ui.set_clip(clip);
            self.draw_node(ui, gpu, mount, node, &mut cache);
        }
        ui.set_clip(None);
        self.drawn = order;
    }

    /// The height of a named font string's text as last measured (0 before).
    pub fn text_height(&self, region: &str) -> f32 {
        self.text_dims.get(region).map_or(0.0, |d| d.1)
    }

    /// Measure the texts of everything shown, so that `hit_test` and `rect_of` know the layout before the first draw.
    pub fn measure(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, screen: (u32, u32)) {
        self.screen = screen;
        let mut order = Vec::new();
        self.collect(0, &mut order);
        self.layout_moneys(ui, gpu, mount);
        for node in order {
            self.measure_texts(ui, gpu, mount, node);
        }
    }

    /// Measure the named font strings of a frame that have no height of their own (see `text_dims`).
    fn measure_texts(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, node: usize) {
        for r in self.nodes[node].regions.clone() {
            let Kind::Text { font: Some(font), text, .. } = &r.kind else { continue };
            if text.is_empty() || r.name.is_empty() || r.size.is_some_and(|(_, h)| h != 0.0) {
                continue;
            }
            let Some(style) = self.registry.font_style(font) else { continue };
            let plain: String = color_spans(text).into_iter().map(|(t, _)| t).collect();
            let fixed = r.size.map(|(w, _)| w);
            let Some(glyphs) = self.glyphs(ui, gpu, mount, &style) else { continue };
            let lines = wrap(glyphs, &plain, fixed.unwrap_or(0.0));
            let width = fixed.unwrap_or_else(|| lines.iter().map(|l| glyphs.width(l)).fold(0.0, f32::max));
            self.text_dims.insert(r.name.clone(), (width, style.height * lines.len() as f32));
        }
    }

    /// Shown frames in drawing order: a frame, then its children.
    fn collect(&self, node: usize, out: &mut Vec<usize>) {
        if !self.nodes[node].shown {
            return;
        }
        out.push(node);
        // Siblings with a higher frame level draw over the others.
        let mut children = self.nodes[node].children.clone();
        children.sort_by_key(|&c| self.nodes[c].level);
        for c in children {
            self.collect(c, out);
        }
    }

    fn draw_node(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, node: usize, cache: &mut Vec<Option<Rect>>) {
        let (s, h) = (screen_scale(self.screen), self.screen.1 as f32);
        if let Some(texture) = self.nodes[node].model_texture {
            let r = self.frame_rect(node, cache, 0);
            ui.quad(Quad { texture, rect: [r.x * s, h - (r.y + r.h) * s, r.w * s, r.h * s], uv: FULL_UV, color: [1.0; 4], additive: false });
        }
        if let Some(b) = self.nodes[node].backdrop.clone() {
            let r = self.frame_rect(node, cache, 0);
            self.draw_backdrop(ui, gpu, mount, &b, [r.x * s, h - (r.y + r.h) * s, r.w * s, r.h * s], s);
        }
        // The bar of a status bar lies over the background of its frame and under the rest of it (its art, its texts).
        let alpha = self.nodes[node].alpha;
        let mut bar_pending = self.nodes[node].bar.is_some();
        let regions = self.nodes[node].regions.clone();
        let hovered = self.hover == Some(node);
        let pressed = self.pressed == Some(node);
        let checked = self.checked.contains(&node);
        let is_disabled = self.disabled.contains(&node);
        let mut draw_order: Vec<&Region> = regions.iter().collect();
        let level = |r: &Region| match r.role {
            Role::Layer(l) => l as u8,
            Role::Normal | Role::Disabled | Role::Pushed => Level::Artwork as u8,
            Role::Checked | Role::Highlight => Level::Highlight as u8,
        };
        draw_order.sort_by_key(|r| level(r));
        for r in draw_order {
            if bar_pending && level(r) > Level::Background as u8 {
                bar_pending = false;
                self.draw_bar(ui, gpu, mount, node, cache);
            }
            let visible = match r.role {
                Role::Layer(_) => true,
                Role::Normal => !is_disabled && !(pressed && regions.iter().any(|o| o.role == Role::Pushed)),
                Role::Pushed => pressed,
                Role::Disabled => is_disabled,
                Role::Highlight => hovered,
                Role::Checked => checked,
            };
            if r.hidden || !visible {
                continue;
            }
            // A font string without a size of its own is as big as its text.
            let text_size = match &r.kind {
                Kind::Text { font, text, .. } if r.size.is_none() && !text.is_empty() => {
                    let plain: String = color_spans(text).into_iter().map(|(t, _)| t).collect();
                    font.as_deref().and_then(|f| self.registry.font_style(f)).and_then(|style| self.glyphs(ui, gpu, mount, &style).map(|g| (g.width(&plain), style.height)))
                }
                _ => None,
            };
            let rect = self.region_rect(node, r, cache, text_size);
            // To pixels, from the top left.
            let px = [rect.x * s, h - (rect.y + rect.h) * s, rect.w * s, rect.h * s];
            match &r.kind {
                Kind::Texture { file, id, coords, color, additive } => {
                    let (texture, tint) = match (id, file, color) {
                        (Some(id), _, c) => (Some(*id), c.unwrap_or([1.0; 4])),
                        (None, Some(f), c) => (self.texture(ui, gpu, mount, f), c.unwrap_or([1.0; 4])),
                        (None, None, Some(c)) => (Some(WHITE), *c),
                        (None, None, None) => (None, [1.0; 4]),
                    };
                    let tint = [tint[0], tint[1], tint[2], tint[3] * alpha];
                    if let Some(texture) = texture {
                        let [l, rr, t, b] = *coords;
                        // A picture drawn for the interface (a portrait) is round, like the frame it sits in.
                        ui.set_round(id.is_some());
                        ui.quad(Quad { texture, rect: px, uv: if file.is_some() || id.is_some() { [l, t, rr, b] } else { FULL_UV }, color: tint, additive: *additive || (r.role == Role::Highlight) });
                        ui.set_round(false);
                    }
                }
                Kind::Text { font, text, justify_h, justify_v, color } => {
                    if text.is_empty() {
                        continue;
                    }
                    let Some(style) = font.as_deref().and_then(|f| self.registry.font_style(f)) else { continue };
                    let tint = color.unwrap_or(style.color);
                    let tint = [tint[0], tint[1], tint[2], tint[3] * alpha];
                    // Colour codes (`|cffRRGGBB` ... `|r`) colour single lines; wrapped text loses them.
                    let plain = color_spans(text).into_iter().map(|(t, _)| t).collect::<String>();
                    let k = self.eff_scale(node);
                    let lines = match self.glyphs(ui, gpu, mount, &style) {
                        Some(g) => wrap(g, &plain, if rect.h >= style.height * k * 1.8 { rect.w / k } else { 0.0 }),
                        None => continue,
                    };
                    let coloured = lines.len() == 1 && text.contains("|c");
                    let line_h = style.height * k;
                    let block = line_h * lines.len() as f32;
                    let top = match justify_v.as_str() {
                        "TOP" => rect.y + rect.h,
                        "BOTTOM" => rect.y + block,
                        _ => rect.y + rect.h / 2.0 + block / 2.0,
                    };
                    let glyphs = self.glyphs(ui, gpu, mount, &style).unwrap();
                    for (i, line) in lines.iter().enumerate() {
                        let x = rect.x + (rect.w - glyphs.width(line) * k) * justify(justify_h);
                        let y_top = h - (top - i as f32 * line_h) * s;
                        let spans = if coloured { color_spans(text) } else { vec![(line.clone(), None)] };
                        for pass in [true, false] {
                            let mut pen = x;
                            for (piece, code) in &spans {
                                let c = code.map_or(tint, |c| [c[0], c[1], c[2], c[3] * alpha]);
                                match (pass, style.shadow) {
                                    (true, Some(((ox, oy), shadow))) => glyphs.draw(ui, piece, pen * s + ox * s, y_top - oy * s, s * k, [shadow[0], shadow[1], shadow[2], shadow[3] * c[3]]),
                                    (false, _) => glyphs.draw(ui, piece, pen * s, y_top, s * k, c),
                                    _ => {}
                                }
                                pen += glyphs.width(piece) * k;
                            }
                        }
                    }
                }
            }
        }
        if bar_pending {
            self.draw_bar(ui, gpu, mount, node, cache);
        }
    }

    /// The coloured bar of a status bar, filled to its value.
    fn draw_bar(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, node: usize, cache: &mut Vec<Option<Rect>>) {
        let (s, h) = (screen_scale(self.screen), self.screen.1 as f32);
        let alpha = self.nodes[node].alpha;
        if let Some((file, color, value)) = self.nodes[node].bar.as_ref().map(|b| (b.file.clone(), [b.color[0], b.color[1], b.color[2], b.color[3] * alpha], b.value)) {
            let r = self.frame_rect(node, cache, 0);
            if let (Some(texture), true) = (self.texture(ui, gpu, mount, &file), value > 0.0) {
                ui.quad(Quad { texture, rect: [r.x * s, h - (r.y + r.h) * s, r.w * s * value, r.h * s], uv: [0.0, 0.0, value, 1.0], color, additive: false });
            }
        }
    }

    /// The eight 16 x 16 pieces of an edge strip: left, right, top, bottom (stored turned on their side), and the four
    /// corners. Each becomes a texture of its own.
    fn edge_pieces(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, file: &str) -> Option<[usize; 8]> {
        let key = format!("edge:{}", file.to_ascii_lowercase());
        if let Some(Some(first)) = self.textures.get(&key) {
            return Some(std::array::from_fn(|i| first + i));
        }
        let blp = mount.read(&format!("{file}.blp")).ok().and_then(|d| Blp::parse(&d).ok())?;
        let (w, tile) = (blp.width as usize, blp.height as usize);
        let pixel = |x: usize, y: usize| &blp.mips[0][(y * w + x) * 4..][..4];
        let mut first = None;
        for i in 0..8 {
            let mut rgba = vec![0u8; tile * tile * 4];
            for y in 0..tile {
                for x in 0..tile {
                    // The top and bottom edges are stored turned a quarter: transpose them upright.
                    let (sx, sy) = if i == 2 || i == 3 { (y, x) } else { (x, y) };
                    rgba[(y * tile + x) * 4..][..4].copy_from_slice(pixel(i * tile + sx, sy));
                }
            }
            let id = ui.add_texture(gpu, tile as u32, tile as u32, &rgba);
            first.get_or_insert(id);
        }
        self.textures.insert(key, first);
        first.map(|f| std::array::from_fn(|i| f + i))
    }

    /// Draw a backdrop over a pixel rectangle `[x, y, w, h]`.
    fn draw_backdrop(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, b: &BackdropDef, rect: [f32; 4], s: f32) {
        let [x, y, w, h] = rect;
        let [il, ir, it, ib] = b.insets.map(|v| v * s);
        if let Some(file) = &b.bg_file {
            if let Some(texture) = self.texture(ui, gpu, mount, file) {
                let color = b.color.unwrap_or([1.0; 4]);
                let (bx, by, bw, bh) = (x + il, y + it, w - il - ir, h - it - ib);
                let tile = if b.tile && b.tile_size > 0.0 { b.tile_size * s } else { 0.0 };
                if tile > 0.0 {
                    let mut ty = 0.0;
                    while ty < bh {
                        let mut tx = 0.0;
                        while tx < bw {
                            let (tw, th) = ((bw - tx).min(tile), (bh - ty).min(tile));
                            push(ui, texture, [bx + tx, by + ty, tw, th], [0.0, 0.0, tw / tile, th / tile], color);
                            tx += tile;
                        }
                        ty += tile;
                    }
                } else {
                    push(ui, texture, [bx, by, bw, bh], FULL_UV, color);
                }
            }
        }
        if let Some(file) = &b.edge_file {
            if let Some(p) = self.edge_pieces(ui, gpu, mount, file) {
                let color = b.border_color.unwrap_or([1.0; 4]);
                let e = if b.edge_size > 0.0 { b.edge_size * s } else { 16.0 * s };
                // Edges between the corners, repeated along their length; corners last.
                let mut along = e;
                while along < w - e {
                    let len = (w - e - along).min(e);
                    let uv = [0.0, 0.0, len / e, 1.0];
                    push(ui, p[2], [x + along, y, len, e], uv, color);
                    push(ui, p[3], [x + along, y + h - e, len, e], uv, color);
                    along += e;
                }
                along = e;
                while along < h - e {
                    let len = (h - e - along).min(e);
                    let uv = [0.0, 0.0, 1.0, len / e];
                    push(ui, p[0], [x, y + along, e, len], uv, color);
                    push(ui, p[1], [x + w - e, y + along, e, len], uv, color);
                    along += e;
                }
                push(ui, p[4], [x, y, e, e], FULL_UV, color);
                push(ui, p[5], [x + w - e, y, e, e], FULL_UV, color);
                push(ui, p[6], [x, y + h - e, e, e], FULL_UV, color);
                push(ui, p[7], [x + w - e, y + h - e, e, e], FULL_UV, color);
            }
        }
    }

    /// Draw `text` with the left end of its line at `x` and the top at `top` (interface units, y up).
    fn text_at(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, style: &FontStyle, text: &str, x: f32, top: f32, tint: [f32; 4]) {
        let (s, h) = (screen_scale(self.screen) * self.text_scale, self.screen.1 as f32);
        let spans = color_spans(text);
        let Some(glyphs) = self.glyphs(ui, gpu, mount, style) else { return };
        let y_top = h - top * s;
        for pass in [true, false] {
            let mut pen = x;
            for (piece, code) in &spans {
                let c = code.unwrap_or(tint);
                match (pass, style.shadow) {
                    (true, Some(((ox, oy), shadow))) => glyphs.draw(ui, piece, pen * s + ox * s, y_top - oy * s, s, [shadow[0], shadow[1], shadow[2], shadow[3] * c[3]]),
                    (false, _) => glyphs.draw(ui, piece, pen * s, y_top, s, c),
                    _ => {}
                }
                pen += glyphs.width(piece);
            }
        }
    }

    /// `PanelTemplates_TabResize`: a tab is as wide as its text plus `padding` and its two 20 unit ends.
    pub fn resize_tab(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, tab: &str, padding: f32) {
        let Some(&(n, r)) = self.regions_by_name.get(&format!("{tab}Text")) else { return };
        let Some(Region { kind: Kind::Text { font: Some(font), text, .. }, .. }) = self.nodes[n].regions.get(r).cloned() else { return };
        let Some(style) = self.registry.font_style(&font) else { return };
        let width = self.text_width(ui, gpu, mount, &style, &text) + padding;
        for part in ["Middle", "MiddleDisabled"] {
            self.set_region_size(&format!("{tab}{part}"), Some(width), None);
        }
        self.set_size(tab, Some(width + 40.0), None);
        self.set_region_size(&format!("{tab}HighlightTexture"), Some(width + 40.0), None);
    }

    /// Draw `text` in a font object, centred on the screen, with the top of its line `top` units above the bottom.
    pub fn draw_text_centered(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, screen: (u32, u32), font: &str, text: &str, top: f32, color: [f32; 4]) {
        self.screen = screen;
        let Some(style) = self.registry.font_style(font) else { return };
        let width = self.text_width(ui, gpu, mount, &style, text);
        let x = (self.root_rect().w - width) / 2.0;
        self.text_at(ui, gpu, mount, &style, text, x, top, color);
    }

    /// Draw `text` in a font object with the middle of its line at pixel `x` and the top at pixel `y` (y down).
    pub fn draw_text_at_pixel(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, screen: (u32, u32), font: &str, text: &str, at: (f32, f32), color: [f32; 4]) {
        self.draw_text_aligned(ui, gpu, mount, screen, font, text, at, 0.5, color);
    }

    /// Like `draw_text_at_pixel`, with the point `x` at `align` along the line (0 its left end, 1 its right end).
    pub fn draw_text_aligned(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, screen: (u32, u32), font: &str, text: &str, (x, y): (f32, f32), align: f32, color: [f32; 4]) {
        self.screen = screen;
        let Some(style) = self.registry.font_style(font) else { return };
        let width = self.text_width(ui, gpu, mount, &style, text);
        let s = screen_scale(screen);
        self.text_at(ui, gpu, mount, &style, text, x / s - width * align, (screen.1 as f32 - y) / s, color);
    }

    /// Draw `text` with its line `line` pixels tall, centred on pixel `x` with the bottom of the line at pixel `y` (y down).
    pub fn draw_text_world(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, screen: (u32, u32), font: &str, text: &str, (x, y): (f32, f32), line: f32, color: [f32; 4]) {
        self.screen = screen;
        let Some(mut style) = self.registry.font_style(font) else { return };
        // The original draws a unit name plain: its font has neither an outline nor a shadow.
        style.shadow = None;
        let s = screen_scale(screen);
        self.text_scale = line / (style.height * s);
        let width = self.text_width(ui, gpu, mount, &style, text);
        let s = s * self.text_scale;
        self.text_at(ui, gpu, mount, &style, text, x / s - width / 2.0, (screen.1 as f32 - y + line) / s, color);
        self.text_scale = 1.0;
    }

    /// Draw a texture file at a pixel rectangle.
    pub fn draw_icon(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, path: &str, rect: [f32; 4]) {
        if let Some(texture) = self.texture(ui, gpu, mount, path) {
            ui.quad(Quad { texture, rect, uv: FULL_UV, color: [1.0; 4], additive: false });
        }
    }

    /// Width of `text` in a font object, in interface units.
    fn text_width(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, style: &FontStyle, text: &str) -> f32 {
        let plain: String = color_spans(text).into_iter().map(|(t, _)| t).collect();
        self.glyphs(ui, gpu, mount, style).map_or(0.0, |g| g.width(&plain))
    }

    /// A `GameTooltip` of `lines` next to `owner` (its bottom left corner at the owner's top right, like
    /// `ANCHOR_RIGHT`; on the other side or lower when it would leave the screen).
    pub fn draw_tooltip(&mut self, ui: &mut Ui, gpu: &Gpu, mount: &Mount, screen: (u32, u32), owner: Rect, lines: &[TipLine]) {
        self.screen = screen;
        let (s, h) = (screen_scale(screen), screen.1 as f32);
        let root = self.root_rect();
        let styles: Vec<Option<FontStyle>> = lines.iter().enumerate().map(|(i, l)| self.registry.font_style(if i == 0 { "GameTooltipHeaderText" } else { l.font })).collect();
        // The size: 10 units of padding around the lines, 2 between them, 10 between the two columns of a line.
        let (mut width, mut height) = (0.0f32, 0.0f32);
        let mut widths = Vec::new();
        for (l, style) in lines.iter().zip(&styles) {
            let Some(style) = style else { widths.push((0.0, 0.0)); continue };
            let (lw, rw) = (self.text_width(ui, gpu, mount, style, &l.left), l.right.as_deref().map_or(0.0, |r| self.text_width(ui, gpu, mount, style, r)));
            width = width.max(lw + if l.right.is_some() { rw + 10.0 } else { 0.0 });
            height += style.height + 2.0;
            widths.push((lw, rw));
        }
        let (w, hgt) = (width + 20.0, height - 2.0 + 20.0);
        let (mut x, mut y) = (owner.x + owner.w, owner.y + owner.h);
        if x + w > root.w {
            x = owner.x - w;
        }
        if y + hgt > root.h {
            y = root.h - hgt;
        }
        let backdrop = BackdropDef {
            bg_file: Some(r"Interface\Tooltips\UI-Tooltip-Background".into()),
            edge_file: Some(r"Interface\Tooltips\UI-Tooltip-Border".into()),
            tile: true,
            tile_size: 16.0,
            edge_size: 16.0,
            insets: [5.0; 4],
            color: Some([0.09, 0.09, 0.19, 1.0]),
            border_color: Some([1.0; 4]),
        };
        self.draw_backdrop(ui, gpu, mount, &backdrop, [x * s, h - (y + hgt) * s, w * s, hgt * s], s);
        let mut top = y + hgt - 10.0;
        for ((l, style), (_, rw)) in lines.iter().zip(&styles).zip(widths) {
            let Some(style) = style else { continue };
            self.text_at(ui, gpu, mount, style, &l.left, x + 10.0, top, l.color);
            if let Some(r) = &l.right {
                self.text_at(ui, gpu, mount, style, r, x + w - 10.0 - rw, top, l.right_color);
            }
            top -= style.height + 2.0;
        }
    }

    // ---- the mouse

    /// The topmost shown, mouse-enabled frame under a pixel position.
    pub fn hit_test(&mut self, pos: (f32, f32), screen: (u32, u32)) -> Option<usize> {
        self.screen = screen;
        let s = screen_scale(screen);
        let (x, y) = (pos.0 / s, (screen.1 as f32 - pos.1) / s);
        let mut cache = vec![None; self.nodes.len()];
        let mut drawn = Vec::new();
        self.collect(0, &mut drawn);
        drawn.into_iter().rev().find(|&i| {
            let n = &self.nodes[i];
            if !n.mouse || self.disabled.contains(&i) {
                return false;
            }
            // What a scroll frame has cut away cannot be clicked.
            if self.clip_of(i, &mut cache).is_some_and(|c| !c.contains(x, y)) {
                return false;
            }
            let r = self.frame_rect(i, &mut cache, 0);
            let [l, ri, t, b] = n.hit_insets;
            Rect { x: r.x + l, y: r.y + b, w: r.w - l - ri, h: r.h - t - b }.contains(x, y)
        })
    }

    /// How far (interface units) what is in a scroll frame reaches below its bottom edge: how far it can be scrolled.
    fn overflow(&self, scroll: usize, cache: &mut Vec<Option<Rect>>) -> f32 {
        let frame = self.frame_rect(scroll, cache, 0);
        let k = self.eff_scale(scroll);
        let mut lowest = frame.y;
        let mut stack = self.nodes[scroll].children.clone();
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n];
            // The scroll bar (and the buttons that hang off it) is no content.
            if !node.shown || node.name.ends_with("ScrollBar") {
                continue;
            }
            lowest = lowest.min(self.frame_rect(n, cache, 0).y);
            for r in node.regions.iter().filter(|r| !r.hidden && !matches!(&r.kind, Kind::Text { text, .. } if text.is_empty())) {
                if matches!(r.role, Role::Layer(_)) {
                    lowest = lowest.min(self.region_rect(n, r, cache, None).y);
                }
            }
            stack.extend(node.children.iter().copied());
        }
        // What was measured is already moved up by the scroll.
        ((frame.y - lowest + self.nodes[scroll].scrolled * k) / k).max(0.0)
    }

    /// The screen size of the last layout (pixels).
    pub fn screen(&self) -> (u32, u32) {
        self.screen
    }

    /// How far a scroll frame is scrolled and how far it can be, in interface units.
    pub fn scroll_state(&mut self, name: &str, screen: (u32, u32)) -> Option<(f32, f32)> {
        self.screen = screen;
        let i = *self.by_name.get(name)?;
        let mut cache = vec![None; self.nodes.len()];
        let max = self.overflow(i, &mut cache);
        Some((self.nodes[i].scrolled, max))
    }

    /// Scroll a scroll frame by `lines` notches of the wheel (negative: down), as far as it goes.
    pub fn scroll_by(&mut self, name: &str, screen: (u32, u32), lines: f32) {
        if let Some((_, max)) = self.scroll_state(name, screen) {
            let node = &mut self.nodes[self.by_name[name]];
            node.scrolled = (node.scrolled - lines * SCROLL_STEP).clamp(0.0, max);
        }
    }

    /// Put a scroll frame back at its top.
    pub fn reset_scroll(&mut self, name: &str) {
        if let Some(&i) = self.by_name.get(name) {
            self.nodes[i].scrolled = 0.0;
        }
    }

    /// The wheel turned `lines` over the point `pos` (pixels): the scroll frame under it moves, as far as there is more to
    /// see. False when no scroll frame is there.
    pub fn scroll_at(&mut self, pos: (f32, f32), screen: (u32, u32), lines: f32) -> bool {
        self.screen = screen;
        let s = screen_scale(screen);
        let (x, y) = (pos.0 / s, (screen.1 as f32 - pos.1) / s);
        let mut cache = vec![None; self.nodes.len()];
        let mut shown = Vec::new();
        self.collect(0, &mut shown);
        let Some(i) = shown.into_iter().rev().find(|&i| self.nodes[i].scroll && self.frame_rect(i, &mut cache, 0).contains(x, y)) else { return false };
        let max = self.overflow(i, &mut cache);
        let node = &mut self.nodes[i];
        node.scrolled = (node.scrolled - lines * SCROLL_STEP).clamp(0.0, max);
        true
    }

}

/// Pixels per interface unit.
fn screen_scale(screen: (u32, u32)) -> f32 {
    screen.1 as f32 / ui_height()
}

fn push(ui: &mut Ui, texture: usize, rect: [f32; 4], uv: [f32; 4], color: [f32; 4]) {
    ui.quad(Quad { texture, rect, uv, color, additive: false });
}

/// The pieces of a text between colour codes: `|cAARRGGBB` starts a colour, `|r` ends it.
fn color_spans(text: &str) -> Vec<(String, Option<[f32; 4]>)> {
    let mut out: Vec<(String, Option<[f32; 4]>)> = vec![(String::new(), None)];
    let mut rest = text;
    while let Some(i) = rest.find('|') {
        out.last_mut().unwrap().0.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        if let Some(code) = tail.strip_prefix('c').and_then(|t| t.get(..8)).filter(|c| c.chars().all(|d| d.is_ascii_hexdigit())) {
            let v = |k: usize| u8::from_str_radix(&code[k..k + 2], 16).unwrap_or(255) as f32 / 255.0;
            out.push((String::new(), Some([v(2), v(4), v(6), v(0)])));
            rest = &tail[9..];
        } else if let Some(t) = tail.strip_prefix('r') {
            out.push((String::new(), None));
            rest = t;
        } else {
            out.last_mut().unwrap().0.push('|');
            rest = tail;
        }
    }
    out.last_mut().unwrap().0.push_str(rest);
    out.retain(|(t, _)| !t.is_empty());
    out
}

/// Break `text` into lines no wider than `width` units (no wrapping when it is 0); `\n` always breaks.
fn wrap(glyphs: &GlyphSet, text: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if width <= 0.0 {
            lines.push(paragraph.to_string());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && glyphs.width(&candidate) > width {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> Registry {
        let doc = Document::parse(xml).unwrap();
        let mut reg = Registry::default();
        for n in doc.root_element().children().filter(|c| c.is_element()) {
            let def = frame_def(&n);
            if let Some(name) = def.name.clone() {
                if !def.is_virtual {
                    reg.order.push(name.clone());
                }
                reg.frames.insert(name, def);
            }
        }
        reg
    }

    fn rect_of(xml: &str, name: &str) -> Rect {
        let mut tree = Tree::new(Rc::new(parse(xml)));
        tree.instantiate("Root").unwrap();
        tree.rect_of(name, (1024, 768)).unwrap()
    }

    const LAYOUT: &str = r#"<Ui>
        <Frame name="Root"><Size><AbsDimension x="200" y="100"/></Size>
            <Anchors><Anchor point="TOPLEFT"><Offset><AbsDimension x="10" y="-20"/></Offset></Anchor></Anchors>
            <Frames>
                <Button name="$parentOk" inherits="Template"><Anchors><Anchor point="BOTTOMRIGHT"><Offset><AbsDimension x="-5" y="5"/></Offset></Anchor></Anchors></Button>
                <Frame name="Stretched"><Anchors>
                    <Anchor point="TOPLEFT" relativeTo="Root"><Offset><AbsDimension x="4" y="-4"/></Offset></Anchor>
                    <Anchor point="BOTTOMRIGHT" relativeTo="RootOk" relativePoint="TOPLEFT"/></Anchors></Frame>
            </Frames></Frame>
        <Button name="Template" virtual="true"><Size><AbsDimension x="40" y="20"/></Size></Button>
    </Ui>"#;

    #[test]
    fn anchors_place_frames_from_the_bottom_left_with_y_up() {
        // Root: its top left 10 right and 20 below the screen's top left (768 high): x 10, top 748, bottom 648.
        let root = rect_of(LAYOUT, "Root");
        assert_eq!((root.x, root.y, root.w, root.h), (10.0, 648.0, 200.0, 100.0));
        // The button takes its size from the template and sits in the bottom right corner, 5 in.
        let ok = rect_of(LAYOUT, "RootOk");
        assert_eq!((ok.x, ok.y, ok.w, ok.h), (10.0 + 200.0 - 5.0 - 40.0, 648.0 + 5.0, 40.0, 20.0));
        // Two anchors stretch: from 4 in and 4 down from Root's top left to the button's top left.
        let s = rect_of(LAYOUT, "Stretched");
        assert_eq!((s.x, s.w), (14.0, ok.x - 14.0));
        assert_eq!((s.y, s.h), (ok.y + ok.h, 748.0 - 4.0 - (ok.y + ok.h)));
    }

    #[test]
    fn templates_merge_and_their_children_are_named_after_the_instance() {
        let reg = parse(
            r#"<Ui>
            <Frame name="Slot" virtual="true"><Size><AbsDimension x="37" y="37"/></Size>
                <Layers><Layer level="BORDER"><Texture name="$parentIconTexture"/></Layer></Layers></Frame>
            <Frame name="Head" inherits="Slot"><Layers><Layer level="ARTWORK"><FontString name="$parentCount" inherits="F" text="3"/></Layer></Layers></Frame></Ui>"#,
        );
        let mut tree = Tree::new(Rc::new(reg));
        tree.instantiate("Head").unwrap();
        let head = &tree.nodes[tree.index("Head").unwrap()];
        assert_eq!(head.size, Some((37.0, 37.0)));
        let names: Vec<&str> = head.regions.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["HeadIconTexture", "HeadCount"]);
        // Texts and textures are set by region name.
        tree.set_text("HeadCount", "12");
        tree.set_texture("HeadIconTexture", Some(r"Interface\Icons\x"));
        let head = &tree.nodes[tree.index("Head").unwrap()];
        assert!(matches!(&head.regions[1].kind, Kind::Text { text, .. } if text == "12"));
        assert!(matches!(&head.regions[0].kind, Kind::Texture { file: Some(f), .. } if f.ends_with('x')));
    }

    #[test]
    fn top_level_frames_naming_a_parent_are_instantiated_under_it() {
        let reg = parse(r#"<Ui><Frame name="Root" hidden="true"/><Frame name="Sub" parent="Root"/><Frame name="Other"/></Ui>"#);
        let mut tree = Tree::new(Rc::new(reg));
        tree.instantiate("Root").unwrap();
        assert!(tree.index("Sub").is_some() && tree.index("Other").is_none());
        // Hidden parents hide their children.
        assert!(!tree.is_name_shown("Sub"));
        tree.show("Root", true);
        assert!(tree.is_name_shown("Sub"));
    }

    #[test]
    fn a_scroll_frame_cuts_what_is_in_it_and_the_wheel_moves_it() {
        let reg = parse(
            r#"<Ui><ScrollFrame name="Box"><Size><AbsDimension x="100" y="100"/></Size><Anchors><Anchor point="TOPLEFT"/></Anchors>
            <ScrollChild><Frame name="Child"><Size><AbsDimension x="100" y="300"/></Size><Frames>
                <Button name="Low"><Size><AbsDimension x="40" y="20"/></Size><Anchors><Anchor point="BOTTOMLEFT"/></Anchors></Button>
            </Frames></Frame></ScrollChild></ScrollFrame></Ui>"#,
        );
        let mut tree = Tree::new(Rc::new(reg));
        tree.instantiate("Box").unwrap();
        tree.show("Box", true);
        let screen = (1024, 768);
        // The button is at the bottom of a frame three times as tall as the box: cut away, so it takes no click.
        let low = tree.index("Low").unwrap();
        assert_eq!(tree.hit_test((10.0, 758.0 - 490.0 + 10.0), screen), None);
        // The wheel over somewhere else does nothing; over the box it scrolls (down is a negative turn), no further than
        // the content goes (200 units: the child reaches 200 below the box).
        assert!(!tree.scroll_at((500.0, 500.0), screen, -1.0));
        assert!(tree.scroll_at((50.0, 50.0), screen, -100.0));
        assert_eq!(tree.nodes[tree.index("Box").unwrap()].scrolled, 200.0);
        // Now the button is inside the box, and takes the click; scrolling back up hides it again.
        assert_eq!(tree.hit_test((10.0, 768.0 - 678.0), screen), Some(low));
        tree.scroll_at((50.0, 50.0), screen, 100.0);
        assert_eq!(tree.hit_test((10.0, 768.0 - 678.0), screen), None);
    }

    #[test]
    fn the_scroll_bar_beside_a_scroll_frame_does_not_move_or_add_to_how_far_it_scrolls() {
        let reg = parse(
            r#"<Ui><ScrollFrame name="Box"><Size><AbsDimension x="100" y="100"/></Size><Anchors><Anchor point="TOPLEFT"/></Anchors>
            <Frames><Slider name="BoxScrollBar"><Size><AbsDimension x="16" y="0"/></Size><Anchors><Anchor point="TOPLEFT" relativePoint="TOPRIGHT"/><Anchor point="BOTTOMLEFT" relativePoint="BOTTOMRIGHT"><Offset><AbsDimension x="0" y="-16"/></Offset></Anchor></Anchors>
                <Frames><Button name="BoxScrollBarDown"><Size><AbsDimension x="16" y="16"/></Size><Anchors><Anchor point="TOP" relativePoint="BOTTOM"/></Anchors></Button></Frames></Slider></Frames>
            <ScrollChild><Frame name="Child"><Size><AbsDimension x="100" y="300"/></Size></Frame></ScrollChild></ScrollFrame></Ui>"#,
        );
        let mut tree = Tree::new(Rc::new(reg));
        tree.instantiate("Box").unwrap();
        tree.show("Box", true);
        let screen = (1024, 768);
        let before = tree.rect_of("BoxScrollBar", screen).unwrap();
        let (_, max) = tree.scroll_state("Box", screen).unwrap();
        assert_eq!(max, 200.0, "the bar's button hangs below the frame, which is no content");
        assert!(tree.scroll_at((50.0, 50.0), screen, -100.0));
        // However often the wheel turns, the content ends where it ends, and the bar stays where it stood.
        assert!(tree.scroll_at((50.0, 50.0), screen, -100.0));
        assert_eq!(tree.scroll_state("Box", screen), Some((200.0, 200.0)));
        let after = tree.rect_of("BoxScrollBar", screen).unwrap();
        assert_eq!((before.x, before.y), (after.x, after.y));
    }

    #[test]
    fn colour_codes_split_a_text_into_coloured_pieces() {
        let spans = color_spans("Armor: |cff20ff2012|r pts");
        let text: Vec<&str> = spans.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(text, ["Armor: ", "12", " pts"]);
        assert_eq!(spans[0].1, None);
        let green = spans[1].1.unwrap();
        assert!((green[0] - 32.0 / 255.0).abs() < 1e-3 && green[1] == 1.0 && green[3] == 1.0);
        assert_eq!(spans[2].1, None);
        assert_eq!(color_spans("a|b").iter().map(|(t, _)| t.as_str()).collect::<String>(), "a|b");
    }

    #[test]
    fn global_strings_and_fonts_are_read() {
        let mut reg = Registry::default();
        reg.load_strings("-- comment\nBLOCK = \"Block\"; -- Block an attack\nTWO = \"a\\nb\";\nNOT_A_STRING = 5;\n");
        assert_eq!((reg.string("BLOCK"), reg.string("TWO"), reg.string("MISSING")), ("Block".into(), "a\nb".into(), "MISSING".into()));
        let doc = Document::parse(r#"<Ui><Font name="Base" font="Fonts\ARIALN.TTF" outline="THICK" virtual="true"><FontHeight><AbsValue val="12"/></FontHeight><Color r="0.6" g="0.6" b="0.6"/></Font>
            <Font name="Big" inherits="Base"><FontHeight><AbsValue val="16"/></FontHeight></Font></Ui>"#)
        .unwrap();
        for n in doc.root_element().children().filter(|c| c.is_element()) {
            reg.fonts.insert(attr(&n, "name").unwrap().to_string(), font_def(&n));
        }
        let big = reg.font_style("Big").unwrap();
        assert_eq!((big.height, big.outline, big.color[0]), (16.0, 2, 0.6));
        assert!(big.file.ends_with("ARIALN.TTF"));
    }

    #[test]
    fn the_original_character_frame_loads_and_lays_out() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let reg = Rc::new(Registry::load(&mount).unwrap());
        let mut tree = Tree::new(reg);
        tree.instantiate("CharacterFrame").unwrap();
        // The frame hangs off the top left of the screen, 104 units down, 384 x 512.
        let frame = tree.rect_of("CharacterFrame", (1024, 768)).unwrap();
        assert_eq!((frame.x, frame.y, frame.w, frame.h), (0.0, 768.0 - 104.0 - 512.0, 384.0, 512.0));
        // Its sub frames and the equipment slots exist, and the slots have their icon region from the template.
        for name in ["PaperDollFrame", "CharacterHeadSlot", "CharacterMainHandSlot", "CharacterSecondaryHandSlot", "CharacterFrameCloseButton"] {
            assert!(tree.index(name).is_some(), "{name} is missing");
        }
        assert!(tree.regions_by_name.contains_key("CharacterHeadSlotIconTexture"));
        // The head slot is 37 x 37 at the top left of the paper doll, as the file says.
        let head = tree.rect_of("CharacterHeadSlot", (1024, 768)).unwrap();
        assert_eq!((head.w, head.h), (37.0, 37.0));
    }
}
