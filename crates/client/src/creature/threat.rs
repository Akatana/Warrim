//! Whom a creature fights: the threat list of the original (`ThreatManager`, `ThreatContainer::selectNextVictim`). Everything that deals damage to a creature, the player
//! and his pets, raises its threat by what it dealt; the creature goes for the one with the most, and changes to another only when that one has far more.

/// Someone on a threat list: the player (`None`) or the pet that is creature `n` of the world.
pub type Who = Option<usize>;

/// What the creature may change its mind by: another must have this share more than the one it fights to be taken at once (`1.3`), or this share more
/// (`1.1`) and be within the reach of a blow.
const SWITCH_FAR: f32 = 1.3;
const SWITCH_NEAR: f32 = 1.1;

#[derive(Clone, Debug, Default)]
pub struct Threat {
    list: Vec<(Who, f32)>,
    victim: Option<Who>,
}

impl Threat {
    /// Someone dealt `amount` (or made the creature notice him with 0).
    pub fn add(&mut self, who: Who, amount: f32) {
        match self.list.iter_mut().find(|t| t.0 == who) {
            Some(t) => t.1 += amount,
            None => self.list.push((who, amount)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The fight is over.
    pub fn clear(&mut self) {
        (self.list.clear(), self.victim = None);
    }

    /// Whom the creature fights now, if it fights anyone: a creature that has noticed no one yet has no list and goes for the player.
    pub fn victim(&self) -> Who {
        self.victim.unwrap_or(None)
    }

    /// Only those `alive` stay on the list (a pet that died, or is out of phase, is gone from it).
    pub fn keep(&mut self, alive: impl Fn(usize) -> bool) {
        self.list.retain(|t| t.0.is_none_or(&alive));
        if self.victim.is_some_and(|v| !self.list.iter().any(|t| t.0 == v)) {
            self.victim = None;
        }
    }

    /// `selectNextVictim`: the list in order of threat; the one it fights stays unless another has more than 110% of its threat and is within a blow's reach (`in_reach`), or more than
    /// 130%; a player is taken over a pet. Returns whom it fights.
    pub fn select(&mut self, in_reach: impl Fn(Who) -> bool) -> Who {
        let mut order: Vec<(Who, f32)> = self.list.clone();
        order.sort_by(|a, b| b.1.total_cmp(&a.1));
        let Some(current) = self.victim.and_then(|v| order.iter().find(|t| t.0 == v).copied()) else {
            self.victim = order.first().map(|t| t.0);
            return self.victim();
        };
        for &(who, threat) in &order {
            if who == current.0 {
                break;
            }
            if who.is_none() && current.0.is_some() || threat > SWITCH_FAR * current.1 || (threat > SWITCH_NEAR * current.1 && in_reach(who)) {
                self.victim = Some(who);
                break;
            }
            // (The list is in order: nothing below this one has more, and the current one stays.)
            if threat <= SWITCH_NEAR * current.1 {
                break;
            }
        }
        self.victim()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_goes_for_the_most_hated_and_changes_only_for_much_more() {
        let mut t = Threat::default();
        // Noticed by the player alone: the player.
        t.add(None, 0.0);
        assert_eq!(t.select(|_| false), None);
        // The player has dealt 100, the pet 105: not 110%, so the player stays the one it fights.
        t.add(None, 100.0);
        t.add(Some(7), 105.0);
        assert_eq!(t.select(|_| true), None);
        // 112 is over 110% and the pet is within reach of a blow: it changes to the pet; from afar it needs 130%.
        t.add(Some(7), 7.0);
        assert_eq!(t.select(|_| false), None, "not 130% from afar");
        assert_eq!(t.select(|_| true), Some(7), "110% within reach");
        // The player has as much again: a player is taken over a pet as soon as he has more than it (the pet comes later on the list).
        t.add(None, 13.0);
        assert_eq!(t.select(|_| false), None);
    }

    #[test]
    fn with_no_one_yet_it_takes_the_most_and_130_percent_is_enough_from_afar() {
        let mut t = Threat::default();
        t.add(Some(1), 100.0);
        t.add(Some(2), 120.0);
        assert_eq!(t.select(|_| false), Some(2));
        t.add(Some(1), 40.0); // 140: over 110% of 120, not 130%
        assert_eq!(t.select(|_| false), Some(2), "not from afar");
        assert_eq!(t.select(|w| w == Some(1)), Some(1), "within reach, 110% is enough");
        t.add(Some(2), 100.0); // 220 against 140: over 130%
        assert_eq!(t.select(|_| false), Some(2));
    }

    #[test]
    fn those_gone_leave_the_list() {
        let mut t = Threat::default();
        t.add(None, 1.0);
        t.add(Some(3), 50.0);
        assert_eq!(t.select(|_| true), Some(3));
        t.keep(|n| n != 3);
        assert_eq!(t.select(|_| true), None, "the pet died: back to the player");
    }
}
