use super::*;

pub(super) struct Gui<'a> {
    pub(super) ui: &'a mut Ui,
    pub(super) font: &'a GlyphSet,
    pub(super) white: usize,
    pub(super) poi_icons: Option<usize>,
    pub(super) s: f32,
    pub(super) input: &'a mut Input,
    pub(super) edit: &'a mut Edit,
    pub(super) status: &'a mut (String, [f32; 4]),
    /// Where the mouse may act now, in units (the form's area).
    pub(super) clip: Option<[f32; 4]>,
    pub(super) modal: &'a mut Modal,
    /// What the references can be (ids of what the content has).
    pub(super) ids: &'a Ids,
    /// Nothing under a chooser that is open may be clicked.
    pub(super) blocked: bool,
    /// The colour of the text of the next field, when it is not the usual.
    pub(super) tint: Option<[f32; 4]>,
    /// How many rows tall the next field is, when it is for long text (shown over several lines).
    pub(super) tall: Option<usize>,
    /// The sections that are open the other way from how they start.
    pub(super) folds: &'a mut HashSet<u64>,
}

/// A number for a field that stays put when the form changes under it.
pub(super) fn fid(tag: &str, n: usize) -> u64 {
    tag.bytes().fold(0xcbf29ce484222325u64 ^ n as u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

impl Gui<'_> {
    pub(super) fn px(&self, v: f32) -> f32 {
        v * self.s
    }

    pub(super) fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.picture(self.white, x, y, w, h, color);
    }

    /// A texture over the rectangle, tinted.
    pub(super) fn picture(&mut self, texture: usize, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        let r = [self.px(x), self.px(y), self.px(w), self.px(h)];
        self.ui.quad(Quad { texture, rect: r, uv: FULL_UV, color, additive: false });
    }

    /// A part of a texture (`uv`: left, top, right, bottom) over the rectangle.
    pub(super) fn picture_part(&mut self, texture: usize, x: f32, y: f32, w: f32, h: f32, uv: [f32; 4]) {
        let r = [self.px(x), self.px(y), self.px(w), self.px(h)];
        self.ui.quad(Quad { texture, rect: r, uv, color: [1.0; 4], additive: false });
    }

    /// The area the quads are cut to now: that of the form, if there is one.
    pub(super) fn restore_clip(&mut self) {
        let clip = self.clip.map(|[x, y, w, h]| [self.px(x), self.px(y), self.px(w), self.px(h)]);
        self.ui.set_clip(clip);
    }

    /// Text with the top of its line at `y`; clipped to `w` units (and to the form's area).
    pub(super) fn text(&mut self, x: f32, y: f32, w: f32, text: &str, color: [f32; 4]) {
        let mut clip = [x, y - 2.0, w, ROW];
        if let Some([cx, cy, cw, ch]) = self.clip {
            let (x0, y0, x1, y1) = (clip[0].max(cx), clip[1].max(cy), (clip[0] + clip[2]).min(cx + cw), (clip[1] + clip[3]).min(cy + ch));
            clip = [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)];
        }
        self.ui.set_clip(Some([self.px(clip[0]), self.px(clip[1]), self.px(clip[2]), self.px(clip[3])]));
        self.font.draw(self.ui, text, self.px(x), self.px(y), self.s, color);
        self.restore_clip();
    }

    pub(super) fn mouse(&self) -> (f32, f32) {
        (self.input.mouse.0 / self.s, self.input.mouse.1 / self.s)
    }

    pub(super) fn inside(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let (mx, my) = self.mouse();
        let in_clip = !self.blocked && self.clip.is_none_or(|[cx, cy, cw, ch]| mx >= cx && mx < cx + cw && my >= cy && my < cy + ch);
        in_clip && mx >= x && mx < x + w && my >= y && my < y + h
    }

    /// A click inside the rectangle, taken.
    pub(super) fn clicked(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let hit = self.input.click && self.inside(x, y, w, h);
        if hit {
            self.input.click = false;
        }
        hit
    }

    pub(super) fn button(&mut self, x: f32, y: f32, w: f32, label: &str) -> bool {
        self.button_in(x, y, w, label, TEXT)
    }

    /// A button whose label is in `color` until the pointer is over it.
    pub(super) fn button_in(&mut self, x: f32, y: f32, w: f32, label: &str, color: [f32; 4]) -> bool {
        let hot = self.inside(x, y, w, ROW);
        self.fill(x, y, w, ROW, if hot { HOT } else { BOX });
        let width = self.font.width(label);
        self.text(x + (w - width) / 2.0, y + 3.0, w, label, if hot && color != RED { GOLD } else { color });
        self.clicked(x, y, w, ROW)
    }

    /// A button for what cannot be taken back: the first click only asks "Sure?" (in red, for three seconds), the second does it.
    /// `tag` tells it from the other such buttons that are on the screen.
    pub(super) fn danger(&mut self, tag: &str, x: f32, y: f32, w: f32, label: &str) -> bool {
        let id = fid(tag, 0);
        let armed = self.edit.armed.is_some_and(|(a, since)| a == id && since.elapsed().as_secs_f32() < 3.0);
        if !armed && self.edit.armed.is_some_and(|(a, _)| a == id) {
            self.edit.armed = None;
        }
        if self.button_in(x, y, w, if armed { "Sure? Click again" } else { label }, if armed { RED } else { TEXT }) {
            if armed {
                self.edit.armed = None;
                return true;
            }
            self.edit.armed = Some((id, std::time::Instant::now()));
        }
        false
    }

    /// A label and a text field after it; true when what was typed is kept. A field for long text (`tall`) shows it over several lines.
    pub(super) fn field(&mut self, tag: &str, n: usize, x: f32, y: f32, w: f32, value: &mut String) -> bool {
        let id = fid(tag, n);
        let color = self.tint.take().unwrap_or(TEXT);
        let rows = self.tall.take().unwrap_or(1);
        let h = rows as f32 * ROW;
        let editing = self.edit.focus == Some(id);
        let hot = self.inside(x, y, w, h);
        self.fill(x, y, w, h, if editing { SELECTED } else if hot { HOT } else { BOX });
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
            if self.input.click && !self.inside(x, y, w, h) {
                *value = self.edit.buffer.clone();
                (self.edit.focus, changed) = (None, true);
            }
        }
        let typing = self.edit.focus == Some(id);
        if typing {
            self.edit.seen = true;
        }
        if rows > 1 {
            // The text over its lines, the last ones when it is longer than the field while it is typed in.
            let shown = if typing { format!("{}_", self.edit.buffer) } else { value.clone() };
            let lines = wrapped(self, w - 10.0, &shown);
            let skip = if typing { lines.len().saturating_sub(rows) } else { 0 };
            for (i, line) in lines.iter().skip(skip).take(rows).enumerate() {
                // More text than the field shows is told by dots on its last line.
                let more = !typing && i + 1 == rows && lines.len() > rows;
                self.text(x + 4.0, y + 3.0 + i as f32 * ROW, w - 8.0, &if more { format!("{line} ...") } else { line.clone() }, if typing { TEXT } else { color });
            }
        } else if typing {
            let shown = format!("{}_", self.edit.buffer);
            let overflow = (self.font.width(&shown) - (w - 10.0)).max(0.0);
            self.text(x + 4.0 - overflow, y + 3.0, w - 6.0 + overflow, &shown, TEXT);
        } else {
            self.text(x + 4.0, y + 3.0, w - 8.0, value, color);
        }
        if self.input.click && self.inside(x, y, w, h) && self.edit.focus != Some(id) {
            self.input.click = false;
            self.edit.want = Some((id, value.clone()));
        }
        changed
    }

    /// Arrows that step through choices: -1 for the left one, 1 for the right one.
    pub(super) fn arrows(&mut self, x: f32, y: f32, w: f32, shown: &str) -> i32 {
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

    pub(super) fn say(&mut self, text: impl Into<String>, color: [f32; 4]) {
        *self.status = (text.into(), color);
    }
}

