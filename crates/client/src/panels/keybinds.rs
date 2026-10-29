//! The key bindings window (the menu's Key Bindings): the original's `KeyBindingFrame` (the `Blizzard_BindingUI` addon) with the commands of this game
//! in its list. As there: click a key button, press the new key (the command shows what it waits for), Unbind Key clears it, Reset To Default, Cancel and
//! Okay. Left out: the character-specific bindings, modifier chords and the main mouse buttons (the side buttons, Mouse4 and Mouse5, bind like keys).

use super::menu::Group;
use super::*;
use crate::keys::{bindable, key_name, Binds, COMMANDS};
use winit::keyboard::KeyCode;

const FRAME: &str = "KeyBindingFrame";
const BAR: &str = "KeyBindingFrameScrollFrameScrollBar";
/// `KEY_BINDINGS_DISPLAYED`: how many lines of the list show.
const DISPLAYED: usize = 17;
const NORMAL_COLOR: &str = "|cffffd200";

pub(super) struct KeyState {
    pub(super) binds: Group<Binds>,
    /// The key button waiting for its key: (command, slot) (`KeyBindingFrame.selected`).
    selected: Option<(usize, usize)>,
    /// The first line of the list that shows (`FauxScrollFrame_GetOffset`).
    scroll: usize,
}

impl Default for KeyState {
    fn default() -> Self {
        KeyState { binds: Group::new(Binds::load()), selected: None, scroll: 0 }
    }
}

/// A line of the list: the name of a group of commands or a command (an index into `COMMANDS`).
#[derive(Clone, Copy)]
enum Row {
    Header(&'static str),
    Command(usize),
}

fn rows() -> Vec<Row> {
    let (mut rows, mut group) = (Vec::new(), "");
    for (i, c) in COMMANDS.iter().enumerate() {
        if c.1 != group {
            group = c.1;
            rows.push(Row::Header(group));
        }
        rows.push(Row::Command(i));
    }
    rows
}

pub(super) fn setup(tree: &mut Tree) -> Result<()> {
    tree.instantiate(FRAME)?;
    // Bindings for all characters: there is one set.
    tree.show("KeyBindingFrameCharacterButton", false);
    Ok(())
}

impl Panels {
    pub fn keys_open(&self) -> bool {
        self.tree.is_name_shown(FRAME)
    }

    /// The name a command is listed by: the original's where it has one for it.
    fn command_name(&self, i: usize) -> String {
        let c = &COMMANDS[i];
        if c.3.is_empty() {
            return c.2.to_string();
        }
        self.tree.string(&format!("BINDING_NAME_{}", c.3))
    }

    /// The menu's Key Bindings button.
    pub(super) fn open_keys(&mut self) {
        self.keys.binds.before = self.keys.binds.now;
        (self.keys.selected, self.keys.scroll) = (None, 0);
        self.tree.set_text("KeyBindingFrameOutputText", "");
        self.tree.show("GameMenuFrame", false);
        self.tree.show(FRAME, true);
        self.events.push(Sfx::Named("igMainMenuOption"));
        self.keys_refresh();
    }

    /// Leave the window for the menu, keeping what was set or going back to what it was.
    pub(super) fn keys_done(&mut self, keep: bool) {
        self.keys.binds.finish(keep, Binds::save);
        self.keys.selected = None;
        self.tree.show(FRAME, false);
    }

    /// The bindings, if they changed since the last call.
    pub fn take_keys(&mut self) -> Option<Binds> {
        self.keys.binds.take()
    }

    /// A key went down or up. True when the window heard it: while it is open the game hears nothing else (but Esc, which closes it unless a key
    /// button is waiting, when it stops the wait).
    pub fn keys_key(&mut self, code: KeyCode, pressed: bool) -> bool {
        if !self.keys_open() {
            return false;
        }
        let Some((cmd, slot)) = self.keys.selected else { return !(pressed && code == KeyCode::Escape) };
        if !pressed {
            return true;
        }
        self.keys.selected = None;
        let output = match code {
            KeyCode::Escape => String::new(),
            KeyCode::Delete => {
                self.keys.binds.now.unbind(cmd, slot);
                self.keys.binds.changed = true;
                String::new()
            }
            code if bindable(code) => {
                let lost = self.keys.binds.now.bind(cmd, slot, code);
                self.keys.binds.changed = true;
                self.events.push(Sfx::Named("igMainMenuOptionCheckBoxOn"));
                // A command left without a key is told (`KEY_UNBOUND_ERROR`).
                match lost.filter(|&l| self.keys.binds.now.0[l] == [None, None]) {
                    Some(l) => self.tree.string("KEY_UNBOUND_ERROR").replacen("%s", &self.command_name(l), 1),
                    None => self.tree.string("KEY_BOUND"),
                }
            }
            _ => return true,
        };
        self.tree.set_text("KeyBindingFrameOutputText", &output);
        self.keys_refresh();
        true
    }

