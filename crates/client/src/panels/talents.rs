//! The talent window (key N): the original's `TalentFrame` (the `Blizzard_TalentUI` addon) filled with the three trees of `talents.rs`. The Lua of that addon
//! (`TalentFrame_Update`, `TalentFrame_DrawLines`) is ported here: the buttons stand on a grid of tiers and columns, the lines and arrows between a talent and
//! what it requires are pieces of two pictures, and a button is green while it can take a rank, gold when full and grey when it cannot be reached.

use super::*;
use super::magic::select_tab;
use crate::talents::{self, Lock, COLUMNS};
use super::trade;

pub const TALENTS: &str = "TalentFrame";
const TREES: usize = 3;
const BUTTONS: usize = 20;
const BRANCHES: usize = 30;
const ARROWS: usize = 30;
const SCROLL: &str = "TalentFrameScrollFrame";
const CHILD: &str = "TalentFrameScrollChildFrame";
const ARROW_FRAME: &str = "TalentFrameArrowFrame";
/// `TALENT_BUTTON_SIZE`, the step from one button to the next, and where the first one stands.
const SIZE: f32 = 32.0;
const STEP: f32 = 63.0;
const START: (f32, f32) = (35.0, 20.0);
const GREEN: [f32; 4] = [0.1, 1.0, 0.1, 1.0];
const MUTED: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

/// The pieces of `UI-TalentBranches`; the sign says whether the line is lit (1) or grey (-1).
#[derive(Clone, Copy)]
enum Branch {
    Up,
    Down,
    Left,
    Right,
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
    TDown,
    TUp,
}

impl Branch {
    /// Texture coordinates (left, right, top, bottom) of the piece; the lit ones are in the top half of the picture, the grey ones in the bottom half.
    fn coords(self, lit: i8) -> [f32; 4] {
        let (l, r) = match self {
            Branch::Up => (0.12890625, 0.25390625),
            Branch::Down => (0.0, 0.125),
            Branch::Left | Branch::Right => (0.2578125, 0.3828125),
            Branch::TopRight => (0.515625, 0.640625),
            Branch::TopLeft => (0.640625, 0.515625),
            Branch::BottomRight => (0.38671875, 0.51171875),
            Branch::BottomLeft => (0.51171875, 0.38671875),
            Branch::TDown => (0.64453125, 0.76953125),
            Branch::TUp => (0.7734375, 0.8984375),
        };
        match (matches!(self, Branch::Up | Branch::Down), lit > 0) {
            (true, true) => [l, r, 0.0, 0.484375],
            (true, false) => [l, r, 0.515625, 1.0],
            (false, true) => [l, r, 0.0, 0.5],
            (false, false) => [l, r, 0.5, 1.0],
        }
    }
}

/// The pieces of `UI-TalentArrows`.
#[derive(Clone, Copy)]
enum Arrow {
    Top,
    Right,
    Left,
}

impl Arrow {
    fn coords(self, lit: i8) -> [f32; 4] {
        let (l, r) = match self {
            Arrow::Top => (0.0, 0.5),
            Arrow::Right => (1.0, 0.5),
            Arrow::Left => (0.5, 1.0),
        };
        if lit > 0 { [l, r, 0.0, 0.5] } else { [l, r, 0.5, 1.0] }
    }
}

/// What lines and arrows touch a place of the grid (`TALENT_BRANCH_ARRAY`): 1 lit, -1 grey, 0 none.
#[derive(Clone, Copy, Default)]
struct Node {
    /// The button that stands here.
    id: Option<usize>,
    up: i8,
    down: i8,
    left: i8,
    right: i8,
    left_arrow: i8,
    right_arrow: i8,
    top_arrow: i8,
}

/// Rows and columns count from 1 as in the original (row 0 and column 0 are not used).
type Grid = [[Node; COLUMNS + 2]; talents::TIERS + 2];

