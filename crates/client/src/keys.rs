//! The keys of the game: what each command is bound to (two keys each, as in the original's key bindings), kept between runs in `keys.ron`.
//! Esc, F1 (editor), F2 (console), F3 (profiler), F5/F9 (quick save and load) and H (the test hit) are fixed and cannot be bound.

use std::collections::BTreeMap;
use winit::keyboard::KeyCode as K;

/// What a key can ask of the game.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmd {
    Forward,
    Back,
    Left,
    Right,
    Jump,
    Down,
    Run,
    Interact,
    Target,
    Nameplates,
    FriendNameplates,
    Sheath,
    View,
    Fly,
    Character,
    Backpack,
    Skills,
    Spellbook,
    Talents,
    Level,
    Map,
    QuestLog,
    /// A button of the action bar, 0 to 11.
    Action(usize),
}

pub const COUNT: usize = 34;

/// Every command with the header it is listed under (`BINDING_HEADER_*`), its name (also how it is saved), the original's name for it
/// (`BINDING_NAME_*`, empty when the original has none) and its keys.
pub const COMMANDS: [(Cmd, &str, &str, &str, [Option<K>; 2]); COUNT] = [
    (Cmd::Forward, "MOVEMENT", "Move forward", "MOVEFORWARD", [Some(K::KeyW), Some(K::ArrowUp)]),
    (Cmd::Back, "MOVEMENT", "Move backward", "MOVEBACKWARD", [Some(K::KeyS), Some(K::ArrowDown)]),
    (Cmd::Left, "MOVEMENT", "Move left", "STRAFELEFT", [Some(K::KeyA), Some(K::ArrowLeft)]),
    (Cmd::Right, "MOVEMENT", "Move right", "STRAFERIGHT", [Some(K::KeyD), Some(K::ArrowRight)]),
    (Cmd::Jump, "MOVEMENT", "Jump / fly up", "JUMP", [Some(K::Space), None]),
    (Cmd::Down, "MOVEMENT", "Sneak / fly down", "", [Some(K::ControlLeft), Some(K::ControlRight)]),
    (Cmd::Run, "MOVEMENT", "Speed up", "", [Some(K::ShiftLeft), Some(K::ShiftRight)]),
    (Cmd::Fly, "MOVEMENT", "Fly", "", [Some(K::KeyF), None]),
    (Cmd::Target, "TARGETING", "Next target", "TARGETNEARESTENEMY", [Some(K::Tab), None]),
    // The original has these on V and Ctrl-V; V is the view here and there are no chords.
    (Cmd::Nameplates, "TARGETING", "Toggle enemy nameplates", "NAMEPLATES", [Some(K::KeyJ), None]),
    (Cmd::FriendNameplates, "TARGETING", "Toggle friendly nameplates", "FRIENDNAMEPLATES", [Some(K::KeyG), None]),
    (Cmd::View, "CAMERA", "Change view", "", [Some(K::KeyV), None]),
    (Cmd::Interact, "MISC", "Interact / loot", "", [Some(K::KeyE), None]),
    (Cmd::Sheath, "MISC", "Sheathe weapon", "TOGGLESHEATH", [Some(K::KeyR), None]),
    (Cmd::Character, "INTERFACE", "Character", "TOGGLECHARACTER0", [Some(K::KeyC), None]),
    (Cmd::Skills, "INTERFACE", "Skills", "TOGGLECHARACTER1", [Some(K::KeyK), None]),
    (Cmd::Backpack, "INTERFACE", "Backpack", "TOGGLEBACKPACK", [Some(K::KeyB), None]),
    (Cmd::Spellbook, "INTERFACE", "Spellbook", "TOGGLESPELLBOOK", [Some(K::KeyP), None]),
    (Cmd::Talents, "INTERFACE", "Talents", "TOGGLETALENTS", [Some(K::KeyN), None]),
    (Cmd::QuestLog, "INTERFACE", "Quest log", "TOGGLEQUESTLOG", [Some(K::KeyL), None]),
    (Cmd::Map, "INTERFACE", "World map", "TOGGLEWORLDMAP", [Some(K::KeyM), None]),
    (Cmd::Level, "INTERFACE", "Level-up points", "", [Some(K::KeyU), None]),
    (Cmd::Action(0), "ACTIONBAR", "Action button 1", "ACTIONBUTTON1", [Some(K::Digit1), None]),
    (Cmd::Action(1), "ACTIONBAR", "Action button 2", "ACTIONBUTTON2", [Some(K::Digit2), None]),
    (Cmd::Action(2), "ACTIONBAR", "Action button 3", "ACTIONBUTTON3", [Some(K::Digit3), None]),
    (Cmd::Action(3), "ACTIONBAR", "Action button 4", "ACTIONBUTTON4", [Some(K::Digit4), None]),
    (Cmd::Action(4), "ACTIONBAR", "Action button 5", "ACTIONBUTTON5", [Some(K::Digit5), None]),
    (Cmd::Action(5), "ACTIONBAR", "Action button 6", "ACTIONBUTTON6", [Some(K::Digit6), None]),
    (Cmd::Action(6), "ACTIONBAR", "Action button 7", "ACTIONBUTTON7", [Some(K::Digit7), None]),
    (Cmd::Action(7), "ACTIONBAR", "Action button 8", "ACTIONBUTTON8", [Some(K::Digit8), None]),
    (Cmd::Action(8), "ACTIONBAR", "Action button 9", "ACTIONBUTTON9", [Some(K::Digit9), None]),
    (Cmd::Action(9), "ACTIONBAR", "Action button 10", "ACTIONBUTTON10", [Some(K::Digit0), None]),
    (Cmd::Action(10), "ACTIONBAR", "Action button 11", "ACTIONBUTTON11", [Some(K::Minus), None]),
    (Cmd::Action(11), "ACTIONBAR", "Action button 12", "ACTIONBUTTON12", [Some(K::Equal), None]),
];

