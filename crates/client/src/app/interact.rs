//! What the player does to the world: the interact key, clicks on creatures and chests, lockpicking.

use super::*;

/// How long the interact key is held before it drags a corpse instead of looting it.
pub(super) const DRAG_HOLD: std::time::Duration = std::time::Duration::from_millis(350);

/// The cast of Mining and of Herb Gathering in the original (`Spell.dbc`), seconds, and the item the mining spell asks to carry (Mining Pick).
const MINING_SECONDS: f32 = 3.2;
const HERBALISM_SECONDS: f32 = 5.0;
const MINING_PICK: &str = "i2901";

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
        // With the line out, the key reels it in.
        if matches!(self.gather, Some(Gather { job: Job::Fish, .. })) {
            return self.reel_in();
        }
        if self.panels.as_ref().is_none_or(|p| p.any_open()) || self.lock.is_some() || self.gather.is_some() {
            return;
        }
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        let corpse = self.wolves.as_ref().and_then(|w| w.lootable_near(feet, LOOT_RANGE));
        let chest = self.chests.as_ref().and_then(|c| c.near(feet, OPEN_RANGE));
        let pickup = self.pickups.as_ref().zip(self.panels.as_ref()).and_then(|(p, panels)| p.near(feet, OPEN_RANGE, &panels.quests.wanted(&panels.inv)));
        let node = self.nodes.as_ref().and_then(|n| n.near(feet, OPEN_RANGE));
        let skin = self.wolves.as_ref().and_then(|w| w.skinnable_near(feet, LOOT_RANGE));
        let fish = if self.has_pole() { self.water_ahead() } else { None };
        let talk = self.wolves.as_ref().and_then(|w| w.talkable_near(feet, TALK_RANGE));
        // Loose props are carried for as long as the key is held (let go in `input.rs`); the key repeating while one is held does nothing else.
        if self.bodies.as_ref().is_some_and(|b| b.holding()) {
            return;
        }
        let body = self.bodies.as_ref().and_then(|b| b.near(feet, crate::physics::GRAB_RANGE));
        let drag = self.bodies.as_ref().and_then(|b| b.near_corpse(feet, crate::physics::GRAB_RANGE));
        // A corpse with loot that can be dragged too: a tap of the key loots it, holding the key drags it (see `input.rs` and the frame).
        if let (Some(i), Some(_), None) = (corpse, drag, self.pending_loot) {
            self.pending_loot = Some((i, Instant::now()));
            return;
        }
        if self.pending_loot.is_some() {
            return;
        }
        if corpse.is_none() && chest.is_none() && pickup.is_none() && node.is_none() && skin.is_none() && fish.is_none() && body.is_none() && drag.is_none() && near_bed(&self.beds, &self.world, feet) {
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
        } else if let Some(i) = node {
            self.use_node(i);
        } else if let Some(i) = skin {
            self.start_skin(i);
        } else if let Some(i) = talk {
            self.talk_to(i);
        } else if let (Some(h), Some(bodies)) = (body.or(drag), &mut self.bodies) {
            bodies.grab(h);
        } else if let Some(spot) = fish {
            self.start_fishing(spot);
        }
    }

    /// Take hold of the nearest part of a corpse once the key was held long enough for it not to be a tap (see `pending_loot`).
    pub(super) fn drag_if_held(&mut self) {
        if self.pending_loot.is_some_and(|(_, since)| since.elapsed() >= DRAG_HOLD) {
            self.pending_loot = None;
            let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
            if let Some(bodies) = &mut self.bodies {
                if let Some(h) = bodies.near_corpse(feet, crate::physics::GRAB_RANGE) {
                    bodies.grab(h);
                }
            }
        }
    }

    /// Open the loot window of corpse `i`.
    pub(super) fn loot_corpse(&mut self, i: usize) {
        let (Some(wolves), Some(panels)) = (&self.wolves, &mut self.panels) else { return };
        if let Some(loot) = wolves.loot(i).cloned() {
            panels.open_loot(i, loot);
            self.set_grab(false);
        } else if wolves.skin_of(i).is_some() {
            self.start_skin(i);
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
            panels.inv.add(crate::items::Stack { item, count: 1, enchant: 0 });
            let text = panels.string("LOOT_ITEM_PUSHED_SELF").replacen("%s", crate::items::get(item).name, 1);
            panels.notice(&text);
            panels.sound("ChestOpen");
        }
    }

    /// Use node `i`: loot a gathering left in it, else begin to gather it (a cast, broken by moving or a fight) if the profession's rank is high enough
    /// and, for mining, a mining pick is carried.
    pub(super) fn use_node(&mut self, i: usize) {
        let (Some(nodes), Some(panels)) = (&mut self.nodes, &mut self.panels) else { return };
        if let Some(loot) = nodes.loot(i).cloned() {
            panels.open_loot(NODE_LOOT + i, loot);
            self.set_grab(false);
            return;
        }
        let Some(kind) = nodes.kind(i) else { return };
        if !panels.must_know(kind.skill) {
            return;
        }
        if panels.skills.rank(kind.skill) < kind.rank {
            let text = panels.string("SPELL_FAILED_MIN_SKILL").replacen("%s", kind.skill.name(), 1).replacen("%d", &kind.rank.to_string(), 1);
            return panels.error(&text);
        }
        let mining = kind.skill == crate::skills::Skill::Mining;
        if mining && !panels.carries(MINING_PICK) {
            let text = panels.string("SPELL_FAILED_TOTEMS").replacen("%s", "Mining Pick", 1);
            return panels.error(&text);
        }
        let seconds = if mining { MINING_SECONDS } else { HERBALISM_SECONDS };
        panels.cast_start(if mining { "Mining" } else { "Herb Gathering" }, seconds);
        // What the original shows for the spell: the work animation, the pick in the hand, the sound.
        let (vis, feet) = (&crate::visuals::gathering(&self.mount)[if mining { 0 } else { 1 }], self.camera.pos - Vec3::Y * EYE_HEIGHT);
        let glows = self.in_ctx(|ctx| crate::casting::Spellcaster::gather_begin(ctx, vis, feet)).unwrap_or_default();
        self.gather = Some(Gather { job: Job::Node(i), left: seconds, glows });
    }

    /// Run `f` with what the spells work on (the character, the windows, the effects); none while the game has no character.
    pub(super) fn in_ctx<R>(&mut self, f: impl FnOnce(&mut crate::casting::Ctx) -> R) -> Option<R> {
        let (Some(player), Some(panels)) = (&mut self.player, &mut self.panels) else { return None };
        let mut ctx = crate::casting::Ctx { player, panels, wolves: None, text: None, effects: &mut self.effects, renderer: &mut self.renderer, gpu: &self.gpu, world: &self.world };
        Some(f(&mut ctx))
    }

    /// The gathering stops (done, broken, or the world was made again): its models go and the character stands as before.
    pub(super) fn end_gather(&mut self) {
        if let Some(b) = self.bobber.take() {
            self.renderer.remove_entity(b.entity);
        }
        if let Some(g) = self.gather.take() {
            self.in_ctx(|ctx| crate::casting::Spellcaster::gather_end(ctx, &g.glows));
        }
    }

    /// One step of gathering: moving or a fight breaks it; at the end the profession may rise and the node's loot is shown.
    pub(super) fn step_gather(&mut self, dt: f32) {
        let Some(g) = &mut self.gather else { return };
        let (job, glows) = (g.job, g.glows.clone());
        g.left -= dt;
        let left = g.left;
        if self.moving || self.in_combat {
            self.end_gather();
            if let Some(panels) = &mut self.panels {
                panels.cast_failed("Interrupted");
            }
            return;
        }
        if left > 0.0 {
            self.in_ctx(|ctx| crate::casting::Spellcaster::gather_follow(ctx, &glows));
            if job == Job::Fish {
                self.step_bobber(dt);
            }
            return;
        }
        self.end_gather();
        let i = match job {
            Job::Node(i) => i,
            Job::Make { recipe, more } => return self.finish_make(recipe, more),
            Job::Skin(creature) => return self.finish_skin(creature),
            // The line ran out: it comes back.
            Job::Fish => return panels_cast_stop(self),
            Job::Lure(item) => {
                panels_cast_stop(self);
                if let Some(panels) = &mut self.panels {
                    panels.finish_lure(item);
                }
                return;
            }
        };
        let (Some(panels), Some(nodes)) = (&mut self.panels, &mut self.nodes) else { return };
        panels.cast_stop();
        let Some(kind) = nodes.kind(i) else { return };
        let roll = nodes.roll();
        panels.gather_skill(kind.skill, kind.rank, roll);
        if let Some(loot) = nodes.gather(i, &panels.quests.wanted(&panels.inv)) {
            panels.open_loot(NODE_LOOT + i, loot);
            self.set_grab(false);
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
        if let Some(nodes) = &self.nodes {
            if let Some(i) = nodes.pick(camera, size, pos) {
                let mining = nodes.kind(i).is_some_and(|k| k.skill == crate::skills::Skill::Mining);
                return (if mining { Kind::Mine } else { Kind::Herb }, Some(Hover::Node(i)));
            }
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
            Hover::Node(i) => self.nodes.as_ref().and_then(|n| n.pos(i)),
        };
        let Some(at) = at else { return false };
        let distance = Vec3::new(at.x - feet.x, 0.0, at.z - feet.z).length();
        let (range, far) = match hover {
            Hover::Creature(i) if self.wolves.as_ref().is_some_and(|w| w.loot(i).is_some() || w.skin_of(i).is_some()) => (LOOT_RANGE, "ERR_LOOT_TOO_FAR"),
            Hover::Creature(_) => (TALK_RANGE, "ERR_USE_TOO_FAR"),
            Hover::Chest(_) | Hover::Pickup(_) | Hover::Node(_) => (OPEN_RANGE, "ERR_USE_TOO_FAR"),
        };
        if self.panels.as_ref().is_none_or(|p| p.any_open()) || self.lock.is_some() || self.gather.is_some() {
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
            Hover::Node(i) => self.use_node(i),
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

/// The cast bar of a line that ran out goes.
fn panels_cast_stop(state: &mut State) {
    if let Some(panels) = &mut state.panels {
        panels.cast_stop();
    }
}
