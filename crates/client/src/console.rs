//! The developer console (F2): a line to type commands on, with the history (Up, Down) and completion (Tab), over the bottom of the
//! screen. It only reads the line and turns it into a `Command`; the game does what the command says (`State::run_command`).

use crate::content;
use crate::font::GlyphSet;
use crate::levels::MAX_LEVEL;
use crate::renderer::Gpu;
use crate::skills::{self, Skill};
use crate::ui::{Quad, Ui, FULL_UV, WHITE};
use anyhow::{Context, Result};

const UI_HEIGHT: f32 = 768.0;
const LINE: f32 = 16.0;
const SHOWN_LINES: usize = 12;
const SUGGESTIONS: usize = 8;
const TEXT: [f32; 4] = [0.95, 0.95, 0.95, 1.0];
const ECHO: [f32; 4] = [0.6, 0.8, 1.0, 1.0];
const OK: [f32; 4] = [0.5, 1.0, 0.5, 1.0];
const BAD: [f32; 4] = [1.0, 0.45, 0.45, 1.0];
const DIM: [f32; 4] = [0.65, 0.65, 0.65, 1.0];

/// (name, usage, what it does) of every command.
const COMMANDS: &[(&str, &str, &str)] = &[
    ("help", "help", "list the commands"),
    ("tgm", "tgm", "god mode: no damage, full health, mana and stamina"),
    ("tcl", "tcl", "toggle collision: fly through everything, or land again"),
    ("tfc", "tfc", "toggle the free camera: look around while the character stays"),
    ("level", "level <1-60>", "set the character's level"),
    ("xp", "xp <amount>", "gain experience"),
    ("gold", "gold <amount>", "add gold (a negative amount takes it)"),
    ("skill", "skill <name|all> <rank|max>", "set the rank of a skill"),
    ("item", "item <id or name> [count]", "put an item in the backpack"),
    ("spell", "spell <id|all>", "learn a spell"),
    ("recipes", "recipes <trade|all>", "learn every recipe of a trade"),
    ("tp", "tp <place> | tp <north> <west>", "teleport to a place or to coordinates"),
    ("heal", "heal", "fill health, mana and stamina"),
    ("respec", "respec", "take back every talent point (and the spells talents taught)"),
    ("time", "time <0-24|auto>", "set the hour of the day, or let it run"),
    ("pos", "pos", "say where the character stands"),
    ("prof", "prof", "show or hide the profiler (F3)"),
    ("logout", "logout", "save and go back to the list of characters"),
    ("clear", "clear", "empty the console"),
];

/// What a line asks the game to do, with the names already resolved.
#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    GodMode,
    Collision,
    FreeCamera,
    Level(u32),
    Xp(u32),
    Gold(i64),
    Skill(Option<Skill>, u32),
    Item(usize, u32),
    Spell(Option<usize>),
    Recipes(Option<Skill>),
    Teleport(f32, f32),
    Heal,
    Respec,
    Time(Option<f32>),
    Pos,
    Profile,
    Logout,
}

pub enum Key {
    Backspace,
    Enter,
    Tab,
    Up,
    Down,
}

pub struct Console {
    pub open: bool,
    font: GlyphSet,
    line: String,
    log: Vec<(String, [f32; 4])>,
    history: Vec<String>,
    /// The history entry the line shows, while it is browsed.
    recall: Option<usize>,
    /// While Tab is pressed over and over: the word that was being completed and the candidate it shows now.
    tab: Option<(String, usize)>,
}

/// The skill with the name `name` (as `Skill::id`, any case).
fn skill_named(name: &str) -> Option<Skill> {
    skills::ALL.iter().copied().find(|s| s.id().eq_ignore_ascii_case(name))
}

