//! What the world around the player does: weather, places that sound, the water the character is in.

use super::*;

impl State {
    /// Moves the weather along; where the player is and what the zone allows are looked up four times a second.
    pub(super) fn update_weather(&mut self, dt: f32) {
        let eye = self.camera.pos;
        self.weather_look -= dt;
        if self.weather_look <= 0.0 {
            self.weather_look = 0.25;
            self.indoors = self.world.indoors(eye);
            self.zone = self.world.area_at(eye).map(|a| self.world.areas.zone(a));
        }
        let (sy, cy) = self.camera.yaw.sin_cos();
        // A jump of the camera (the first frames, a teleport) is not a speed.
        let velocity = ((eye - self.last_eye) / dt.max(1e-3)).clamp_length_max(25.0);
        let motion = crate::weather::Motion { eye, velocity, forward: Vec3::new(sy, 0.0, -cy), indoors: self.indoors };
        self.weather.update(dt, &motion, &self.world, self.zone);
    }

    /// Looks for what is heard from places round the ears: the liquid within nine yards (the nearest of each kind, at most two kinds, as the original
    /// does) and the models that play a sound all the time (the six nearest within their reach); and whether the feet are in water.
    pub(super) fn look_for_sounds(&mut self) {
        let Some(bank) = self.audio.as_ref().map(|a| &a.bank) else { return };
        let ears = self.camera.pos;
        let mut places = Vec::new();
        for (kind, speeds) in self.world.nearby_liquids(ears, 9.0).iter().enumerate() {
            let nearest = speeds.iter().enumerate().find_map(|(speed, at)| at.map(|at| (speed, at)));
            if let Some((entry, at)) = nearest.and_then(|(speed, at)| bank.water_sound(kind, speed).map(|e| (e, at))) {
                if places.len() < 2 {
                    places.push(((1u64 << 60) | kind as u64, entry, at));
                }
            }
        }
        let mut sources = self.world.nearby_sounds(ears, 130.0);
        sources.sort_by(|a, b| a.0.distance(ears).total_cmp(&b.0.distance(ears)));
        places.extend(sources.into_iter().take(6).map(|(at, entry)| {
            let key = ((at.x * 8.0) as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((at.z * 8.0) as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ entry as u64;
            (2u64 << 60 | key >> 4, entry, at)
        }));
        self.places = places;
        let feet = ears - Vec3::Y * EYE_HEIGHT;
        self.wading = self.world.liquid_at(feet.x, feet.z, feet.y).is_some_and(|level| level > feet.y + 0.1);
    }

    /// What the character does to the water it is in: the spray of a footstep in shallow water (`FUN_005fbf70`: less than half its height deep, walking
    /// or running by the speed, at the surface), and the ripples (`FUN_005fa760`, started also by `FUN_006030c0` when the water crosses 0.4 of the
    /// height) that `ripples.rs` makes and draws.
    pub(super) fn water_effects(&mut self, dt: f32, camera: Vec3) {
        use crate::ripples::{State, Walker};
        const SPRAY_RUN: &str = r"Particles\FootStepSprayWater.m2";
        const SPRAY_WALK: &str = r"Particles\FootStepSprayWaterWalk.m2";
        let size = self.player.as_ref().map_or_else(Default::default, |p| p.size());
        let height = size.height;
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        // The camera under the surface (its lowest edge of the near plane, 0.2 yd under it) hears the loop of the sea.
        let under = self.world.liquid_at(camera.x, camera.z, camera.y).is_some_and(|level| camera.y - 0.2 < level);
        if let Some(audio) = &mut self.audio {
            audio.set_underwater(under);
        }
        let level = self.input.water.filter(|_| self.controller.mode == Mode::Walk);
        let depth = level.map_or(0.0, |level| level - feet.y);
        let swimming = self.controller.swimming();
        let stepped = std::mem::take(&mut self.stepped);
        if let (Some(level), true) = (level, stepped && !swimming && depth > 0.0 && depth < 0.5 * height) {
            // (the original: running when faster than twice the walking speed, 2.5)
            let run = self.speed > 5.0;
            let at = glam::Mat4::from_translation(Vec3::new(feet.x, level, feet.z));
            self.effects.spawn(&mut self.renderer, &self.gpu, if run { SPRAY_RUN } else { SPRAY_WALK }, false, at);
        }
        self.ripples.update(dt);
        if let Some(level) = level {
            // Where it goes: the way it faces, turned by the keys (the original's offsets: a strafe is a quarter turn, with forward or back an eighth
            // or three eighths, and backing is half a turn).
            let (i, quarter) = (&self.input, std::f32::consts::FRAC_PI_4);
            let turn = if i.left {
                if i.forward { quarter } else if i.back { 3.0 * quarter } else { 2.0 * quarter }
            } else if i.right {
                -if i.forward { quarter } else if i.back { 3.0 * quarter } else { 2.0 * quarter }
            } else if i.back && !i.forward {
                4.0 * quarter
            } else {
                0.0
            };
            let yaw = self.camera.yaw - turn;
            let moving = i.forward || i.back || i.left || i.right;
            // The water reaching 0.4 of the height, either way, makes a splash (not for a swimmer).
            let crossed = (depth > 0.4 * height) != (self.water_depth > 0.4 * height) && !swimming;
            let state = if crossed { State::Entering } else if moving { State::Moving } else { State::Idle };
            if crossed {
                if let Some(audio) = &mut self.audio {
                    audio.splash(height);
                }
            }
            let walker = Walker { feet, level, height, scale: size.scale, speed: self.speed, heading: (yaw.sin(), -yaw.cos()) };
            let world = &self.world;
            self.ripples.make(&walker, state, |at, half| world.liquid_quads(at.x, at.z, feet.y, half));
        }
        self.water_depth = depth;
        if let Some(draw) = &mut self.ripple_draw {
            draw.draw(&self.ripples, feet, &mut self.renderer, &self.gpu);
        }
    }
}
