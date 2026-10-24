//! The talents: three trees of passive perks (`content/talents.ron`), learned with a point per level from level 10. A tier opens with 5 points per tier before it
//! spent in its tree, a talent can ask for another to be at its full rank, and a hybrid talent asks for points spent in another tree. The window is
//! `panels/talents.rs`; what the perks do to the character is `Perks`, which `Stats::of_with` takes in.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The first level that gives a point.
pub const FIRST_LEVEL: u32 = 10;
pub const TIERS: usize = 7;
pub const COLUMNS: usize = 4;
/// Points spent in a tree before the next tier of it opens, per tier.
const PER_TIER: u32 = 5;

/// What one rank of a talent does. Amounts are percent (or percentage points for chances); `...From(attribute, percent)` adds that share of an attribute
/// (0 strength, 1 agility, 2 stamina, 3 intellect, 4 spirit) to attack power, ranged attack power or spell damage and healing.
#[derive(Clone, Copy, Debug, Deserialize)]
pub enum Perk {
    /// Nothing: the talent teaches a spell (`grants`).
    Spell,
    AttackPower(f32),
    RangedAttackPower(f32),
    Armor(f32),
    Health(f32),
    Mana(f32),
    Stamina(f32),
    Crit(f32),
    SpellCrit(f32),
    Dodge(f32),
    Parry(f32),
    Hit(f32),
    SpellHit(f32),
    ManaRegen(f32),
    HealthRegen(f32),
    RunSpeed(f32),
    Haste(f32),
    AttackPowerFrom(usize, f32),
    RangedAttackPowerFrom(usize, f32),
    SpellPowerFrom(usize, f32),
}