/// The next (or previous) of `options` after `current`, wrapping round.
pub(super) fn step<'a>(options: &'a [String], current: &str, by: i32) -> &'a str {
    let at = options.iter().position(|o| o == current).unwrap_or(0) as i32;
    &options[(at + by).rem_euclid(options.len() as i32) as usize]
}

/// One line of a form: its label, and a text field for a string.
pub(super) fn text_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut String) -> bool {
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    let changed = g.field(label, n, x + LABEL_WIDTH, *y, w - LABEL_WIDTH, value);
    *y += ROW + 3.0;
    changed
}

/// How a number of type `T` is asked for in a message: "a whole number, 0 or more", "a whole number", "a number".
pub(super) fn number_words<T>() -> &'static str {
    let name = std::any::type_name::<T>();
    if name.starts_with('u') {
        "a whole number, 0 or more"
    } else if name.starts_with('i') {
        "a whole number"
    } else {
        "a number (a point for decimals)"
    }
}

/// A line for long text: its field is `lines` rows tall and shows the text over them.
pub(super) fn text_area_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, lines: usize, value: &mut String) -> bool {
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    g.tall = Some(lines);
    let changed = g.field(label, n, x + LABEL_WIDTH, *y, w - LABEL_WIDTH, value);
    *y += lines as f32 * ROW + 3.0;
    changed
}

/// A line for a value that is written and read as text: changed when it was typed and read.
pub(super) fn value_row<T: Display + FromStr>(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut T) -> bool {
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
            g.say(format!("{label}: \"{}\" is not {}", text.trim(), number_words::<T>()), RED);
            false
        }
    }
}