/// `TalentFrame_DrawLines`: the lines from a talent at (`bt`, `bc`) to the one it requires at (`t`, `c`), lit when the requirements are met. A layout that
/// cannot be drawn (something stands in the way) draws nothing.
fn draw_lines(g: &mut Grid, (bt, bc): (usize, usize), (t, c): (usize, usize), met: bool) {
    let m: i8 = if met { 1 } else { -1 };
    // The same column: straight down, with an arrow into the talent.
    if bc == c {
        if (t + 1..bt).any(|i| g[i][bc].id.is_some()) {
            return;
        }
        for i in t..bt {
            g[i][bc].down = m;
            if i + 1 < bt {
                g[i + 1][bc].up = m;
            }
        }
        g[bt][bc].top_arrow = m;
        return;
    }
    // The same row: across.
    if bt == t {
        let (left, right) = (bc.min(c), bc.max(c));
        if (left + 1..right).any(|i| g[t][i].id.is_some()) {
            return;
        }
        for i in left..right {
            g[t][i].right = m;
            g[t][i + 1].left = m;
        }
        if bc < c { g[bt][bc].right_arrow = m } else { g[bt][bc].left_arrow = m }
        return;
    }
    // Diagonal: down first and then across, or, when something stands in the way, across first and then down.
    let (left, right) = (bc.min(c), bc.max(c));
    let (a, b) = if left == c { (left + 1, right) } else { (left, right - 1) };
    if !(a..=b).any(|i| g[t][i].id.is_some()) {
        g[t][bc].down = m;
        g[bt][bc].up = m;
        for i in t..bt {
            g[i][bc].down = m;
            g[i + 1][bc].up = m;
        }
        for i in left..right {
            g[t][i].right = m;
            g[t][i + 1].left = m;
        }
        g[bt][bc].top_arrow = m;
        return;
    }
    let (a, b) = if left == bc { (left + 1, right) } else { (left, right - 1) };
    if (a..=b).any(|i| g[bt][i].id.is_some()) {
        return;
    }
    for i in t..bt {
        g[i][c].up = m;
        g[i + 1][c].down = m;
    }
    if bc < c { g[bt][bc].right_arrow = m } else { g[bt][bc].left_arrow = m }
}

/// The pieces to put on the grid, in the order of `TalentFrame_Update`: (piece, lit, x, y) from the top left of the scroll child.
fn pieces(g: &Grid) -> (Vec<(Branch, i8, f32, f32)>, Vec<(Arrow, i8, f32, f32)>) {
    let (mut branches, mut arrows) = (Vec::new(), Vec::new());
    let mut ignore_up = false;
    for i in 1..=talents::TIERS + 1 {
        for j in 1..=COLUMNS {
            let node = g[i][j];
            let x = (j - 1) as f32 * STEP + START.0 + 2.0;
            let y = -((i - 1) as f32 * STEP) - START.1 - 2.0;
            if node.id.is_some() {
                if node.up != 0 {
                    if !ignore_up {
                        branches.push((Branch::Up, node.up, x, y + SIZE));
                    } else {
                        ignore_up = false;
                    }
                }
                if node.down != 0 {
                    branches.push((Branch::Down, node.down, x, y - SIZE + 1.0));
                }
                if node.left != 0 {
                    branches.push((Branch::Left, node.left, x - SIZE, y));
                }
                if node.right != 0 {
                    // A grey line coming down into the next place makes this piece grey too.
                    let next = g[i][j + 1];
                    if next.left != 0 && next.down < 0 {
                        branches.push((Branch::Right, next.down, x + SIZE, y));
                    } else {
                        branches.push((Branch::Right, node.right, x + SIZE + 1.0, y));
                    }
                }
                if node.right_arrow != 0 {
                    arrows.push((Arrow::Right, node.right_arrow, x + SIZE / 2.0 + 5.0, y));
                }
                if node.left_arrow != 0 {
                    arrows.push((Arrow::Left, node.left_arrow, x - SIZE / 2.0 - 5.0, y));
                }
                if node.top_arrow != 0 {
                    arrows.push((Arrow::Top, node.top_arrow, x, y + SIZE / 2.0 + 5.0));
                }
            } else if node.up != 0 && node.left != 0 && node.right != 0 {
                branches.push((Branch::TUp, node.up, x, y));
            } else if node.down != 0 && node.left != 0 && node.right != 0 {
                branches.push((Branch::TDown, node.down, x, y));
            } else if node.left != 0 && node.down != 0 {
                branches.push((Branch::TopRight, node.left, x, y));
                branches.push((Branch::Down, node.down, x, y - 32.0));
            } else if node.left != 0 && node.up != 0 {
                branches.push((Branch::BottomRight, node.left, x, y));
            } else if node.left != 0 && node.right != 0 {
                branches.push((Branch::Right, node.right, x + SIZE, y));
                branches.push((Branch::Left, node.left, x + 1.0, y));
            } else if node.right != 0 && node.down != 0 {
                branches.push((Branch::TopLeft, node.right, x, y));
                branches.push((Branch::Down, node.down, x, y - 32.0));
            } else if node.right != 0 && node.up != 0 {
                branches.push((Branch::BottomLeft, node.right, x, y));
            } else if node.up != 0 && node.down != 0 {
                branches.push((Branch::Up, node.up, x, y));
                branches.push((Branch::Down, node.down, x, y - 32.0));
                ignore_up = true;
            }
        }
    }
    (branches, arrows)
}

