//! Making things: the work of the trades. The trade skill window asks for a recipe a number of times; each time is a cast (the recipe's own time, with the
//! stance, the tool in the hand and the sound that the original's spell shows) at the station it asks for, with the tools carried and the reagents in the
//! pack. At the end the reagents are used up, the item is in the pack and the skill may rise by how hard the recipe is for its rank.

use super::*;
use crate::loot::random;
use crate::recipes;
use crate::skills::Skill;

impl State {
    /// The window asked to make recipe `r` (a place in the content) `times` times.
    pub(super) fn start_craft(&mut self, r: usize, times: u32) {
        if self.gather.is_some() {
            if let Some(panels) = &mut self.panels {
                panels.error("Another action is in progress.");
            }
            return;
        }
        self.begin_make(r, times.max(1));
    }

    /// Check what the recipe asks for and begin one cast of it (`more` times left, this one included). False if it cannot be done (the window says why).
    fn begin_make(&mut self, r: usize, more: u32) -> bool {
        let recipe = &content::get().recipes[r];
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        let Some(panels) = &mut self.panels else { return false };
        let refuse = if !panels.skills.knows(recipe.skill) {
            Some(format!("You have not learned {}. A teacher in a town can teach it.", recipe.skill.name()))
        } else if !panels.recipes.knows(recipe) {
            Some(panels.string("SPELL_FAILED_NOT_KNOWN"))
        } else if let Some(tool) = recipes::missing_tool(&panels.inv, recipe) {
            Some(panels.string("SPELL_FAILED_TOTEMS").replacen("%s", crate::items::get(tool).name, 1))
        } else if recipe.focus != 0 && !near_station(&self.stations, feet, recipe.focus) {
            Some(panels.string("SPELL_FAILED_REQUIRES_SPELL_FOCUS").replacen("%s", recipes::station_name(recipe.focus), 1))
        } else if recipe.item.is_none() && recipes::target(&panels.inv, recipe).is_none() {
            Some("You are not wearing anything this fits.".to_string())
        } else if recipes::makeable(&panels.inv, recipe) == 0 {
            let missing = recipe.reagents.iter().find(|&&(item, n)| panels.inv.count(item) < n).map_or("", |&(item, _)| crate::items::get(item).name);
            Some(panels.string("SPELL_FAILED_REAGENTS").replacen("%s", missing, 1))
        } else {
            None
        };
        if let Some(why) = refuse {
            panels.error(&why);
            return false;
        }
        let seconds = recipe.cast as f32 / 1000.0;
        if seconds > 0.0 {
            panels.cast_start(&recipe.name, seconds);
        }
        // The original's work of the spell: the stance, the tool in the hand, the sound.
        let vis = crate::visuals::spell(&self.mount, recipe.spell);
        self.in_ctx(|ctx| {
            ctx.player.dismount();
        });
        let glows = self.in_ctx(|ctx| crate::casting::Spellcaster::gather_begin(ctx, &vis, feet)).unwrap_or_default();
        self.gather = Some(Gather { job: Job::Make { recipe: r, more }, left: seconds.max(0.05), glows });
        true
    }

    /// The pack item in `slot` was picked for Disenchant: the character casts the original's spell for a few seconds, and then the item is broken.
    pub(super) fn start_disenchant(&mut self, slot: usize) {
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        let Some(panels) = &mut self.panels else { return };
        if self.gather.is_some() || self.moving {
            return panels.error("Another action is in progress.");
        }
        panels.cast_start("Disenchant", crate::disenchant::SECONDS);
        let vis = crate::visuals::spell(&self.mount, crate::disenchant::SPELL);
        let glows = self.in_ctx(|ctx| crate::casting::Spellcaster::gather_begin(ctx, &vis, feet)).unwrap_or_default();
        self.gather = Some(Gather { job: Job::Disenchant(slot), left: crate::disenchant::SECONDS, glows });
    }

    /// The cast of Disenchant ended: the item is broken and what it gave is in the pack.
    pub(super) fn finish_disenchant(&mut self, slot: usize) {
        let mut rng = (self.start.elapsed().as_nanos() as u32) | 1;
        let Some(panels) = &mut self.panels else { return };
        panels.cast_stop();
        if let Some(loot) = panels.finish_disenchant(slot, &mut rng).filter(|l| !l.is_empty()) {
            panels.open_loot(FISH_LOOT, loot);
            self.set_grab(false);
        }
    }