/// A line for an optional string: empty is none.
pub(super) fn option_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut Option<String>) -> bool {
    let mut text = value.clone().unwrap_or_default();
    if !text_row(g, y, x, w, label, n, &mut text) {
        return false;
    }
    *value = Some(text).filter(|t| !t.trim().is_empty());
    true
}

/// A line for an optional number: empty is none.
pub(super) fn option_value_row<T: Display + FromStr>(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut Option<T>) -> bool {
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
            g.say(format!("{label}: \"{}\" is not {} (leave it empty for none)", text.trim(), number_words::<T>()), RED);
            false
        }
    }
}

/// A grey line under a row, saying what it means.
pub(super) fn hint(g: &mut Gui, y: &mut f32, x: f32, text: &str) {
    let w = WIDTH - x - LABEL_WIDTH - 8.0;
    for row in wrapped(g, w, text) {
        g.text(x + LABEL_WIDTH, *y - 2.0, w, &row, DIM);
        *y += ROW - 2.0;
    }
    *y += 3.0;
}

/// A line for a list of strings, written with commas.
pub(super) fn list_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut Vec<String>, kind: Option<Ref>) -> bool {
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
pub(super) fn stacks_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut Vec<(String, u32)>) -> bool {
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
            g.say(format!("{label}: write item:count, item:count (for example longsword:1, linen_cloth:5)"), RED);
            false
        }
    }
}

/// A line of numbers, each in a field of its own with its name before it (up to four in a line, else three).
pub(super) fn numbers_row<T: Display + FromStr + Copy>(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, names: &[&str], value: &mut [T]) -> bool {
    let columns = if value.len() <= 4 { value.len() } else { 3 };
    let cell = (w - LABEL_WIDTH) / columns as f32;
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    let mut changed = false;
    for (k, v) in value.iter_mut().enumerate() {
        let (cx, cy) = (x + LABEL_WIDTH + (k % columns) as f32 * cell, *y + (k / columns) as f32 * (ROW + 3.0));
        let name = names.get(k).copied().unwrap_or("");
        let named = g.font.width(name) + 4.0;
        g.text(cx, cy + 3.0, named, name, DIM);
        let mut text = v.to_string();
        if g.field(label, n + k, cx + named, cy, cell - named - 6.0, &mut text) {
            match text.trim().parse::<T>() {
                Ok(parsed) => (*v, changed) = (parsed, true),
                Err(_) => g.say(format!("{label} {name}: \"{}\" is not {}", text.trim(), number_words::<T>()), RED),
            }
        }
    }
    *y += value.len().div_ceil(columns) as f32 * (ROW + 3.0);
    changed
}

/// The coins of an amount of copper as the game writes them: `1g 20s 5c`.
pub(super) fn coins(copper: u32) -> String {
    let (gold, silver, copper) = (copper / 10_000, copper / 100 % 100, copper % 100);
    match (gold, silver) {
        (0, 0) => format!("{copper}c"),
        (0, _) => format!("{silver}s {copper}c"),
        _ => format!("{gold}g {silver}s {copper}c"),
    }
}

/// A line for an amount of copper, with the same in gold, silver and copper beside the field.
pub(super) fn money_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut u32) -> bool {
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    let mut text = value.to_string();
    let mut changed = false;
    if g.field(label, n, x + LABEL_WIDTH, *y, 110.0, &mut text) {
        match text.trim().parse::<u32>() {
            Ok(v) => (*value, changed) = (v, true),
            Err(_) => g.say(format!("{label}: \"{}\" is not {}", text.trim(), number_words::<u32>()), RED),
        }
    }
    g.text(x + LABEL_WIDTH + 118.0, *y + 3.0, w - LABEL_WIDTH - 118.0, &format!("= {}", coins(*value)), DIM);
    *y += ROW + 3.0;
    changed
}

/// A line of arrows over a few choices.
pub(super) fn choice_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, options: &[String], current: &mut String) -> bool {
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    let by = g.arrows(x + LABEL_WIDTH, *y, (w - LABEL_WIDTH).min(260.0), current);
    *y += ROW + 3.0;
    if by != 0 {
        *current = step(options, current, by).to_string();
    }
    by != 0
}

/// A line with a switch that says what it is set to in its own words.
pub(super) fn toggle_row(g: &mut Gui, y: &mut f32, x: f32, label: &str, value: &mut bool, on: &str, off: &str) -> bool {
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    let w = (g.font.width(on).max(g.font.width(off)) + 16.0).max(60.0);
    let clicked = g.button_in(x + LABEL_WIDTH, *y, w, if *value { on } else { off }, if *value { GREEN } else { DIM });
    *y += ROW + 3.0;
    if clicked {
        *value = !*value;
    }
    clicked
}

