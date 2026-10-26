//! Skills, trained by use like Skyrim's: every weapon family, fists, blocking, sneaking, lockpicking and each of the nine classes
//! (whose spells it is that train them) and each kind of armor has a rank from 1 to `MAX`. Doing the thing a little trains the skill (a hit with a sword,
//! a blow on the shield, a lock opened, a spell cast); a higher rank makes it better (see the `*_factor` functions) and, for a
//! class, opens the spells whose tomes ask for it. The names and the shape of the skills window are the original's.

use serde::{Deserialize, Serialize};

pub const MAX: u32 = 100;

/// The top rank of a profession, as in the original (the other skills stop at `MAX`).
pub const PROFESSION_MAX: u32 = 300;
/// How many ranks each step of a trade (apprentice, journeyman, expert, artisan) adds to what it can be taken to.
pub const PROFESSION_STEP: u32 = 75;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub enum Skill {
    Swords,
    Axes,
    Maces,
    Daggers,
    Unarmed,
    Blocking,
    TwoHandedSwords,
    TwoHandedAxes,
    TwoHandedMaces,
    Polearms,
    Staves,
    FistWeapons,
    Spears,
    Bows,
    Guns,
    Crossbows,
    Thrown,
    Wands,
    Sneaking,
    Lockpicking,
    Riding,
    Swimming,
    Diving,
    Cloth,
    Leather,
    Chain,
    Plate,
    Warrior,
    Paladin,
    Hunter,
    Rogue,
    Priest,
    Shaman,
    Mage,
    Warlock,
    Druid,
    Alchemy,
    Blacksmithing,
    Enchanting,
    Engineering,
    Herbalism,
    Leatherworking,
    Mining,
    Skinning,
    Tailoring,
    Cooking,
    Fishing,
    FirstAid,
}

/// How many skills there are.
pub const COUNT: usize = 48;

/// The rank of the original's skills of riding: apprentice, and journeyman (the nearest the game has, with only these two, to the original's tiers). Not trained by use:
/// a riding instructor teaches them for a price.
pub const APPRENTICE_RIDING: u32 = 75;
pub const JOURNEYMAN_RIDING: u32 = 150;

pub const ALL: [Skill; COUNT] = [
    Skill::Swords,
    Skill::Axes,
    Skill::Maces,
    Skill::Daggers,
    Skill::Unarmed,
    Skill::Blocking,
    Skill::TwoHandedSwords,
    Skill::TwoHandedAxes,
    Skill::TwoHandedMaces,
    Skill::Polearms,
    Skill::Staves,
    Skill::FistWeapons,
    Skill::Spears,
    Skill::Bows,
    Skill::Guns,
    Skill::Crossbows,
    Skill::Thrown,
    Skill::Wands,
    Skill::Sneaking,
    Skill::Lockpicking,
    Skill::Riding,
    Skill::Swimming,
    Skill::Diving,
    Skill::Cloth,
    Skill::Leather,
    Skill::Chain,
    Skill::Plate,
    Skill::Warrior,
    Skill::Paladin,
    Skill::Hunter,
    Skill::Rogue,
    Skill::Priest,
    Skill::Shaman,
    Skill::Mage,
    Skill::Warlock,
    Skill::Druid,
    Skill::Alchemy,
    Skill::Blacksmithing,
    Skill::Enchanting,
    Skill::Engineering,
    Skill::Herbalism,
    Skill::Leatherworking,
    Skill::Mining,
    Skill::Skinning,
    Skill::Tailoring,
    Skill::Cooking,
    Skill::Fishing,
    Skill::FirstAid,
];

/// The headings of the skills window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Class,
    Weapon,
    Armor,
    General,
    Professions,
    Secondary,
}

impl Group {
    pub fn name(self) -> &'static str {
        match self {
            Group::Class => "Class Skills",
            Group::Weapon => "Weapon Skills",
            Group::Armor => "Armor Proficiencies",
            Group::General => "General Skills",
            Group::Professions => "Professions",
            Group::Secondary => "Secondary Skills",
        }
    }
}

impl Skill {
    /// The most a rank of the skill can be: `MAX`, but riding is bought in two steps beyond it and the professions go to 300.
    pub fn top(self) -> u32 {
        match self {
            Skill::Riding => JOURNEYMAN_RIDING,
            s if s.is_profession() => PROFESSION_MAX,
            _ => MAX,
        }
    }

