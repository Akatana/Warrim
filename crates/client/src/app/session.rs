//! The character as saved, and the content taken into use again while the game runs.

use super::*;

impl State {
    /// The character as it is now, by ids: where it stands, what it carries, its level, quests, skills and discoveries.
    pub(super) fn capture(&self) -> Option<save::Save> {
        let (panels, player) = (self.panels.as_ref()?, self.player.as_ref()?);
        let [north, west, _] = render_to_wow(self.camera.pos);
        let health = player.vitals.health.fraction().max(0.05);
        let mut saved = save::Save::capture(&panels.inv, panels.progress, &panels.quests, (north, west), self.camera.yaw.to_degrees(), self.camera.pitch.to_degrees(), health);
        saved.skills = panels.skills.parts();
        saved.spells = panels.known.iter().map(|&i| content::spell(i).id.clone()).collect();
        saved.bar = self.actionbar.as_ref().map_or_else(Vec::new, |b| b.layout());
        saved.discovered = content::get().pois.iter().zip(&panels.found).filter(|(_, &f)| f).map(|(p, _)| p.id.clone()).collect();
        saved.explored = panels.explored.clone();
        saved.hour = (!self.fixed_time).then_some(self.hour);
        (saved.mana, saved.stamina) = (player.vitals.mana.fraction(), player.vitals.stamina.fraction());
        saved.cooldowns = content::get().spells.iter().zip(&panels.cooldowns).filter(|(_, &left)| left > 0.0).map(|(s, &left)| (s.id.clone(), left)).collect();
        saved.mobs = self.wolves.as_ref().map_or_else(Vec::new, |w| w.snapshot());
        saved.chests = self.chests.as_ref().map_or_else(Vec::new, |c| c.snapshot());
        saved.steed = player.steed_info();
        saved.sheathed = player.is_sheathed();
        saved.height = Some(self.camera.pos.y - EYE_HEIGHT);
        (saved.buffs, saved.item_cooldowns, saved.hearth) = panels.consumed.snapshot();
        saved.sneaking = player.is_sneaking();
        saved.look = self.look;
        Some(saved)
    }

    /// Write the saved game: the character, what it carries, its level and quests.
    pub(super) fn save_game(&mut self) {
        self.last_save = Instant::now();
        let (Some(path), Some(saved)) = (&self.save_path, self.capture()) else { return };
        if let Err(e) = save::write(path, &saved) {
            log::warn!("cannot save the game: {e:#}");
        }
    }

    /// Take the content files (as the editor saved them) into use while the game runs: the character is kept, by ids, and the creatures and
    /// chests are put in the world again from the new content (corpses, loot and wounds are forgotten, like after a respawn). If that
    /// cannot be done (the spells changed), the game starts again instead.
    pub(super) fn apply_content(&mut self) {
        let started = Instant::now();
        let live = self.reload_content();
        match &live {
            Ok(()) => log::info!("applied the content live in {:?}", started.elapsed()),
            Err(e) => {
                log::warn!("cannot apply the content live: {e:#}");
                self.restart = true;
            }
        }
        if let Some(editor) = &mut self.editor {
            editor.applied(&mut self.renderer, live.is_ok());
        }
    }

    pub(super) fn reload_content(&mut self) -> Result<()> {
        let saved = self.capture();
        content::reload(&self.mount)?;
        self.world.set_props(content::get().props.clone());
        if let Some(old) = self.wolves.take() {
            old.retire(&mut self.renderer);
        }
        if let Some(old) = self.chests.take() {
            old.retire(&mut self.renderer);
        }
        if let Some(old) = self.pickups.take() {
            old.retire(&mut self.renderer);
        }
        if let Some(old) = self.bodies.take() {
            old.retire(&mut self.renderer);
        }
        (self.lock, self.hover, self.hint, self.cast_request) = (None, None, None, None);
        if let Some(panels) = &mut self.panels {
            panels.forget_world();
            if let Some(saved) = &saved {
                panels.restore(saved);
            }
        }
        if let (Some(player), Some(panels)) = (&mut self.player, &self.panels) {
            player.forget_items();
            player.set_equipment(&mut self.renderer, &self.gpu, &panels.inv.equipment, &panels.stats);
        }
        if let Some(audio) = &mut self.audio {
            audio.bank.reload_cries(&self.mount).unwrap_or_else(|e| log::warn!("no cries: {e:#}"));
        }
        self.wolves = Creatures::load(&self.mount, &ModelCache::default(), &mut self.renderer, &self.gpu, &spawn_list()).map_err(|e| log::warn!("no creatures: {e:#}")).ok();
        if let Some(wolves) = &mut self.wolves {
            wolves.set_spell_anims(self.foes.anims());
        }
        self.chests = Chests::load(&self.mount, &ModelCache::default(), &mut self.renderer, &self.gpu).map_err(|e| log::warn!("no chests: {e:#}")).ok();
        self.pickup_props = pickup_props();
        self.beds = bed_spots(&content::get().props, &self.map);
        self.load_pickups();
        Ok(())
    }

    /// Put the props to pick up and those with physics in the world, in place of those there were (the props changed).
    pub(super) fn load_pickups(&mut self) {
        if let Some(old) = self.pickups.take() {
            old.retire(&mut self.renderer);
        }
        if let Some(old) = self.bodies.take() {
            old.retire(&mut self.renderer);
        }
        self.bodies = Bodies::load(&self.mount, &ModelCache::default(), &mut self.renderer, &self.gpu, &self.pickup_props, &self.map).map_err(|e| log::warn!("no physics props: {e:#}")).ok();
        self.pickups = Pickups::load(&self.mount, &ModelCache::default(), &mut self.renderer, &self.gpu, &self.pickup_props, &self.map).map_err(|e| log::warn!("no pickups: {e:#}")).ok();
    }
}
