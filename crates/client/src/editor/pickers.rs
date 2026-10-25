use super::*;

/// What a chooser shows of the entry it is on, in the picture beside its list.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum Shows {
    Nothing,
    /// The entry is the file of a creature model.
    Model,
    /// The entry is the number of a set of skins of this model.
    Skin(String),
    /// The entry is a row of `ItemDisplayInfo.dbc`, to be worn or held in this way.
    Display(Wear),
}

/// A line of a chooser: what it gives back, and what it says.
pub(super) struct Entry {
    pub(super) value: String,
    pub(super) label: String,
}

/// A window over the editor with a list to choose one of, narrowed by what is typed.
pub(super) struct Chooser {
    /// The field that asked, to take the answer.
    pub(super) tag: u64,
    pub(super) title: String,
    pub(super) entries: Vec<Entry>,
    pub(super) find: String,
    pub(super) selected: Option<usize>,
    pub(super) scroll: usize,
    pub(super) shows: Shows,
}

#[derive(Default)]
pub(super) struct Modal {
    pub(super) open: Option<Chooser>,
    pub(super) picked: Option<(u64, String)>,
    /// A field asked for the gallery of creatures or chests (opened by the editor, which has the pictures).
    pub(super) want: Option<(u64, Pickable, String)>,
    /// The picker of the icon of a place is open (on this icon), and the one that was clicked in it.
    pub(super) icons: Option<usize>,
    pub(super) icon_picked: Option<usize>,
}

/// What the gallery over a field lets one choose from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Pickable {
    Creatures,
    Chests,
}

/// What a field can refer to by its id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Ref {
    Creature,
    Item,
    Loot,
    Quest,
    Spell,
    NodeKind,
}

impl Ref {
    pub(super) fn title(self) -> &'static str {
        match self {
            Ref::Creature => "Choose a creature",
            Ref::Item => "Choose an item",
            Ref::Loot => "Choose a loot table",
            Ref::Quest => "Choose a quest",
            Ref::Spell => "Choose a spell",
            Ref::NodeKind => "Choose a kind of node",
        }
    }
}

/// The ids (and what they are called) of everything a field can refer to.
#[derive(Default)]
pub(super) struct Ids {
    pub(super) creatures: Vec<(String, String)>,
    pub(super) items: Vec<(String, String)>,
    pub(super) loot: Vec<(String, String)>,
    pub(super) quests: Vec<(String, String)>,
    pub(super) spells: Vec<(String, String)>,
    pub(super) nodes: Vec<(String, String)>,
}

impl Ids {
    pub(super) fn of(raws: &Raws) -> Ids {
        Ids {
            creatures: raws.creatures.iter().map(|c| (c.id.clone(), c.name.clone())).collect(),
            items: raws.items.iter().map(|i| (i.id.clone(), i.name.clone())).collect(),
            loot: raws.loot.keys().map(|k| (k.clone(), String::new())).collect(),
            quests: raws.quests.iter().map(|q| (q.id.clone(), q.title.clone())).collect(),
            spells: content::get().spells.iter().map(|s| (s.id.clone(), s.name.clone())).collect(),
            nodes: raws.node_kinds.iter().map(|k| (k.id.clone(), format!("{}, {} {}", k.name, k.skill.name(), k.rank))).collect(),
        }
    }

    pub(super) fn of_kind(&self, kind: Ref) -> &[(String, String)] {
        match kind {
            Ref::Creature => &self.creatures,
            Ref::Item => &self.items,
            Ref::Loot => &self.loot,
            Ref::Quest => &self.quests,
            Ref::Spell => &self.spells,
            Ref::NodeKind => &self.nodes,
        }
    }

    pub(super) fn has(&self, kind: Ref, id: &str) -> bool {
        self.of_kind(kind).iter().any(|(i, _)| i == id)
    }
}

/// The width of the button that opens a chooser.
pub(super) const BROWSE: f32 = 28.0;

impl Gui<'_> {
    /// What was chosen for the field `tag`, once.
    pub(super) fn picked(&mut self, tag: u64) -> Option<String> {
        if self.modal.picked.as_ref().is_some_and(|(t, _)| *t == tag) {
            return self.modal.picked.take().map(|(_, v)| v);
        }
        None
    }

    /// Open a chooser for the field `tag`, on the entry with the value `current`.
    pub(super) fn open_chooser(&mut self, tag: u64, title: &str, shows: Shows, entries: Vec<Entry>, current: &str) {
        let selected = entries.iter().position(|e| e.value == current);
        self.modal.open = Some(Chooser { tag, title: title.to_string(), entries, find: String::new(), selected, scroll: selected.map_or(0, |s| s.saturating_sub(10)), shows });
        // What is typed goes to its find field at once.
        self.edit.want = Some((fid("chooser", 0), String::new()));
    }

    /// The `...` button that opens a chooser over the ids of `kind` for the field `tag`.
    pub(super) fn browse_ids(&mut self, tag: u64, x: f32, y: f32, kind: Ref, current: &str) {
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
pub(super) fn text_row_ref(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut String, kind: Ref, append: Option<&str>) -> bool {
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
pub(super) fn option_ref_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut Option<String>, kind: Ref) -> bool {
    let mut text = value.clone().unwrap_or_default();
    if !text_row_ref(g, y, x, w, label, n, &mut text, kind, None) {
        return false;
    }
    *value = Some(text.trim().to_string()).filter(|t| !t.is_empty());
    true
}

/// The window that chooses (drawn over everything else of the editor, which can not be clicked meanwhile): a find field, the list it
/// narrows, and for some lists a picture of the entry the list is on. Enter or "Choose" gives the entry back, Esc or "Cancel" does not.
pub(super) fn chooser(g: &mut Gui, pv: &mut Preview) {
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
pub(super) fn icon_chooser(g: &mut Gui) {
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
