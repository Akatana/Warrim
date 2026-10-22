//! Doors to other places and maps: the original's area triggers (dungeon entrances and exits, tower portals).

use super::*;

impl State {
    /// Take the character through the portal he has just walked into, if he has and is high enough; says why when not.
    pub(super) fn walk_into_portals(&mut self) {
        // The ground is not under him yet (a trip began, a character appeared): nothing to test.
        if self.snap_to_ground.is_some() || self.loading.is_some() {
            return;
        }
        let feet = render_to_wow(self.camera.pos - Vec3::Y * EYE_HEIGHT);
        let inside = content::get().portals.iter().find(|p| p.map.eq_ignore_ascii_case(&self.map) && p.contains(feet));
        if self.in_portal == inside.map(|p| p.id) {
            return;
        }
        self.in_portal = inside.map(|p| p.id);
        let (Some(portal), Some(panels)) = (inside, &mut self.panels) else { return };
        match portal.refuse(panels.progress.level, self.look.race.horde()) {
            Some(text) => panels.error(&text),
            None => {
                log::info!("portal {} ({}) to {}", portal.id, portal.name, portal.to_map);
                // The original's orientation turns counter-clockwise from north, the camera's yaw clockwise.
                self.enter(&portal.to_map, portal.to, -portal.facing);
            }
        }
    }

    /// Put the character at a place (north, west, feet height) of a map, with the camera turned to `yaw`.
    pub(super) fn enter(&mut self, map: &str, (north, west, height): (f32, f32, f32), yaw: f32) {
        if !map.eq_ignore_ascii_case(&self.map) {
            if let Err(e) = self.switch_map(map) {
                log::warn!("cannot go to {map}: {e:#}");
                if let Some(panels) = &mut self.panels {
                    panels.error("You cannot go there.");
                }
                return;
            }
        }
        self.camera.yaw = yaw;
        self.teleport(north, west, Some(height));
        // He arrives on a portal's doorstep (the way back, mostly): that one takes him only once he has left it and come again.
        self.in_portal = content::get().portals.iter().find(|p| p.map.eq_ignore_ascii_case(map) && p.contains([north, west, height])).map(|p| p.id);
    }

    /// The ship he stands on has sailed into another map: take him there, on its deck where he stood.
    pub(super) fn cross_with_ship(&mut self, cross: crate::lifts::Cross) {
        if let Err(e) = self.switch_map(&cross.map) {
            log::warn!("cannot sail on to {}: {e:#}", cross.map);
            return;
        }
        let Some((feet, yaw)) = self.lifts.as_ref().and_then(|l| l.arrive(&cross, self.camera.yaw)) else { return };
        self.camera.yaw = yaw.rem_euclid(std::f32::consts::TAU);
        let [north, west, height] = render_to_wow(feet);
        self.teleport(north, west, Some(height));
    }

    /// Back where the game starts: dead, or home by the hearthstone.
    pub(super) fn go_home(&mut self) {
        if self.map.eq_ignore_ascii_case(&self.home) {
            (self.camera.pos.x, self.camera.pos.z) = self.spawn;
            self.snap_to_ground = Some(0.0);
        } else {
            let [north, west, _] = render_to_wow(Vec3::new(self.spawn.0, 0.0, self.spawn.1));
            let home = self.home.clone();
            self.enter(&home, (north, west, 0.0), self.camera.yaw);
            // No floor height is known for the start: the ground is found by dropping onto it.
            self.saved_feet = None;
        }
    }

    /// Load another map in place of the one being played: its terrain or dungeon, its light, its props, its loading screen.
    fn switch_map(&mut self, map: &str) -> Result<()> {
        let radius = self.world.radius;
        let world = World::open(self.mount.clone(), map, radius)?;
        self.sky = Sky::load(&self.mount, map, radius as f32 * wowfmt::adt::TILE_SIZE * 0.95).map_err(|e| log::warn!("no light data: {e:#}")).ok();
        self.renderer.set_water_textures(&self.gpu, &world.water_textures());
        self.world = world;
        // The loading screen of the new map: its picture is made once (kept by map), the bar is the one there is.
        if let Some(bar) = self.loading.as_ref().or(self.spare_loading.as_ref()).map(|l| l.with_screen(l.screen())) {
            self.screens.entry(self.map.to_lowercase()).or_insert(bar.screen());
            let key = map.to_lowercase();
            if !self.screens.contains_key(&key) {
                match Loading::screen_of(&self.mount, map, &mut self.renderer.ui, &self.gpu) {
                    Ok(screen) => drop(self.screens.insert(key.clone(), screen)),
                    Err(e) => log::warn!("no loading screen: {e:#}"),
                }
            }
            self.spare_loading = self.screens.get(&key).map(|&screen| bar.with_screen(screen));
        }
        self.map = map.to_string();
        self.fires = Default::default();
        self.beds = bed_spots(&content::get().props, map);
        self.load_pickups();
        Ok(())
    }
}