/// What the words after `words[0]` may be at argument `n` (counting from 0).
fn arguments(command: &str, n: usize) -> Vec<(String, String)> {
    let c = content::get();
    // An item is found by its name as well as by its id: what it completes to, and what it is searched by and shown as.
    if (command, n) == ("item", 0) {
        return c.items.iter().map(|i| (i.id.to_string(), format!("{}  {}", i.id, i.name))).collect();
    }
    let plain: Vec<String> = match (command, n) {
        ("spell", 0) => c.spells.iter().map(|s| s.id.clone()).chain(["all".into()]).collect(),
        ("skill", 0) => skills::ALL.iter().map(|s| s.id().to_lowercase()).chain(["all".into()]).collect(),
        ("skill", 1) => vec!["max".into()],
        ("recipes", 0) => skills::ALL.iter().filter(|s| s.is_profession()).map(|s| s.id().to_lowercase()).chain(["all".into()]).collect(),
        ("tp", 0) => c.pois.iter().map(|p| p.id.clone()).collect(),
        ("time", 0) => vec!["auto".into()],
        _ => Vec::new(),
    };
    plain.into_iter().map(|v| (v.clone(), v)).collect()
}

/// The ways to finish the last word of `line`, each with how it is shown: names that start with it, or else that contain it (an item's name counts).
fn labelled(line: &str) -> Vec<(String, String)> {
    let words: Vec<&str> = line.split_whitespace().collect();
    // After a space the word being typed is a new, empty one.
    let (done, word) = if line.is_empty() || line.ends_with(char::is_whitespace) { (&words[..], "") } else { (&words[..words.len() - 1], words[words.len() - 1]) };
    let pool: Vec<(String, String)> = match done.split_first() {
        None => COMMANDS.iter().map(|c| (c.0.to_string(), c.0.to_string())).collect(),
        Some((command, rest)) => arguments(command, rest.len()),
    };
    let word = word.to_lowercase();
    let mut found: Vec<(String, String)> = pool.iter().filter(|n| n.0.to_lowercase().starts_with(&word)).cloned().collect();
    if found.is_empty() {
        found = pool.into_iter().filter(|n| n.1.to_lowercase().contains(&word)).collect();
    }
    found
}

/// What the last word of `line` can be completed to.
fn candidates(line: &str) -> Vec<String> {
    labelled(line).into_iter().map(|c| c.0).collect()
}

/// A whole number, from a word.
fn number(word: Option<&&str>, what: &str) -> Result<i64, String> {
    word.ok_or(format!("missing {what}"))?.parse().map_err(|_| format!("{what} must be a whole number"))
}