/// Where a command is in `COMMANDS`.
pub fn index(cmd: Cmd) -> usize {
    COMMANDS.iter().position(|c| c.0 == cmd).unwrap_or(0)
}

macro_rules! names {
    ($($key:ident $name:literal),* $(,)?) => { &[$((K::$key, $name)),*] };
}

/// The keys that can be bound, and how they are written.
const KEYS: &[(K, &str)] = names![
    KeyA "A", KeyB "B", KeyC "C", KeyD "D", KeyE "E", KeyF "F", KeyG "G", KeyH "H", KeyI "I", KeyJ "J", KeyK "K", KeyL "L", KeyM "M",
    KeyN "N", KeyO "O", KeyP "P", KeyQ "Q", KeyR "R", KeyS "S", KeyT "T", KeyU "U", KeyV "V", KeyW "W", KeyX "X", KeyY "Y", KeyZ "Z",
    Digit0 "0", Digit1 "1", Digit2 "2", Digit3 "3", Digit4 "4", Digit5 "5", Digit6 "6", Digit7 "7", Digit8 "8", Digit9 "9",
    F4 "F4", F6 "F6", F7 "F7", F8 "F8", F10 "F10", F11 "F11", F12 "F12",
    ArrowUp "Up", ArrowDown "Down", ArrowLeft "Left", ArrowRight "Right",
    Space "Space", Tab "Tab", Enter "Enter", Insert "Insert", Home "Home", End "End", PageUp "PgUp", PageDown "PgDn",
    ShiftLeft "LShift", ShiftRight "RShift", ControlLeft "LCtrl", ControlRight "RCtrl", AltLeft "LAlt", AltRight "RAlt",
    Minus "-", Equal "=", BracketLeft "[", BracketRight "]", Semicolon ";", Quote "'", Comma ",", Period ".", Slash "/", Backslash "\\", Backquote "`",
    Numpad0 "Num0", Numpad1 "Num1", Numpad2 "Num2", Numpad3 "Num3", Numpad4 "Num4", Numpad5 "Num5", Numpad6 "Num6", Numpad7 "Num7", Numpad8 "Num8", Numpad9 "Num9",
    // The side buttons of a mouse; the window passes them on as these keys.
    BrowserBack "Mouse4", BrowserForward "Mouse5",
    NumpadAdd "Num+", NumpadSubtract "Num-", NumpadMultiply "Num*", NumpadDivide "Num/",
];

/// How a key is written, if it can be bound.
pub fn key_name(code: K) -> Option<&'static str> {
    KEYS.iter().find(|k| k.0 == code).map(|k| k.1)
}