    /// A trade (mining, tailoring, fishing...), whose ranks rise by what it makes or gathers, not by the uses that train the others.
    pub fn is_profession(self) -> bool {
        matches!(self.group(), Group::Professions | Group::Secondary)
    }

    /// The rank the skill's bar is full at: for riding the tier the rank is in.
    pub fn cap(self, rank: u32) -> u32 {
        match self {
            Skill::Riding if rank > APPRENTICE_RIDING => JOURNEYMAN_RIDING,
            Skill::Riding => APPRENTICE_RIDING,
            s => s.top(),
        }
    }

    pub fn index(self) -> usize {
        ALL.iter().position(|&s| s == self).expect("every skill is in ALL")
    }

    /// What saves call it.
    pub fn id(self) -> String {
        format!("{self:?}")
    }

    pub fn from_id(id: &str) -> Option<Skill> {
        ALL.iter().copied().find(|s| s.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Skill::Swords => "Swords",
            Skill::Axes => "Axes",
            Skill::Maces => "Maces",
            Skill::Daggers => "Daggers",
            Skill::Unarmed => "Unarmed",
            Skill::TwoHandedSwords => "Two-Handed Swords",
            Skill::TwoHandedAxes => "Two-Handed Axes",
            Skill::TwoHandedMaces => "Two-Handed Maces",
            Skill::Polearms => "Polearms",
            Skill::Staves => "Staves",
            Skill::FistWeapons => "Fist Weapons",
            Skill::Spears => "Spears",
            Skill::Bows => "Bows",
            Skill::Guns => "Guns",
            Skill::Crossbows => "Crossbows",
            Skill::Thrown => "Thrown",
            Skill::Wands => "Wands",
            Skill::Blocking => "Block",
            Skill::Sneaking => "Stealth",
            Skill::Lockpicking => "Lockpicking",
            Skill::Riding => "Riding",
            Skill::Swimming => "Swimming",
            Skill::Diving => "Diving",
            Skill::Cloth => "Cloth",
            Skill::Leather => "Leather",
            Skill::Chain => "Mail",
            Skill::Plate => "Plate Mail",
            Skill::Warrior => "Warrior",
            Skill::Paladin => "Paladin",
            Skill::Hunter => "Hunter",
            Skill::Rogue => "Rogue",
            Skill::Priest => "Priest",
            Skill::Shaman => "Shaman",
            Skill::Mage => "Mage",
            Skill::Warlock => "Warlock",
            Skill::Druid => "Druid",
            Skill::Alchemy => "Alchemy",
            Skill::Blacksmithing => "Blacksmithing",
            Skill::Enchanting => "Enchanting",
            Skill::Engineering => "Engineering",
            Skill::Herbalism => "Herbalism",
            Skill::Leatherworking => "Leatherworking",
            Skill::Mining => "Mining",
            Skill::Skinning => "Skinning",
            Skill::Tailoring => "Tailoring",
            Skill::Cooking => "Cooking",
            Skill::Fishing => "Fishing",
            Skill::FirstAid => "First Aid",
        }
    }