/// Read a line. The error is what to tell the player.
pub fn parse(line: &str) -> Result<Command, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some((name, args)) = words.split_first() else { return Err("empty".into()) };
    let name = name.to_lowercase();
    let c = content::get();
    let usage = || COMMANDS.iter().find(|c| c.0 == name).map_or(String::new(), |c| format!("usage: {}", c.1));
    let command = match name.as_str() {
        "help" => Command::Help,
        "tgm" => Command::GodMode,
        "tcl" => Command::Collision,
        "tfc" => Command::FreeCamera,
        "heal" => Command::Heal,
        "respec" => Command::Respec,
        "pos" => Command::Pos,
        "prof" => Command::Profile,
        "logout" => Command::Logout,
        "level" => Command::Level(number(args.first(), "level")?.clamp(1, MAX_LEVEL as i64) as u32),
        "xp" => Command::Xp(number(args.first(), "amount")?.clamp(0, u32::MAX as i64) as u32),
        "gold" => Command::Gold(number(args.first(), "amount")?),
        "skill" => {
            // (A skill takes the most it can: 100, and 300 for a trade.)
            let rank = match args.get(1).map(|r| r.to_lowercase()).as_deref() {
                Some("max") => skills::PROFESSION_MAX as i64,
                _ => number(args.get(1), "rank")?,
            };
            let rank = rank.clamp(1, skills::PROFESSION_MAX as i64) as u32;
            match args.first().map(|s| s.to_lowercase()).as_deref() {
                Some("all") => Command::Skill(None, rank),
                Some(skill) => Command::Skill(Some(skill_named(skill).ok_or(format!("no skill {skill}"))?), rank),
                None => return Err(usage()),
            }
        }
        "item" => {
            // `item <id> [count]`, or by the name (as many words as it has): `item fishing pole`, `item shiny bauble 5`.
            let (words, count) = match args.split_last() {
                Some((last, rest)) if !rest.is_empty() && last.parse::<i64>().is_ok() => (rest, number(Some(last), "count")?.clamp(1, 1000) as u32),
                _ => (args, 1),
            };
            if words.is_empty() {
                return Err(usage());
            }
            let query = words.join(" ").to_lowercase();
            let exact = c.items.iter().position(|i| i.id.eq_ignore_ascii_case(&query)).or_else(|| c.items.iter().position(|i| i.name.eq_ignore_ascii_case(&query)));
            let item = match exact {
                Some(i) => i,
                None => {
                    let like: Vec<usize> = (0..c.items.len()).filter(|&i| c.items[i].name.to_lowercase().contains(&query)).collect();
                    match like.as_slice() {
                        [one] => *one,
                        [] => return Err(format!("no item {query}")),
                        many => return Err(format!("{} items have that in the name, such as {}: say more of it, or use the id", many.len(), many.iter().take(3).map(|&i| format!("{} ({})", c.items[i].name, c.items[i].id)).collect::<Vec<_>>().join(", "))),
                    }
                }
            };
            Command::Item(item, count)
        }
        "recipes" => match args.first().map(|s| s.to_lowercase()).as_deref() {
            Some("all") => Command::Recipes(None),
            Some(skill) => Command::Recipes(Some(skill_named(skill).filter(|s| s.is_profession()).ok_or(format!("no trade {skill}"))?)),
            None => return Err(usage()),
        },
        "spell" => match args.first().map(|s| s.to_lowercase()).as_deref() {
            Some("all") => Command::Spell(None),
            Some(id) => Command::Spell(Some(c.spells.iter().position(|s| s.id.eq_ignore_ascii_case(id)).ok_or(format!("no spell {id}"))?)),
            None => return Err(usage()),
        },
        "tp" => match args {
            [place] => {
                let poi = c.pois.iter().find(|p| p.id.eq_ignore_ascii_case(place)).ok_or(format!("no place {place}"))?;
                Command::Teleport(poi.at.0, poi.at.1)
            }
            [north, west] => Command::Teleport(north.parse().map_err(|_| "north must be a number")?, west.parse().map_err(|_| "west must be a number")?),
            _ => return Err(usage()),
        },
        "time" => match args.first() {
            Some(a) if a.eq_ignore_ascii_case("auto") => Command::Time(None),
            Some(hour) => Command::Time(Some(hour.parse::<f32>().map_err(|_| "the hour must be a number")?.rem_euclid(24.0))),
            None => return Err(usage()),
        },
        _ => return Err(format!("no command {name} (try help)")),
    };
    Ok(command)
}

impl Console {
    pub fn new(ui: &mut Ui, gpu: &Gpu, font: &fontdue::Font) -> Result<Console> {
        let mut console = Console { open: false, font: GlyphSet::new(ui, gpu, font, 12.0, 1).context("console font")?, line: String::new(), log: Vec::new(), history: Vec::new(), recall: None, tab: None };
        console.say("Developer console. Tab completes, Up and Down recall, F2 closes. Type help.", DIM);
        Ok(console)
    }

    pub fn say(&mut self, text: &str, color: [f32; 4]) {
        self.log.extend(text.lines().map(|l| (l.to_string(), color)));
    }

    pub fn reply(&mut self, result: Result<String, String>) {
        match result {
            Ok(text) => self.say(&text, OK),
            Err(text) => self.say(&text, BAD),
        }
    }

