//! The player's quest log: what he has accepted, how far along each quest is, and what he has finished. The quests
//! themselves are in `content/quests.ron`; this is the state and the rules (no graphics, so it is tested on its own).

use crate::content::{self, Goal};
use crate::items::{Inventory, Refusal};

/// The original's limit on quests carried at once.
pub const MAX_ACTIVE: usize = 20;

/// A quest being worked on: its index in the content and the kills so far for each of its goals (0 for goals that
/// are about carried items, which are counted in the backpack).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Active {
    pub quest: usize,
    pub kills: Vec<u32>,
}

/// What a creature shows over its head for the player's quests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Marker {
    /// A quest to give: the yellow "!".
    Available,
    /// A quest ready to turn in: the yellow "?".
    Complete,
    /// A quest taken but not done yet, to be turned in here: the grey "?".
    Incomplete,
}

/// A step of progress on a goal, for the message that tells the player (`"Wolf slain: 1/2"`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Progress {
    pub quest: usize,
    pub goal: usize,
    pub have: u32,
    pub need: u32,
}

/// Who is asking about quests: the level and the race (the original's bit mask, 1 human... 128 troll) of the character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Who {
    pub level: u32,
    pub race: u32,
}

impl Who {
    /// Whether the character may take the quest: he is high enough, and of a race it is for.
    pub fn may(self, quest: &content::Quest) -> bool {
        self.level >= quest.min_level && (quest.races == 0 || quest.races & self.race != 0)
    }
}

impl Default for Who {
    fn default() -> Who {
        Who { level: 1, race: 1 }
    }
}

/// What the compass and the map point to for a quest: kinds of creatures (indexes) with their picture, an item that chests may hold, and places (north, west).
#[derive(Clone, Debug, PartialEq)]
pub struct Lead {
    pub kinds: Vec<usize>,
    pub item: Option<usize>,
    pub icon: Icon,
    pub places: Vec<(f32, f32)>,
}

/// The picture of a place on the compass and the map: a skull for a creature to kill (or to take the item from), a chest, the bag of a
/// vendor, or the "?" of a turn in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    Skull,
    Chest,
    Vendor,
    TurnIn,
}

#[derive(Default, Debug)]
pub struct QuestLog {
    pub active: Vec<Active>,
    /// Finished quests (indexes).
    pub done: Vec<usize>,
    /// The active quest the compass points to (a quest index), if any.
    pub tracked: Option<usize>,
}

impl QuestLog {
    /// What to look for to get on with a quest being worked on: its first goal that is not met yet (the creatures to kill, the
    /// creatures that drop the item or else sell it, and the item for the chests that hold it), or its turn in when every goal is met.
    /// Where those are now is up to the world (`Creatures::positions`, `Chests::holding`).
    pub fn lead(&self, quest: usize, inv: &Inventory) -> Lead {
        let c = content::get();
        let q = content::quest(quest);
        let kinds = |wanted: &dyn Fn(&content::CreatureDef) -> bool| (0..c.creatures.len()).filter(|&i| wanted(&c.creatures[i])).collect::<Vec<_>>();
        let open = q.goals.iter().enumerate().find(|&(i, g)| self.have(quest, i, inv) < g.count()).map(|(_, g)| *g);
        match open {
            None => Lead { kinds: vec![q.turn_in], item: None, icon: Icon::TurnIn, places: Vec::new() },
            Some(Goal::Kill { creature, .. }) => Lead { kinds: vec![creature], item: None, icon: Icon::Skull, places: Vec::new() },
            Some(Goal::Reach { at, .. }) => Lead { kinds: Vec::new(), item: None, icon: Icon::TurnIn, places: vec![at] },
            Some(Goal::Collect { item, .. }) => {
                let drops = kinds(&|cr| cr.loot.as_ref().is_some_and(|t| c.loot[t].items.iter().any(|d| d.0 == item)));
                if drops.is_empty() {
                    Lead { kinds: kinds(&|cr| cr.vendor.contains(&item)), item: Some(item), icon: Icon::Vendor, places: Vec::new() }
                } else {
                    Lead { kinds: drops, item: Some(item), icon: Icon::Skull, places: Vec::new() }
                }
            }
        }
    }