    /// The cast of a recipe ended: it is made, the skill may rise, and the next one begins if the window asked for more.
    pub(super) fn finish_make(&mut self, r: usize, more: u32) {
        let recipe = &content::get().recipes[r];
        let Some(panels) = &mut self.panels else { return };
        panels.cast_stop();
        let mut rng = (self.start.elapsed().as_nanos() as u32) | 1;
        let more = if recipe.item.is_none() {
            // An enchantment goes on the worn item that fits it, once.
            let Some(slot) = recipes::target(&panels.inv, recipe) else {
                return panels.error("You are not wearing anything this fits.");
            };
            if !recipes::spend(&mut panels.inv, recipe) {
                return panels.error("Missing reagent.");
            }
            panels.enchant_slot(slot, recipe.enchant);
            let (worn, enchant) = (panels.inv.equipment[slot].map_or("", |s| crate::items::get(s.item).name), content::get().enchants.get(&recipe.enchant).map_or("", |e| e.name.as_str()));
            let text = format!("You enchant your {worn}: {enchant}.");
            panels.notice(&text);
            1
        } else {
            let Some(made) = recipes::make(&mut panels.inv, recipe, &mut rng) else {
                return panels.error("Missing reagent.");
            };
            let name = crate::items::get(made.item).name;
            let text = if made.count > 1 { panels.string("LOOT_ITEM_CREATED_SELF_MULTIPLE").replacen("%s", name, 1).replacen("%d", &made.count.to_string(), 1) } else { panels.string("LOOT_ITEM_CREATED_SELF").replacen("%s", name, 1) };
            panels.notice(&text);
            more
        };
        let roll = random(&mut rng, 10_000) as f32 / 10_000.0;
        panels.skill_up(recipe.skill, (recipe.ranks.1, recipe.ranks.2, recipe.ranks.3), roll);
        panels.refresh();
        if more > 1 {
            self.begin_make(r, more - 1);
        }
    }

    /// Begin to skin the corpse of creature `i`: a knife has to be carried and the rank of skinning has to be high enough for the beast.
    pub(super) fn start_skin(&mut self, i: usize) {
        if self.gather.is_some() {
            return;
        }
        let (Some(wolves), Some(panels)) = (&self.wolves, &mut self.panels) else { return };
        let Some((_, level)) = wolves.skin_of(i) else { return };
        let required = skinning_rank(level);
        if !panels.must_know(Skill::Skinning) {
            return;
        }
        if panels.skills.rank(Skill::Skinning) < required {
            let text = panels.string("SPELL_FAILED_MIN_SKILL").replacen("%s", Skill::Skinning.name(), 1).replacen("%d", &required.to_string(), 1);
            return panels.error(&text);
        }
        if !panels.carries(SKINNING_KNIFE) {
            let text = panels.string("SPELL_FAILED_TOTEMS").replacen("%s", "Skinning Knife", 1);
            return panels.error(&text);
        }
        panels.cast_start("Skinning", SKINNING_SECONDS);
        let (vis, feet) = (crate::visuals::spell(&self.mount, SKINNING_SPELL), self.camera.pos - Vec3::Y * EYE_HEIGHT);
        let glows = self.in_ctx(|ctx| crate::casting::Spellcaster::gather_begin(ctx, &vis, feet)).unwrap_or_default();
        self.gather = Some(Gather { job: Job::Skin(i), left: SKINNING_SECONDS, glows });
    }

    /// The knife is done: the hide is on the corpse's loot (a window), the skill may rise.
    pub(super) fn finish_skin(&mut self, i: usize) {
        let (Some(wolves), Some(panels)) = (&mut self.wolves, &mut self.panels) else { return };
        panels.cast_stop();
        let Some((table, level)) = wolves.skin_of(i) else { return };
        wolves.mark_skinned(i);
        let mut rng = (self.start.elapsed().as_nanos() as u32) | 1;
        let mut loot = content::drop_table(table).map(|t| t.roll(&mut rng)).unwrap_or_default();
        loot.only_wanted(&panels.quests.wanted(&panels.inv));
        panels.gather_skill(Skill::Skinning, skinning_rank(level), random(&mut rng, 10_000) as f32 / 10_000.0);
        if !loot.is_empty() {
            panels.open_loot(i, loot);
            self.set_grab(false);
        }
    }
}

/// The rank of skinning a beast of this level asks for (the server's rule: none up to level 9, then ten a level to level 19, then five a level).
fn skinning_rank(level: u32) -> u32 {
    match level {
        0..=9 => 0,
        10..=19 => (level - 10) * 10,
        _ => level * 5,
    }
}

/// The skinning knife the spell of skinning asks to carry, the spell of the original (its stance and sound) and how long the work takes, seconds.
const SKINNING_KNIFE: &str = "i7005";
const SKINNING_SPELL: u32 = 8613;
const SKINNING_SECONDS: f32 = 2.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_beasts_ask_for_the_rank_of_skinning_the_server_gives_them() {
        assert_eq!([skinning_rank(1), skinning_rank(9), skinning_rank(10), skinning_rank(15), skinning_rank(20), skinning_rank(60)], [0, 0, 0, 50, 100, 300]);
    }
}