/// Whether a key can be bound. (H is a test key of the game, and a bound H would be heard first.)
pub fn bindable(code: K) -> bool {
    key_name(code).is_some() && code != K::KeyH
}

/// The keys of every command: two slots each, in the order of `COMMANDS`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Binds(pub [[Option<K>; 2]; COUNT]);

impl Default for Binds {
    fn default() -> Self {
        Binds(COMMANDS.map(|c| c.4))
    }
}

impl Binds {
    /// The saved bindings; a command missing from the file keeps its default, a key that is no longer known leaves the slot empty.
    pub fn load() -> Binds {
        let saved: BTreeMap<String, [String; 2]> = crate::save::load_settings("keys.ron");
        let mut binds = Binds::default();
        for (slots, c) in binds.0.iter_mut().zip(&COMMANDS) {
            if let Some(names) = saved.get(c.2) {
                *slots = names.each_ref().map(|n| KEYS.iter().find(|k| k.1 == n).map(|k| k.0).filter(|&k| bindable(k)));
            }
        }
        binds
    }

    pub fn save(&self) {
        let saved: BTreeMap<_, _> = COMMANDS.iter().zip(&self.0).map(|(c, slots)| (c.2, slots.map(|k| k.and_then(key_name).unwrap_or("")))).collect();
        crate::save::write_settings("keys.ron", &saved);
    }

    /// The command a key is bound to.
    pub fn command(&self, code: K) -> Option<Cmd> {
        self.0.iter().position(|s| s.contains(&Some(code))).map(|i| COMMANDS[i].0)
    }

    /// Bind `code` to a slot of the command at `index`; the key is taken from whatever else had it. Returns the (index of the) command that lost it.
    pub fn bind(&mut self, index: usize, slot: usize, code: K) -> Option<usize> {
        let mut lost = None;
        for (i, slots) in self.0.iter_mut().enumerate() {
            for (s, key) in slots.iter_mut().enumerate() {
                if *key == Some(code) && (i, s) != (index, slot) {
                    *key = None;
                    lost = Some(i).filter(|_| i != index).or(lost);
                }
            }
        }
        self.0[index][slot] = Some(code);
        lost
    }

    /// Clear a slot; as in the original's list the second key of a command then stands in the first slot.
    pub fn unbind(&mut self, index: usize, slot: usize) {
        self.0[index][slot] = None;
        if slot == 0 {
            self.0[index].swap(0, 1);
        }
    }

    /// How the keys of the action bar are written on its buttons: the first key of each.
    pub fn action_labels(&self) -> [String; 12] {
        std::array::from_fn(|n| {
            self.0[index(Cmd::Action(n))].iter().flatten().find_map(|&k| key_name(k)).unwrap_or("").to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_all_bindable_and_none_is_used_twice() {
        let keys: Vec<K> = Binds::default().0.iter().flatten().flatten().copied().collect();
        assert!(keys.iter().all(|&k| bindable(k)));
        assert!(keys.iter().all(|k| keys.iter().filter(|o| o == &k).count() == 1));
        assert_eq!(Binds::default().command(K::KeyW), Some(Cmd::Forward));
        assert_eq!(Binds::default().command(K::Digit0), Some(Cmd::Action(9)));
    }

    #[test]
    fn a_key_goes_to_the_last_command_that_asked_for_it() {
        let mut binds = Binds::default();
        let (jump, interact) = (index(Cmd::Jump), index(Cmd::Interact));
        // E (Interact) onto the second slot of Jump.
        assert_eq!(binds.bind(jump, 1, K::KeyE), Some(interact));
        assert_eq!(binds.command(K::KeyE), Some(Cmd::Jump));
        assert_eq!(binds.0[interact], [None, None]);
        // Binding a key again to the other slot of the same command moves it there without naming a loser.
        assert_eq!(binds.bind(jump, 0, K::KeyE), None);
        assert_eq!(binds.0[jump], [Some(K::KeyE), None]);
    }

    #[test]
    fn action_buttons_are_labelled_with_their_keys() {
        let mut binds = Binds::default();
        assert_eq!(binds.action_labels()[9], "0");
        binds.bind(index(Cmd::Action(0)), 0, K::KeyQ);
        assert_eq!(binds.action_labels()[0], "Q");
    }
}