/// Load the window and take away what this game has no use for.
pub fn setup(tree: &mut Tree) -> Result<()> {
    tree.instantiate(TALENTS)?;
    tree.show(TALENTS, false);
    // Three trees, not five.
    for tab in TREES + 1..=5 {
        tree.show(&format!("TalentFrameTab{tab}"), false);
    }
    Ok(())
}

impl Panels {
    pub fn talents_open(&self) -> bool {
        self.tree.is_name_shown(TALENTS)
    }

    /// The talents learned (for the save).
    pub fn talent_ranks(&self) -> Vec<(String, u8)> {
        self.talents.saved()
    }

    /// `respec`: every rank is given back, with the spells they taught.
    pub fn respec(&mut self) {
        for spell in talents::trees().iter().flat_map(|t| &t.talents).filter_map(|t| t.spell()) {
            self.forget(spell);
        }
        self.talents.clear();
        self.stats_dirty = true;
        self.refresh();
        self.fill_talents();
    }

    /// For the headless picture: a character of level 30 with points spent in all three trees (`--panel talents`).
    pub fn talents_preview(&mut self) {
        self.set_level(30);
        for (id, ranks) in [("brute_force", 5), ("toughness", 5), ("vitality", 3), ("heavy_blows", 3), ("weapon_mastery", 1), ("arcane_mind", 2), ("keen_eye", 1)] {
            let (tree, t) = talents::trees().iter().enumerate().find_map(|(i, tr)| tr.talents.iter().find(|t| t.id == id).map(|t| (i, t))).expect("talent");
            for _ in 0..ranks {
                self.talents.learn(tree, t, 30);
            }
        }
        self.refresh();
    }

    /// `ToggleTalentFrame`: a left window, like the character sheet.
    pub fn toggle_talents(&mut self) {
        let on = !self.talents_open();
        if on {
            self.tree.show(CHARACTER, false);
            self.tree.show(SPELLBOOK, false);
            self.tree.show(trade::TRADE, false);
            self.close_loot();
            self.close_gossip();
            self.close_merchant();
            self.close_quest();
            self.hide_log();
        }
        self.tree.show(TALENTS, on);
        self.events.push(Sfx::Named(if on { "TalentScreenOpen" } else { "TalentScreenClose" }));
        self.fill_talents();
        self.after_close();
    }