    /// What the skill is, for the detail of the skills window (the original's detail is the skill's description alone).
    pub fn description(self) -> String {
        match self {
            weapon if weapon.group() == Group::Weapon && weapon != Skill::Unarmed && weapon != Skill::Blocking => format!("Skill with {}. Rises with every hit you land with one, and so does the damage they do.", weapon.name().to_lowercase()),
            Skill::Unarmed => "Skill in fighting with your fists. Rises with every hit you land with nothing in your hand, and so does the damage they do.".into(),
            Skill::Blocking => "Skill at taking blows on the shield. Rises with every blow blocked, and a block costs less stamina.".into(),
            Skill::Sneaking => "Skill at moving unseen. Rises while you sneak about, and creatures notice you from less far away.".into(),
            Skill::Lockpicking => "Skill at picking locks. Rises with every lock you open, and the sweet spot of a lock grows wider.".into(),
            Skill::Swimming => "Skill at swimming. Rises while you swim, and swimming tires you less.".into(),
            Skill::Diving => "Skill at holding your breath. Rises while your head is under water, and you can stay under longer before you start to drown.".into(),
            armor if armor.group() == Group::Armor => format!("Skill in wearing {} armor. Rises with every blow that lands on you, in the share of the pieces you wear of it, and each piece of it stops more damage.", armor.name().to_lowercase()),
            Skill::Mining => "Skill at mining. Learned from a teacher in a town. Lets you dig ore and stones from mineral veins with a mining pick; the richer veins ask for a higher rank, and every vein within reach of your rank can teach you more.".into(),
            Skill::Herbalism => "Skill at gathering herbs. Learned from a teacher in a town. Lets you pick plants for their leaves and roots; the rarer herbs ask for a higher rank, and every herb within reach of your rank can teach you more.".into(),
            Skill::Skinning => "Skill at skinning. Learned from a teacher in a town. Lets you strip the hide from the beasts you have killed with a skinning knife; the tougher beasts ask for a higher rank, and every skinning within reach of your rank can teach you more.".into(),
            Skill::Fishing => "Skill at fishing. Learned from a teacher in a town. Lets you cast a line into the water with a fishing pole and catch what bites; the rank rises with every catch.".into(),
            trade if trade.is_profession() => format!("Skill in {}. Learned from a teacher in a town, who teaches the step after each step as the rank allows. Recipes are taught by trainers and by the patterns, plans and formulas you find; every making that is hard enough for your rank raises it.", trade.name().to_lowercase()),
            Skill::Riding => "Skill at riding. Learned from a riding instructor, for a price; it lets you ride the mounts that ask for it.".into(),
            class => format!("Skill in the arts of the {}. Rises with every spell of the class you cast, and its spells grow stronger. The greater tomes of the class ask for its rank.", class.name().to_lowercase()),
        }
    }

    /// The skill of a skill line of the original's tables (`SkillLine.dbc`, what items and trainers ask for): the trades.
    pub fn of_line(line: u32) -> Option<Skill> {
        Some(match line {
            43 => Skill::Swords,
            44 => Skill::Axes,
            45 => Skill::Bows,
            46 => Skill::Guns,
            54 => Skill::Maces,
            55 => Skill::TwoHandedSwords,
            160 => Skill::TwoHandedMaces,
            172 => Skill::TwoHandedAxes,
            173 => Skill::Daggers,
            176 => Skill::Thrown,
            136 => Skill::Staves,
            162 => Skill::Unarmed,
            226 => Skill::Crossbows,
            228 => Skill::Wands,
            229 => Skill::Polearms,
            473 => Skill::FistWeapons,
            433 => Skill::Blocking,
            633 => Skill::Lockpicking,
            155 => Skill::Swimming,
            762 => Skill::Riding,
            415 => Skill::Cloth,
            414 => Skill::Leather,
            413 => Skill::Chain,
            293 => Skill::Plate,
            164 => Skill::Blacksmithing,
            165 => Skill::Leatherworking,
            171 => Skill::Alchemy,
            182 => Skill::Herbalism,
            185 => Skill::Cooking,
            186 => Skill::Mining,
            197 => Skill::Tailoring,
            202 => Skill::Engineering,
            333 => Skill::Enchanting,
            356 => Skill::Fishing,
            393 => Skill::Skinning,
            129 => Skill::FirstAid,
            _ => return None,
        })
    }

    /// One of the nine classes, whose spells train it.
    pub fn is_class(self) -> bool {
        self.group() == Group::Class
    }

    pub fn group(self) -> Group {
        match self {
            Skill::Sneaking | Skill::Lockpicking | Skill::Riding | Skill::Swimming | Skill::Diving => Group::General,
            Skill::Alchemy | Skill::Blacksmithing | Skill::Enchanting | Skill::Engineering | Skill::Herbalism | Skill::Leatherworking | Skill::Mining | Skill::Skinning | Skill::Tailoring => Group::Professions,
            Skill::Cooking | Skill::Fishing | Skill::FirstAid => Group::Secondary,
            Skill::Cloth | Skill::Leather | Skill::Chain | Skill::Plate => Group::Armor,
            Skill::Warrior | Skill::Paladin | Skill::Hunter | Skill::Rogue | Skill::Priest | Skill::Shaman | Skill::Mage | Skill::Warlock | Skill::Druid => Group::Class,
            _ => Group::Weapon,
        }
    }