/// A yes/no switch.
pub(super) fn switch_row(g: &mut Gui, y: &mut f32, x: f32, label: &str, value: &mut bool) -> bool {
    toggle_row(g, y, x, label, value, "Yes", "No")
}

/// A row of buttons that switch the bits of a mask on and off (lit: on). A mask of none stands for everyone.
pub(super) fn mask_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, names: &[(&str, u32)], value: &mut u32) -> bool {
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    let (mut bx, left, right) = (x + LABEL_WIDTH, x + LABEL_WIDTH, x + w);
    let mut changed = false;
    for &(name, bit) in names {
        let bw = g.font.width(name) + 14.0;
        if bx + bw > right {
            (bx, *y) = (left, *y + ROW + 3.0);
        }
        let on = *value & bit != 0;
        if g.button_in(bx, *y, bw, name, if on { GREEN } else { DIM }) {
            *value ^= bit;
            changed = true;
        }
        bx += bw + 4.0;
    }
    *y += ROW + 3.0;
    if *value == 0 {
        hint(g, y, x, "None lit: it is for everyone.");
    }
    changed
}

/// A line that opens and closes the part of the form under it (`key` tells it from the other sections; `open` is how it starts).
/// True while it is open: the rows of the section are laid out only then.
pub(super) fn section(g: &mut Gui, y: &mut f32, x: f32, w: f32, key: &str, title: &str, open: bool) -> bool {
    let id = fid(key, 7);
    let (hot, shown) = (g.inside(x, *y, w, ROW), open != g.folds.contains(&id));
    g.fill(x, *y, w, ROW, if hot { HOT } else { PANEL_LINE });
    g.text(x + 4.0, *y + 3.0, w - 8.0, &format!("{} {title}", if shown { "[-]" } else { "[+]" }), GOLD);
    let shown = if g.clicked(x, *y, w, ROW) {
        if !g.folds.remove(&id) {
            g.folds.insert(id);
        }
        !shown
    } else {
        shown
    };
    *y += ROW + 4.0;
    shown
}

/// The row above a list of a tab: a button that makes a new entry, and one that deletes the selected one (after asking once more).
/// True for each that was pressed.
pub(super) fn list_header(g: &mut Gui, top: f32, tag: &str, new: &str, delete: bool) -> (bool, bool) {
    let made = g.button(8.0, top, 128.0, new);
    let gone = delete && g.danger(tag, 142.0, top, 100.0, "Delete");
    (made, gone)
}

/// "Set to my position" and "Teleport there" under a row of coordinates; true when the first was pressed.
pub(super) fn here_and_go(g: &mut Gui, y: &mut f32, x: f32, at: (f32, f32), actions: &mut Vec<Action>) -> bool {
    let here = g.button(x, *y, 150.0, "Set to my position");
    if g.button(x + 156.0, *y, 110.0, "Teleport there") {
        actions.push(Action::Teleport(at.0, at.1));
    }
    *y += ROW + 8.0;
    here
}

/// What the right side of a tab says while nothing is selected.
pub(super) fn empty_pane(g: &mut Gui, x: f32, top: f32, w: f32, text: &str) {
    g.text(x, top + 4.0, w, text, DIM);
}

/// How far from the editor's eye the creatures are that it lists and can pick in the world, yards.
const LISTED_RANGE: f32 = 300.0;

pub(super) fn near_here(at: (f32, f32), me: Place) -> bool {
    (at.0 - me.north).hypot(at.1 - me.west) <= LISTED_RANGE
}

/// A list of lines to choose one of, scrolled by the wheel, with a field above it that narrows it to the lines that have all the words
/// typed (the first two of its `rows` are for the field); true when the choice changed. `pick.selected` is a line of `lines`.
pub(super) fn list(g: &mut Gui, x: f32, y: f32, w: f32, rows: usize, lines: &[String], pick: &mut Pick) -> bool {
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
pub(super) fn fresh(ids: &[&str], base: &str) -> String {
    (1..).map(|n| format!("{base}_{n}")).find(|id| !ids.contains(&id.as_str())).expect("there is a number left")
}

/// `text` broken at spaces into rows that are at most `w` units wide.
pub(super) fn wrapped(g: &Gui, w: f32, text: &str) -> Vec<String> {
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

/// The right side of a tab: a form in a scrolled area. `fill` lays out the rows from the `y` it is given.
pub(super) fn form(g: &mut Gui, p: &mut Pick, x: f32, top: f32, w: f32, bottom: f32, fill: impl FnOnce(&mut Gui, &mut f32, &mut Pick)) {
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

/// The name of a file without its folders and ending.
pub(super) fn file_stem(path: &str) -> &str {
    let name = path.rsplit('\\').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}