    /// The items that the quests being worked on still want: those of their collect goals that are not all in the bags yet.
    pub fn wanted(&self, inv: &Inventory) -> Vec<usize> {
        let begun = content::get().item_quests.iter().filter(|&(&item, &quest)| inv.count(item) == 0 && !self.is_active(quest) && !self.done.contains(&quest)).map(|(&item, _)| item);
        self.active.iter().flat_map(|a| &content::quest(a.quest).goals).filter_map(|g| if let Goal::Collect { item, count } = *g { Some((item, count)) } else { None }).filter(|&(item, count)| inv.count(item) < count).map(|(item, _)| item).chain(begun).collect()
    }

    /// The quest that using `item` begins, if it can be taken now.
    pub fn begun_by(&self, item: usize, who: Who) -> Option<usize> {
        let c = content::get();
        let quest = *c.item_quests.get(&item)?;
        (!self.is_active(quest) && !self.done.contains(&quest) && c.quests[quest].requires.is_none_or(|r| self.done.contains(&r)) && who.may(&c.quests[quest])).then_some(quest)
    }

    pub fn is_active(&self, quest: usize) -> bool {
        self.active.iter().any(|a| a.quest == quest)
    }

    /// The quests creature `creature` can give: not taken or done yet, with what they require done.
    pub fn offered_by(&self, creature: usize, who: Who) -> Vec<usize> {
        let c = content::get();
        if c.creatures[creature].attackable() {
            return Vec::new();
        }
        c.giving.get(&creature).into_iter().flatten().copied().filter(|&q| !self.is_active(q) && !self.done.contains(&q) && c.quests[q].requires.is_none_or(|r| self.done.contains(&r)) && who.may(&c.quests[q])).collect()
    }

    /// The quests being worked on that creature `creature` takes back, each with whether it can be turned in now.
    pub fn taken_by(&self, creature: usize, inv: &Inventory) -> Vec<(usize, bool)> {
        if !content::get().taking.contains_key(&creature) || content::get().creatures[creature].attackable() {
            return Vec::new();
        }
        self.active.iter().filter(|a| content::quest(a.quest).turn_in == creature).map(|a| (a.quest, self.complete(a.quest, inv))).collect()
    }

    /// How many of goal `goal` of an active quest are done.
    pub fn have(&self, quest: usize, goal: usize, inv: &Inventory) -> u32 {
        let Some(active) = self.active.iter().find(|a| a.quest == quest) else { return 0 };
        match content::quest(quest).goals[goal] {
            Goal::Kill { count, .. } => active.kills[goal].min(count),
            Goal::Collect { item, count } => inv.count(item).min(count),
            Goal::Reach { .. } => active.kills[goal].min(1),
        }
    }

    /// Whether every goal of an active quest is met.
    pub fn complete(&self, quest: usize, inv: &Inventory) -> bool {
        self.is_active(quest) && content::quest(quest).goals.iter().enumerate().all(|(i, g)| self.have(quest, i, inv) >= g.count())
    }

    /// Take a quest. False when the log is full or the quest was taken or done already.
    pub fn accept(&mut self, quest: usize) -> bool {
        if self.active.len() >= MAX_ACTIVE || self.is_active(quest) || self.done.contains(&quest) {
            return false;
        }
        self.active.push(Active { quest, kills: vec![0; content::quest(quest).goals.len()] });
        self.tracked = Some(quest);
        true
    }

    /// Give a quest up (it can be taken again).
    pub fn abandon(&mut self, quest: usize) {
        self.active.retain(|a| a.quest != quest);
        self.tracked = self.tracked.filter(|&t| t != quest);
    }

    /// A creature of kind `creature` was killed: every kill goal for it that is not met yet moves on.
    pub fn killed(&mut self, creature: usize) -> Vec<Progress> {
        let mut steps = Vec::new();
        for active in &mut self.active {
            for (goal, g) in content::quest(active.quest).goals.iter().enumerate() {
                if let Goal::Kill { creature: c, count } = *g {
                    if c == creature && active.kills[goal] < count {
                        active.kills[goal] += 1;
                        steps.push(Progress { quest: active.quest, goal, have: active.kills[goal], need: count });
                    }
                }
            }
        }
        steps
    }

    /// The character stands at (north, west) of `map`: every walk to a place that is within reach moves on.
    pub fn arrived(&mut self, map: &str, north: f32, west: f32) -> Vec<Progress> {
        let mut steps = Vec::new();
        for active in &mut self.active {
            for (goal, g) in content::quest(active.quest).goals.iter().enumerate() {
                if let Goal::Reach { map: m, at, radius } = *g {
                    if active.kills[goal] == 0 && m.eq_ignore_ascii_case(map) && (at.0 - north).hypot(at.1 - west) <= radius {
                        active.kills[goal] = 1;
                        steps.push(Progress { quest: active.quest, goal, have: 1, need: 1 });
                    }
                }
            }
        }
        steps
    }