    /// The skill a weapon trains: by its subclass in the original's tables (`ItemSubClass`); the weapons that are none of the families are
    /// fought with like fists.
    pub fn of_weapon(subclass: u8) -> Skill {
        match subclass {
            0 => Skill::Axes,
            1 => Skill::TwoHandedAxes,
            2 => Skill::Bows,
            3 => Skill::Guns,
            4 => Skill::Maces,
            5 => Skill::TwoHandedMaces,
            6 => Skill::Polearms,
            7 => Skill::Swords,
            8 => Skill::TwoHandedSwords,
            10 => Skill::Staves,
            13 => Skill::FistWeapons,
            15 => Skill::Daggers,
            16 => Skill::Thrown,
            17 => Skill::Spears,
            18 => Skill::Crossbows,
            19 => Skill::Wands,
            _ => Skill::Unarmed,
        }
    }
}

/// The skill that wearing a piece of armor trains, by its subclass in the original's tables (shields are the blocking skill's).
fn of_armor(item: &crate::items::ItemDef) -> Option<Skill> {
    match (item.class, item.subclass) {
        (4, 1) => Some(Skill::Cloth),
        (4, 2) => Some(Skill::Leather),
        (4, 3) => Some(Skill::Chain),
        (4, 4) => Some(Skill::Plate),
        _ => None,
    }
}