    /// `TalentFrame_Update`: tabs, points, the background of the tree, the buttons and the lines between them.
    pub(super) fn fill_talents(&mut self) {
        if !self.talents_open() {
            return;
        }
        let (trees, free) = (talents::trees(), self.talents.free(self.progress.level));
        let selected = self.talent_tab.min(trees.len() - 1);
        let current = &trees[selected];
        let spent = self.talents.spent(selected);
        for (i, tree) in trees.iter().enumerate() {
            let tab = format!("TalentFrameTab{}", i + 1);
            self.tree.set_text(&format!("{tab}Text"), &tree.name);
            select_tab(&mut self.tree, &tab, i == selected);
        }
        let spent_line = format!("{} |cffffffff{spent}|r", self.string("MASTERY_POINTS_SPENT").replacen("%s", &current.name, 1));
        self.tree.set_text("TalentFrameSpentPoints", &spent_line);
        self.tree.set_text("TalentFrameTalentPointsText", &free.to_string());
        let base = format!(r"Interface\TalentFrame\{}-", current.art);
        for part in ["TopLeft", "TopRight", "BottomLeft", "BottomRight"] {
            self.tree.set_texture(&format!("TalentFrameBackground{part}"), Some(&format!("{base}{part}")));
        }

        // The grid first, so that a line can tell whether something stands in its way.
        let mut grid = Grid::default();
        for (i, t) in current.talents.iter().enumerate().take(BUTTONS) {
            grid[t.tier][t.column].id = Some(i + 1);
        }
        for n in 1..=BUTTONS {
            let button = format!("TalentFrameTalent{n}");
            let Some(t) = current.talents.get(n - 1) else {
                self.tree.show(&button, false);
                continue;
            };
            let (rank, lock) = (self.talents.rank(t), self.talents.lock(selected, t));
            // With no point to spend only the talents with points in them stand out.
            let dim = free == 0 && rank == 0;
            let tier_open = !matches!(lock, Some(Lock::Tier(_)));
            let prereq = t.requires.as_deref().and_then(|id| current.talents.iter().find(|o| o.id == id));
            let prereq_full = prereq.is_none_or(|p| self.talents.rank(p) >= p.ranks);
            let lines_met = tier_open && !dim && prereq_full;
            if let Some(p) = prereq {
                draw_lines(&mut grid, (t.tier, t.column), (p.tier, p.column), lines_met);
            }
            let reachable = lines_met && !matches!(lock, Some(Lock::Hybrid(..)));
            self.tree.set_point(&button, Point::TopLeft, Some(CHILD), Some(Point::TopLeft), (t.column - 1) as f32 * STEP + START.0, -((t.tier - 1) as f32 * STEP) - START.1);
            self.tree.set_texture(&format!("{button}IconTexture"), Some(&format!(r"Interface\Icons\{}", t.icon)));
            self.tree.set_text(&format!("{button}Rank"), &rank.to_string());
            let (icon, slot, number) = if reachable {
                // Green while a rank can still be taken, gold when it is full.
                let full = rank >= t.ranks;
                ([1.0; 4], if full { GOLD } else { GREEN }, if full { GOLD } else { GREEN })
            } else {
                ([0.65, 0.65, 0.65, 1.0], MUTED, MUTED)
            };
            self.tree.set_region_tint(&format!("{button}IconTexture"), icon);
            self.tree.set_region_tint(&format!("{button}Slot"), slot);
            self.tree.set_region_tint(&format!("{button}RankBorder"), if reachable { WHITE } else { MUTED });
            self.tree.set_text_color(&format!("{button}Rank"), number);
            let numbered = reachable || rank > 0;
            self.tree.set_region_hidden(&format!("{button}RankBorder"), !numbered);
            self.tree.set_region_hidden(&format!("{button}Rank"), !numbered);
            self.tree.show(&button, true);
        }

        let (branches, arrows) = pieces(&grid);
        for n in 1..=BRANCHES {
            let name = format!("TalentFrameBranch{n}");
            let piece = branches.get(n - 1);
            self.tree.set_region_hidden(&name, piece.is_none());
            if let Some(&(branch, lit, x, y)) = piece {
                self.tree.set_region_coords(&name, branch.coords(lit));
                self.tree.set_region_point(&name, Point::TopLeft, Some(CHILD), Some(Point::TopLeft), x, y);
            }
        }
        for n in 1..=ARROWS {
            let name = format!("TalentFrameArrow{n}");
            let piece = arrows.get(n - 1);
            self.tree.set_region_hidden(&name, piece.is_none());
            if let Some(&(arrow, lit, x, y)) = piece {
                self.tree.set_region_coords(&name, arrow.coords(lit));
                self.tree.set_region_point(&name, Point::TopLeft, Some(ARROW_FRAME), Some(Point::TopLeft), x, y);
            }
        }
    }

