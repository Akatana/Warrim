//! One frame of the game.

use super::*;

impl State {
    pub(super) fn frame(&mut self) {
        {
            let _t = crate::profile::scope("frame");
            self.frame_body();
        }
        crate::profile::end_frame();
    }

    fn frame_body(&mut self) {
        use crate::profile::scope;
        let now = Instant::now();
        // The game stands still while the Esc menu (or one of its option windows) is open.
        let paused = self.panels.as_ref().is_some_and(|p| p.menu_open() || p.options_open());
        let dt = if paused { 0.0 } else { (now - self.last_frame).as_secs_f32().min(0.1) };
        self.last_frame = now;
        // The game saves itself now and then (and when it ends).
        if self.last_save.elapsed() >= Duration::from_secs(SAVE_SECONDS) {
            self.save_game();
        }

        // The eyes of a spell come back when their time is up, or when they have gone as far as they can.
        if let Some((left, reach)) = &mut self.scout {
            *left -= dt;
            let far = self.free.as_ref().is_some_and(|f| (f.camera.pos - self.camera.pos).length() > *reach);
            if *left <= 0.0 || far {
                self.scout = None;
                self.detached = false;
            }
        }
        // While the editor is open its camera flies free: it starts behind and above the player, looking where he looks.
        if !self.detached && !self.editor.as_ref().is_some_and(|e| e.open) {
            self.free = None;
        } else if self.free.is_none() {
            let (forward, mut controller) = (self.camera.forward(), Controller::new(Mode::Fly));
            controller.fly_speed = FREE_SPEED;
            self.free = Some(FreeCam { camera: Camera { pos: self.camera.pos - forward * 5.0 + Vec3::Y * 1.5, ..self.camera }, controller });
        }
        let watching = self.free.as_ref().map_or(self.camera.pos, |f| f.camera.pos);
        let t = scope("world.update (streaming)");
        self.world.update(watching.x, watching.z, &self.gpu, &mut self.renderer);
        drop(t);
        let t = scope("weather, sky, steer");
        if !self.fixed_time {
            // One game minute per second, like the original.
            self.hour = (self.hour + dt / 60.0) % 24.0;
        }
        self.update_weather(dt);
        if let Some(a) = self.sky.as_ref().and_then(|s| s.atmosphere(self.camera.pos, self.hour, self.weather.storm())) {
            self.renderer.atmosphere = a;
        }

        self.steer(dt);
        drop(t);

        if let Some(loading) = &mut self.loading {
            let pending = self.world.pending_count();
            loading.hold -= dt;
            if self.snap_to_ground.is_none() && pending == 0 && loading.hold <= 0.0 {
                self.spare_loading = self.loading.take();
            } else {
                // The terrain step of the bar: how many of the tiles that waited have been fetched.
                loading.peak = loading.peak.max(pending);
                loading.report(Stage::Terrain, 1.0 - pending as f32 / loading.peak.max(1) as f32);
                present_loading(&self.surface, &self.gpu, &mut self.renderer, loading, (self.config.width, self.config.height));
                return;
            }
        }
        let t = scope("acquire frame");
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.gpu.device, &self.config);
                return;
            }
            Err(e) => {
                log::warn!("surface error: {e}");
                return;
            }
        };
        drop(t);
        let view = frame.texture.create_view(&Default::default());
        let mut render_camera = Camera { pos: self.camera.pos, ..self.camera };
        let t = scope("audio");
        let t2 = scope("area, room lookup");
        self.place_look -= dt;
        if self.place_look <= 0.0 {
            self.place_look = 0.1;
            (self.area_now, self.room_now) = self.world.place_at(self.camera.pos);
        }
        drop(t2);
        let t2 = scope("audio.update, weather");
        if let Some(audio) = &mut self.audio {
            audio.update(dt, self.area_now, self.room_now, self.hour);
            audio.set_weather(self.weather.sound(), self.indoors, dt);
        }
        drop(t2);
        let t2 = scope("look_for_sounds");
        self.sound_look -= dt;
        if self.sound_look <= 0.0 {
            self.sound_look = 0.1;
            self.look_for_sounds();
        }
        drop(t2);
        let t2 = scope("set_places");
        if let Some(audio) = &mut self.audio {
            audio.set_listener(self.camera.pos, self.camera.yaw);
            // The lifts that go sound from where they are, which changes with every frame.
            match &self.lifts {
                Some(lifts) => audio.set_places(&[&self.places[..], &lifts.hums()].concat()),
                None => audio.set_places(&self.places),
            }
            audio.set_wading(self.wading);
        }
        drop(t2);
        drop(t);
        let t = scope("animate player");
        self.animate_player(dt, &mut render_camera);
        if let Some(free) = &self.free {
            render_camera = Camera { pos: free.camera.pos, ..free.camera };
        }
        self.last_eye = self.camera.pos;
        self.eye = Some(crate::propedit::Eye::of(&render_camera, (self.config.width, self.config.height)));
        drop(t);
        let t = scope("water effects, panels, spells");
        if let Some(item) = self.panels.as_mut().and_then(|p| p.take_lure()) {
            self.start_lure(item);
        }
        if self.later.is_some() && self.moving {
            self.later = None;
        } else if self.later.is_some() && self.player.as_ref().is_some_and(|p| !p.drawing()) {
            match self.later.take() {
                Some(Later::Fish(spot)) => self.start_fishing(spot),
                Some(Later::Lure(item)) => self.start_lure(item),
                None => {}
            }
        }
        self.water_effects(dt, render_camera.pos);
        let size = (self.config.width, self.config.height);
        if let Some(panels) = &mut self.panels {
            panels.tick_cooldowns(dt);
            self.cast_request = panels.take_spell_click().or(self.cast_request);
            if let Some(bar) = &mut self.actionbar {
                panels.take_learned().into_iter().for_each(|spell| bar.learn(spell));
                panels.take_forgotten().into_iter().for_each(|spell| bar.forget(spell));
                if let Some(item) = bar.take_item_use() {
                    panels.use_item_of(item);
                }
                // (An action lifted off the bar is carried by the bar itself.)
                if let Some(a) = panels.dragging() {
                    bar.set_drag(Some(a));
                }
            }
        }
        // The spells: one asked for, the one being cast, the missiles in flight, and the effects they are made of.
        if let (Some(player), Some(panels)) = (&mut self.player, &mut self.panels) {
            let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
            let mut ctx = crate::casting::Ctx { player, panels, wolves: self.wolves.as_mut(), text: self.combat_text.as_mut(), effects: &mut self.effects, renderer: &mut self.renderer, gpu: &self.gpu, world: &self.world };
            if let Some(spell) = self.cast_request.take() {
                self.caster.begin(&mut ctx, spell, feet);
            }
            if let Some(shot) = ctx.player.take_shot() {
                self.caster.shoot(&mut ctx, shot, feet, self.camera.forward());
            }
            self.caster.update(&mut ctx, dt, feet, self.moving);
        }
        // A charge: the character leaps at the target, arriving in the time a jump of this height takes.
        if let Some(at) = self.caster.take_leap() {
            self.leap_to(at);
        }
        drop(t);
        let t = scope("fires, effects");
        self.fires.update(&self.world, render_camera.pos, &mut self.effects, &mut self.renderer, &self.gpu);
        self.effects.update(dt, &render_camera, &mut self.renderer, &self.gpu);
        drop(t);
        let t = scope("fight (creatures, foes, labels)");
        self.fight(dt, &render_camera, size);
        drop(t);
        let t = scope("chests, pickups, bodies");
        if let Some(chests) = &mut self.chests {
            let world = &self.world;
            chests.update(dt, |x, z| world.height_at(x, z), &mut self.renderer, &self.gpu);
        }
        if std::mem::take(&mut self.pickups_stale) {
            self.load_pickups();
        }
        if let Some(pickups) = &mut self.pickups {
            let world = &self.world;
            pickups.update(dt, self.camera.pos, |x, z| world.height_at(x, z), &mut self.renderer);
        }
        if let Some(nodes) = &mut self.nodes {
            nodes.update(dt, self.camera.pos, &self.mount, &self.gpu, &mut self.renderer);
        }
        self.drag_if_held();
        if let Some(bodies) = &mut self.bodies {
            let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
            let forward = Vec3::new(self.camera.yaw.sin(), 0.0, -self.camera.yaw.cos());
            bodies.update(dt, feet, forward, &self.world, &mut self.renderer);
        }
        drop(t);
        let t = scope("cursor, lock");
        self.step_lock(dt);
        if let Some((recipe, times)) = self.panels.as_mut().and_then(|p| p.take_craft()) {
            self.start_craft(recipe, times);
        }
        self.step_gather(dt);
        (self.pointer, self.hover) = {
            let (kind, hover) = self.cursor_context(&render_camera, size);
            (kind, hover)
        };
        drop(t);
        let t = scope("upkeep");
        self.upkeep(dt);
        drop(t);
        let t = scope("draw interface");
        let discovered = self.draw_interface(dt, &render_camera, size);
        drop(t);
        if discovered {
            self.save_game();
        }
        let drawing = Instant::now();
        let t = scope("renderer.render");
        self.renderer.prepare_weather(&self.gpu, &self.weather, &render_camera, (self.config.width, self.config.height));
        self.renderer.render(
            &self.gpu,
            &view,
            (self.config.width, self.config.height),
            &render_camera,
            self.start.elapsed().as_secs_f32(),
            self.world.tiles(),
        );
        drop(t);
        self.render_time += drawing.elapsed();
        let _t = scope("present");
        self.window.pre_present_notify();
        frame.present();
        if std::mem::take(&mut self.reconfigure) {
            self.surface.configure(&self.gpu.device, &self.config);
        }

        self.count_frame();
    }

    /// Drop onto the terrain at the first frame, then turn the keys and the spells that hold the character into where he moves.
    fn steer(&mut self, dt: f32) {
        // First-frame spawn: drop onto the terrain as soon as it is available.
        if let Some(waited) = self.snap_to_ground {
            // With a saved height, wait for the surrounding tiles too: their buildings hold the floor.
            let (x, z) = (self.camera.pos.x, self.camera.pos.z);
            let ground = match self.saved_feet {
                Some(_) if self.world.pending_count() > 0 => None,
                Some(feet) => crate::collision::Collider::floor(&Solid(&self.world, self.lifts.as_ref()), x, z, feet + 1.0).or_else(|| self.world.height_at(x, z)),
                None => self.world.height_at(x, z),
            };
            if let Some(ground) = ground {
                let lift = if self.controller.mode == Mode::Fly { 30.0 } else { EYE_HEIGHT };
                self.camera.pos.y = ground + lift;
                log::info!("spawned at height {ground:.2} (saved {:?}, terrain {:?})", self.saved_feet, self.world.height_at(x, z));
                self.snap_to_ground = None;
                self.saved_feet = None;
            } else if let Some(feet) = self.saved_feet.filter(|_| waited > 3.0 && self.world.pending_count() == 0) {
                // Nothing to stand on at the height given, with all the tiles in (a door that opens over a void): he drops in from there.
                self.camera.pos.y = feet + EYE_HEIGHT;
                (self.snap_to_ground, self.saved_feet) = (None, None);
            } else {
                self.snap_to_ground = Some(waited + dt);
            }
        } else {
            // Shift sprints (on foot or in the saddle) for as long as there is stamina.
            let sneaking = self.controller.mode == Mode::Walk && self.player.as_ref().is_some_and(|p| p.is_sneaking());
            self.input.sneak = sneaking;
            // The water under the feet (deep enough, the character swims: stamina, tiredness and the skill in swimming are the player's).
            let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
            self.input.water = self.world.liquid_at(feet.x, feet.z, feet.y);
            if let Some(p) = &self.player {
                self.controller.set_height(p.size().height);
            }
            let swimming = self.controller.swimming();
            self.input.tired = self.player.as_ref().is_some_and(|p| p.is_worn_out());
            self.input.sprint = self.input.fast && self.controller.mode == Mode::Walk && !sneaking && !swimming && self.player.as_ref().is_none_or(|p| p.can_sprint());
            // (what the worn gear adds to the speed of mounts counts only while riding)
            let gear_mount = self.panels.as_ref().map_or(0.0, |p| p.stats.mount_speed);
            self.input.mount_bonus = self.player.as_ref().map_or(0.0, |p| if p.speed_bonus() > 0.0 { p.speed_bonus() + gear_mount } else { 0.0 });
            self.input.gear_bonus = self.panels.as_ref().map_or(0.0, |p| p.stats.run_speed);
            self.input.burden = self.panels.as_ref().map_or(0.0, |pn| pn.burden());
            // Spells that have hold of the character: slowed, rooted, stunned, or running in fear from where the spell came from.
            let held = &self.foes.afflictions;
            self.input.burden = self.input.burden.max(held.slow());
            self.caster.set_blocked(held.cast_blocked());
            if let Some(p) = &mut self.player {
                p.set_held_back(held.swing_blocked(), held.shot_blocked());
            }
            if held.stunned() || held.rooted() || held.feared().is_some() {
                (self.input.forward, self.input.back, self.input.left, self.input.right, self.input.jump_pressed) = (false, false, false, false, false);
            }
            if let Some(from) = held.feared() {
                let want = (feet.x - from.x).atan2(-(feet.z - from.z));
                let turn = (want - self.camera.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                self.camera.yaw = (self.camera.yaw + turn.clamp(-3.0 * dt, 3.0 * dt)).rem_euclid(std::f32::consts::TAU);
                self.input.forward = true;
            }
            if let Some(p) = &mut self.player {
                p.set_sprinting(self.input.sprint);
                p.set_swimming(swimming, self.panels.as_ref().map_or(1.0, |pn| pn.skills.swim_cost_factor()));
                // The head under the surface holds the breath, as long as the skill in diving allows.
                let eye = self.camera.pos;
                let submerged = self.controller.mode == Mode::Walk && self.world.liquid_at(eye.x, eye.z, eye.y).is_some_and(|level| eye.y < level);
                p.set_diving(submerged, self.panels.as_ref().map_or(30.0, |pn| if pn.stats.water_breathing { 3600.0 } else { pn.skills.breath_seconds() }));
                if let Some(panels) = &mut self.panels {
                    panels.set_breath(p.breath());
                    // Holding the breath trains the skill in diving.
                    if submerged {
                        panels.train(crate::skills::Skill::Diving, dt / 2.0);
                    }
                }
                // A jump on foot costs stamina; without enough of it the character stays on the ground.
                if self.input.jump_pressed && self.controller.mode == Mode::Walk && !self.controller.airborne() && !swimming && !p.try_jump() {
                    self.input.jump_pressed = false;
                } else if self.input.jump_pressed {
                    // Jumping stands up.
                    p.set_sneaking(false);
                }
            }
            // The lifts go on; one that the player stands on takes him along.
            if let Some(lifts) = &mut self.lifts {
                let (feet, world) = (self.camera.pos - Vec3::Y * EYE_HEIGHT, &self.world);
                let airborne = self.controller.airborne();
                let ride = lifts.advance(dt, feet, airborne, |x, z| crate::movement::TriangleSource::is_loaded(world, [-z as f64, -x as f64, 0.0]), &mut self.renderer, &mut self.effects);
                if let Some(audio) = &mut self.audio {
                    ride.bangs.iter().for_each(|&(entry, at)| audio.play_3d(crate::audio::Sfx::Entry(entry), 0, at));
                }
                self.controller.carry(ride.by);
                // Being carried is not moving: the animation, the footsteps and the wind go by what he does on the lift.
                if self.controller.mode == Mode::Walk {
                    self.last_eye += ride.by;
                }
                // What a ship turns with him turns the view the other way.
                self.camera.yaw = (self.camera.yaw - ride.turn).rem_euclid(std::f32::consts::TAU);
                if let Some(cross) = ride.cross {
                    self.cross_with_ship(cross);
                }
            }
            // No walking, turning or jumping while a lock is being picked.
            if let Some(free) = &mut self.free {
                free.controller.update(&mut free.camera, &mut self.input, &self.world, dt);
            } else if self.lock.is_none() {
                self.controller.update(&mut self.camera, &mut self.input, &Solid(&self.world, self.lifts.as_ref()), dt);
            }
            if let Some(height) = self.controller.take_landing() {
                // (A spell that lets him fall softly takes the harm out of it.)
                let height = if self.panels.as_ref().is_some_and(|p| p.stats.slow_fall) { 0.0 } else { height };
                if let (Some(taken), Some(text)) = (self.player.as_mut().and_then(|p| p.take_fall(height)), &mut self.combat_text) {
                    text.damage_taken(taken);
                }
            }
        }
    }

    /// Animate the player from the way he moved, and the sounds that makes; the camera the picture is seen from comes back through `render_camera`.
    fn animate_player(&mut self, dt: f32, render_camera: &mut Camera) {
        if let Some(player) = &mut self.player {
            let flat = (self.camera.pos - self.last_eye) * Vec3::new(1.0, 0.0, 1.0) / dt.max(1e-3);
            let (sy, cy) = self.camera.yaw.sin_cos();
            // Flying is not walking: no footsteps, the jump pose.
            let airborne = self.controller.airborne() || self.controller.mode == Mode::Fly;
            let swimming = self.controller.swimming();
            // Food and drink (a buff that needs calm; bandages need a skill) are eaten with the eating animation.
            let eating = self.panels.as_ref().is_some_and(|p| p.consumed.buffs.iter().any(|b| b.needs_calm && crate::items::get(b.item).skill.0 == 0));
            let motion = Motion { speed: flat.length(), forward_speed: flat.dot(Vec3::new(sy, 0.0, -cy)), side_speed: flat.dot(Vec3::new(cy, 0.0, sy)), airborne, swimming, emote: eating.then_some(crate::animator::EAT), ..Default::default() };
            let world = &self.world;
            *render_camera = player.update(dt, &self.camera, motion, |from, to| world.camera_distance(from, to), &mut self.renderer, &self.gpu);
            self.moving = motion.speed > 0.5 || airborne;
            self.speed = motion.speed;
            // Swimming trains the skill in swimming; so does treading water, less.
            if let (true, Some(panels)) = (swimming, &mut self.panels) {
                panels.train(crate::skills::Skill::Swimming, dt / if motion.speed > 0.5 || (self.camera.pos.y - self.last_eye.y).abs() > 0.01 { 3.0 } else { 12.0 });
            }
            // Sneaking about (not standing still) trains the skill.
            if let (true, Some(panels)) = (player.is_sneaking() && motion.speed > 0.5 && !airborne, &mut self.panels) {
                panels.train(crate::skills::Skill::Sneaking, dt / 3.0);
            }
            let events = player.take_events();
            self.stepped |= events.contains(&Sfx::Footstep);
            if let Some(audio) = &mut self.audio {
                let surface = world.surface_under(self.camera.pos - Vec3::Y * EYE_HEIGHT) as u32;
                events.into_iter().for_each(|sfx| audio.play(sfx, surface));
            }
        }
    }

    /// The fight: the player's blows, the creatures and the spells they cast, what lands on the player, and the windows that close when he walks off.
    fn fight(&mut self, dt: f32, render_camera: &Camera, size: (u32, u32)) {
        // Nothing can hurt the player while the editor is open.
        let safe = self.god || self.free.is_some();
        if let (Some(wolves), Some(player)) = (&mut self.wolves, &mut self.player) {
            let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
            if let Some(why) = player.take_refusal() {
                if let Some(panels) = &mut self.panels {
                    panels.error(why);
                }
            }
            if let Some((swing, power)) = player.take_swing_hit() {
                let skill = self.panels.as_ref().map_or(crate::skills::Skill::Unarmed, |p| p.weapon_skill(player.is_off_hand()));
                let factor = self.panels.as_ref().map_or(1.0, |p| p.skills.damage_factor(skill));
                let heavy = if swing == crate::player::Swing::Heavy { crate::player::HEAVY_DAMAGE } else { 1.0 };
                // A swing that comes soon after the last one does only the share of its damage that the time between is of the weapon's speed.
                let crit = player.chance(self.panels.as_ref().map_or(0.0, |p| p.stats.crit));
                let damage = (player.roll_damage() * factor * heavy * power * if crit { 2.0 } else { 1.0 }).round().max(1.0);
                // A sneaking player's first blow on a creature that has not noticed him is heavier.
                let (level, hit, versus, speed, flat) = self.panels.as_ref().map_or((1, 0.0, [0; 11], 2.0, [0; 11]), |p| (p.progress.level, p.stats.hit, p.stats.ap_versus, p.stats.speed, p.stats.damage_versus));
                // Wielding a weapon in each hand makes every blow miss 19% more often.
                let hit = if player.is_dual_wielding() { hit - crate::player::DUAL_WIELD_MISS } else { hit };
                let sneak = if player.is_sneaking() { SNEAK_ATTACK } else { 1.0 };
                // A blow may miss; what the gear adds against the type of the creature is attack power, which adds damage as it does for all blows.
                let hits = wolves.strike(feet, self.camera.yaw, damage, sneak, &mut |mob, kind| {
                    (!player.chance(crate::levels::melee_miss(level, mob, hit))).then(|| versus[kind as usize % 11] as f32 / crate::items::ATTACK_POWER_PER_DPS * speed + flat[kind as usize % 11] as f32)
                });
                if hits.iter().any(|h| h.1 > 0.0) {
                    player.emit(Sfx::SwordHit);
                    if let Some(panels) = &mut self.panels {
                        panels.train(skill, heavy * power);
                    }
                }
                for (at, dealt, sneak, id) in hits {
                    if let Some(text) = &mut self.combat_text {
                        if dealt > 0.0 {
                            text.damage_dealt(at, dealt as u32);
                        } else {
                            text.message("Miss");
                        }
                    }
                    // The spells of the weapon in the hand that go off on a blow that lands.
                    if let (true, Some(panels)) = (dealt > 0.0, &mut self.panels) {
                        if let Some(weapon) = panels.inv.equipment[if player.is_off_hand() { crate::items::OFF_HAND } else { crate::items::MAIN_HAND }] {
                            let (level, speed) = (panels.progress.level, panels.stats.speed);
                            let mut out = Vec::new();
                            crate::procs::on_hit(weapon, speed, level, &mut || panels.consumed.dice(), &mut out);
                            let timed = panels.consumed.on_blow();
                            crate::procs::on_timed(&timed, speed, level, &mut || panels.consumed.dice(), &mut out);
                            if player.is_extra_swing() {
                                out.retain(|o| !matches!(o, crate::procs::Outcome::Extra(_)));
                            }
                            crate::procs::apply(out, Some(id), player, panels, wolves, self.combat_text.as_mut());
                        }
                    }
                    if let (true, Some(panels)) = (sneak, &mut self.panels) {
                        panels.notice("Sneak attack!");
                    }
                }
            }
            let world = &self.world;
            if let Some(panels) = &self.panels {
                wolves.set_markers(render_camera.pos, |kind| panels.marker(kind));
            }
            // The target frame follows the selected creature.
            let view = wolves.target_view(feet);
            if let Some(panels) = &mut self.panels {
                panels.set_target(view);
            }
            // A creature that starts at no height (a new game) finds no floor within a yard of it: it takes the terrain.
            let auras = self.foes.afflictions.spells();
            let watch = crate::creature::Watch {
                feet,
                yaw: self.camera.yaw,
                level: self.panels.as_ref().map_or(1, |p| p.progress.level),
                health: player.vitals.health.fraction(),
                mana: player.vitals.mana.fraction(),
                casting: self.caster.casting(),
                auras: &auras,
                sneaking: player.is_sneaking().then(|| self.panels.as_ref().map_or(1.0, |p| p.skills.notice_factor())),
            };
            wolves.ragdolls = self.ragdolls;
            let timing = Instant::now();
            let t = crate::profile::scope("creatures.update");
            let (acts, mut cries) = wolves.update(dt, &watch, |x, z, y| crate::collision::Collider::floor(world, x, z, y + 1.0).or_else(|| world.height_at(x, z)), |from, to| world.blocked(from, to), self.bodies.as_mut(), &mut self.renderer, &self.gpu);
            drop(t);
            self.creature_time += timing.elapsed();
            // The ticks of damage over time float up as numbers, like the blows.
            for (at, damage) in wolves.take_ticks() {
                if let Some(text) = &mut self.combat_text {
                    text.damage_dealt(at, damage);
                }
            }
            let t = crate::profile::scope("foes, effects, damage");
            // What they did: the swings that landed, the spells begun and gone off, and the auras that hold the player.
            let mut effects = Vec::new();
            for act in acts {
                effects.extend(self.foes.handle(act, feet, wolves, &mut self.effects, &mut self.renderer, &self.gpu));
            }
            effects.extend(self.foes.update(dt, feet, wolves, &mut self.effects, &mut self.renderer, &self.gpu));
            cries.extend(self.foes.take_sounds());
            // A bite on the shield rings instead of sounding like teeth in flesh.
            let mut blocked = false;
            if let Some(panels) = &self.panels {
                player.set_block_cost(panels.skills.block_cost_factor());
                player.set_mitigation(panels.stats.mitigation());
                player.set_regen(panels.stats.health_regen_always + if self.in_combat { 0.0 } else { panels.stats.health_regen }, panels.stats.mana_regen, panels.stats.mana_regen_casting);
            }
            if self.god {
                for r in [&mut player.vitals.health, &mut player.vitals.mana, &mut player.vitals.stamina] {
                    r.current = r.max;
                }
            }
            if safe {
                self.foes.afflictions.clear();
            }
            if let Some(panels) = &mut self.panels {
                panels.set_afflictions(self.foes.afflictions.all());
            }
            for effect in effects.into_iter().filter(|_| !safe) {
                use crate::foes::Effect;
                // What lands as damage, and whether a shield and armor deal with it (blows) or not (magic).
                let mut resisted = 1.0;
                let hit = match effect {
                    Effect::Hurt { amount, physical, school } => {
                        resisted = self.panels.as_ref().map_or(1.0, |p| 1.0 - p.stats.resistance(school));
                        Some((physical, amount))
                    }
                    Effect::Drain { amount, burn } => {
                        let took = player.drain_mana(amount);
                        (burn && took > 0.0).then_some((false, took))
                    }
                    Effect::Interrupt => {
                        self.caster.interrupt();
                        None
                    }
                    Effect::Knock { from, speed, up } => {
                        self.controller.knock_back(Vec3::new(feet.x - from.x, 0.0, feet.z - from.z).normalize_or_zero(), speed, up);
                        None
                    }
                };
                let Some((physical, mut amount)) = hit else { continue };
                // A blow is dodged, parried or blocked by one roll of the dice (the chances follow each other, as in the original's attack table).
                if physical {
                    let (dodge, parry, block, held) = self.panels.as_ref().map_or((0.0, 0.0, 0.0, 0.0), |p| (p.stats.dodge, p.stats.parry, p.stats.block, p.stats.block_value));
                    let roll = player.percent();
                    let word = if roll < dodge {
                        Some("Dodge")
                    } else if roll < dodge + parry {
                        Some("Parry")
                    } else if roll < dodge + parry + block {
                        // A block holds back some of the damage; only all of it is a block in the text.
                        amount = (amount - held).max(0.0);
                        (amount <= 0.0).then_some("Block")
                    } else {
                        None
                    };
                    if let (Some(word), Some(text)) = (word, &mut self.combat_text) {
                        text.message(word);
                    }
                    if let Some(word @ ("Dodge" | "Parry")) = word {
                        player.avoided(word == "Parry");
                    }
                    if word.is_some() {
                        continue;
                    }
                }
                // What absorbs blows takes up what it can of a physical one.
                if let (true, Some(panels)) = (physical, &mut self.panels) {
                    let left = panels.consumed.absorb(amount);
                    if left < amount {
                        if let (true, Some(text)) = (left <= 0.0, &mut self.combat_text) {
                            text.message("Absorb");
                        }
                        amount = left;
                    }
                    if amount <= 0.0 {
                        continue;
                    }
                }
                match if physical { player.take_hit(amount) } else { Some(player.take_magic(amount * resisted)) } {
                    None => {
                        blocked = true;
                        if let Some(panels) = &mut self.panels {
                            panels.train(crate::skills::Skill::Blocking, 1.0);
                        }
                    }
                    // The original's combat text shows blocks only with its "Show Damage Reduction" option, off by default.
                    Some(taken) => {
                        if let Some(text) = &mut self.combat_text {
                            text.damage_taken(taken);
                        }
                        // Every kind of armor worn is trained by its share of what is worn (by blows, not by magic).
                        if let (true, Some(panels)) = (physical, &mut self.panels) {
                            for (skill, share) in crate::skills::Skills::armor_shares(&panels.inv.equipment) {
                                panels.train(skill, share);
                            }
                        }
                        // What hits back at whoever strikes: shields and the spells of the gear that go off when struck.
                        if let (true, Some(panels), Some(attacker)) = (physical, &mut self.panels, wolves.attacker(feet)) {
                            let shields: Vec<(f32, u32)> = panels.consumed.shields().collect();
                            let reactions = panels.consumed.reactions();
                            let mut out = Vec::new();
                            let level = panels.progress.level;
                            crate::procs::struck(&panels.inv.equipment, &shields, &reactions, level, &mut || panels.consumed.dice(), &mut out);
                            crate::procs::apply(out, Some(attacker), player, panels, wolves, self.combat_text.as_mut());
                        }
                    }
                }
            }
            if let Some(text) = &mut self.combat_text {
                // "Entering Combat" when a wolf goes for the player, "Leaving Combat" 5 seconds after the last one stops.
                if wolves.engaged() {
                    self.quiet = 0.0;
                    if !std::mem::replace(&mut self.in_combat, true) {
                        log::info!("entering combat: {:?}", wolves.engaged_by(self.camera.pos));
                        text.message("Entering Combat");
                    }
                } else if self.in_combat {
                    self.quiet += dt;
                    if self.quiet >= 5.0 {
                        self.in_combat = false;
                        text.message("Leaving Combat");
                    }
                }
                // "Health Low" once when health falls to a fifth (`COMBAT_TEXT_LOW_HEALTH_THRESHOLD`).
                let low = player.vitals.health.fraction() <= 0.2 && player.vitals.health.current > 0.0;
                if low && !std::mem::replace(&mut self.low_health, true) {
                    text.message("Health Low");
                }
                self.low_health &= low;
            }
            drop(t);
            cries.retain(|&(sfx, _)| !(blocked && matches!(sfx, Sfx::Cry(Cry::Bite, _))));
            // The wolves' sounds come from where they are, heard from the camera.
            if let Some(audio) = &mut self.audio {
                audio.set_listener(render_camera.pos, render_camera.yaw);
                for (sfx, at) in cries {
                    audio.play_3d(sfx, world.surface_under(at - Vec3::Y * 0.8) as u32, at);
                }
            }
            let t = crate::profile::scope("plates, loot, hints");
            wolves.draw_plates(&mut self.renderer.ui, &render_camera, size);
            // Looting: the window closes when the player walks away, and what is left stays on the corpse.
            if let Some(panels) = &mut self.panels {
                let (chests, nodes) = (&mut self.chests, &mut self.nodes);
                if panels.looting().and_then(|i| if i >= FISH_LOOT { None } else if i >= NODE_LOOT { nodes.as_ref()?.pos(i - NODE_LOOT) } else if i >= CHEST_LOOT { chests.as_ref()?.pos(i - CHEST_LOOT) } else { wolves.corpse_pos(i) }).is_some_and(|at| (at - feet).length() > LOOT_LEAVE) {
                    panels.close_loot();
                }
                if panels.talking_to().and_then(|i| wolves.corpse_pos(i)).is_some_and(|at| (at - feet).length() > LOOT_LEAVE) {
                    panels.close_gossip();
                    panels.close_merchant();
                    panels.close_trainer();
                    panels.close_quest();
                }
                if let Some((id, left)) = panels.take_closed_loot() {
                    match (chests, nodes) {
                        _ if id >= FISH_LOOT => {}
                        (_, Some(nodes)) if id >= NODE_LOOT => nodes.set_loot(id - NODE_LOOT, left),
                        (Some(chests), _) if id >= CHEST_LOOT && id < NODE_LOOT => chests.set_loot(id - CHEST_LOOT, left),
                        _ => wolves.set_loot(id, left),
                    }
                }
                let chest = self.chests.as_ref().and_then(|c| Some(c.lock(c.near(feet, OPEN_RANGE)?).is_some()));
                let pickup = self.pickups.as_ref().is_some_and(|p| p.near(feet, OPEN_RANGE, &panels.quests.wanted(&panels.inv)).is_some());
                let skin = wolves.skinnable_near(feet, LOOT_RANGE).is_some();
                let fish_on = self.bobber.as_ref().is_some_and(|b| b.bite_left.is_some());
                let fish_ready = self.gather.is_none() && crate::fishing::pole_in_hand(&panels.inv) && fishing::water_ahead_of(&self.camera, &self.world).is_some();
                let node = self.nodes.as_ref().and_then(|n| n.kind(n.near(feet, OPEN_RANGE)?)).map(|k| k.skill == crate::skills::Skill::Mining);
                let bed = near_bed(&self.beds, &self.world, feet);
                let body = self.bodies.as_ref().is_some_and(|b| b.near(feet, crate::physics::GRAB_RANGE).is_some());
                let drag = self.bodies.as_ref().is_some_and(|b| b.near_corpse(feet, crate::physics::GRAB_RANGE).is_some());
                self.hint = match (panels.any_open() || self.lock.is_some(), wolves.lootable_near(feet, LOOT_RANGE), wolves.talkable_near(feet, TALK_RANGE), chest) {
                    (true, ..) if panels.loot_open() => Some("Press E to loot all"),
                    (true, ..) => None,
                    (_, Some(_), ..) if drag => Some("Press E to loot, hold E to drag"),
                    (_, Some(_), ..) => Some("Press E to loot"),
                    (_, None, _, Some(true)) => Some("Press E to pick the lock"),
                    (_, None, _, Some(false)) => Some("Press E to open"),
                    (_, None, _, None) if skin => Some("Press E to skin"),
                    (_, None, _, None) if fish_on => Some("Press E to reel in"),
                    (_, None, _, None) if fish_ready => Some("Press E to fish"),
                    (_, None, _, None) if pickup => Some("Press E to pick up"),
                    (_, None, _, None) if node == Some(true) => Some("Press E to mine"),
                    (_, None, _, None) if node == Some(false) => Some("Press E to gather"),
                    (_, None, _, None) if body => Some("Hold E to pick up"),
                    (_, None, _, None) if drag => Some("Hold E to drag"),
                    (_, None, _, None) if bed => Some("Press E to sleep"),
                    (_, None, Some(_), None) => Some("Press E to talk"),
                    _ => None,
                };
                // Creatures that died to the sword count for the kill goals of quests.
                panels.on_kills(&wolves.take_deaths());
                let [north, west, _] = render_to_wow(feet);
                panels.arrive(&self.map, north, west);
                wolves.set_wanted(panels.quests.wanted(&panels.inv));
                wolves.set_tomes(panels.learnable_tomes());
                panels.set_tracking(Some(wolves));
                panels.update_quests();
                let _t = crate::profile::scope("name labels");
                panels.draw_labels(&mut self.renderer.ui, &self.gpu, size, &wolves.labels(&render_camera, size, self.names, |from, to| self.world.ray_hit(from, to).is_some()));
            }
            drop(t);
            if let Some(text) = &mut self.combat_text {
                text.update(dt);
                text.draw(&mut self.renderer.ui, &render_camera, size);
            }
            // Dead: back at the spawn point with everything restored.
            if let (true, Some(share)) = (player.vitals.health.current <= 0.0, self.panels.as_mut().and_then(|p| p.consumed.take_ward())) {
                // His soul was kept: he gets up where he fell.
                log::info!("you died, and rose again");
                player.revive();
                (player.vitals.health.current, player.vitals.mana.current) = (player.vitals.health.max * share, player.vitals.mana.max * share);
                wolves.lose_player();
                self.foes.clear(&mut self.effects);
                if let Some(panels) = &mut self.panels {
                    panels.notice("You rise again.");
                }
            } else if player.vitals.health.current <= 0.0 {
                log::info!("you died");
                player.revive();
                self.foes.clear(&mut self.effects);
                if let Some(panels) = &mut self.panels {
                    panels.consumed.clear();
                    panels.refresh();
                }
                self.go_home();
                self.controller.reset();
            }
        }
    }

    /// The character leaps at a place, arriving in the time a jump of this height takes (a charge).
    fn leap_to(&mut self, at: Vec3) {
        let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
        let to = Vec3::new(at.x - feet.x, 0.0, at.z - feet.z);
        let (up, reach) = (5.0, (to.length() - 1.5).max(0.0));
        let airtime = 2.0 * up / crate::movement::GRAVITY;
        self.controller.knock_back(to.normalize_or_zero(), (reach / airtime).min(45.0), up);
    }

    /// The character is put this many yards ahead (a blink), short of whatever is in the way, and down on the ground there.
    fn blink(&mut self, yards: f32) {
        let forward = self.camera.forward();
        let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
        let from = self.camera.pos;
        let mut reach = yards;
        // Not through a wall: as far as the first thing in the way, less a body's width.
        if let Some(hit) = self.world.ray_hit(from, from + flat * yards) {
            reach = (hit.distance(from) - 0.8).max(0.0);
        }
        self.camera.pos += flat * reach;
        self.controller.reset();
        if !self.world.indoors(self.camera.pos - Vec3::Y * (EYE_HEIGHT - 1.0)) {
            self.snap_to_ground = Some(0.0);
        }
    }

    /// What spells asked of the game (see `Panels::take_requests`).
    fn spell_requests(&mut self) {
        use crate::panels::Request;
        for request in self.panels.as_mut().map(|p| p.take_requests()).unwrap_or_default() {
            match request {
                Request::Cleanse(mask) => self.foes.afflictions.cleanse(mask),
                Request::Vanish => {
                    if let Some(wolves) = &mut self.wolves {
                        wolves.lose_player();
                    }
                }
                Request::Blink(yards) => self.blink(yards),
                Request::Leap(at) => self.leap_to(at),
                Request::Foe(creature) => {
                    let (feet, yaw) = (self.camera.pos - Vec3::Y * EYE_HEIGHT, self.camera.yaw);
                    if let Some(wolves) = &mut self.wolves {
                        wolves.call_foe(creature, feet + Vec3::new(yaw.sin(), 0.0, -yaw.cos()) * 4.0, yaw + std::f32::consts::PI, 300.0);
                    }
                }
                Request::Tame(creature) => {
                    let (feet, level) = (self.camera.pos - Vec3::Y * EYE_HEIGHT, self.panels.as_ref().map_or(1, |p| p.progress.level));
                    let tamed = self.wolves.as_mut().is_some_and(|w| w.tame_for_quest(creature, feet, level));
                    if let Some(panels) = &mut self.panels {
                        if tamed {
                            panels.notice("The beast is tame.");
                        } else {
                            panels.error("There is no such beast to tame.");
                        }
                    }
                }
                Request::Scout { seconds, reach } => {
                    self.scout = Some((seconds, reach));
                    self.detached = true;
                }
                Request::Form { display, percent } => {
                    if let Some(player) = &mut self.player {
                        if let Err(e) = player.shift(display, percent, &mut self.renderer, &self.gpu) {
                            log::warn!("cannot take the shape: {e:#}");
                        }
                    }
                }
            }
        }
    }

    /// The things that go on whatever the frame shows: level up, sleep, travel, items in use, the steed, a changed set of equipment.
    fn upkeep(&mut self, dt: f32) {
        self.spell_requests();
        self.walk_into_portals();
        if let Some(panels) = &mut self.panels {
            let feet = Mat4::from_translation(self.camera.pos - Vec3::Y * EYE_HEIGHT);
            if panels.take_level_up() {
                self.levelfx = self.effects.spawn(&mut self.renderer, &self.gpu, crate::effects::LEVEL_UP, false, feet);
            }
            if let Some(id) = self.levelfx {
                self.effects.set_world(id, feet);
                if !self.effects.active(id) {
                    self.levelfx = None;
                }
            }
        }
        // Slept: the clock is at the hour chosen, and health, mana and stamina are full.
        if let Some(hour) = self.panels.as_mut().and_then(|p| p.take_sleep()) {
            self.hour = hour;
            if let Some(player) = &mut self.player {
                let v = &mut player.vitals;
                for r in [&mut v.health, &mut v.mana, &mut v.stamina] {
                    r.current = r.max;
                }
            }
            if let Some(panels) = &mut self.panels {
                panels.notice("You wake up rested.");
            }
            self.set_grab(true);
        }
        // A place picked on the world map: go there and hand the mouse back to the game.
        if let Some((north, west, height)) = self.panels.as_mut().and_then(|p| p.take_travel()) {
            self.teleport(north, west, height);
            self.set_grab(true);
        }
        // What using items began goes on: food and drink restore, elixirs run out, the way home counts down.
        let mut home_now = false;
        if let (Some(panels), Some(player)) = (&mut self.panels, &mut self.player) {
            panels.set_calm(!self.moving && !self.in_combat && !player.is_striking());
            panels.set_mounted(player.is_mounted());
            if panels.mounting() && self.world.indoors(self.camera.pos - Vec3::Y * (EYE_HEIGHT - 1.0)) {
                panels.cancel_mount("You can't mount here.");
            }
            let way_home = panels.tick_items(dt);
            if panels.take_stats_dirty() {
                player.apply_stats(&panels.stats);
            }
            if let Some((display, percent)) = panels.take_steed() {
                let inside = self.world.indoors(self.camera.pos - Vec3::Y * (EYE_HEIGHT - 1.0));
                if self.in_combat && !player.is_mounted() {
                    panels.error("You can't do that while in combat.");
                } else if inside && !player.is_mounted() {
                    panels.error("You can't mount here.");
                } else if let Err(e) = player.ride(display, percent, &mut self.renderer, &self.gpu) {
                    log::warn!("cannot ride: {e:#}");
                    panels.error("You can't mount that.");
                }
            }
            // A rider who goes indoors is put down: no horse goes through a door.
            self.indoor_check -= dt;
            if player.is_riding() && self.indoor_check <= 0.0 {
                self.indoor_check = 0.25;
                if self.world.indoors(self.camera.pos - Vec3::Y * (EYE_HEIGHT - 1.0)) {
                    player.dismount();
                    panels.error("You can't mount here.");
                }
            }
            let (heal, mana) = panels.take_restores();
            player.heal(heal);
            let mana_bar = &mut player.vitals.mana;
            mana_bar.current = (mana_bar.current + mana).min(mana_bar.max);
            home_now = way_home == Some(crate::consume::WayHome::Arrived);
        }
        if home_now {
            self.go_home();
        }
        if let (Some(panels), Some(player)) = (&mut self.panels, &mut self.player) {
            if panels.take_equipment_change() {
                player.set_equipment(&mut self.renderer, &self.gpu, &panels.inv.equipment, &panels.stats);
            }
            let _t = crate::profile::scope("portraits, paper doll");
            panels.render_pictures(&mut self.renderer, &self.gpu, player, &self.world);
            if let Some(wolves) = &self.wolves {
                panels.render_target(&mut self.renderer, &self.gpu, wolves, &self.world);
            }
        }
    }

    /// Draw the action bar, the windows, the editor, the console and the pointer, and take in what the windows asked of the game. True when
    /// something was discovered (the game is saved).
    fn draw_interface(&mut self, dt: f32, render_camera: &Camera, size: (u32, u32)) -> bool {
        if let (Some(bar), Some(player)) = (&self.actionbar, &self.player) {
            let cooldowns = self.panels.as_ref().map_or(&[][..], |p| &p.cooldowns[..]);
            let items = |i| self.panels.as_ref().map_or((0, 0.0), |p| p.item_state(i));
            bar.draw(&mut self.renderer.ui, size, player, self.panels.as_ref().is_some_and(|p| p.character_open()), self.panels.as_ref().map_or(crate::levels::Progress::new(), |p| p.progress), cooldowns, &items);
        }
        if let (Some(petbar), Some((orders, dead))) = (&self.petbar, self.wolves.as_ref().and_then(|w| w.pet_orders())) {
            petbar.draw(&mut self.renderer.ui, size, orders, dead, self.cursor);
        }
        let mut discovered = false;
        if let Some(panels) = &mut self.panels {
            let t = crate::profile::scope("windows");
            panels.draw(&mut self.renderer.ui, &self.gpu, size);
            panels.draw_overlay(&mut self.renderer.ui, &self.gpu, size, dt, self.hint);
            drop(t);
            panels.set_quest_targets(self.wolves.as_ref(), self.chests.as_ref(), self.pickups.as_ref());
            if self.lock.is_none() {
                let marks = self.nodes.as_ref().filter(|_| panels.compass_nodes()).map_or_else(Vec::new, |n| n.marks(self.camera.pos, self.camera.yaw).into_iter().filter(|m| panels.skills.knows(m.2)).collect());
                panels.draw_compass(&mut self.renderer.ui, &self.gpu, size, self.camera.pos, self.camera.yaw, &marks);
            }
            panels.draw_map(&mut self.renderer.ui, &self.gpu, size, self.camera.pos, self.camera.yaw);
            panels.draw_level(&mut self.renderer.ui, &self.gpu, size);
            if let Some(bar) = &self.actionbar {
                bar.draw_drag(&mut self.renderer.ui, size);
            }
            if let Some((_, lock)) = &self.lock {
                panels.draw_lockpick(&mut self.renderer.ui, &self.gpu, size, lock, self.lock_art.as_ref(), self.lock_time);
            }
            if self.player.as_ref().is_some_and(|p| p.is_sneaking()) {
                panels.draw_sneak(&mut self.renderer.ui, &self.gpu, size);
            }
            if self.lock.is_none() {
                discovered = panels.discover(self.camera.pos - Vec3::Y * EYE_HEIGHT);
                discovered |= panels.explore(self.area_now);
            }
            if self.grabbed && self.lock.is_none() {
                let alert = self.player.as_ref().is_some_and(|p| p.is_sneaking()).then(|| self.wolves.as_ref().map_or(0.0, |w| w.alert(self.camera.pos - Vec3::Y * EYE_HEIGHT, panels.progress.level)));
                if let Some(cursors) = &self.cursors {
                    cursors.reticle(&mut self.renderer.ui, size, alert);
                }
            }
            // The editor, over the windows and under the pointer.
            let hover = self.cursor.filter(|_| self.editor.as_ref().is_some_and(|e| e.placing())).and_then(|p| {
                let (origin, dir) = self.eye?.ray(glam::Vec2::new(p.0, p.1));
                self.world.ray_hit(origin, origin + dir * 400.0)
            });
            let editor_scope = crate::profile::scope("editor");
            if let Some(editor) = &mut self.editor {
                let [north, west, eye] = render_to_wow(self.camera.pos);
                editor.set_hover(hover);
                for action in editor.draw(&mut self.renderer.ui, size, crate::editor::Place { north, west, height: eye - EYE_HEIGHT, yaw: self.camera.yaw.to_degrees().rem_euclid(360.0), map: crate::editor::interned(&self.map) }, crate::propedit::Eye::of(&render_camera, size), &|x, z| self.world.height_at(x, z)) {
                    match action {
                        crate::editor::Action::Teleport(north, west) => match &mut self.free {
                            // The camera goes to look at the place from a little way off and above.
                            Some(free) => {
                                let target = Vec3::new(-west, self.world.height_at(-west, -north).unwrap_or(free.camera.pos.y), -north);
                                let (sy, cy) = free.camera.yaw.sin_cos();
                                free.camera.pos = target - Vec3::new(sy, 0.0, -cy) * 16.0 + Vec3::Y * 9.0;
                                free.camera.pitch = -0.45;
                            }
                            None => {
                                (self.camera.pos.x, self.camera.pos.z) = (-west, -north);
                                self.snap_to_ground = Some(0.0);
                                self.controller.reset();
                            }
                        },
                        crate::editor::Action::Apply => self.apply = true,
                        crate::editor::Action::Props(props) => {
                            // Those to pick up and those with physics are drawn by `pickups` and `bodies`, made again at the start of the next frame when they changed.
                            let pickups: Vec<content::Prop> = props.iter().filter(|p| p.pickup.is_some() || p.physics).cloned().collect();
                            if pickups != self.pickup_props {
                                (self.pickup_props, self.pickups_stale) = (pickups, true);
                            }
                            self.beds = bed_spots(&props, &self.map);
                            self.world.set_props(props);
                        }
                    }
                }
                editor.render_preview(&mut self.renderer, &self.gpu, &self.world, self.camera.pos, dt);
                editor.render_gallery(&mut self.renderer, &self.gpu, &self.world, self.camera.pos, false);
                editor.update_ghosts(&mut self.renderer, &self.gpu, self.world.refreshing(), dt);
            }
            drop(editor_scope);
            if let Some(console) = &self.console {
                console.draw(&mut self.renderer.ui, size);
                if crate::profile::enabled() {
                    let (fps, rows) = crate::profile::report();
                    console.draw_profile(&mut self.renderer.ui, size, fps, &rows);
                }
            }
            // The pointer, over everything; what the mouse carries is drawn instead of it.
            if let (Some(cursors), Some(pos), false) = (&self.cursors, self.cursor, panels.carrying()) {
                cursors.draw(&mut self.renderer.ui, self.pointer, pos);
            }
            if let Some(audio) = &mut self.audio {
                panels.take_events().into_iter().for_each(|sfx| audio.play(sfx, 0));
                if let Some(sound) = panels.take_sound() {
                    audio.apply(&sound);
                }
            }
            if let Some(i) = panels.take_interface() {
                (self.controller.look_scale, self.controller.invert_y) = (i.look_scale(), i.invert_mouse);
                if let Some(free) = &mut self.free {
                    (free.controller.look_scale, free.controller.invert_y) = (i.look_scale(), i.invert_mouse);
                }
                if let Some(p) = &mut self.player {
                    p.follow = i.follow;
                }
                (self.names, self.ragdolls) = (i.npc_names, i.ragdolls);
            }
            if let Some(binds) = panels.take_keys() {
                self.binds = binds;
                if let Some(bar) = &mut self.actionbar {
                    bar.set_hotkeys(&mut self.renderer.ui, &self.gpu, &binds.action_labels());
                }
            }
            if let Some(v) = panels.take_video() {
                self.weather.detail = v.weather_scale();
                self.effects.detail = v.particle_scale();
                (self.renderer.gamma, self.renderer.prop_reach, self.renderer.shadows) = (v.gamma, v.prop_reach(), v.shadows);
                crate::uixml::set_ui_scale(v.interface_scale());
                v.apply_window(&self.window);
                // The surface is set up again once this frame is shown (it cannot be while the frame is held).
                if self.config.present_mode != v.present_mode() {
                    self.config.present_mode = v.present_mode();
                    self.reconfigure = true;
                }
                if let Some(sky) = &mut self.sky {
                    sky.set_view_distance(v.distance());
                }
            }
            match panels.take_menu_action() {
                Some(crate::panels::MenuAction::Logout) => (self.logout, self.restart) = (true, true),
                Some(crate::panels::MenuAction::Quit) => self.quit = true,
                None => {}
            }
        }
        discovered
    }

    /// Count the frame, and now and then show the speed and the place in the window's title.
    fn count_frame(&mut self) {
        self.frames += 1;
        if self.title_timer.elapsed() >= Duration::from_millis(500) {
            let fps = self.frames as f32 / self.title_timer.elapsed().as_secs_f32();
            let render_ms = self.render_time.as_secs_f32() * 1000.0 / self.frames.max(1) as f32;
            let creature_ms = self.creature_time.as_secs_f32() * 1000.0 / self.frames.max(1) as f32;
            log::info!("{fps:.0} fps, render {render_ms:.2} ms on the CPU, creatures {creature_ms:.2} ms");
            self.render_time = Duration::ZERO;
            self.creature_time = Duration::ZERO;
            let [wx, wy, wz] = render_to_wow(self.camera.pos);
            let area = self
                .world
                .area_at(self.camera.pos)
                .map(|a| self.world.areas.display(a))
                .unwrap_or_else(|| "...".into());
            let loading = if self.world.pending_count() > 0 {
                format!(" | loading {} tiles", self.world.pending_count())
            } else {
                String::new()
            };
            self.window.set_title(&format!(
                "rsworld | {area} | ({wx:.0}, {wy:.0}, {wz:.0}) | {fps:.0} fps ({render_ms:.1} ms draw) | {:?} | {}{loading}",
                self.controller.mode,
                match self.player.as_ref().map(|p| p.view) {
                    Some(View::Third) => "third person",
                    Some(View::First) => "first person",
                    None => "no player",
                },
            ));
            self.frames = 0;
            self.title_timer = Instant::now();
        }
    }
}