/// How many uses take a skill from `rank` to the next.
fn uses_needed(rank: u32) -> f32 {
    2.0 + rank as f32 * 0.2
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skills {
    rank: [u32; COUNT],
    /// Uses towards the next rank.
    uses: [f32; COUNT],
    /// What the worn gear adds to each skill (an enchantment of mining, a pair of gloves of herbalism): it counts when the skill is asked what it can do, not when it is trained.
    bonus: [i32; COUNT],
    /// The highest rank a trade can be taken to: 0 while it has not been learned, then 75, 150, 225 or 300 as the apprentice, the journeyman, the expert and the
    /// artisan are learned from a teacher (`Lesson::step`). The other skills have no such limit.
    max: [u32; COUNT],
}

impl Skills {
    /// A new character: rank 1 in everything, but 5 in what it starts with (the sword, the shield, its class).
    pub fn new() -> Skills {
        let mut s = Skills { rank: [1; COUNT], uses: [0.0; COUNT], bonus: [0; COUNT], max: [0; COUNT] };
        for skill in [Skill::Swords, Skill::Blocking, Skill::Paladin] {
            s.rank[skill.index()] = 5;
        }
        s
    }

    /// What the skill is worth now: its rank and what the gear adds.
    pub fn rank(&self, skill: Skill) -> u32 {
        (self.rank[skill.index()] as i32 + self.bonus[skill.index()]).max(1) as u32
    }

    /// The rank itself, trained or taught.
    pub fn pure(&self, skill: Skill) -> u32 {
        self.rank[skill.index()]
    }

    /// What the gear adds to the skill.
    pub fn bonus(&self, skill: Skill) -> i32 {
        self.bonus[skill.index()]
    }

    pub fn set_bonus(&mut self, bonus: [i32; COUNT]) {
        self.bonus = bonus;
    }

    /// Whether the skill can be used: all of them but the trades are, from the start; a trade has to be learned from a teacher.
    pub fn knows(&self, skill: Skill) -> bool {
        !skill.is_profession() || self.max[skill.index()] > 0
    }

    /// The most the skill can be taken to now.
    pub fn top_of(&self, skill: Skill) -> u32 {
        if skill.is_profession() { self.max[skill.index()] } else { skill.top() }
    }

    /// The rank the skill's bar is full at: for a trade the step learned.
    pub fn cap(&self, skill: Skill) -> u32 {
        if skill.is_profession() { self.max[skill.index()].max(PROFESSION_STEP) } else { skill.cap(self.rank[skill.index()]) }
    }

    /// Learn a step of a trade (1 the apprentice, up to 4 the artisan) from a teacher: the trade can be taken to 75 ranks more; a new trade begins at rank 1.
    pub fn teach(&mut self, skill: Skill, step: u32) {
        let i = skill.index();
        self.max[i] = self.max[i].max(step.clamp(1, 4) * PROFESSION_STEP);
    }

    /// The step of a trade that has been learned (0 for none).
    pub fn step(&self, skill: Skill) -> u32 {
        self.max[skill.index()] / PROFESSION_STEP
    }

    /// For the save: the trades that are learned, with the step of each.
    pub fn steps(&self) -> Vec<(String, u32)> {
        ALL.iter().filter(|s| s.is_profession() && self.knows(**s)).map(|&s| (s.id(), self.step(s))).collect()
    }

    /// Back from the save (a save from before there were teachers has none: a trade it has taken beyond rank 1 was learned, as far as its rank goes).
    pub fn set_steps(&mut self, steps: &[(String, u32)]) {
        for (id, step) in steps {
            if let Some(skill) = Skill::from_id(id).filter(|s| s.is_profession()) {
                self.max[skill.index()] = (*step).clamp(1, 4) * PROFESSION_STEP;
                self.rank[skill.index()] = self.rank[skill.index()].min(self.max[skill.index()]);
            }
        }
    }

    /// A rank bought, not trained (riding): it never goes down.
    pub fn learn(&mut self, skill: Skill, rank: u32) {
        let i = skill.index();
        self.rank[i] = self.rank[i].max(rank.min(skill.top()));
    }

    /// A rank set outright (the console), with the progress towards the next one cleared.
    pub fn set(&mut self, skill: Skill, rank: u32) {
        let i = skill.index();
        if skill.is_profession() {
            self.teach(skill, rank.div_ceil(PROFESSION_STEP));
        }
        self.rank[i] = rank.clamp(1, self.top_of(skill));
        self.uses[i] = 0.0;
    }

    /// How far along the way from rank 1 to `MAX` the skill is, 0 to 1.
    pub fn power(&self, skill: Skill) -> f32 {
        (self.rank(skill) - 1) as f32 / (MAX - 1) as f32
    }

    /// The skill was used `uses` times (a swing, a block, a spell): returns its new rank if it rose.
    pub fn train(&mut self, skill: Skill, uses: f32) -> Option<u32> {
        let i = skill.index();
        if self.rank[i] >= self.top_of(skill) {
            return None;
        }
        self.uses[i] += uses;
        let before = self.rank[i];
        while self.rank[i] < self.top_of(skill) && self.uses[i] >= uses_needed(self.rank[i]) {
            self.uses[i] -= uses_needed(self.rank[i]);
            self.rank[i] += 1;
        }
        if self.rank[i] >= self.top_of(skill) {
            self.uses[i] = 0.0;
        }
        (self.rank[i] > before).then_some(self.rank[i])
    }

    /// The server's rule for what a profession does (CMaNGOS `SkillGainChance`): one rank with a chance by how the skill stands against the ranks at which the work
    /// turns yellow, green and grey: below yellow (orange) always, then 75%, 25% and, from grey on, never. `roll` is a number from 0 to 1. Returns the new rank if it rose.
    pub fn skill_up(&mut self, skill: Skill, (yellow, green, grey): (u32, u32, u32), roll: f32) -> Option<u32> {
        let i = skill.index();
        let have = self.rank[i];
        let chance = if have >= grey {
            0.0
        } else if have >= green {
            0.25
        } else if have >= yellow {
            0.75
        } else {
            1.0
        };
        self.raise(skill, chance, roll)
    }

    /// One rank with this chance (0 to 1; `roll` is a number from 0 to 1), if the skill is not at its top: the new rank if it rose.
    pub fn raise(&mut self, skill: Skill, chance: f32, roll: f32) -> Option<u32> {
        let i = skill.index();
        (self.rank[i] < self.top_of(skill) && roll < chance).then(|| {
            self.rank[i] += 1;
            self.rank[i]
        })
    }

    /// Gathering what asks for `required` (`UpdateGatherSkill`): yellow 25 above it, green 50 and grey 100.
    pub fn gather(&mut self, skill: Skill, required: u32, roll: f32) -> Option<u32> {
        self.skill_up(skill, (required + 25, required + 50, required + 100), roll)
    }

    /// How much of the next rank is done, 0 to 1.
    #[cfg(test)]
    pub fn progress(&self, skill: Skill) -> f32 {
        let i = skill.index();
        if self.rank[i] >= MAX { 1.0 } else { (self.uses[i] / uses_needed(self.rank[i])).clamp(0.0, 1.0) }
    }

    /// What a rank is worth: the damage of a weapon or a spell is multiplied by up to twice at `MAX` for weapons and spells
    /// (`damage_factor`), a block costs up to half less stamina, those who sneak are noticed from up to half the distance less,
    /// and the sweet spot of a lock is up to half wider.
    pub fn damage_factor(&self, skill: Skill) -> f32 {
        1.0 + self.power(skill) * if skill.is_class() { 1.0 } else { 0.5 }
    }

    pub fn block_cost_factor(&self) -> f32 {
        1.0 - 0.5 * self.power(Skill::Blocking)
    }

    pub fn notice_factor(&self) -> f32 {
        1.0 - 0.5 * self.power(Skill::Sneaking)
    }

    /// Swimming drains up to half less stamina at the top rank.
    pub fn swim_cost_factor(&self) -> f32 {
        1.0 - 0.5 * self.power(Skill::Swimming)
    }

    /// Seconds the breath lasts under water: half a minute at rank 1, two minutes at the top rank.
    pub fn breath_seconds(&self) -> f32 {
        30.0 + 90.0 * self.power(Skill::Diving)
    }

    pub fn lock_window_factor(&self) -> f32 {
        1.0 + 0.5 * self.power(Skill::Lockpicking)
    }

    /// How many of the pieces of armor worn are of each kind, as shares that add up to 1 (none when nothing is worn). Pieces are counted, not
    /// their armor: cloth stops far less than plate, and would otherwise be trained far less.
    pub fn armor_shares(equipment: &[Option<crate::items::Stack>; crate::items::SLOTS]) -> Vec<(Skill, f32)> {
        let mut by_kind = [0.0; 4];
        for item in equipment.iter().flatten().map(|s| crate::items::get(s.item)) {
            if let Some(skill) = of_armor(item) {
                by_kind[skill.index() - Skill::Cloth.index()] += 1.0;
            }
        }
        let total: f32 = by_kind.iter().sum();
        [Skill::Cloth, Skill::Leather, Skill::Chain, Skill::Plate].into_iter().zip(by_kind).filter(|_| total > 0.0).map(|(s, a)| (s, a / total)).filter(|&(_, share)| share > 0.0).collect()
    }

    /// The armor points the skills add to what is worn: each piece's armor is worth up to half more at the top rank of its kind, so mixed
    /// kinds count each by its share.
    pub fn armor_bonus(&self, equipment: &[Option<crate::items::Stack>; crate::items::SLOTS]) -> i32 {
        let bonus: f32 = equipment.iter().flatten().map(|s| crate::items::get(s.item)).filter_map(|i| Some(i.armor.max(0) as f32 * 0.5 * self.power(of_armor(i)?))).sum();
        bonus.round() as i32
    }

    /// For the save: the rank and the uses towards the next one, by id.
    pub fn parts(&self) -> Vec<(String, u32, f32)> {
        ALL.iter().map(|&s| (s.id(), self.rank(s), self.uses[s.index()])).collect()
    }

    /// Back from the save; skills it does not know are dropped, and what it does not mention stays at rank 1.
    pub fn from_parts(parts: &[(String, u32, f32)]) -> Skills {
        let mut s = Skills { rank: [1; COUNT], uses: [0.0; COUNT], bonus: [0; COUNT], max: [0; COUNT] };
        for (id, rank, uses) in parts {
            if let Some(skill) = Skill::from_id(id) {
                s.rank[skill.index()] = (*rank).clamp(1, skill.top());
                s.uses[skill.index()] = if *rank >= skill.top() { 0.0 } else { uses.clamp(0.0, uses_needed(*rank)) };
            }
        }
        // A save from before there were teachers: a trade that was taken beyond rank 1 was learned, up to the step its rank is in.
        for skill in ALL.into_iter().filter(|s| s.is_profession() && s.top() == PROFESSION_MAX) {
            if s.rank[skill.index()] > 1 {
                s.teach(skill, s.rank[skill.index()].div_ceil(PROFESSION_STEP));
            }
        }
        s
    }
}

impl Default for Skills {
    fn default() -> Skills {
        Skills::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn riding_is_learned_in_steps_beyond_the_top_of_the_other_skills_and_is_kept_by_a_save() {
        let mut s = Skills::new();
        assert_eq!((s.rank(Skill::Riding), Skill::Riding.cap(1)), (1, APPRENTICE_RIDING));
        s.learn(Skill::Riding, APPRENTICE_RIDING);
        s.learn(Skill::Riding, 10);
        assert_eq!((s.rank(Skill::Riding), Skill::Riding.cap(75)), (75, APPRENTICE_RIDING));
        s.learn(Skill::Riding, JOURNEYMAN_RIDING);
        assert_eq!((s.rank(Skill::Riding), Skill::Riding.cap(150)), (150, JOURNEYMAN_RIDING));
        assert_eq!(Skills::from_parts(&s.parts()).rank(Skill::Riding), 150);
        assert_eq!(Skill::Swords.cap(5), MAX);
    }

    #[test]
    fn use_trains_a_skill_rank_by_rank_up_to_the_top() {
        let mut s = Skills::new();
        assert_eq!((s.rank(Skill::Mage), s.rank(Skill::Swords)), (1, 5));
        // Rank 1 takes 2.2 uses: two do not do it, the third does, and what is left over counts.
        assert_eq!(s.train(Skill::Mage, 2.0), None);
        assert!((s.progress(Skill::Mage) - 2.0 / 2.2).abs() < 1e-4);
        assert_eq!(s.train(Skill::Mage, 1.0), Some(2));
        // A big enough use can rise several ranks, and the top is the top.
        assert!(s.train(Skill::Mage, 10_000.0).is_some());
        assert_eq!((s.rank(Skill::Mage), s.power(Skill::Mage), s.progress(Skill::Mage)), (MAX, 1.0, 1.0));
        assert_eq!(s.train(Skill::Mage, 1.0), None);
        // It takes some hundreds of uses to get there honestly.
        let mut honest = Skills::new();
        let uses = (0..).find(|_| honest.train(Skill::Axes, 1.0).is_some_and(|r| r == MAX)).unwrap();
        assert!((400..3000).contains(&uses), "{uses} uses");
    }

    #[test]
    fn a_rank_is_worth_more_damage_cheaper_blocks_a_quieter_step_and_a_wider_lock() {
        let mut s = Skills::new();
        let (d0, b0, n0, l0) = (s.damage_factor(Skill::Swords), s.block_cost_factor(), s.notice_factor(), s.lock_window_factor());
        assert!(d0 >= 1.0 && b0 <= 1.0 && n0 == 1.0 && l0 == 1.0);
        assert_eq!((s.swim_cost_factor(), s.breath_seconds()), (1.0, 30.0));
        for skill in [Skill::Swords, Skill::Mage, Skill::Blocking, Skill::Sneaking, Skill::Lockpicking, Skill::Swimming, Skill::Diving] {
            s.train(skill, 1e6);
        }
        assert_eq!((s.swim_cost_factor(), s.breath_seconds()), (0.5, 120.0));
        assert_eq!((s.damage_factor(Skill::Swords), s.damage_factor(Skill::Mage)), (1.5, 2.0));
        assert_eq!((s.block_cost_factor(), s.notice_factor(), s.lock_window_factor()), (0.5, 0.5, 1.5));
    }

    #[test]
    fn a_profession_goes_to_300_and_gathering_rises_it_by_the_servers_chances() {
        let mut s = Skills::new();
        assert_eq!((Skill::Mining.top(), Skill::Swords.top()), (PROFESSION_MAX, MAX));
        // A trade is not there until a teacher teaches it, and then it goes as far as the step learned (75 ranks for each) and no further.
        assert!(!s.knows(Skill::Mining) && s.knows(Skill::Swords) && s.gather(Skill::Mining, 1, 0.0).is_none());
        s.teach(Skill::Mining, 1);
        assert_eq!((s.knows(Skill::Mining), s.cap(Skill::Mining), s.step(Skill::Mining)), (true, 75, 1));
        s.set(Skill::Mining, 75);
        assert_eq!(s.gather(Skill::Mining, 75, 0.0), None, "the apprentice stops at 75");
        s.teach(Skill::Mining, 2);
        assert_eq!(s.gather(Skill::Mining, 75, 0.0), Some(76));
        s.teach(Skill::Mining, 4);
        s.set(Skill::Mining, 1);
        // At the rank a node asks for, and up to 24 above it, every gathering teaches; 25 above, three in four; 50 above, one in four; 100 above, none.
        assert_eq!(s.gather(Skill::Mining, 1, 0.99), Some(2));
        assert_eq!(s.gather(Skill::Mining, 2, 0.99), Some(3));
        s.set(Skill::Mining, 30);
        assert_eq!((s.gather(Skill::Mining, 1, 0.74), s.gather(Skill::Mining, 1, 0.75)), (Some(31), None));
        s.set(Skill::Mining, 60);
        assert_eq!((s.gather(Skill::Mining, 1, 0.24), s.gather(Skill::Mining, 1, 0.25)), (Some(61), None));
        s.set(Skill::Mining, 101);
        assert_eq!(s.gather(Skill::Mining, 1, 0.0), None);
        s.set(Skill::Mining, 300);
        assert_eq!(s.gather(Skill::Mining, 300, 0.0), None);
        let mut back = Skills::from_parts(&s.parts());
        // A save from before the teachers has no steps: a trade taken beyond rank 1 counts as learned; the steps of a new save are kept.
        assert_eq!((back.rank(Skill::Mining), back.step(Skill::Mining), back.knows(Skill::Herbalism)), (300, 4, false));
        s.teach(Skill::Herbalism, 2);
        back.set_steps(&s.steps());
        assert_eq!((back.step(Skill::Herbalism), back.step(Skill::Mining), back.cap(Skill::Herbalism)), (2, 4, 150));
    }

    #[test]
    fn weapons_train_by_family_and_every_skill_has_a_name_a_heading_and_an_id_that_comes_back() {
        assert_eq!([Skill::of_weapon(7), Skill::of_weapon(4), Skill::of_weapon(8), Skill::of_weapon(99)], [Skill::Swords, Skill::Maces, Skill::TwoHandedSwords, Skill::Unarmed]);
        for s in ALL {
            assert_eq!(Skill::from_id(&s.id()), Some(s));
            assert!(!s.name().is_empty());
        }
        assert_eq!(ALL.iter().filter(|s| s.is_class()).count(), 9);
        assert_eq!(ALL.iter().filter(|s| s.group() == Group::Weapon).count(), 18);
        assert_eq!(ALL.iter().filter(|s| s.group() == Group::General).count(), 5);
        assert_eq!((ALL.iter().filter(|s| s.group() == Group::Professions).count(), ALL.iter().filter(|s| s.group() == Group::Secondary).count()), (9, 3));
        assert_eq!(ALL.iter().filter(|s| s.group() == Group::Armor).count(), 4);
    }

    #[test]
    fn armor_skills_make_each_piece_stop_more_and_mixed_kinds_split_the_training_by_share() {
        use crate::items::{self, Stack, SLOTS};
        let piece = |subclass: u8, not: Option<items::Wear>| items::all().iter().position(|i| i.class == 4 && i.subclass == subclass && i.armor > 0 && i.wear != items::Wear::None && Some(i.wear) != not).map(|item| Stack { item, count: 1, enchant: 0 }).expect("some armor");
        let cloth = piece(1, None);
        let plate = piece(4, Some(items::get(cloth.item).wear));
        let mut worn: [Option<Stack>; SLOTS] = [None; SLOTS];
        assert!(Skills::armor_shares(&worn).is_empty());
        for stack in [cloth, plate] {
            worn[(0..SLOTS).find(|&slot| items::get(stack.item).wear.fits(slot)).unwrap()] = Some(stack);
        }
        let shares = Skills::armor_shares(&worn);
        let p = items::get(plate.item).armor as f32;
        // One piece of each: half and half, whatever they stop.
        assert_eq!(shares, [(Skill::Cloth, 0.5), (Skill::Plate, 0.5)]);
        // Only the kind that is worn counts: master the plate and the cloth adds nothing to it.
        let mut s = Skills::new();
        assert_eq!(s.armor_bonus(&worn), 0);
        s.train(Skill::Plate, 1e6);
        assert_eq!(s.armor_bonus(&worn), (p * 0.5).round() as i32);
    }

    #[test]
    fn every_weapon_of_the_game_trains_a_weapon_family() {
        for item in crate::items::all().iter().filter(|i| i.weapon.is_some()) {
            // A weapon of the original has the subclass of one family of weapons; ours, made by hand, say what they are too.
            assert!(item.class == 2, "{} has damage but is not a weapon of the tables", item.name);
        }
        assert_eq!(Skill::of_weapon(crate::items::get(crate::items::find("longsword")).subclass), Skill::Swords);
    }

    #[test]
    fn skills_survive_a_save_and_a_save_of_other_times_is_forgiven() {
        let mut s = Skills::new();
        s.train(Skill::Daggers, 30.0);
        s.train(Skill::Priest, 4.0);
        assert_eq!(Skills::from_parts(&s.parts()), s);
        let odd = Skills::from_parts(&[("Basketweaving".into(), 50, 0.0), ("Mage".into(), 9999, 7.0), ("Rogue".into(), 0, -3.0)]);
        assert_eq!((odd.rank(Skill::Mage), odd.rank(Skill::Rogue), odd.rank(Skill::Swords)), (MAX, 1, 1));
    }
}
