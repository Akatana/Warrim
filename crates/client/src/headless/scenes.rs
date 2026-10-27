//! What is drawn into the picture besides the world: a player model, the editor, a frame of the interface, the glue screens.

use super::*;

pub(super) fn view(args: &Args, player_mount: &Arc<Mount>, renderer: &mut Renderer, gpu: &Gpu, world: &World, render_camera: &mut Camera, camera: Camera, w: u32, h: u32) -> Result<()> {
    let Some(v) = &args.view else { return Ok(()) };
    let mut player = Player::load(&player_mount, &ModelCache::default(), renderer, gpu, &args.look)?;
    player.view = if v == "third" { View::Third } else { View::First };
    player.body_turn = args.pose_turn.to_radians();
    player.set_sneaking(args.sneak);
    if let Some((display, percent)) = args.mount {
        player.ride(display, percent, renderer, gpu)?;
    }
    let motion = match args.pose.as_str() {
        "walk" => Motion { speed: 2.5, forward_speed: 2.5, ..Default::default() },
        "run" => Motion { speed: 7.0, forward_speed: 7.0, ..Default::default() },
        "back" => Motion { speed: 2.5, forward_speed: -2.5, ..Default::default() },
        "strafe" => Motion { speed: 7.0, side_speed: 7.0, ..Default::default() },
        "jump" => Motion { airborne: true, vertical_speed: 3.0, ..Default::default() },
        "fall" => Motion { airborne: true, vertical_speed: -15.0, ..Default::default() },
        "swim" => Motion { speed: 4.7, forward_speed: 4.7, swimming: true, ..Default::default() },
        "swim-idle" => Motion { swimming: true, ..Default::default() },
        "swim-back" => Motion { speed: 2.5, forward_speed: -2.5, swimming: true, ..Default::default() },
        "swim-left" => Motion { speed: 4.7, side_speed: -4.7, swimming: true, ..Default::default() },
        _ => Motion::default(),
    };
    // The preview turns the view at this rate (radians per second) for the "turn-left" / "turn-right" poses.
    let turn = match args.pose.as_str() {
        "turn-left" => -2.0,
        "turn-right" => 2.0,
        _ => 0.0,
    };
    // The character sheet and the backpack, for the panel previews; worn items dress the character first.
    let mut panels = if !args.panel.is_empty() || !args.equip.is_empty() || !args.wear.is_empty() || args.npc || args.level.is_some() || args.target || args.compass || !args.cast.is_empty() { Some(crate::panels::Panels::new(player_mount.clone(), renderer, gpu)?) } else { None };
    if let Some(panels) = &mut panels {
        for &slot in &args.equip {
            let _ = panels.inv.equip_from_bag(slot, 60);
        }
        for id in &args.wear {
            // (`id#803`: with the enchantment 803 on it)
            let (id, enchant) = id.split_once('#').map_or((id.as_str(), 0), |(id, e)| (id, e.parse().unwrap_or(0)));
            match crate::items::all().iter().position(|i| i.id == id) {
                Some(item) => {
                    panels.inv.bag = vec![None; crate::items::BACKPACK_SIZE];
                    panels.inv.add(crate::items::Stack { item, count: 1, enchant });
                    if let Err(e) = panels.inv.equip_from_bag(0, 99) {
                        log::warn!("{id} is not put on: {e:?}");
                    }
                }
                None => log::warn!("no item {id}"),
            }
        }
        if let Some(level) = args.level {
            panels.progress = crate::levels::Progress { level, xp: args.xp };
        }
        // A quest taken ("taken": the grey mark over its giver) or ready to turn in ("ready": the yellow one).
        let wolves_quest = crate::content::quest_index("q33");
        if args.panel.iter().any(|p| p == "taken" || p == "ready") {
            panels.quests.accept(wolves_quest);
        }
        if args.panel.iter().any(|p| p == "ready") {
            panels.inv.add(crate::items::Stack { item: crate::items::find("tough_wolf_meat"), count: 8, enchant: 0 });
        }
        panels.refresh();
        player.set_equipment(renderer, gpu, &panels.inv.equipment, &panels.stats);
    }
    let feet = camera.pos - glam::Vec3::Y * EYE_HEIGHT;
    let (sy, cy) = camera.yaw.sin_cos();
    let yaw = camera.yaw;
    let mut look = camera;
    let mut text = crate::combattext::CombatText::load(&player_mount, &mut renderer.ui, gpu)?;
    let (wolf, guard) = (crate::content::creature_index("wolf"), crate::content::creature_index("deputy_willem"));
    let (ahead, aside) = (glam::Vec3::new(sy, 0.0, -cy), glam::Vec3::new(cy, 0.0, sy));
    let mut spawns = Vec::new();
    if args.wolf {
        // A second wolf far away stays idle: two wolves must not share a pose.
        spawns.extend([(wolf, feet + ahead * 4.0 + aside * 1.5, yaw + std::f32::consts::PI, 5.0), (wolf, feet + ahead * 30.0, yaw, 5.0)]);
    }
    if args.spawns {
        spawns.extend(crate::content::get().spawns.iter().filter(|s| s.map.as_deref().is_none_or(|m| m.eq_ignore_ascii_case(&args.map))).map(|s| (crate::content::creature_index(&s.creature), crate::coords::wow_to_render([s.at.0, s.at.1, s.height.unwrap_or(0.0)]), s.facing.to_radians(), s.wander)));
    }
    // Any creatures of the content, standing in a row ahead (`--creature kobold_miner,garrick_thorne`).
    for (i, id) in args.creatures.iter().enumerate() {
        spawns.push((crate::content::creature_index(id), feet + ahead * args.creature_at + aside * (i as f32 * 2.2 - args.creatures.len() as f32 + 1.0), yaw + std::f32::consts::PI, 5.0));
    }
    if args.npc {
        spawns.push((guard, feet + ahead * 5.0 - aside * 2.0, yaw + std::f32::consts::PI, 5.0));
    }
    let mut wolves = if spawns.is_empty() { None } else { Some(crate::creature::Creatures::load(&player_mount, &ModelCache::default(), renderer, gpu, &spawns)?) };
    if let Some(wolves) = &mut wolves {
        wolves.set_budget(usize::MAX);
    }
    // The chests of the world, shut or open.
    if let Some(state) = &args.chests {
        let mut chests = crate::chest::Chests::load(&player_mount, &ModelCache::default(), renderer, gpu)?;
        if state == "open" {
            (0..2).for_each(|i| drop(chests.open(i, &[])));
        }
        for _ in 0..120 {
            chests.update(1.0 / 60.0, |x, z| world.height_at(x, z), renderer, gpu);
        }
    }
    let mut effects = crate::effects::Effects::new(player_mount.clone());
    let mut foes = crate::foes::Foes::new(crate::visuals::load_classic(&player_mount).unwrap_or_default());
    if let Some(wolves) = &mut wolves {
        wolves.set_spell_anims(foes.anims());
    }
    let mut fires = crate::fires::Fires::default();
    let mut ripples = crate::ripples::Ripples::default();
    let mut ripple_draw = crate::ripples::RippleDraw::new(&player_mount, &ModelCache::default(), renderer, gpu);
    let mut caster = crate::casting::Spellcaster::new(crate::visuals::load(&player_mount)?);
    // An effect of the game's models (`--effect Spells\Fire_ImpactDD_Low_Chest.m2,300`) or the level up (`--levelfx 400`), caught that
    // many milliseconds after it began: six yards ahead, or round the character.
    let preview = |effects: &mut crate::effects::Effects, renderer: &mut crate::renderer::Renderer, path: &str, ms: f32, at: glam::Vec3| {
        let id = effects.spawn(renderer, gpu, path, false, glam::Mat4::from_translation(at));
        for _ in 0..(ms / 16.7) as usize {
            effects.update(1.0 / 60.0, &look, renderer, gpu);
        }
        log::info!("effect {path}: {:?}, {} particles alive at {at:?}", id, id.map_or(0, |i| effects.particles(i)));
    };
    if let Some((path, ms)) = &args.effect {
        preview(&mut effects, renderer, path, *ms, feet + ahead * 6.0 + glam::Vec3::Y);
    }
    if let Some(ms) = args.levelfx {
        preview(&mut effects, renderer, crate::effects::LEVEL_UP, ms, feet);
    }
    match args.pose.as_str() {
        "attack" => {
            player.attack_press();
            player.attack_release();
        }
        "heavy" => player.heavy_attack(),
        "block" => player.set_block(true),
        "sheath" => player.toggle_sheath(),
        "bow" | "shoot" => player.shoot_press(),
        p if p.starts_with("anim") => player.set_precast(Some(p[4..].parse().unwrap_or(0))),
        _ => {}
    }
    // Spells cast one after the other (`--cast fireball,holy_light`) at the first creature, which is selected.
    let mut queue: Vec<usize> = args.cast.iter().map(|id| crate::content::spell_index(id)).rev().collect();
    if let (false, Some(wolves)) = (queue.is_empty(), &mut wolves) {
        wolves.set_target(0);
    }
    if let (true, Some(wolves)) = (args.kill, &mut wolves) {
        // Whatever stands within reach ahead (the first of `--creature`) dies at once.
        wolves.strike(feet + ahead * 3.0, yaw, 1e4, 1.0, &mut |_, _| Some(0.0));
    }
    for frame in 0..(args.pose_time * 60.0).max(1.0) as usize {
        look.yaw += turn / 60.0;
        // `--pose shoot`: the string is let go after two seconds.
        if args.pose == "shoot" && frame == 120 {
            player.shoot_release();
        }
        *render_camera = player.update(1.0 / 60.0, &look, motion, |from, to| world.camera_distance(from, to), renderer, gpu);
        if let Some(panels) = &mut panels {
            panels.update_cast_bar(1.0 / 60.0);
            let mut ctx = crate::casting::Ctx { player: &mut player, panels, wolves: wolves.as_mut(), text: Some(&mut text), effects: &mut effects, renderer: renderer, gpu: gpu, world: world };
            if let (false, Some(spell)) = (caster.casting(), queue.last().copied()) {
                queue.pop();
                ctx.panels.cooldowns.iter_mut().for_each(|c| *c = 0.0);
                caster.begin(&mut ctx, spell, feet);
            }
            if let Some(shot) = ctx.player.take_shot() {
                caster.shoot(&mut ctx, shot, feet, look.forward());
            }
            caster.update(&mut ctx, 1.0 / 60.0, feet, false);
        }
        if let (Some(kind), Some(level)) = (&args.ripples, world.liquid_at(feet.x, feet.z, feet.y)) {
            let (state, speed) = match kind.as_str() {
                "walk" => (crate::ripples::State::Moving, 2.5),
                "run" => (crate::ripples::State::Moving, 7.0),
                _ => (crate::ripples::State::Idle, 0.0),
            };
            let walker = crate::ripples::Walker { feet: glam::Vec3::new(feet.x, level - 0.6, feet.z), level, height: crate::movement::DEFAULT_HEIGHT, scale: 1.0, speed, heading: (sy, -cy) };
            ripples.update(1.0 / 60.0);
            ripples.make(&walker, state, |at, half| world.liquid_quads(at.x, at.z, level - 0.6, half));
            if let Some(draw) = &mut ripple_draw {
                draw.draw(&ripples, feet, renderer, gpu);
            }
        }
        fires.update(world, render_camera.pos, &mut effects, renderer, gpu);
        effects.update(1.0 / 60.0, &render_camera, renderer, gpu);
        if let Some(wolves) = &mut wolves {
            if player.take_swing_hit().is_some() {
                let damage = player.roll_damage();
                for (at, ..) in wolves.strike(feet, yaw, damage, 1.0, &mut |_, _| Some(0.0)) {
                    text.damage_dealt(at, damage as u32);
                }
            }
            text.update(1.0 / 60.0);
            if let Some(panels) = &panels {
                wolves.set_markers(render_camera.pos, |kind| panels.marker(kind));
            }
            let watch = crate::creature::Watch { feet, yaw, level: 1, health: player.vitals.health.fraction(), mana: 1.0, casting: false, auras: &foes.afflictions.spells(), sneaking: args.sneak.then_some(1.0) };
            let (acts, _) = wolves.update(1.0 / 60.0, &watch, |x, z, y| crate::collision::Collider::floor(world, x, z, y + 1.0).or_else(|| world.height_at(x, z)), |from, to| world.blocked(from, to), None, renderer, gpu);
            let mut effect_list: Vec<crate::foes::Effect> = acts.into_iter().flat_map(|act| foes.handle(act, feet, wolves, &mut effects, renderer, gpu)).collect();
            effect_list.extend(foes.update(1.0 / 60.0, feet, wolves, &mut effects, renderer, gpu));
            for effect in effect_list {
                if let crate::foes::Effect::Hurt { amount, physical, .. } = effect {
                    if physical { player.take_hit(amount); } else { player.take_magic(amount); }
                }
            }
        }
    }
    log::info!("fires: {} burning of {} in the loaded tiles", fires.burning(), world.fires().count());
    if let Some(wolves) = &wolves {
        log::info!("creatures in the world: {} made, {} with a model", wolves.counts().0, wolves.counts().1);
        log::info!("the pet: {:?}", wolves.pet_kind());
        wolves.draw_plates(&mut renderer.ui, &render_camera, (w, h));
        if let Some(panels) = &mut panels {
            panels.draw_labels(&mut renderer.ui, gpu, (w, h), &wolves.labels(&render_camera, (w, h), crate::interface::InterfaceSettings::load().npc_names, |from, to| world.ray_hit(from, to).is_some()));
        }
        text.draw(&mut renderer.ui, &render_camera, (w, h));
    }
    // The target frame with the first creature selected.
    if let (Some(panels), Some(wolves), true) = (&mut panels, &mut wolves, args.target && args.panel.is_empty()) {
        wolves.set_target(0);
        // (the circle shows here whatever the option says)
        if let Some(mut draw) = crate::selection::SelectionDraw::new(player_mount, &ModelCache::default(), renderer, gpu) {
            draw.draw(wolves.target_circle(), |x, z, y| crate::collision::Collider::floor(world, x, z, y), renderer, gpu);
        }
        // A few debuffs on it, to see how they are listed (a bleed, a curse, a poison).
        for (name, icon, dispel) in [("Rend", "Ability_Gouge", 0), ("Curse of Agony", "Spell_Shadow_CurseOfSargeras", 2), ("Deadly Poison", "Ability_Poisons", 4)] {
            wolves.set_source(name, icon, dispel);
            wolves.afflict(0, crate::creature::Status::Over { amount: 1.0, period: 3.0, seconds: 30.0 });
        }
        wolves.set_source("", "", 0);
        let view = wolves.target_view(feet);
        panels.set_target(view);
        panels.render_target(renderer, gpu, wolves, world);
        panels.draw(&mut renderer.ui, gpu, (w, h));
    }
    if let (Some(panels), false) = (&mut panels, args.panel.is_empty()) {
        // A character who has learned every spell and has trained a few skills, for the pictures of those windows.
        if args.panel.iter().any(|p| p == "spellbook" || p == "skills") {
            for spell in 0..crate::content::get().spells.len() {
                panels.learn(spell);
            }
            for (i, skill) in crate::skills::ALL.iter().enumerate() {
                panels.skills.train(*skill, (i * i * 7) as f32);
            }
        }
        // A fishing pole in the hand and lures in the pack, the backpack open (`--panel lure`, then `--clicks` on the lures).
        if args.panel.iter().any(|p| p == "lure") {
            panels.skills.set(crate::skills::Skill::Fishing, 60);
            panels.inv.bag = vec![None; crate::items::BACKPACK_SIZE];
            for (id, count) in [("i6256", 1), ("i6529", 20), ("i6530", 20)] {
                panels.inv.add(crate::items::Stack { item: crate::items::find(id), count, enchant: 0 });
            }
            if !args.panel.iter().any(|p| p == "no-pole") {
                let _ = panels.inv.equip_from_bag(0, 60);
            }
            panels.toggle_backpack();
        }
        // Two tomes in the pack, read: a basic one is learned, a greater one asks for a rank the character has not got.
        if args.panel.iter().any(|p| p == "tome") {
            for id in ["tome_fireball", "tome_arcane_explosion"] {
                panels.inv.add(crate::items::Stack { item: crate::items::find(id), count: 1, enchant: 0 });
            }
            let first = panels.inv.bag.iter().position(|s| s.is_some_and(|s| crate::items::get(s.item).tome.is_some())).unwrap_or(0);
            panels.read_tome(first);
            panels.read_tome(first + 1);
            panels.toggle_backpack();
        }
        if args.panel.iter().any(|p| p == "character" || p == "skills") {
            panels.toggle_character();
        }
        if args.panel.iter().any(|p| p == "skills") {
            panels.show_skills_tab(true);
        }
        if args.panel.iter().any(|p| p == "recipes") {
            panels.recipes_preview();
        }
        if args.panel.iter().any(|p| p == "spellbook") {
            panels.toggle_spellbook();
        }
        if args.panel.iter().any(|p| p == "trainer") {
            panels.trainer_preview();
        }
        if args.panel.iter().any(|p| p == "trade") {
            panels.trade_preview(crate::skills::Skill::Blacksmithing);
        }
        if args.panel.iter().any(|p| p == "enchant") {
            panels.trade_preview(crate::skills::Skill::Enchanting);
        }
        if args.panel.iter().any(|p| p == "talents") {
            panels.talents_preview();
            panels.toggle_talents();
        }
        if args.panel.iter().any(|p| p == "map" || p == "map-zoom") {
            panels.found.iter_mut().take(args.discover).for_each(|f| *f = true);
            panels.explore(world.area_at(render_camera.pos));
            panels.toggle_map();
            if args.panel.iter().any(|p| p == "map-zoom") {
                panels.map_zoom(args.cursor.unwrap_or((640.0, 360.0)), (w, h), 6.0);
            }
        }
        if args.panel.iter().any(|p| p == "menu" || p == "sound" || p == "video" || p == "interface" || p == "dropdown" || p == "keys") {
            panels.open_menu();
            if args.panel.iter().any(|p| p == "keys") {
                panels.menu_click("GameMenuButtonKeybindings");
            }
            if args.panel.iter().any(|p| p == "sound") {
                panels.menu_click("GameMenuButtonSoundOptions");
            }
            if args.panel.iter().any(|p| p == "video") {
                panels.menu_click("GameMenuButtonOptions");
            }
            if args.panel.iter().any(|p| p == "dropdown") {
                panels.menu_click("GameMenuButtonOptions");
                panels.set_video_modes(vec![(1920, 1080, 165), (1920, 1080, 60), (1280, 720, 60)]);
                panels.menu_click(if args.panel.iter().any(|p| p == "mode") { "OptionsFrameMultiSampleDropDownButton" } else { "OptionsFrameResolutionDropDownButton" });
            }
            if args.panel.iter().any(|p| p == "interface") {
                panels.menu_click("GameMenuButtonUIOptions");
            }
        }
        if args.panel.iter().any(|p| p == "bag") {
            panels.toggle_backpack();
        }
        if args.panel.iter().any(|p| p == "loot") {
            let stack = |item, count| crate::items::Stack { item, count, enchant: 0 };
            panels.open_loot(0, crate::loot::Loot { copper: 7, items: vec![stack(crate::items::find("tough_wolf_meat"), 2), stack(crate::items::find("ruined_pelt"), 1)] });
        }
        if args.panel.iter().any(|p| p == "gossip") {
            panels.open_gossip(0, crate::content::creature("deputy_willem"));
        }
        if args.panel.iter().any(|p| p == "trainer") {
            panels.open_gossip(0, crate::content::creature("instructor_hale"));
        }
        // An elixir, a scroll and food at work, for the icons of the buffs.
        if args.panel.iter().any(|p| p == "buffs") {
            for id in ["i3825", "i954", "i117"] {
                let item = crate::items::find(id);
                panels.inv.add(crate::items::Stack { item, count: 1, enchant: 0 });
                let slot = panels.inv.bag.iter().position(|s| s.is_some_and(|s| s.item == item)).unwrap_or(0);
                let _ = panels.consumed.use_item(&mut panels.inv, slot, 60, true);
            }
            panels.refresh();
        }
        for (name, page) in [("merchant", 0), ("merchant2", 1), ("merchant3", 2)] {
            if args.panel.iter().any(|p| p == name) {
                panels.open_merchant(0, crate::content::creature("trader_aldric"));
                panels.merchant_page_to(page);
            }
        }
        // The quest window at each of its screens, and the tracker with a quest half done.
        let (willem, wolves_quest) = (crate::content::creature("c196"), crate::content::quest_index("q33"));
        for (name, screen) in [("quest", crate::panels::QuestScreen::Detail), ("quest-progress", crate::panels::QuestScreen::Progress), ("quest-reward", crate::panels::QuestScreen::Reward)] {
            if args.panel.iter().any(|p| p == name) {
                panels.quests.accept(wolves_quest);
                panels.open_quest(0, willem, wolves_quest, screen);
            }
        }
        if args.panel.iter().any(|p| p == "log") {
            panels.quests.accept(wolves_quest);
            panels.quests.accept(crate::content::quest_index("q5261"));
            panels.on_kills(&[crate::content::creature_index("c6")]);
            panels.toggle_quest_log();
        }
        if args.panel.iter().any(|p| p == "abandon") {
            panels.quests.accept(wolves_quest);
            panels.toggle_quest_log();
            panels.layout(&mut renderer.ui, gpu, (w, h));
            // The log's "Abandon Quest" button asks for confirmation.
            panels.mouse_down((75.0, 513.0), (w, h), true);
            panels.mouse_up((75.0, 513.0), (w, h), true);
        }
        if args.panel.iter().any(|p| p == "sleep") {
            panels.ask_sleep();
        }
        if args.panel.iter().any(|p| p == "levelup" || p == "levelhint") {
            panels.gain_xp(1000);
        }
        if args.panel.iter().any(|p| p == "levelup") {
            panels.toggle_level();
        }
        if args.panel.iter().any(|p| p == "lockpick") {
            // A lock half turned with the pick off the sweet spot: the pick strains.
            let mut lock = crate::lockpick::Lockpick::new(crate::lockpick::Level::Adept, 20.0);
            lock.rotate(60.0);
            lock.update(0.4, true);
            let art = crate::lockpick::LockArt::load(&player_mount, &mut renderer.ui, gpu)?;
            panels.draw_lockpick(&mut renderer.ui, gpu, (w, h), &lock, Some(&art), 0.01);
        }
        if args.panel.iter().any(|p| p == "tracker") {
            panels.quests.accept(wolves_quest);
            panels.quests.accept(crate::content::quest_index("q7"));
            panels.on_kills(&[crate::content::creature_index("c6")]);
        }
        if args.panel.iter().any(|p| p == "error") {
            panels.error("Inventory is full.");
        }
        panels.layout(&mut renderer.ui, gpu, (w, h));
        for &pos in &args.clicks {
            // A negative x is a right click at that distance from the left.
            let (pos, left) = ((pos.0.abs(), pos.1), pos.0 >= 0.0);
            panels.hover(Some(pos), (w, h));
            panels.mouse_down(pos, (w, h), true);
            panels.mouse_up(pos, (w, h), left);
        }
        panels.take_equipment_change();
        player.set_equipment(renderer, gpu, &panels.inv.equipment, &panels.stats);
        panels.hover(args.cursor.or(args.clicks.last().copied()), (w, h));
        panels.render_pictures(renderer, gpu, &player, world);
        if let (Some(wolves), true) = (&mut wolves, args.target) {
            // The target after the windows opened: the one talked to shows his face in them.
            wolves.set_target(0);
            let view = wolves.target_view(feet);
            panels.set_target(view);
            panels.render_target(renderer, gpu, wolves, world);
        }
        if args.panel.iter().any(|p| p == "breath") {
            panels.set_breath(Some((20.0, 30.0)));
        }
        panels.draw(&mut renderer.ui, gpu, (w, h));
        panels.draw_overlay(&mut renderer.ui, gpu, (w, h), 0.0, args.panel.iter().any(|p| p == "hint").then_some("Press E to loot"));
        if args.compass {
            panels.found.iter_mut().take(args.discover).for_each(|f| *f = true);
            panels.draw_compass(&mut renderer.ui, gpu, (w, h), render_camera.pos, render_camera.yaw, &crate::nodes::Nodes::load(&args.map).marks(render_camera.pos, render_camera.yaw));
        }
        panels.draw_map(&mut renderer.ui, gpu, (w, h), render_camera.pos, render_camera.yaw);
        panels.draw_level(&mut renderer.ui, gpu, (w, h));
        if args.sneak {
            panels.draw_sneak(&mut renderer.ui, gpu, (w, h));
        }
    }
    // The player previews show the interface too.
    let bar = crate::actionbar::ActionBar::load(&player_mount, &mut renderer.ui, gpu)?;
    bar.draw(&mut renderer.ui, (w, h), &player, args.panel.iter().any(|p| p == "character"), panels.as_ref().map_or(crate::levels::Progress::new(), |p| p.progress), panels.as_ref().map_or(&[][..], |p| &p.cooldowns[..]), &|i| panels.as_ref().map_or((0, 0.0), |p| p.item_state(i)));
    // `--panel pet` shows the pet bar without a pet.
    if args.panel.iter().any(|p| p == "pet") {
        crate::petbar::PetBar::load(&player_mount, &mut renderer.ui, gpu).draw(&mut renderer.ui, (w, h), crate::creature::PetOrders { attack: Some(0), ..Default::default() }, false, args.cursor);
    }
    if args.reticle {
        let art = crate::cursor::Cursors::load(&player_mount, &mut renderer.ui, gpu)?;
        art.reticle(&mut renderer.ui, (w, h), args.sneak.then(|| args.alert.unwrap_or_else(|| wolves.as_ref().map_or(0.0, |c| c.alert(feet, 1)))));
    }
    // The mouse pointer at `--cursor`, as the game would draw it over what is there.
    if let Some(pos) = args.cursor {
        let art = crate::cursor::Cursors::load(&player_mount, &mut renderer.ui, gpu)?;
        let over_window = panels.as_mut().and_then(|p| p.cursor_at(pos, (w, h)));
        let kind = over_window.or_else(|| wolves.as_ref().and_then(|c| c.pick(&render_camera, (w, h), pos)).map(|p| p.1)).unwrap_or(crate::cursor::Kind::Point);
        if !panels.as_ref().is_some_and(|p| p.carrying()) {
            art.draw(&mut renderer.ui, kind, pos);
        }
    }
    Ok(())
}