    /// The profiler's last second over the top right corner: each scope with its average and worst milliseconds per frame.
    pub fn draw_profile(&self, ui: &mut Ui, size: (u32, u32), fps: f32, rows: &[crate::profile::Row]) {
        let s = size.1 as f32 / UI_HEIGHT;
        let (name_w, col_w) = (200.0, 44.0);
        let width = (name_w + 2.0 * col_w + 16.0) * s;
        let height = (rows.len() + 1) as f32 * LINE * s + 12.0 * s;
        let left = size.0 as f32 - width - 6.0 * s;
        ui.quad(Quad { texture: WHITE, rect: [left, 6.0 * s, width, height], uv: FULL_UV, color: [0.0, 0.0, 0.0, 0.75], additive: false });
        let right = |text: &str, edge: f32| edge - self.font.width(text) * s;
        let mut y = 10.0 * s;
        self.font.draw(ui, &format!("{fps:.0} fps (CPU ms per frame)"), left + 8.0 * s, y, s, OK);
        self.font.draw(ui, "avg", right("avg", left + (8.0 + name_w + col_w) * s), y, s, DIM);
        self.font.draw(ui, "worst", right("worst", left + (8.0 + name_w + 2.0 * col_w) * s), y, s, DIM);
        for r in rows {
            y += LINE * s;
            let color = if r.avg >= 4.0 { BAD } else if r.avg >= 1.0 { [1.0, 0.9, 0.4, 1.0] } else { TEXT };
            self.font.draw(ui, r.name, left + (8.0 + 8.0 * r.depth as f32) * s, y, s, color);
            let (avg, worst) = (format!("{:.2}", r.avg), format!("{:.2}", r.worst));
            self.font.draw(ui, &avg, right(&avg, left + (8.0 + name_w + col_w) * s), y, s, color);
            self.font.draw(ui, &worst, right(&worst, left + (8.0 + name_w + 2.0 * col_w) * s), y, s, DIM);
        }
    }

    pub fn help() -> String {
        COMMANDS.iter().map(|(_, usage, what)| format!("{usage} - {what}")).collect::<Vec<_>>().join("\n")
    }

    pub fn clear(&mut self) {
        self.log.clear();
    }

    pub fn typed(&mut self, text: &str) {
        self.line.extend(text.chars().filter(|c| !c.is_control()));
        self.tab = None;
    }

    /// Enter gives back the line to run (it is echoed and put in the history).
    pub fn key(&mut self, key: Key) -> Option<String> {
        match key {
            Key::Backspace => {
                self.line.pop();
                self.tab = None;
            }
            Key::Enter => {
                let line = std::mem::take(&mut self.line);
                (self.recall, self.tab) = (None, None);
                if line.trim().is_empty() {
                    return None;
                }
                self.say(&format!("> {line}"), ECHO);
                if self.history.last() != Some(&line) {
                    self.history.push(line.clone());
                }
                return Some(line);
            }
            Key::Tab => self.complete(),
            Key::Up | Key::Down => {
                let last = self.history.len();
                let next = match (self.recall, matches!(key, Key::Up)) {
                    (None, true) => last.checked_sub(1),
                    (Some(i), true) => Some(i.saturating_sub(1)),
                    (Some(i), false) if i + 1 < last => Some(i + 1),
                    _ => None,
                };
                self.recall = next;
                self.line = next.map_or(String::new(), |i| self.history[i].clone());
                self.tab = None;
            }
        }
        None
    }

    /// Tab: the last word becomes the first way to finish it, and each press after that the next one.
    fn complete(&mut self) {
        let (base, n) = match self.tab.take() {
            Some((base, n)) => (base, n + 1),
            None => (self.line.rsplit(char::is_whitespace).next().unwrap_or("").to_string(), 0),
        };
        let head = &self.line[..self.line.len() - self.line.rsplit(char::is_whitespace).next().unwrap_or("").len()];
        // The head of the line stays as it was; the candidates are those of the line as it was before the first Tab.
        let found = candidates(&format!("{head}{base}"));
        if found.is_empty() {
            return;
        }
        let pick = &found[n % found.len()];
        self.line = format!("{head}{pick}");
        if found.len() == 1 {
            self.line.push(' ');
        } else {
            self.tab = Some((base, n % found.len()));
        }
    }

