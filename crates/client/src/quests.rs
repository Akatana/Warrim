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

/// What the compass and the map point to for a quest: kinds of creatures (indexes) with their picture, and an item that chests may hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Lead {
    pub kinds: Vec<usize>,
    pub item: Option<usize>,
    pub icon: Icon,
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
        let open = q.goals.iter().enumerate().find(|&(i, g)| self.have(quest, i, inv) < match *g { Goal::Kill { count, .. } | Goal::Collect { count, .. } => count }).map(|(_, g)| *g);
        match open {
            None => Lead { kinds: vec![q.turn_in], item: None, icon: Icon::TurnIn },
            Some(Goal::Kill { creature, .. }) => Lead { kinds: vec![creature], item: None, icon: Icon::Skull },
            Some(Goal::Collect { item, .. }) => {
                let drops = kinds(&|cr| cr.loot.as_ref().is_some_and(|t| c.loot[t].items.iter().any(|d| d.0 == item)));
                if drops.is_empty() {
                    Lead { kinds: kinds(&|cr| cr.vendor.contains(&item)), item: Some(item), icon: Icon::Vendor }
                } else {
                    Lead { kinds: drops, item: Some(item), icon: Icon::Skull }
                }
            }
        }
    }

    /// The items that the quests being worked on still want: those of their collect goals that are not all in the bags yet.
    pub fn wanted(&self, inv: &Inventory) -> Vec<usize> {
        self.active.iter().flat_map(|a| &content::quest(a.quest).goals).filter_map(|g| if let Goal::Collect { item, count } = *g { Some((item, count)) } else { None }).filter(|&(item, count)| inv.count(item) < count).map(|(item, _)| item).collect()
    }

    pub fn is_active(&self, quest: usize) -> bool {
        self.active.iter().any(|a| a.quest == quest)
    }

    /// The quests creature `creature` can give: not taken or done yet, with what they require done.
    pub fn offered_by(&self, creature: usize) -> Vec<usize> {
        let quests = &content::get().quests;
        (0..quests.len()).filter(|&q| quests[q].giver == creature && !self.is_active(q) && !self.done.contains(&q) && quests[q].requires.is_none_or(|r| self.done.contains(&r))).collect()
    }

    /// The quests being worked on that creature `creature` takes back, each with whether it can be turned in now.
    pub fn taken_by(&self, creature: usize, inv: &Inventory) -> Vec<(usize, bool)> {
        self.active.iter().filter(|a| content::quest(a.quest).turn_in == creature).map(|a| (a.quest, self.complete(a.quest, inv))).collect()
    }

    /// How many of goal `goal` of an active quest are done.
    pub fn have(&self, quest: usize, goal: usize, inv: &Inventory) -> u32 {
        let Some(active) = self.active.iter().find(|a| a.quest == quest) else { return 0 };
        match content::quest(quest).goals[goal] {
            Goal::Kill { count, .. } => active.kills[goal].min(count),
            Goal::Collect { item, count } => inv.count(item).min(count),
        }
    }

    /// Whether every goal of an active quest is met.
    pub fn complete(&self, quest: usize, inv: &Inventory) -> bool {
        self.is_active(quest) && content::quest(quest).goals.iter().enumerate().all(|(i, g)| {
            let need = match *g {
                Goal::Kill { count, .. } | Goal::Collect { count, .. } => count,
            };
            self.have(quest, i, inv) >= need
        })
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
    pub fn marker(&self, creature: usize, inv: &Inventory) -> Option<Marker> {
        if self.taken_by(creature, inv).iter().any(|&(_, ready)| ready) {
            Some(Marker::Complete)
        } else if !self.offered_by(creature).is_empty() {
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

    fn willem() -> usize {
        content::creature_index("deputy_willem")
    }

    #[test]
    fn a_quest_is_offered_taken_progressed_completed_and_turned_in_with_its_reward() {
        let (wolves, word) = (content::quest_index("wolves_in_the_valley"), content::quest_index("a_word_with_the_trader"));
        let mut log = QuestLog::default();
        let mut inv = Inventory::new();
        // Only the first quest of the chain is offered, and the giver shows the "!".
        assert_eq!(log.offered_by(willem()), vec![wolves]);
        assert_eq!(log.marker(willem(), &inv), Some(Marker::Available));
        assert!(log.accept(wolves) && !log.accept(wolves), "a quest is taken once");
        // Taken but not done: nothing more to give, and the grey "?" at the one who takes it back.
        assert_eq!((log.offered_by(willem()), log.marker(willem(), &inv)), (vec![], Some(Marker::Incomplete)));
        assert_eq!(log.marker(content::creature_index("trader_aldric"), &inv), None);
        // Two kills and a piece of meat are needed; a kill of another kind or too many kills change nothing.
        assert!(!log.complete(wolves, &inv));
        assert_eq!(log.killed(content::creature_index("deputy_willem")), vec![]);
        let wolf = content::creature_index("wolf");
        assert_eq!(log.killed(wolf), vec![Progress { quest: wolves, goal: 0, have: 1, need: 2 }]);
        assert_eq!(log.killed(wolf).len(), 1);
        assert!(log.killed(wolf).is_empty(), "the goal is met: more kills do not count");
        assert!(!log.complete(wolves, &inv), "the meat is still missing");
        let meat = crate::items::find("tough_wolf_meat");
        assert!(inv.add(Stack { item: meat, count: 3 }));
        assert!(log.complete(wolves, &inv) && log.marker(willem(), &inv) == Some(Marker::Complete));
        // The turn in takes one meat (not all), pays copper and gives the linen.
        let linen = crate::items::find("linen_cloth");
        let (linen_before, money) = (inv.count(linen), inv.money);
        // One of two rewards is chosen, and only the chosen one is given.
        assert_eq!(log.turn_in(wolves, &mut inv, Some(1)), Ok(()));
        assert_eq!((inv.count(meat), inv.count(linen), inv.money), (2, linen_before + 2, money + 100));
        // Of the two (the original's rewards of Wolves Across the Border) the second was chosen: only it is given.
        assert_eq!((inv.count(crate::items::find("i80")), inv.count(crate::items::find("i6070"))), (0, 1));
        assert!(log.active.is_empty() && log.done == vec![wolves]);
        // The next one of the chain is offered now; it is taken to somebody else.
        assert_eq!(log.offered_by(willem()), vec![word]);
        assert!(log.accept(word));
        let aldric = content::creature_index("trader_aldric");
        assert_eq!(log.taken_by(aldric, &inv), vec![(word, true)], "a quest without goals is done as soon as it is taken");
        assert_eq!(log.marker(aldric, &inv), Some(Marker::Complete));
    }

    #[test]
    fn a_turn_in_with_a_full_pack_grows_it_and_abandoning_gives_the_quest_back() {
        let wolves = content::quest_index("wolves_in_the_valley");
        let (wolf, meat) = (content::creature_index("wolf"), crate::items::find("tough_wolf_meat"));
        let mut log = QuestLog::default();
        log.accept(wolves);
        log.killed(wolf);
        log.killed(wolf);
        // A full pack whose meat stack keeps its slot when one is taken: the jerkin and the linen go in a new row.
        let mut inv = Inventory::new();
        inv.bag = vec![Some(Stack { item: crate::items::find("hearthstone"), count: 1 }); crate::items::BACKPACK_SIZE];
        inv.bag[15] = Some(Stack { item: meat, count: 3 });
        assert_eq!(log.turn_in(wolves, &mut inv, Some(0)), Ok(()));
        assert_eq!((inv.count(crate::items::find("i80")), inv.count(crate::items::find("linen_cloth"))), (1, 2));
        assert!(inv.bag.len() > crate::items::BACKPACK_SIZE && log.done == vec![wolves]);
        // Abandoning gives a quest back.
        let mut log = QuestLog::default();
        assert!(log.accept(wolves));
        assert_eq!(log.tracked, Some(wolves));
        log.abandon(wolves);
        assert_eq!(log.tracked, None);
        assert_eq!(log.offered_by(willem()), vec![wolves]);
    }

    #[test]
    fn the_tracked_quest_leads_to_the_creatures_of_its_open_goal_and_then_to_the_turn_in() {
        let wolves = content::quest_index("wolves_in_the_valley");
        let (wolf, willem) = (content::creature_index("wolf"), content::creature_index("deputy_willem"));
        let mut log = QuestLog::default();
        let mut inv = Inventory::new();
        log.accept(wolves);
        assert_eq!(log.lead(wolves, &inv), Lead { kinds: vec![wolf], item: None, icon: Icon::Skull }, "wolves first");
        log.killed(wolf);
        log.killed(wolf);
        let meat = crate::items::find("tough_wolf_meat");
        let lead = log.lead(wolves, &inv);
        assert!(lead.kinds.contains(&wolf) && lead.item == Some(meat) && lead.icon == Icon::Skull, "then whatever drops the meat, or holds it in a chest");
        inv.add(Stack { item: meat, count: 3 });
        assert_eq!(log.lead(wolves, &inv), Lead { kinds: vec![willem], item: None, icon: Icon::TurnIn }, "then the turn in");
    }
}