    /// A click on a button of the window. False when it was none of them.
    pub(super) fn keys_click(&mut self, name: &str) -> bool {
        if !name.starts_with(FRAME) {
            return false;
        }
        if let Some((n, key)) = name.strip_prefix("KeyBindingFrameBinding").and_then(|r| r.split_once("Key")) {
            let row = n.parse::<usize>().ok().and_then(|n| rows().get(self.keys.scroll + n.checked_sub(1)?).copied());
            if let (Some(Row::Command(i)), Some(slot)) = (row, key.strip_suffix("Button").and_then(|k| k.parse::<usize>().ok())) {
                // The button of the selected one again lets go of it.
                self.keys.selected = Some((i, slot - 1)).filter(|s| Some(*s) != self.keys.selected);
                let output = self.keys.selected.map_or(String::new(), |_| self.tree.string("BIND_KEY_TO_COMMAND").replacen("%s", &self.command_name(i), 1));
                self.tree.set_text("KeyBindingFrameOutputText", &output);
                self.events.push(Sfx::Named("igMainMenuOptionCheckBoxOn"));
                self.keys_refresh();
            }
            return true;
        }
        match name {
            "KeyBindingFrameOkayButton" => self.options_done(true),
            "KeyBindingFrameCancelButton" => self.options_done(false),
            "KeyBindingFrameDefaultButton" => {
                self.keys.binds.set(Binds::default());
                self.keys.selected = None;
                self.tree.set_text("KeyBindingFrameOutputText", "");
                self.keys_refresh();
            }
            "KeyBindingFrameUnbindButton" => {
                if let Some((cmd, slot)) = self.keys.selected.take() {
                    self.keys.binds.now.unbind(cmd, slot);
                    self.keys.binds.changed = true;
                }
                self.tree.set_text("KeyBindingFrameOutputText", "");
                self.keys_refresh();
            }
            _ if name.starts_with(BAR) => self.keys_wheel(if name.ends_with("UpButton") { 1.0 } else { -1.0 }),
            _ => return false,
        }
        true
    }

    /// The wheel (or an arrow of the scroll bar): the list moves a line.
    pub(super) fn keys_wheel(&mut self, lines: f32) {
        let last = rows().len() - DISPLAYED;
        let k = &mut self.keys;
        k.scroll = if lines > 0.0 { k.scroll.saturating_sub(1) } else { (k.scroll + 1).min(last) };
        self.keys_refresh();
    }

    /// Show the list (`KeyBindingFrame_Update`).
    fn keys_refresh(&mut self) {
        let rows = rows();
        let not_bound = format!("{NORMAL_COLOR}{}|r", self.tree.string("NOT_BOUND"));
        for n in 1..=DISPLAYED {
            let line = format!("{FRAME}Binding{n}");
            let row = rows.get(self.keys.scroll + n - 1).copied();
            self.tree.show(&line, row.is_some());
            let header = matches!(row, Some(Row::Header(_)));
            self.tree.set_region_hidden(&format!("{line}Header"), !header);
            self.tree.set_region_hidden(&format!("{line}Description"), header);
            match row {
                Some(Row::Header(group)) => self.tree.set_text(&format!("{line}Header"), &self.tree.string(&format!("BINDING_HEADER_{group}"))),
                Some(Row::Command(i)) => {
                    self.tree.set_text(&format!("{line}Description"), &self.command_name(i));
                    for slot in 0..2 {
                        let button = format!("{line}Key{}Button", slot + 1);
                        let key = self.keys.binds.now.0[i][slot].and_then(key_name);
                        self.tree.set_text(&format!("{button}Text"), &key.map_or(not_bound.clone(), str::to_string));
                        self.tree.set_alpha(&button, if key.is_some() { 1.0 } else { 0.8 });
                        self.tree.lock_highlight(&button, self.keys.selected == Some((i, slot)));
                    }
                }
                None => {}
            }
            for slot in 1..=2 {
                self.tree.show(&format!("{line}Key{slot}Button"), matches!(row, Some(Row::Command(_))));
            }
        }
        self.tree.set_enabled("KeyBindingFrameUnbindButton", self.keys.selected.is_some());
        // The scroll bar: its knob is 16 units and travels the length of the bar less its own.
        self.tree.show(BAR, true);
        let screen = self.tree.screen();
        if let Some(track) = self.tree.rect_of(BAR, screen) {
            let at = self.keys.scroll as f32 / (rows.len() - DISPLAYED) as f32;
            self.tree.set_region_point(&format!("{BAR}ThumbTexture"), Point::TopLeft, Some(BAR), Some(Point::TopLeft), 0.0, -(track.h - 16.0).max(0.0) * at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_listed_once_under_a_header() {
        let rows = rows();
        assert!(matches!(rows[0], Row::Header(_)));
        assert_eq!(rows.iter().filter(|r| matches!(r, Row::Command(_))).count(), COMMANDS.len());
        assert!(rows.len() > DISPLAYED);
    }
}
