//! Fishing: the line is cast with E at water in front, with a fishing pole in the hand (the stance, the sound and the line of the original's spell); a bobber lies on the
//! water, a fish bites after a while and the bobber dips for a few seconds: E then reels in what the water of the area holds. The rules are `fishing.rs`'s.

use super::*;
use crate::fishing::{self, BITE_SECONDS, CAST_RANGE, LINE_SECONDS};
use crate::loot::random;
use crate::skills::Skill;

/// The model of the bobber (the display of the fishing bobber object of the original), the spell of fishing (its work in the hands) and the sounds of
/// the original's table: the cast, a fish hooked, the reel in.
const BOBBER: &str = r"World\Goober\G_FishingBobber.m2";
const FISHING_SPELL: u32 = 7620;
const CAST_SOUND: u32 = 3354;
const HOOKED_SOUND: u32 = 3355;
/// The animations of the character: the cast of the line and the wait with it out.
use crate::animator::FISHING_CAST;
const FISHING_LOOP: u16 = 134;
/// Seconds to put on what has no cast time of its own in the spell table.
const LURE_SECONDS: f32 = 5.0;
const REEL_SOUND: u32 = 3407;

impl State {
    /// Where a line cast now would land (see `water_ahead_of`).
    pub(super) fn water_ahead(&self) -> Option<Vec3> {
        water_ahead_of(&self.camera, &self.world)
    }

    /// Whether a fishing pole is in the hand.
    pub(super) fn has_pole(&self) -> bool {
        self.panels.as_ref().is_some_and(|p| fishing::pole_in_hand(&p.inv))
    }

    /// Cast the line to `spot` (the surface of the water): the bobber lies there until a fish bites or the line is reeled in or runs out.
    pub(super) fn start_fishing(&mut self, spot: Vec3) {
        if self.gather.is_some() || self.moving {
            return;
        }
        let Some(panels) = &mut self.panels else { return };
        if !panels.must_know(Skill::Fishing) {
            return;
        }
        if self.player.as_mut().is_some_and(|p| p.draw_weapons()) {
            self.later = Some(Later::Fish(spot));
            return;
        }
        let Some(panels) = &mut self.panels else { return };
        panels.cast_start("Fishing", LINE_SECONDS);
        panels.emit(Sfx::Entry(CAST_SOUND));
        let (vis, feet) = (crate::visuals::spell(&self.mount, FISHING_SPELL), self.camera.pos - Vec3::Y * EYE_HEIGHT);
        let glows = self.in_ctx(|ctx| {
            // The pole is thrown out with the cast of the spell, and held while the line is out.
            let glows = crate::casting::Spellcaster::gather_begin(ctx, &vis, feet);
            ctx.player.cast_anim(vis.cast.anim.unwrap_or(FISHING_CAST));
            ctx.player.set_precast(Some(FISHING_LOOP));
            glows
        });
        let glows = glows.unwrap_or_default();
        let mut rng = (self.start.elapsed().as_nanos() as u32) | 1;
        let bite_in = fishing::bite_after(&mut rng);
        if let Some(model) = ModelCache::default().get(&self.mount, BOBBER) {
            let entity = self.renderer.add_entity(&self.gpu, &model);
            self.renderer.set_entity(entity, spot, Mat4::IDENTITY, true);
            self.bobber = Some(Bobber { entity, pos: spot, bite_in, bite_left: None, time: 0.0 });
        }
        self.gather = Some(Gather { job: Job::Fish, left: LINE_SECONDS, glows });
    }

    /// A lure (or a stone) is to go on the weapon: the character holds the pole, plays the original's work of its spell and the item is used when the cast is over.
    pub(super) fn start_lure(&mut self, item: usize) {
        if self.gather.is_some() || self.moving {
            return self.panels.as_mut().map_or((), |p| p.error("Another action is in progress."));
        }
        if self.player.as_mut().is_some_and(|p| p.draw_weapons()) {
            self.later = Some(Later::Lure(item));
            return;
        }
        let Some(panels) = &mut self.panels else { return };
        let def = crate::items::get(item);
        // The cast of the item's own spell.
        let seconds = def.spells.first().and_then(|s| content::classic_spell(s.spell)).map_or(LURE_SECONDS, |s| s.cast as f32 / 1000.0).max(0.1);
        panels.cast_start(def.name, seconds);
        let vis = def.spells.first().map(|s| crate::visuals::spell(&self.mount, s.spell)).unwrap_or_default();
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        let glows = self.in_ctx(|ctx| crate::casting::Spellcaster::gather_begin(ctx, &vis, feet));
        self.gather = Some(Gather { job: Job::Lure(item), left: seconds, glows: glows.unwrap_or_default() });
    }