pub(super) fn editor(args: &Args, player_mount: &Arc<Mount>, renderer: &mut Renderer, gpu: &Gpu, world: &World, render_camera: &Camera, w: u32, h: u32) -> Result<()> {
    let Some((tab, n, steps)) = &args.editor else { return Ok(()) };
    let font = fontdue::Font::from_bytes(player_mount.read(r"Fonts\ARIALN.TTF")?, fontdue::FontSettings::default()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut editor = crate::editor::Editor::new(renderer, gpu, &font, player_mount.clone())?;
    editor.show(tab, *n);
    editor.mouse_moved((-1.0, -1.0));
    let place = crate::editor::Place { north: -8949.0, west: -132.0, height: 85.0, yaw: 0.0, map: "Azeroth" };
    // Each step is a frame: a click is taken by the frame after it.
    for step in steps {
        editor.draw(&mut renderer.ui, (w, h), place, crate::propedit::Eye::of(&render_camera, (w, h)), &|x, z| world.height_at(x, z));
        editor.render_preview(renderer, gpu, world, render_camera.pos, 0.5);
        editor.update_ghosts(renderer, gpu, false, 0.5);
        renderer.ui.discard();
        match step.split_once(':') {
            Some(("t", text)) => editor.typed(text),
            // `press:x/y`: the left button goes down over this pixel of the world (selects what is there, or takes hold of the gizmo).
            Some(("press", xy)) => {
                let (x, y) = xy.split_once('/').context("press:x/y")?;
                editor.mouse_moved((x.parse()?, y.parse()?));
                editor.world_press(&|x, z| world.height_at(x, z), &[]);
            }
            // `wheel:n`: the wheel turns n lines over the form (a negative number scrolls down).
            Some(("wheel", n)) => {
                editor.mouse_moved((450.0, 300.0));
                editor.wheel(n.parse()?);
            }
            // `back:n`: n times Backspace.
            Some(("back", n)) => (0..n.parse::<usize>()?).for_each(|_| editor.key(crate::editor::Key::Backspace)),
            // `hover:x/y`: the pointer is over this pixel of the world (for the ghost of what is put down).
            Some(("hover", xy)) => {
                let (x, y) = xy.split_once('/').context("hover:x/y")?;
                let (origin, dir) = crate::propedit::Eye::of(&render_camera, (w, h)).ray(glam::Vec2::new(x.parse()?, y.parse()?));
                editor.set_hover(world.ray_hit(origin, origin + dir * 400.0));
            }
            Some((x, y)) => {
                editor.mouse_moved((x.parse()?, y.parse()?));
                editor.click();
            }
            None => {}
        }
    }
    // The frame that takes the last click, then the one that is shown (after the gallery's pictures were made).
    editor.draw(&mut renderer.ui, (w, h), place, crate::propedit::Eye::of(&render_camera, (w, h)), &|x, z| world.height_at(x, z));
    editor.render_preview(renderer, gpu, world, render_camera.pos, 0.5);
    editor.render_gallery(renderer, gpu, world, render_camera.pos, true);
    renderer.ui.discard();
    editor.draw(&mut renderer.ui, (w, h), place, crate::propedit::Eye::of(&render_camera, (w, h)), &|x, z| world.height_at(x, z));
    editor.render_preview(renderer, gpu, world, render_camera.pos, 0.5);
    editor.render_gallery(renderer, gpu, world, render_camera.pos, true);
    renderer.ui.discard();
    editor.draw(&mut renderer.ui, (w, h), place, crate::propedit::Eye::of(&render_camera, (w, h)), &|x, z| world.height_at(x, z));
    editor.update_ghosts(renderer, gpu, false, 0.5);
    Ok(())
}

pub(super) fn show(args: &Args, player_mount: &Arc<Mount>, renderer: &mut Renderer, gpu: &Gpu, w: u32, h: u32) -> Result<()> {
    let Some(first) = args.show.first() else { return Ok(()) };
    let mut tree = crate::uixml::Tree::new(std::rc::Rc::new(if args.glue { crate::uixml::Registry::load_glue(&player_mount)? } else { crate::uixml::Registry::load(&player_mount)? }));
    tree.instantiate(first)?;
    args.show.iter().for_each(|n| tree.show(n, true));
    tree.draw(&mut renderer.ui, gpu, &player_mount, (w, h));
    Ok(())
}

pub(super) fn glue_screen(args: &Args, player_mount: &Arc<Mount>, renderer: &mut Renderer, gpu: &Gpu, w: u32, h: u32) -> Result<()> {
    let Some(screen) = &args.glue_screen else { return Ok(()) };
    let dir = args.glue_dir.clone().unwrap_or_else(crate::save::characters_dir);
    let mut glue = crate::glue::Glue::new(player_mount.clone(), renderer, gpu, dir, Some(screen))?;
    glue.draw(renderer, gpu, (w, h), 0.1);
    renderer.ui.discard();
    glue.typed(&args.glue_type);
    for &pos in &args.clicks {
        glue.mouse_moved(pos);
        glue.mouse_down(pos);
        glue.mouse_up(pos);
        glue.draw(renderer, gpu, (w, h), 0.1);
        renderer.ui.discard();
    }
    if let Some(cursor) = args.cursor {
        glue.mouse_moved(cursor);
    }
    glue.draw(renderer, gpu, (w, h), 0.1);
    log::info!("glue outcome: {:?}", glue.outcome());
    Ok(())
}

pub(super) fn loading(args: &Args, player_mount: &Arc<Mount>, renderer: &mut Renderer, gpu: &Gpu, w: u32, h: u32) -> Result<()> {
    let Some(progress) = args.loading else { return Ok(()) };
    use crate::loading::Stage::*;
    let mut loading = crate::loading::Loading::load(&player_mount, &args.map, &mut renderer.ui, gpu)?;
    [Interface, Terrain, Objects].into_iter().for_each(|s| loading.report(s, progress));
    loading.draw(&mut renderer.ui, (w, h));
    Ok(())
}