impl Perk {
    /// What one rank gives.
    pub fn amount(self) -> f32 {
        match self {
            Perk::Spell => 0.0,
            Perk::AttackPower(a) | Perk::RangedAttackPower(a) | Perk::Armor(a) | Perk::Health(a) | Perk::Mana(a) | Perk::Stamina(a) | Perk::Crit(a) | Perk::SpellCrit(a) | Perk::Dodge(a) | Perk::Parry(a) | Perk::Hit(a) | Perk::SpellHit(a) | Perk::ManaRegen(a) | Perk::HealthRegen(a) | Perk::RunSpeed(a) | Perk::Haste(a) => a,
            Perk::AttackPowerFrom(_, a) | Perk::RangedAttackPowerFrom(_, a) | Perk::SpellPowerFrom(_, a) => a,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Talent {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub tier: usize,
    pub column: usize,
    pub ranks: u8,
    pub perk: Perk,
    /// The tooltip, with `{}` for the total of the ranks learned.
    pub text: String,
    /// The talent of the same tree that has to be at its full rank first.
    #[serde(default)]
    pub requires: Option<String>,
    /// Tree and points that have to be spent in it first (a hybrid talent).
    #[serde(default)]
    pub needs: Option<(usize, u32)>,
    /// The id of the spell (`spells.ron`) that learning the first rank teaches.
    #[serde(default)]
    pub grants: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tree {
    pub name: String,
    /// The file stem of the four background pictures in `Interface\TalentFrame`.
    pub art: String,
    pub talents: Vec<Talent>,
}

impl Talent {
    /// The spell (as the content numbers its spells) that this talent teaches.
    pub fn spell(&self) -> Option<usize> {
        self.grants.as_deref().map(crate::content::spell_index)
    }

    /// The tooltip text for `rank` ranks learned.
    pub fn describe(&self, rank: u8) -> String {
        let total = self.perk.amount() * rank as f32;
        self.text.replacen("{}", &if total.fract() == 0.0 { format!("{total:.0}") } else { format!("{total:.1}") }, 1)
    }
}

pub fn trees() -> &'static [Tree] {
    static TREES: OnceLock<Vec<Tree>> = OnceLock::new();
    TREES.get_or_init(|| ron::from_str(&crate::content::source("talents.ron", include_str!("../../../content/talents.ron"))).expect("talents.ron"))
}

/// Why a talent cannot be learned now (points left aside).
#[derive(Clone, Debug, PartialEq)]
pub enum Lock {
    /// Points to spend in the tree first.
    Tier(u32),
    /// The name of the talent that has to be at its full rank.
    Requires(&'static str),
    /// The tree and the points to spend in it.
    Hybrid(&'static str, u32),
}

/// The ranks learned, by the id of the talent.
#[derive(Clone, Debug, Default)]
pub struct Ranks(HashMap<&'static str, u8>);

impl Ranks {
    pub fn rank(&self, talent: &Talent) -> u8 {
        self.0.get(talent.id.as_str()).copied().unwrap_or(0)
    }

    /// The points spent in tree `tree`.
    pub fn spent(&self, tree: usize) -> u32 {
        trees()[tree].talents.iter().map(|t| self.rank(t) as u32).sum()
    }

    pub fn total(&self) -> u32 {
        (0..trees().len()).map(|t| self.spent(t)).sum()
    }

    /// The points a character of this level has left to spend.
    pub fn free(&self, level: u32) -> u32 {
        (level + 1).saturating_sub(FIRST_LEVEL).saturating_sub(self.total())
    }

    /// What stands in the way of a talent, if anything.
    pub fn lock(&self, tree: usize, talent: &'static Talent) -> Option<Lock> {
        let need = (talent.tier as u32 - 1) * PER_TIER;
        if self.spent(tree) < need {
            return Some(Lock::Tier(need));
        }
        if let Some(r) = talent.requires.as_deref().and_then(|id| trees()[tree].talents.iter().find(|t| t.id == id)) {
            if self.rank(r) < r.ranks {
                return Some(Lock::Requires(&r.name));
            }
        }
        match talent.needs {
            Some((other, points)) if self.spent(other) < points => Some(Lock::Hybrid(&trees()[other].name, points)),
            _ => None,
        }
    }

    /// Every learned talent is within its ranks and has what it asks for.
    fn valid(&self) -> bool {
        trees().iter().enumerate().all(|(i, tree)| tree.talents.iter().all(|t| self.rank(t) == 0 || (self.rank(t) <= t.ranks && self.lock(i, t).is_none())))
    }

    /// Learn a rank of a talent of tree `tree` (a character of this `level`). False when it cannot be.
    pub fn learn(&mut self, tree: usize, talent: &'static Talent, level: u32) -> bool {
        if self.free(level) == 0 || self.rank(talent) >= talent.ranks || self.lock(tree, talent).is_some() {
            return false;
        }
        *self.0.entry(talent.id.as_str()).or_default() += 1;
        true
    }

    /// Give a rank back, unless a talent learned since needs it. False when it cannot be.
    pub fn unlearn(&mut self, talent: &'static Talent) -> bool {
        let Some(rank) = self.0.get_mut(talent.id.as_str()).filter(|r| **r > 0) else { return false };
        *rank -= 1;
        if self.valid() {
            return true;
        }
        *self.0.get_mut(talent.id.as_str()).unwrap() += 1;
        false
    }

    /// Take every rank back.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// What the ranks learned add up to.
    pub fn perks(&self) -> Perks {
        let mut p = Perks::default();
        for t in trees().iter().flat_map(|t| &t.talents) {
            p.add(t.perk, self.rank(t) as f32);
        }
        p
    }

    /// (id, rank) of each talent learned, for the save.
    pub fn saved(&self) -> Vec<(String, u8)> {
        let mut out: Vec<_> = self.0.iter().filter(|(_, r)| **r > 0).map(|(id, r)| (id.to_string(), *r)).collect();
        out.sort();
        out
    }

    /// From a save: talents that are gone from the file are dropped and ranks held to the most there are.
    pub fn from_saved(saved: &[(String, u8)]) -> Ranks {
        let mut out = Ranks::default();
        for (id, rank) in saved {
            if let Some(t) = trees().iter().flat_map(|t| &t.talents).find(|t| t.id == *id) {
                out.0.insert(t.id.as_str(), (*rank).min(t.ranks));
            }
        }
        out
    }
}

/// The sum of the perks, by what they change. Percent, or percentage points for chances.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Perks {
    pub attack_power: f32,
    pub ranged_attack_power: f32,
    pub armor: f32,
    pub health: f32,
    pub mana: f32,
    pub stamina: f32,
    pub crit: f32,
    pub spell_crit: f32,
    pub dodge: f32,
    pub parry: f32,
    pub hit: f32,
    pub spell_hit: f32,
    pub mana_regen: f32,
    pub health_regen: f32,
    pub run_speed: f32,
    pub haste: f32,
    /// The share of each attribute (strength, agility, stamina, intellect, spirit) added to attack power, ranged attack power and spell power.
    pub attack_power_from: [f32; 5],
    pub ranged_attack_power_from: [f32; 5],
    pub spell_power_from: [f32; 5],
}

impl Perks {
    fn add(&mut self, perk: Perk, ranks: f32) {
        let a = perk.amount() * ranks;
        match perk {
            Perk::Spell => {}
            Perk::AttackPower(_) => self.attack_power += a,
            Perk::RangedAttackPower(_) => self.ranged_attack_power += a,
            Perk::Armor(_) => self.armor += a,
            Perk::Health(_) => self.health += a,
            Perk::Mana(_) => self.mana += a,
            Perk::Stamina(_) => self.stamina += a,
            Perk::Crit(_) => self.crit += a,
            Perk::SpellCrit(_) => self.spell_crit += a,
            Perk::Dodge(_) => self.dodge += a,
            Perk::Parry(_) => self.parry += a,
            Perk::Hit(_) => self.hit += a,
            Perk::SpellHit(_) => self.spell_hit += a,
            Perk::ManaRegen(_) => self.mana_regen += a,
            Perk::HealthRegen(_) => self.health_regen += a,
            Perk::RunSpeed(_) => self.run_speed += a,
            Perk::Haste(_) => self.haste += a,
            Perk::AttackPowerFrom(i, _) => self.attack_power_from[i] += a,
            Perk::RangedAttackPowerFrom(i, _) => self.ranged_attack_power_from[i] += a,
            Perk::SpellPowerFrom(i, _) => self.spell_power_from[i] += a,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(id: &str) -> (usize, &'static Talent) {
        trees().iter().enumerate().find_map(|(i, t)| t.talents.iter().find(|t| t.id == id).map(|t| (i, t))).unwrap()
    }

    #[test]
    fn the_trees_fit_the_original_window_and_can_be_climbed() {
        assert_eq!(trees().len(), 3);
        // Every spell a talent teaches is taught by one talent, and no talent teaches a spell with a tome.
        for (i, spell) in crate::content::get().spells.iter().enumerate().filter(|(_, s)| s.talent) {
            assert_eq!(trees().iter().flat_map(|t| &t.talents).filter(|t| t.spell() == Some(i)).count(), 1, "{}", spell.name);
        }
        for (i, tree) in trees().iter().enumerate() {
            // At most 20 buttons; one talent to a place; ids unique across the trees (the save goes by id).
            assert!(tree.talents.len() <= 20, "{}", tree.name);
            let mut places = std::collections::HashSet::new();
            for t in &tree.talents {
                assert!((1..=TIERS).contains(&t.tier) && (1..=COLUMNS).contains(&t.column) && t.ranks >= 1, "{}", t.id);
                assert!(places.insert((t.tier, t.column)), "{}: place taken", t.id);
                assert!(t.text.contains("{}") || matches!(t.perk, Perk::Spell), "{}", t.id);
                if let Some(id) = &t.grants {
                    assert!(t.tier >= 3, "{}: a spell is not for the first two rows", t.id);
                    assert!(crate::content::get().spells.iter().any(|s| s.id == *id && s.talent), "{}: no talent spell {id}", t.id);
                }
                assert!(trees().iter().flat_map(|t| &t.talents).filter(|o| o.id == t.id).count() == 1, "{}: id twice", t.id);
                if let Some((other, points)) = t.needs {
                    assert!(other != i && other < trees().len() && points > 0, "{}", t.id);
                }
                if let Perk::AttackPowerFrom(a, _) | Perk::RangedAttackPowerFrom(a, _) | Perk::SpellPowerFrom(a, _) = t.perk {
                    assert!(a < 5, "{}", t.id);
                }
                // The window draws the line to a prerequisite in the same column straight down: nothing may stand between.
                if let Some(r) = &t.requires {
                    let r = tree.talents.iter().find(|o| o.id == *r).unwrap_or_else(|| panic!("{}: no talent {r}", t.id));
                    assert!(r.tier < t.tier, "{}", t.id);
                    if r.column == t.column {
                        assert!(!tree.talents.iter().any(|o| o.column == t.column && o.tier > r.tier && o.tier < t.tier), "{}: blocked", t.id);
                    }
                }
            }
            // Every tier can be reached with the ranks of the ones before it.
            for tier in 2..=TIERS {
                let before: u32 = tree.talents.iter().filter(|t| t.tier < tier).map(|t| t.ranks as u32).sum();
                assert!(before >= (tier as u32 - 1) * PER_TIER || !tree.talents.iter().any(|t| t.tier == tier), "{}: tier {tier} cannot be opened", tree.name);
            }
        }
    }

    #[test]
    fn a_point_a_level_from_level_ten_and_the_gates_hold() {
        let mut ranks = Ranks::default();
        assert_eq!((ranks.free(9), ranks.free(10), ranks.free(60)), (0, 1, 51));
        let (v, force) = find("brute_force");
        assert!(!ranks.learn(v, force, 9));
        assert!(ranks.learn(v, force, 10));
        assert!(!ranks.learn(v, force, 10), "no point left");
        // Vitality asks for Brute Force at its full rank; the second tier asks for 5 points in the tree.
        let (_, vitality) = find("vitality");
        for _ in 0..4 {
            assert!(ranks.learn(v, force, 60));
        }
        assert!(ranks.learn(v, vitality, 60));
        // A hybrid talent asks for points in another tree.
        let (a, blade) = find("spellblade");
        assert_eq!(ranks.lock(a, blade), Some(Lock::Tier(15)));
        // Taking a rank of Brute Force back is refused while Vitality stands on it, and the perks follow the ranks.
        assert!(!ranks.unlearn(force));
        assert_eq!(ranks.perks().attack_power, 10.0);
        assert!(ranks.unlearn(vitality) && ranks.unlearn(force));
        assert_eq!(Ranks::from_saved(&ranks.saved()).total(), ranks.total());
    }
}