    /// A second of the line: the bobber rides the water, and after a while a fish bites and the bobber dips; the bite is gone if nobody reels in.
    pub(super) fn step_bobber(&mut self, dt: f32) {
        let (Some(b), Some(panels)) = (&mut self.bobber, &mut self.panels) else { return };
        b.time += dt;
        match &mut b.bite_left {
            Some(left) => {
                *left -= dt;
                if *left <= 0.0 {
                    b.bite_left = None;
                    b.bite_in = f32::INFINITY;
                }
            }
            None => {
                b.bite_in -= dt;
                if b.bite_in <= 0.0 {
                    b.bite_left = Some(BITE_SECONDS);
                    panels.emit(Sfx::Entry(HOOKED_SOUND));
                }
            }
        }
        // It rides the swell a little, and when a fish has it, it is pulled under again and again.
        let lift = (b.time * 2.0).sin() * 0.04 - if b.bite_left.is_some() { 0.12 + 0.1 * (b.time * 18.0).sin().abs() } else { 0.0 };
        self.renderer.set_entity(b.entity, b.pos + Vec3::Y * lift, Mat4::IDENTITY, true);
    }

    /// Whether a fish is on the line now (for the hint).
    pub(super) fn fish_on(&self) -> bool {
        self.bobber.as_ref().is_some_and(|b| b.bite_left.is_some())
    }

    /// E while the line is out: reel in. A fish on it may be caught; with none the line only comes back.
    pub(super) fn reel_in(&mut self) {
        let Some(spot) = self.bobber.as_ref().map(|b| (b.pos, b.bite_left.is_some())) else { return };
        self.end_gather();
        let Some(panels) = &mut self.panels else { return };
        panels.cast_stop();
        panels.emit(Sfx::Entry(REEL_SOUND));
        if !spot.1 {
            return panels.error(&panels.string("ERR_FISH_NOT_HOOKED"));
        }
        let c = content::get();
        let Some(water) = self.world.area_at(spot.0).and_then(|a| c.fishing.get(&a)) else {
            return panels.error(&panels.string("SPELL_FAILED_NO_FISH"));
        };
        let mut rng = (self.start.elapsed().as_nanos() as u32) | 1;
        let rank = panels.skills.rank(Skill::Fishing);
        let roll = 1 + random(&mut rng, 100);
        if fishing::catch_chance(rank, water.skill) < roll {
            return panels.error(&panels.string("ERR_FISH_ESCAPED"));
        }
        let mut loot = content::drop_table(&water.table).map(|t| t.roll(&mut rng)).unwrap_or_default();
        loot.only_wanted(&panels.quests.wanted(&panels.inv));
        panels.raise_skill(Skill::Fishing, fishing::rise_chance(rank) as f32 / 100.0, random(&mut rng, 10_000) as f32 / 10_000.0);
        if !loot.is_empty() {
            panels.open_loot(FISH_LOOT, loot);
            self.set_grab(false);
        }
    }
}

/// Where a line cast from where `camera` stands would land: the first water in front, from a few yards to the reach of the line, at the surface.
pub(super) fn water_ahead_of(camera: &Camera, world: &World) -> Option<Vec3> {
    let feet = camera.pos - Vec3::Y * EYE_HEIGHT;
    let (sy, cy) = camera.yaw.sin_cos();
    let ahead = Vec3::new(sy, 0.0, -cy);
    let mut d = 5.0;
    while d <= CAST_RANGE {
        let p = feet + ahead * d;
        if let Some(level) = world.liquid_level(p.x, p.z) {
            // Water, not a puddle in a hollow: the ground is under it.
            if world.height_at(p.x, p.z).is_none_or(|ground| ground < level - 0.3) && (level - feet.y).abs() < 12.0 {
                return Some(Vec3::new(p.x, level, p.z));
            }
        }
        d += 0.5;
    }
    None
}
