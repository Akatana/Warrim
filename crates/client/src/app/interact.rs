//! What the player does to the world: the interact key, clicks on creatures and chests, lockpicking.

use super::*;

impl State {
    /// Open or close the character sheet or the backpack; the mouse is free while one is open.
    pub(super) fn toggle_panel(&mut self, character: bool) {
        let Some(panels) = &mut self.panels else { return };
        if character {
            panels.toggle_character_tab(false);
        } else {
            panels.toggle_backpack();
        }
        let open = panels.any_open();
        self.set_grab(!open);
    }

    /// The interact key: loot the nearest corpse in reach, or else talk to the nearest friendly creature. The mouse
    /// is free while a window is open.
    pub(super) fn interact(&mut self) {
        if self.panels.as_ref().is_none_or(|p| p.any_open()) || self.lock.is_some() {
            return;
        }
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        let corpse = self.wolves.as_ref().and_then(|w| w.lootable_near(feet, LOOT_RANGE));
        let chest = self.chests.as_ref().and_then(|c| c.near(feet, OPEN_RANGE));
        let pickup = self.pickups.as_ref().zip(self.panels.as_ref()).and_then(|(p, panels)| p.near(feet, OPEN_RANGE, &panels.quests.wanted(&panels.inv)));
        let talk = self.wolves.as_ref().and_then(|w| w.talkable_near(feet, TALK_RANGE));
        // Loose props are carried for as long as the key is held (let go in `input.rs`); the key repeating while one is held does nothing else.
        if self.bodies.as_ref().is_some_and(|b| b.holding()) {
            return;
        }
        let body = self.bodies.as_ref().and_then(|b| b.near(feet, crate::physics::GRAB_RANGE));
        if corpse.is_none() && chest.is_none() && pickup.is_none() && body.is_none() && near_bed(&self.beds, &self.world, feet) {
            if let Some(panels) = &mut self.panels {
                panels.ask_sleep();
            }
            self.set_grab(false);
            return;
        }
        if let Some(i) = corpse {
            self.loot_corpse(i);
        } else if let Some(i) = chest {
            self.use_chest(i);
        } else if let Some(i) = pickup {
            self.use_pickup(i);
        } else if let Some(i) = talk {
            self.talk_to(i);
        } else if let (Some(i), Some(bodies)) = (body, &mut self.bodies) {
            bodies.grab(i);
        }
    }

    /// Open the loot window of corpse `i`.
    pub(super) fn loot_corpse(&mut self, i: usize) {
        let (Some(wolves), Some(panels)) = (&self.wolves, &mut self.panels) else { return };
        if let Some(loot) = wolves.loot(i).cloned() {
            panels.open_loot(i, loot);
            self.set_grab(false);
        }
    }

    /// Talk to creature `i` (which turns to face the one who speaks to him), if it has something to say.
    pub(super) fn talk_to(&mut self, i: usize) {
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        let (Some(wolves), Some(panels)) = (&mut self.wolves, &mut self.panels) else { return };
        let Some(def) = wolves.def(i).filter(|d| d.gossip.is_some()) else { return };
        wolves.face(i, feet);
        wolves.set_target(i);
        panels.open_gossip(i, def);
        self.set_grab(false);
    }

    /// Pick up prop `i`: its item goes into the bag (the quest that wants it counts it), and the prop is gone for a while.
    pub(super) fn use_pickup(&mut self, i: usize) {
        let (Some(pickups), Some(panels)) = (&mut self.pickups, &mut self.panels) else { return };
        if let Some(item) = pickups.take(i) {
            panels.inv.add(crate::items::Stack { item, count: 1 });
            let text = panels.string("LOOT_ITEM_PUSHED_SELF").replacen("%s", crate::items::get(item).name, 1);
            panels.notice(&text);
            panels.sound("ChestOpen");
        }
    }

    /// Use chest `i`: a locked one is picked (on a sweet spot nobody tells) with a lockpick, one that is not opens at once into the
    /// loot window.
    pub(super) fn use_chest(&mut self, i: usize) {
        let (Some(chests), Some(panels)) = (&mut self.chests, &mut self.panels) else { return };
        if let Some(level) = chests.lock(i) {
            if panels.picks() == 0 {
                panels.error("This chest is locked. You need a lockpick.");
                return;
            }
            let sweet = (self.start.elapsed().subsec_nanos() % 1400) as f32 / 10.0 - 70.0;
            let mut lock = Lockpick::new(level, sweet);
            lock.window_factor = panels.skills.lock_window_factor();
            self.lock = Some((i, lock));
            (self.lock_turn, self.lock_time, self.input) = (false, 0.0, Input::default());
            return;
        }
        panels.sound("ChestOpen");
        if let Some(loot) = chests.open(i, &panels.quests.wanted(&panels.inv)) {
            panels.open_loot(CHEST_LOOT + i, loot);
            self.set_grab(false);
        }
    }