    pub fn draw(&self, ui: &mut Ui, size: (u32, u32)) {
        if !self.open {
            return;
        }
        let s = size.1 as f32 / UI_HEIGHT;
        let width = size.0 as f32;
        let shown = self.log.len().min(SHOWN_LINES);
        let all = labelled(&self.line);
        let mut hints: Vec<String> = all.iter().take(SUGGESTIONS).map(|h| h.1.clone()).collect();
        if all.len() > SUGGESTIONS {
            hints.push(format!("... and {} more: type more of the name", all.len() - SUGGESTIONS));
        }
        let hints = &hints[..];
        let height = (shown + 1) as f32 * LINE + 10.0;
        let top = 0.0;
        let fill = |ui: &mut Ui, rect: [f32; 4], color: [f32; 4]| ui.quad(Quad { texture: WHITE, rect, uv: FULL_UV, color, additive: false });
        fill(ui, [0.0, top, width, height * s], [0.0, 0.0, 0.0, 0.8]);
        for (i, (text, color)) in self.log[self.log.len() - shown..].iter().enumerate() {
            self.font.draw(ui, text, 8.0 * s, (top + (5.0 + i as f32 * LINE) * s).max(0.0), s, *color);
        }
        let y = (shown as f32 * LINE + 5.0) * s;
        // A blinking bar after the text.
        let prompt = format!("> {}", self.line);
        self.font.draw(ui, &prompt, 8.0 * s, y, s, TEXT);
        let caret = 8.0 * s + self.font.width(&prompt) * s;
        if (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() / 500)) % 2 == 0 {
            fill(ui, [caret + s, y, 6.0 * s, 12.0 * s], TEXT);
        }
        // The ways to finish the word, under the line.
        if !hints.is_empty() && !self.line.is_empty() {
            let w = hints.iter().map(|h| self.font.width(h)).fold(0.0, f32::max) + 12.0;
            let x = (caret).min(width - w * s);
            let box_top = height * s;
            fill(ui, [x, box_top, w * s, (hints.len() as f32 * LINE + 6.0) * s], [0.05, 0.05, 0.1, 0.9]);
            let current = self.tab.as_ref().map(|(_, n)| *n);
            for (i, hint) in hints.iter().enumerate() {
                let color = if current == Some(i) { OK } else { DIM };
                self.font.draw(ui, hint, x + 6.0 * s, box_top + (3.0 + i as f32 * LINE) * s, s, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_complete_by_prefix() {
        assert_eq!(candidates("tg"), vec!["tgm"]);
        assert_eq!(candidates("").len(), COMMANDS.len());
        assert!(candidates("s").contains(&"skill".to_string()) && candidates("s").contains(&"spell".to_string()));
    }

    #[test]
    fn arguments_complete_from_the_content() {
        let first = content::get().items[0].id.to_string();
        assert!(candidates("item ").contains(&first));
        assert!(candidates(&format!("item {}", &first[..2])).contains(&first));
        assert_eq!(candidates("skill sw")[0], "swords");
        assert_eq!(candidates("skill swords m"), vec!["max"]);
        assert!(candidates("tgm ").is_empty());
    }

    #[test]
    fn an_item_is_found_by_its_name_too() {
        let c = content::get();
        let pole = c.items.iter().position(|i| i.id == "i6256").unwrap();
        assert_eq!(parse("item i6256"), Ok(Command::Item(pole, 1)));
        assert_eq!(parse("item Fishing Pole"), Ok(Command::Item(pole, 1)));
        assert_eq!(parse("item fishing pole 3"), Ok(Command::Item(pole, 3)));
        assert!(parse("item shiny bauble 20").is_ok());
        assert!(parse("item zzz no such").is_err() && parse("item sword").unwrap_err().contains("items have that in the name"));
        // The words of a name find it among the ways to finish the word, shown with the name.
        assert!(labelled("item pole").iter().any(|l| l.0 == "i6256" && l.1.contains("Fishing Pole")));
    }

    #[test]
    fn lines_are_read() {
        assert_eq!(parse("tgm"), Ok(Command::GodMode));
        assert_eq!((parse("tcl"), parse("TFC")), (Ok(Command::Collision), Ok(Command::FreeCamera)));
        assert_eq!(parse("LEVEL 99"), Ok(Command::Level(MAX_LEVEL)));
        assert_eq!(parse("gold -5"), Ok(Command::Gold(-5)));
        assert_eq!(parse("skill swords max"), Ok(Command::Skill(Some(Skill::Swords), skills::PROFESSION_MAX)));
        assert_eq!(parse("skill all 10"), Ok(Command::Skill(None, 10)));
        assert_eq!(parse("tp 1.5 -2"), Ok(Command::Teleport(1.5, -2.0)));
        assert_eq!(parse("time 25"), Ok(Command::Time(Some(1.0))));
        let item = &content::get().items[0].id;
        assert_eq!(parse(&format!("item {item} 3")), Ok(Command::Item(0, 3)));
        assert!(parse("level").is_err() && parse("item nothing_like_it").is_err() && parse("frobnicate").is_err());
    }
}