    /// The talent that button `n` of the window stands for, with the tree it is in.
    fn talent_of(&self, name: &str) -> Option<(usize, &'static talents::Talent)> {
        let n = name.strip_prefix("TalentFrameTalent")?.parse::<usize>().ok()?;
        let tree = self.talent_tab.min(talents::trees().len() - 1);
        Some((tree, talents::trees()[tree].talents.get(n.checked_sub(1)?)?))
    }

    /// The tooltip of a talent button: the name, the rank, what stands in the way, what the rank does and what the next one does.
    pub(super) fn talent_lines(&self, name: &str) -> Option<Vec<TipLine>> {
        let (tree, t) = self.talent_of(name)?;
        let rank = self.talents.rank(t);
        let mut lines = vec![TipLine::new(t.name.clone(), WHITE), TipLine::new(format!("Rank {rank}/{}", t.ranks), WHITE)];
        match self.talents.lock(tree, t) {
            Some(Lock::Tier(points)) => lines.push(TipLine::new(format!("Requires {points} points in {} Talents", talents::trees()[tree].name), RED)),
            Some(Lock::Requires(other)) => lines.push(TipLine::new(format!("Requires {other} at full rank"), RED)),
            Some(Lock::Hybrid(other, points)) => lines.push(TipLine::new(format!("Requires {points} points in {other} Talents"), RED)),
            None => {}
        }
        if !matches!(t.perk, talents::Perk::Spell) {
            lines.push(TipLine::new(t.describe(rank.max(1)), GOLD));
        }
        // What it teaches, as the spellbook shows it.
        if let Some(spell) = t.spell() {
            lines.push(TipLine::new("Teaches:", WHITE));
            lines.extend(self.spell_lines(spell).into_iter().skip(1));
        }
        if rank > 0 && rank < t.ranks {
            lines.push(TipLine::new("Next rank:", WHITE));
            lines.push(TipLine::new(t.describe(rank + 1), GOLD));
        }
        if rank < t.ranks && self.talents.free(self.progress.level) > 0 && self.talents.lock(tree, t).is_none() {
            lines.push(TipLine::new("Click to learn", GREEN));
        }
        if rank > 0 {
            lines.push(TipLine::new("Right-click to give a rank back", GREY));
        }
        Some(lines)
    }

    /// A click on the window. False when it was none of its buttons.
    pub(super) fn talent_click(&mut self, name: &str, left: bool) -> bool {
        match name {
            "TalentFrameCloseButton" | "TalentFrameCancelButton" => self.toggle_talents(),
            _ => {
                if let Some(tab) = name.strip_prefix("TalentFrameTab").and_then(|n| n.parse::<usize>().ok()) {
                    self.talent_tab = tab - 1;
                    self.events.push(Sfx::Named("igCharacterInfoTab"));
                } else if let Some((tree, t)) = self.talent_of(name) {
                    let level = self.progress.level;
                    let changed = if left { self.talents.learn(tree, t, level) } else { self.talents.unlearn(t) };
                    if changed {
                        // The first rank of a talent that teaches a spell teaches it; giving that rank back takes it away.
                        if let Some(spell) = t.spell() {
                            match (left, self.talents.rank(t)) {
                                (true, 1) if self.learn(spell) => self.notice(&format!("You have learned a new spell: {}.", content::spell(spell).name)),
                                (false, 0) => self.forget(spell),
                                _ => {}
                            }
                        }
                        self.stats_dirty = true;
                        self.refresh();
                    }
                } else {
                    return false;
                }
            }
        }
        self.fill_talents();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_runs_down_a_column_and_across_a_row() {
        // A talent at tier 3 that requires the one at tier 1 of its column: a line through tier 2 and an arrow into it.
        let mut g = Grid::default();
        draw_lines(&mut g, (3, 2), (1, 2), true);
        assert_eq!((g[1][2].down, g[2][2].up, g[2][2].down, g[3][2].top_arrow), (1, 1, 1, 1));
        // Across: the arrow points at the talent from the side of the one it requires.
        let mut g = Grid::default();
        draw_lines(&mut g, (2, 1), (2, 3), false);
        assert_eq!((g[2][1].right, g[2][2].left, g[2][2].right, g[2][3].left, g[2][1].right_arrow), (-1, -1, -1, -1, -1));
    }
}