    /// The pointer for what is under it now, and what that is (for a click): over a window, or over a creature or chest in the world.
    pub(super) fn cursor_context(&mut self, camera: &Camera, size: (u32, u32)) -> (Kind, Option<Hover>) {
        let Some(pos) = self.cursor else { return (Kind::Point, None) };
        if let Some(kind) = self.panels.as_mut().and_then(|p| p.cursor_at(pos, size)) {
            return (kind, None);
        }
        if let Some((i, kind)) = self.wolves.as_ref().and_then(|w| w.pick(camera, size, pos)) {
            return (kind, Some(Hover::Creature(i)));
        }
        if let Some((i, kind)) = self.chests.as_ref().and_then(|c| c.pick(camera, size, pos)) {
            return (kind, Some(Hover::Chest(i)));
        }
        if let Some(i) = self.pickups.as_ref().zip(self.panels.as_ref()).and_then(|(p, panels)| p.pick(camera, size, pos, &panels.quests.wanted(&panels.inv))) {
            return (Kind::Interact, Some(Hover::Pickup(i)));
        }
        (Kind::Point, None)
    }

    /// A click in the world with the mouse free: the left button selects the creature under the pointer, the right one uses it
    /// (talk, loot, open) if it is near enough. False when the click was on nothing.
    pub(super) fn world_click(&mut self, right: bool) -> bool {
        let Some(hover) = self.hover else { return false };
        if !right {
            if let (Hover::Creature(i), Some(wolves)) = (hover, &mut self.wolves) {
                wolves.set_target(i);
                return true;
            }
            return false;
        }
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        let at = match hover {
            Hover::Creature(i) => self.wolves.as_ref().and_then(|w| w.corpse_pos(i)),
            Hover::Chest(i) => self.chests.as_ref().and_then(|c| c.pos(i)),
            Hover::Pickup(i) => self.pickups.as_ref().and_then(|p| p.pos(i)),
        };
        let Some(at) = at else { return false };
        let distance = Vec3::new(at.x - feet.x, 0.0, at.z - feet.z).length();
        let (range, far) = match hover {
            Hover::Creature(i) if self.wolves.as_ref().is_some_and(|w| w.loot(i).is_some()) => (LOOT_RANGE, "ERR_LOOT_TOO_FAR"),
            Hover::Creature(_) => (TALK_RANGE, "ERR_USE_TOO_FAR"),
            Hover::Chest(_) | Hover::Pickup(_) => (OPEN_RANGE, "ERR_USE_TOO_FAR"),
        };
        if self.panels.as_ref().is_none_or(|p| p.any_open()) || self.lock.is_some() {
            return true;
        }
        if distance > range {
            if let Some(p) = &mut self.panels {
                let text = p.string(far);
                p.error(&text);
            }
            return true;
        }
        match hover {
            Hover::Creature(i) if range == LOOT_RANGE => self.loot_corpse(i),
            Hover::Creature(i) => self.talk_to(i),
            Hover::Chest(i) => self.use_chest(i),
            Hover::Pickup(i) => self.use_pickup(i),
        }
        true
    }

    /// Stop picking the lock.
    pub(super) fn end_lock(&mut self) {
        (self.lock, self.lock_turn) = (None, false);
    }

    /// One step of lockpicking: the lock turns or falls back; a pick that breaks is gone (and with the last one the picking
    /// ends); a lock that turns all the way opens the chest and shows what is in it.
    pub(super) fn step_lock(&mut self, dt: f32) {
        let Some((chest, lock)) = &mut self.lock else { return };
        self.lock_time += dt;
        let (chest, event) = (*chest, lock.update(dt, self.lock_turn));
        let (Some(panels), Some(chests)) = (&mut self.panels, &mut self.chests) else { return };
        match event {
            Some(Event::Broke) => {
                panels.train(crate::skills::Skill::Lockpicking, 1.0);
                panels.sound("Item - Lockpick");
                panels.use_pick();
                if panels.picks() == 0 {
                    panels.error("Your last lockpick broke.");
                    self.end_lock();
                } else {
                    panels.notice("Your lockpick broke.");
                }
            }
            Some(Event::Opened) => {
                panels.train(crate::skills::Skill::Lockpicking, 3.0);
                panels.sound("ChestOpen");
                if let Some(loot) = chests.open(chest, &panels.quests.wanted(&panels.inv)) {
                    panels.open_loot(CHEST_LOOT + chest, loot);
                    self.end_lock();
                    self.set_grab(false);
                } else {
                    self.end_lock();
                }
            }
            None => {}
        }
    }
}
