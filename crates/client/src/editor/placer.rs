use super::*;

/// What the mouse and keys do in the world while the editor is open, over the world beside the window (the lines for what is going on
/// come first).
pub(super) fn hud(g: &mut Gui, placer: &Placer, props: bool, width: f32) {
    let mut lines: Vec<(String, [f32; 4])> = Vec::new();
    if placer.placing && placer.aim == Aim::Stop {
        lines.push(("Adding stops to the patrol: every click in the world adds one stop there (a blue mark shows where). Esc stops.".to_string(), GOLD));
    } else if placer.placing && placer.aim != Aim::Prop {
        let what = if placer.aim == Aim::Creature { placer.creature.as_str() } else { file_stem(&placer.chest) };
        lines.push((format!("Putting down {what}: click stands it there, facing the way the arrow points. Hold the right button and move the mouse to turn it ({:.0} deg). Esc stops.", placer.turn), GOLD));
    } else if placer.placing {
        lines.push((format!("Putting down {}: click puts it there. Hold the right button and move the mouse to turn it ({:.0} deg), the wheel sizes it (x{:.2}), Shift + wheel raises it ({:+.2}). Esc stops.", file_stem(&placer.model), placer.turn, placer.scale, placer.lift), GOLD));
    } else if props {
        lines.push(("Click a model to select it, then drag the arrows, the ring or the knob that appear on it (Move, Turn, Scale in the window).".to_string(), GOLD));
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

/// The doodads and buildings of the game that can be put into the world, and the one that clicks in the world put down.
pub(super) struct Placer {
    pub(super) mount: Arc<mpq::Mount>,
    /// The files, listed once when the gallery is first opened.
    pub(super) files: Option<Vec<String>>,
    pub(super) model: String,
    pub(super) placing: bool,
    pub(super) gallery: Gallery,
    /// The gallery window, while it is open, and what it showed when it was last closed.
    pub(super) view: Option<View>,
    pub(super) memory: Option<Memory>,
    pub(super) models: Models,
    /// What the gizmo does, the part of it being dragged, and the part under the pointer.
    pub(super) tool: Mode,
    pub(super) drag: Option<Drag>,
    /// What is dragged is a creature or a chest: it is applied (saved) when it is let go.
    pub(super) drag_actor: bool,
    pub(super) hot: Option<Handle>,
    /// Where the pointer touches the world while models are put down, and whether the ghost of a dragged prop stays until the world shows it.
    pub(super) hover: Option<Vec3>,
    pub(super) hold: bool,
    /// What the editor asks of the game besides what a frame's buttons do.
    pub(super) out: Vec<Action>,
    /// What clicks in the world put down: a prop (`model`), a creature (`creature`) or a chest (`chest`, its model).
    pub(super) aim: Aim,
    pub(super) creature: String,
    pub(super) chest: String,
    /// Whether the ghost of what is put down is shown (else a mark on the ground shows where it will stand).
    pub(super) ghosted: bool,
    /// How the model that is put down is turned (degrees), how big it is and how far above the ground it stands.
    pub(super) turn: f32,
    pub(super) scale: f32,
    pub(super) lift: f32,
}

impl Placer {
    pub(super) fn files(&mut self) -> &[String] {
        let mount = &self.mount;
        self.files.get_or_insert_with(|| mount.list().into_iter().filter(|f| placeable(f)).collect())
    }

    /// Start putting a creature or a chest down: the next clicks in the world stand it there, facing as `facing`.
    pub(super) fn start(&mut self, aim: Aim, what: &str, facing: f32) {
        match aim {
            Aim::Creature => self.creature = what.to_string(),
            Aim::Chest => self.chest = what.to_string(),
            Aim::Prop | Aim::Stop => {}
        }
        (self.aim, self.placing, self.turn) = (aim, true, facing);
    }

    /// Open the gallery on the model `current` for the field `tag`, which takes the one that is chosen.
    pub(super) fn open_gallery_for(&mut self, g: &mut Gui, tag: u64, current: &str) {
        self.open_gallery(g, current);
        if let Some(v) = &mut self.view {
            v.answer = Some(tag);
        }
    }

    /// Open the gallery on the model `current`.
    pub(super) fn open_gallery(&mut self, g: &mut Gui, current: &str) {
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
pub(super) enum Aim {
    Prop,
    Creature,
    Chest,
    /// A stop of the patrol of the creature selected.
    Stop,
}

/// The chest models the game can open and show (the picks and the opening animation are those of these two).
pub(super) const CHEST_MODELS: [(&str, &str); 2] = [(r"World\Generic\ActiveDoodads\Chest01\Chest01.m2", "Chest01"), (r"World\Generic\ActiveDoodads\Chest03\Chest03.m2", "Chest03")];

/// The gallery window: the folders of the models on the left, the pictures of the models of the folder on the right.
pub(super) struct View {
    pub(super) find: String,
    /// The folders as lines for `list` (the first is all models); the one chosen is `folders.selected`.
    pub(super) lines: Vec<String>,
    pub(super) folders: Pick,
    /// For each file: its lower-case name to search in, and its folder (an index of `keys`).
    pub(super) index: Vec<(String, usize)>,
    /// The first row of pictures on show.
    pub(super) scroll: usize,
    pub(super) selected: Option<String>,
    /// For the creatures and the chests: the window has their pictures in a grid and no folders.
    pub(super) entries: Option<Entries>,
    /// The field that asked for a model to replace its own: choosing gives the model back to it and closes the window (else it starts putting the model down).
    pub(super) answer: Option<u64>,
}

/// What a field chooses from in the gallery: its tag (the answer goes to it), the title and the entries.
pub(super) struct Entries {
    pub(super) tag: u64,
    pub(super) title: &'static str,
    pub(super) list: Vec<GalleryEntry>,
    /// The window stays when one is chosen (the field is for what is put down: more can be chosen, and put down).
    pub(super) keep_open: bool,
}

pub(super) struct GalleryEntry {
    /// What is given back, what is shown under the picture, the lower-case words that are searched, the key of the picture and what it
    /// is made of (none: a model file, which is the key).
    pub(super) value: String,
    pub(super) label: String,
    pub(super) search: String,
    pub(super) key: String,
    pub(super) subject: Option<Subject>,
}

impl View {
    pub(super) fn of_entries(tag: u64, title: &'static str, list: Vec<GalleryEntry>, keep_open: bool, current: &str) -> View {
        let selected = Some(current.to_string()).filter(|c| list.iter().any(|e| e.value == *c));
        View { find: String::new(), lines: Vec::new(), folders: Pick { selected: 0, scroll: 0, form_scroll: 0.0, find: String::new(), salt: 91 }, index: Vec::new(), scroll: 0, selected, entries: Some(Entries { tag, title, list, keep_open }), answer: None }
    }

    pub(super) fn new(files: &[String], current: &str) -> View {
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
        View { find: String::new(), lines, folders: Pick { selected: at, scroll: at.saturating_sub(8), form_scroll: 0.0, find: String::new(), salt: 90 }, index, scroll: 0, selected, entries: None, answer: None }
    }
}

/// What the model gallery showed when it was closed (the words, the folder and how far it was scrolled), so that it opens on the same again.
pub(super) struct Memory {
    pub(super) find: String,
    pub(super) folder: usize,
    pub(super) folder_scroll: usize,
    pub(super) folder_find: String,
    pub(super) scroll: usize,
}

impl Memory {
    pub(super) fn of(v: &View) -> Memory {
        Memory { find: v.find.clone(), folder: v.folders.selected, folder_scroll: v.folders.scroll, folder_find: v.folders.find.clone(), scroll: v.scroll }
    }
}

/// The folder of a model in the gallery: its place under `World`, three levels at most, without the folders that every doodad has.
pub(super) fn category_of(file: &str) -> String {
    let dirs: Vec<&str> = file.split('\\').skip(1).collect();
    dirs[..dirs.len().saturating_sub(1)].iter().filter(|d| !d.eq_ignore_ascii_case("passivedoodads")).take(3).copied().collect::<Vec<_>>().join("\\")
}

/// Pictures of the gallery in a row and in a column.
pub(super) const GALLERY_COLUMNS: usize = 4;
pub(super) const GALLERY_ROWS: usize = 4;

/// The gallery window (drawn over everything else of the editor): the models as pictures in a grid, narrowed by a folder and by the words
/// typed. A second click on a picture or Enter chooses it, Esc or "Cancel" does not.
pub(super) fn gallery(g: &mut Gui, placer: &mut Placer) {
    let Some(mut v) = placer.view.take() else { return };
    if v.entries.is_some() {
        return entries_window(g, placer, v);
    }
    let (px, py, pw, ph) = (30.0, 40.0, 660.0, 680.0);
    g.fill(0.0, 0.0, WIDTH, UI_HEIGHT, [0.0, 0.0, 0.0, 0.6]);
    g.fill(px, py, pw, ph, [0.07, 0.07, 0.1, 1.0]);
    g.text(px + 16.0, py + 12.0, pw - 32.0, if v.answer.is_some() { "Choose the model (doodads and buildings)" } else { "Choose a model to put down (doodads and buildings)" }, GOLD);
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
    g.text(gx, below, 430.0, &format!("{} models  ({}; the wheel scrolls)", shown.len(), if v.answer.is_some() { "click one to choose it" } else { "click one, then click in the world to put it down" }), DIM);
    g.text(gx, below + ROW, 430.0, hovered.map(|h| h.as_str()).or(v.selected.as_deref()).unwrap_or(""), TEXT);
    let closed = g.button(px + 16.0, py + ph - 34.0, 110.0, if v.answer.is_some() { "Cancel" } else { "Close" });
    if esc || closed {
        g.edit.focus = None;
        g.edit.want = None;
        placer.memory = Some(Memory::of(&v));
        return;
    }
    // Choosing a model starts putting it down: the next click in the world, beside this window, stands it there. The window stays, to choose
    // another one.
    if let Some(file) = confirm.or(if enter { v.selected.clone() } else { None }) {
        if let Some(tag) = v.answer {
            g.modal.picked = Some((tag, file));
            g.edit.focus = None;
            g.edit.want = None;
            placer.memory = Some(Memory::of(&v));
            return;
        }
        (placer.model, placer.aim, placer.placing) = (file, Aim::Prop, true);
        g.edit.focus = None;
        g.edit.want = None;
    }
    placer.view = Some(v);
}

/// The gallery of creatures or chests: their pictures in a grid, narrowed by the words typed. Choosing one gives its value back to the field
/// that asked (as a chooser does).
pub(super) fn entries_window(g: &mut Gui, placer: &mut Placer, mut v: View) {
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
    pub(super) fn open_entries(&mut self, g: &mut Gui, raws: &Raws, tag: u64, what: Pickable, current: &str) {
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
pub(super) fn placeable(file: &str) -> bool {
    let f = file.to_ascii_lowercase();
    let group = f.strip_suffix(".wmo").is_some_and(|s| s.len() > 4 && s.as_bytes()[s.len() - 4] == b'_' && s[s.len() - 3..].bytes().all(|b| b.is_ascii_digit()));
    f.starts_with(r"world\") && !f.starts_with(r"world\maps\") && (f.ends_with(".m2") || (f.ends_with(".wmo") && !group))
}