    /// Turn in an active, complete quest: the items it asks for are taken, the reward is given (with the one of its
    /// `choice` items that was picked, which must be given when there are any and only then). The pack grows to
    /// hold the reward, so this is never refused.
    pub fn turn_in(&mut self, quest: usize, inv: &mut Inventory, choice: Option<usize>) -> Result<(), Refusal> {
        assert!(self.complete(quest, inv), "only a complete quest can be turned in");
        let q = content::quest(quest);
        assert_eq!(choice.is_some(), !q.choice.is_empty(), "a reward is chosen when there is a choice");
        let picked = choice.map(|i| q.choice[i]);
        for g in &q.goals {
            if let Goal::Collect { item, count } = *g {
                inv.remove(item, count);
            }
        }
        for stack in picked.iter().chain(&q.items) {
            inv.add(*stack);
        }
        inv.money += q.copper;
        self.abandon(quest);
        self.done.push(quest);
        Ok(())
    }

    /// What the creature shows for the quests it gives or takes back.
    pub fn marker(&self, creature: usize, inv: &Inventory, who: Who) -> Option<Marker> {
        if self.taken_by(creature, inv).iter().any(|&(_, ready)| ready) {
            Some(Marker::Complete)
        } else if !self.offered_by(creature, who).is_empty() {
            Some(Marker::Available)
        } else if !self.taken_by(creature, inv).is_empty() {
            Some(Marker::Incomplete)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::Stack;

    const WHO: Who = Who { level: 2, race: 1 };

    /// Deputy Willem, who sends new arrivals to Marshal McBride.
    fn willem() -> usize {
        content::creature_index("c823")
    }

    #[test]
    fn a_quest_is_offered_taken_progressed_completed_and_turned_in_with_its_reward() {
        let (threat, kobolds) = (content::quest_index("q783"), content::quest_index("q7"));
        let (mcbride, vermin) = (content::creature_index("c197"), content::creature_index("c6"));
        let mut log = QuestLog::default();
        let mut inv = Inventory::new();
        // Only the first quest of the chain is offered, and the giver shows the "!".
        assert_eq!(log.offered_by(willem(), WHO), vec![threat]);
        assert_eq!(log.marker(willem(), &inv, WHO), Some(Marker::Available));
        assert!(log.accept(threat) && !log.accept(threat), "a quest is taken once");
        // Taken: nothing more to give, no grey "?" at the one who gave it, and a quest without goals is done as soon as it is taken, at the one who takes it back.
        assert_eq!((log.offered_by(willem(), WHO), log.marker(willem(), &inv, WHO)), (vec![], None));
        assert_eq!(log.taken_by(mcbride, &inv), vec![(threat, true)]);
        assert_eq!(log.marker(mcbride, &inv, WHO), Some(Marker::Complete));
        assert_eq!(log.turn_in(threat, &mut inv, None), Ok(()));
        // The next ones are offered now: the marshal's first task, and Willem's others.
        assert_eq!(log.offered_by(mcbride, WHO), vec![kobolds]);
        assert!(log.offered_by(willem(), WHO).len() >= 2 && !log.offered_by(willem(), WHO).contains(&threat));
        assert!(log.accept(kobolds));
        assert_eq!(log.marker(mcbride, &inv, WHO), Some(Marker::Incomplete));
        // Ten kobolds are needed; a kill of another kind or too many kills change nothing.
        assert!(!log.complete(kobolds, &inv));
        assert_eq!(log.killed(willem()), vec![]);
        for have in 1..=10 {
            assert_eq!(log.killed(vermin), vec![Progress { quest: kobolds, goal: 0, have, need: 10 }]);
        }
        assert!(log.killed(vermin).is_empty(), "the goal is met: more kills do not count");
        assert!(log.complete(kobolds, &inv) && log.marker(mcbride, &inv, WHO) == Some(Marker::Complete));
        let money = inv.money;
        assert_eq!(log.turn_in(kobolds, &mut inv, None), Ok(()));
        assert_eq!(inv.money, money + 25);
        assert!(log.active.is_empty() && log.done == vec![threat, kobolds]);
    }

    #[test]
    fn a_turn_in_with_a_full_pack_grows_it_and_abandoning_gives_the_quest_back() {
        let (threat, meat_quest) = (content::quest_index("q783"), content::quest_index("q33"));
        let meat = crate::items::find("tough_wolf_meat");
        let mut log = QuestLog::default();
        log.done.extend(["q783", "q5261"].map(content::quest_index));
        assert!(log.accept(meat_quest));
        // A full pack whose meat stack keeps its slot when eight are taken: the shoes go in a new row.
        let mut inv = Inventory::new();
        inv.bag = vec![Some(Stack { item: crate::items::find("hearthstone"), count: 1, enchant: 0 }); crate::items::BACKPACK_SIZE];
        inv.bag[15] = Some(Stack { item: meat, count: 10, enchant: 0 });
        assert_eq!(log.turn_in(meat_quest, &mut inv, Some(0)), Ok(()));
        assert_eq!((inv.count(crate::items::find("i80")), inv.count(meat)), (1, 2));
        assert!(inv.bag.len() > crate::items::BACKPACK_SIZE && log.done.contains(&meat_quest));
        // Abandoning gives a quest back.
        let mut log = QuestLog::default();
        assert!(log.accept(threat));
        assert_eq!(log.tracked, Some(threat));
        log.abandon(threat);
        assert_eq!(log.tracked, None);
        assert_eq!(log.offered_by(willem(), WHO), vec![threat]);
    }

    #[test]
    fn an_item_that_begins_a_quest_is_wanted_until_the_quest_is_taken_and_opens_it() {
        let c = content::get();
        let (&item, &quest) = c.item_quests.iter().find(|(&i, &q)| c.quests[q].requires.is_none() && !c.quests[q].goals.iter().any(|g| matches!(g, Goal::Collect { item, .. } if *item == i))).expect("a quest begun by an item");
        // Of a race it is for, and high enough.
        let who = Who { level: 60, race: if c.quests[quest].races == 0 { 1 } else { 1 << c.quests[quest].races.trailing_zeros() } };
        let (mut log, inv) = (QuestLog::default(), Inventory::new());
        assert!(log.wanted(&inv).contains(&item), "what drops it is let drop it");
        assert_eq!(log.begun_by(item, who), Some(quest));
        assert!(c.giving.values().flatten().all(|&q| q != quest), "nobody offers it");
        log.accept(quest);
        assert!(!log.wanted(&inv).contains(&item) && log.begun_by(item, who).is_none());
    }

    #[test]
    fn a_walk_to_a_place_is_done_when_the_character_gets_there() {
        let c = content::get();
        let (quest, goal, map, at, radius) = c.quests.iter().enumerate().find_map(|(i, q)| q.goals.iter().enumerate().find_map(|(n, g)| if let Goal::Reach { map, at, radius } = *g { Some((i, n, map, at, radius)) } else { None })).expect("a walk to a place");
        let mut log = QuestLog::default();
        assert!(log.accept(quest));
        // Too far, or in another map: nothing.
        assert!(log.arrived(map, at.0 + radius + 5.0, at.1).is_empty() && log.arrived("Nowhere", at.0, at.1).is_empty());
        assert_eq!(log.arrived(map, at.0, at.1 + radius * 0.5).len(), 1);
        assert_eq!(log.have(quest, goal, &Inventory::new()), 1);
        assert!(log.arrived(map, at.0, at.1).is_empty(), "it is done once");
    }

    #[test]
    fn the_tracked_quest_leads_to_the_creatures_of_its_open_goal_and_then_to_the_turn_in() {
        let meat_quest = content::quest_index("q33");
        let (wolf, eagan) = (content::creature_index("c299"), content::creature_index("c196"));
        let mut log = QuestLog::default();
        let mut inv = Inventory::new();
        log.accept(meat_quest);
        let meat = crate::items::find("tough_wolf_meat");
        let lead = log.lead(meat_quest, &inv);
        assert!(lead.kinds.contains(&wolf) && lead.item == Some(meat) && lead.icon == Icon::Skull, "whatever drops the meat, or holds it in a chest");
        inv.add(Stack { item: meat, count: 8, enchant: 0 });
        assert_eq!(log.lead(meat_quest, &inv), Lead { kinds: vec![eagan], item: None, icon: Icon::TurnIn, places: Vec::new() }, "then the turn in");
    }
}
