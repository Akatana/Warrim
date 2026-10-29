//! Drawing: the pictures, overlays, scroll bars, the target frame and the whole interface.

use super::*;

/// An icon of the buff frame: its picture (file), the seconds left and the spell of the original that put it there, if the game knows it.
pub(super) struct PlayerAura {
    pub(super) icon: String,
    pub(super) left: f32,
    pub(super) spell: Option<u32>,
}

impl Panels {
    // ---- the pictures

    /// Aim the doll and portrait cameras at the player and draw the character's model into them.
    pub fn render_pictures(&mut self, renderer: &mut Renderer, gpu: &Gpu, player: &Player, world: &World) {
        if !self.character_open() && !self.talents_open() && !self.trade_open() {
            return;
        }
        let (feet, facing) = (player.feet(), player.facing());
        let ids = player.entities();
        // In front of the character, turned by the rotate buttons: 5.2 yards away at the height of its middle.
        let around = facing + std::f32::consts::PI + self.doll_turn;
        let (sy, cy) = (around.sin(), around.cos());
        let mut doll = Camera::new(feet + Vec3::new(-sy * 5.2, 1.0, cy * 5.2), around, 0.0);
        (doll.fov_y, doll.aspect, doll.near, doll.far) = (25f32.to_radians(), DOLL_SIZE.0 as f32 / DOLL_SIZE.1 as f32, 0.1, 80.0);
        renderer.render_offscreen(gpu, self.doll.0, &doll, &ids, world.tiles());
        // The portrait is the face, straight on, from a yard and a quarter.
        let front = facing + std::f32::consts::PI;
        let (sf, cf) = (front.sin(), front.cos());
        let mut face = Camera::new(feet + Vec3::new(-sf * 1.25, 1.84, cf * 1.25), front, 0.0);
        (face.fov_y, face.aspect, face.near, face.far) = (30f32.to_radians(), 1.0, 0.1, 40.0);
        renderer.render_offscreen(gpu, self.portrait.0, &face, &ids, world.tiles());
    }

    /// Queue the messages that show whether or not a panel is open: the error at the top and a `hint` low on the screen.
    pub fn draw_overlay(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32), dt: f32, hint: Option<&str>) {
        let mount = self.mount.clone();
        // `UIErrorsFrame` sits 122 units below the top, its lines stacked downward; the text is `ErrorFont`.
        self.update_cast_bar(dt);
        for (_, _, age) in &mut self.messages {
            *age += dt;
        }
        self.messages.retain(|(_, _, age)| *age < ERROR_SECONDS);
        let s = screen.1 as f32 / ui_height();
        for (i, (text, color, age)) in self.messages.clone().into_iter().enumerate() {
            let alpha = if age < ERROR_SECONDS - ERROR_FADE { 1.0 } else { ((ERROR_SECONDS - age) / ERROR_FADE).max(0.0) };
            self.tree.draw_text_aligned(ui, gpu, &mount, screen, "ErrorFont", &text, (screen.0 as f32 / 2.0, (122.0 + 20.0 * i as f32) * s), 0.5, [color[0], color[1], color[2], alpha]);
        }
        if self.breath_shown {
            // The label of the breath bar, at the top of `MirrorTimer1` (96 units below the top of the screen).
            self.tree.draw_text_aligned(ui, gpu, &self.mount.clone(), screen, "GameFontHighlight", "Breath", (screen.0 as f32 / 2.0, 96.0 * s), 0.5, WHITE);
        }
        self.draw_tracker(ui, gpu, screen);
        self.draw_trade(ui, gpu, screen);
        self.draw_load(ui, gpu, screen);
        if let Some(hint) = hint {
            self.tree.draw_text_centered(ui, gpu, &mount, screen, "GameFontHighlight", hint, 190.0, WHITE);
        }
    }

    /// What the character carries out of what it can, under the title of the open backpack (red when it is slowed).
    pub(super) fn draw_load(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32)) {
        let Some(frame) = self.backpack_open().then(|| self.tree.rect_of(BACKPACK, screen)).flatten() else { return };
        let (mount, s, (weight, capacity)) = (self.mount.clone(), screen.1 as f32 / ui_height(), self.load());
        let at = ((frame.x + frame.w - 10.0 * CONTAINER_SCALE) * s, screen.1 as f32 - (frame.y + frame.h - 30.0 * CONTAINER_SCALE) * s);
        let color = if weight > capacity { RED } else { WHITE };
        self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontHighlightSmall", &format!("Weight: {weight:.1} / {capacity:.0}"), at, 1.0, color);
    }

    /// The quests being worked on, at the top right of the screen like `QuestWatchFrame`: the title and a line for each goal.
    pub(super) fn draw_tracker(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32)) {
        let mount = self.mount.clone();
        let s = screen.1 as f32 / ui_height();
        let (x, mut y) = (screen.0 as f32 - 24.0 * s, 160.0 * s);
        let step = 15.0 * s;
        for active in self.quests.active.clone() {
            let q = content::quest(active.quest);
            self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontNormal", &q.title, (x, y), 1.0, GOLD);
            y += step;
            if q.goals.is_empty() {
                self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontHighlightSmall", &format!(" - {}", q.objective), (x, y), 1.0, WHITE);
                y += step;
            }
            for (g, goal) in q.goals.iter().enumerate() {
                let (name, need) = match *goal {
                    Goal::Kill { creature, count } => (format!("{} slain", content::get().creatures[creature].name), count),
                    Goal::Collect { item, count } => (crate::items::get(item).name.to_string(), count),
                    Goal::Reach { .. } => ("Destination".to_string(), 1),
                };
                let have = self.quests.have(active.quest, g, &self.inv);
                let color = if have >= need { GREY } else { WHITE };
                self.tree.draw_text_aligned(ui, gpu, &mount, screen, "GameFontHighlightSmall", &format!(" - {name}: {have}/{need}"), (x, y), 1.0, color);
                y += step;
            }
            y += 5.0 * s;
        }
    }

    /// The wheel turned over a window: its scroll frame moves. False when there is nothing there to scroll.
    pub fn scroll(&mut self, pos: (f32, f32), screen: (u32, u32), lines: f32) -> bool {
        if self.map_open {
            self.map_zoom(pos, screen, lines);
            return true;
        }
        if self.keys_open() {
            self.keys_wheel(lines);
            return true;
        }
        if self.sleep_step(lines.signum()) {
            return true;
        }
        self.any_open() && (self.backpack_wheel(pos, screen, lines) || self.skills_wheel(lines) || self.trade_wheel(lines) || self.trainer_wheel(lines) || self.tree.scroll_at(pos, screen, lines))
    }

    /// The wheel over the backpack moves its window a row up or down the list of slots.
    pub(super) fn backpack_wheel(&mut self, pos: (f32, f32), screen: (u32, u32), lines: f32) -> bool {
        let over = self.backpack_open() && self.tree.hit_test(pos, screen).is_some_and(|h| self.tree.name(h).starts_with(BACKPACK));
        if over && lines != 0.0 {
            let last = self.inv.bag.len().saturating_sub(BACKPACK_SIZE);
            self.bag_scroll = if lines > 0.0 { self.bag_scroll.saturating_sub(CONTAINER_COLUMNS) } else { (self.bag_scroll + CONTAINER_COLUMNS).min(last) };
            self.refresh();
        }
        over
    }

    /// Show the scroll bar of every scroll frame that has more to see, with its knob where the frame is scrolled to.
    pub(super) fn update_scroll_bars(&mut self, screen: (u32, u32)) {
        for frame in SCROLL_FRAMES {
            let bar = format!("{frame}ScrollBar");
            let state = self.tree.scroll_state(frame, screen).filter(|_| self.tree.is_name_shown(frame));
            self.tree.show(&bar, state.is_some_and(|(_, max)| max > 0.0));
            if let (Some((at, max)), Some(track)) = (state.filter(|(_, max)| *max > 0.0), self.tree.rect_of(&bar, screen)) {
                // The knob is 16 units and travels the length of the bar less its own.
                let travel = (track.h - 16.0).max(0.0);
                self.tree.set_region_point(&format!("{bar}ThumbTexture"), Point::TopLeft, Some(&bar), Some(Point::TopLeft), 0.0, -travel * at / max);
            }
        }
    }

    /// Show the target (`TargetFrame_Update`): its name, level in the colour of how hard it is, health, and the colour of its
    /// name plate; nothing hides the frame. The hostile ones are red, the neutral ones yellow, the friendly ones green.
    pub fn set_target(&mut self, view: Option<crate::creature::TargetView>) {
        // Whoever is talked to shows his face in the portrait of the talk, trade and quest windows (the book otherwise).
        let face = view.as_ref().filter(|v| self.talking_to() == Some(v.id)).map(|_| self.target_portrait.1);
        for portrait in ["GossipFramePortrait", "MerchantFramePortrait", "QuestFramePortrait", "ClassTrainerFramePortrait"] {
            self.tree.set_region_texture(portrait, face);
        }
        let Some(v) = view else {
            if self.target.take().is_some() {
                self.tree.show("TargetFrame", false);
                self.events.push(Sfx::Named("INTERFACESOUND_LOSTTARGETUNIT"));
            }
            return;
        };
        if self.target != Some(v.id) {
            self.events.push(Sfx::Named(if v.hostile || v.neutral { "igCreatureAggroSelect" } else { "igCharacterNPCSelect" }));
        }
        self.target = Some(v.id);
        self.tree.show("TargetFrame", true);
        self.tree.set_text("TargetName", &v.name);
        let level = self.progress.level;
        let skull = levels::is_skull(level, v.level);
        self.tree.set_region_hidden("TargetHighLevelTexture", !skull);
        self.tree.set_region_hidden("TargetLevelText", skull);
        self.tree.set_text("TargetLevelText", &v.level.to_string());
        self.tree.set_text_color("TargetLevelText", if v.hostile || v.neutral { levels::difficulty_color(level, v.level) } else { GOLD });
        self.tree.set_region_hidden("TargetDeadText", !v.dead);
        self.tree.set_region_tint("TargetFrameNameBackground", if v.hostile { [1.0, 0.0, 0.0, 1.0] } else if v.neutral { [1.0, 1.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0, 1.0] });
        self.tree.set_bar("TargetFrameHealthBar", v.fraction, Some([0.0, 1.0, 0.0, 1.0]));
        self.set_target_auras(&v);
    }

    /// The auras of the target as `TargetDebuffButton_Update` lists them: the icons of its helpful ones (5) and harmful ones (16, their borders in the colour of their dispel kind),
    /// the harmful ones first under a hostile target and the helpful ones first under a friendly one, the first six harmful ones larger while they are fewer than six.
    fn set_target_auras(&mut self, v: &crate::creature::TargetView) {
        // `DebuffTypeColor` of BuffFrame.lua: none, magic, curse, disease, poison.
        const COLORS: [[f32; 4]; 5] = [[0.8, 0.0, 0.0, 1.0], [0.2, 0.6, 1.0, 1.0], [0.6, 0.0, 1.0, 1.0], [0.6, 0.4, 0.0, 1.0], [0.0, 0.6, 0.0, 1.0]];
        let icon = |name: &str| format!(r"Interface\Icons\{name}");
        for i in 1..=5 {
            let button = format!("TargetFrameBuff{i}");
            match v.buffs.get(i - 1) {
                Some(aura) => {
                    self.tree.show(&button, true);
                    self.tree.set_texture(&format!("{button}Icon"), Some(&icon(&aura.icon)));
                }
                None => self.tree.show(&button, false),
            }
        }
        for i in 1..=16 {
            let button = format!("TargetFrameDebuff{i}");
            match v.debuffs.get(i - 1) {
                Some(d) => {
                    self.tree.show(&button, true);
                    self.tree.set_texture(&format!("{button}Icon"), Some(&icon(&d.icon)));
                    self.tree.set_region_tint(&format!("{button}Border"), COLORS[(d.dispel as usize).min(4)]);
                    // The seconds it has left are written in the corner (where a stack's count would be).
                    let text = self.lasts_text(d.left).replace(' ', "");
                    self.tree.set_text(&format!("{button}Count"), &text);
                    self.tree.set_region_hidden(&format!("{button}Count"), false);
                }
                None => self.tree.show(&button, false),
            }
        }
        self.target_auras = (v.debuffs.clone(), v.buffs.clone());
        if v.hostile || v.neutral {
            self.tree.set_point("TargetFrameDebuff1", Point::TopLeft, Some("TargetFrame"), Some(Point::BottomLeft), 5.0, 32.0);
            self.tree.set_point("TargetFrameBuff1", Point::TopLeft, Some("TargetFrameDebuff7"), Some(Point::BottomLeft), 0.0, -2.0);
        } else {
            self.tree.set_point("TargetFrameBuff1", Point::TopLeft, Some("TargetFrame"), Some(Point::BottomLeft), 5.0, 32.0);
            self.tree.set_point("TargetFrameDebuff1", Point::TopLeft, Some("TargetFrameBuff1"), Some(Point::BottomLeft), 0.0, -2.0);
        }
        let (size, border) = if v.debuffs.len() >= 6 { (17.0, 19.0) } else { (21.0, 23.0) };
        for i in 1..=5 {
            self.tree.set_size(&format!("TargetFrameBuff{i}"), Some(size), Some(size));
        }
        for i in 1..=6 {
            self.tree.set_size(&format!("TargetFrameDebuff{i}"), Some(size), Some(size));
            self.tree.set_region_size(&format!("TargetFrameDebuff{i}Border"), Some(border), Some(border));
        }
    }

    /// Say which spells of creatures have hold of the character (every frame): their icons are shown with the buffs.
    pub fn set_afflictions(&mut self, list: &[crate::foes::Affliction]) {
        self.afflicted = list.iter().map(|a| (format!(r"Interface\Icons\{}", a.icon), a.left, a.spell)).collect();
    }

    /// The seconds left as a buff button writes them: `12s`, `5m`, `1h`.
    fn lasts_text(&self, left: f32) -> String {
        let lasts = left.ceil() as u32;
        match lasts {
            3600.. => self.string("HOUR_ONELETTER_ABBR").replace("%d", &(lasts / 3600).to_string()),
            60.. => self.string("MINUTE_ONELETTER_ABBR").replace("%d", &(lasts / 60).to_string()),
            _ => self.string("SECOND_ONELETTER_ABBR").replace("%d", &lasts.to_string()),
        }
    }

    /// Show the buffs as the icons of `BuffFrame`: the helpful ones in its first 16 buttons, the harmful ones (red borders, by dispel kind) in the last 8.
    pub(super) fn update_buffs(&mut self) {
        let aura = |icon: String, left: f32, spell: Option<u32>| PlayerAura { icon, left, spell };
        let mut helpful: Vec<PlayerAura> = self.consumed.buffs.iter().map(|b| {
            let item = crate::items::get(b.item);
            aura(item.icon.to_string(), b.left, item.spells.first().map(|s| s.spell))
        }).collect();
        helpful.extend(self.consumed.shown.iter().map(|(icon, left)| aura(format!(r"Interface\Icons\{icon}"), *left, None)));
        let mut harmful: Vec<PlayerAura> = self.afflicted.iter().map(|(icon, left, spell)| aura(icon.clone(), *left, Some(*spell))).collect();
        if self.consumed.bandaged > 0.0 {
            harmful.push(aura(format!(r"Interface\Icons\{}", content::classic_spell(crate::consume::RECENTLY_BANDAGED).map_or("INV_Misc_Bandage_08", |s| s.icon.as_str())), self.consumed.bandaged, Some(crate::consume::RECENTLY_BANDAGED)));
        }
        self.tree.show("BuffFrame", !helpful.is_empty() || !harmful.is_empty());
        let durations = self.show_durations();
        for i in 0..24 {
            let button = format!("BuffButton{i}");
            // `DebuffTypeColor` of BuffFrame.lua: none, magic, curse, disease, poison.
            const COLORS: [[f32; 4]; 5] = [[0.8, 0.0, 0.0, 1.0], [0.2, 0.6, 1.0, 1.0], [0.6, 0.0, 1.0, 1.0], [0.6, 0.4, 0.0, 1.0], [0.0, 0.6, 0.0, 1.0]];
            match if i < 16 { helpful.get(i) } else { harmful.get(i - 16) } {
                Some(PlayerAura { icon, left, spell }) => {
                    self.tree.show(&button, true);
                    self.tree.set_texture(&format!("{button}Icon"), Some(icon));
                    if i >= 16 {
                        let dispel = spell.and_then(content::classic_spell).map_or(0, |s| s.dispel as usize);
                        self.tree.set_region_tint(&format!("{button}Border"), COLORS[dispel.min(4)]);
                    }
                    self.tree.set_region_hidden(&format!("{button}Duration"), !durations);
                    let text = self.lasts_text(*left);
                    self.tree.set_text(&format!("{button}Duration"), &text);
                }
                None => {
                    self.tree.show(&button, false);
                    // The label is drawn on its own, so the one of the buff that ran out is hidden with it.
                    self.tree.set_region_hidden(&format!("{button}Duration"), true);
                }
            }
        }
        self.player_auras = (helpful, harmful);
    }

    pub fn target_shown(&self) -> bool {
        self.target.is_some()
    }

    /// Draw the target's head into the portrait of the target frame.
    pub fn render_target(&mut self, renderer: &mut Renderer, gpu: &Gpu, creatures: &crate::creature::Creatures, world: &World) {
        let Some(shot) = self.target.and_then(|t| creatures.portrait_shot(t)) else { return };
        renderer.render_offscreen(gpu, self.target_portrait.0, &shot.0, &shot.1, world.tiles());
    }

    /// Lay the open panels out for `screen` (what drawing does first), for clicks that come before the first draw.
    pub fn layout(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32)) {
        self.update_buffs();
        let mount = self.mount.clone();
        self.tree.measure(ui, gpu, &mount, screen);
    }

    /// Queue the names floating over creatures (`UnitName`: the game font, tinted by how the creature feels about you).
    pub fn draw_labels(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32), labels: &[crate::creature::Label]) {
        let mount = self.mount.clone();
        for l in labels {
            // The name stands one em above the subname, which stands on the point.
            let name_y = if l.sub.is_some() { l.y - l.line } else { l.y };
            self.tree.draw_text_world(ui, gpu, &mount, screen, "GameFontNormalLarge", &l.text, (l.x, name_y), l.line, l.color, false);
            if let Some(sub) = &l.sub {
                self.tree.draw_text_world(ui, gpu, &mount, screen, "GameFontNormalLarge", sub, (l.x, l.y), l.line, l.color, false);
            }
        }
    }

    /// Queue the nameplates (`CGNamePlateFrame`, `FUN_007cb250`): a 0.1 x 0.025 frame (of the screen diagonal, 128 x 32 px at 1024 x 768) with the `Nameplate-Border` over a health bar
    /// at its bottom left, the name standing on the middle of the frame, and at its right end the level in its difficulty colour or the skull.
    /// With a target the other plates are half transparent (`FUN_007cb950`).
    pub fn draw_plates(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32), plates: &[crate::creature::Plate], has_target: bool) {
        const FONT: &str = "GameFontNormalSmall";
        let mount = self.mount.clone();
        // The original's screen fractions (`FUN_0041ad10`: 0.8 of 1024 px at 4:3) are of the screen diagonal.
        let k = (screen.0 as f32).hypot(screen.1 as f32);
        let (w, h) = (0.1 * k, 0.025 * k);
        let player = self.progress.level;
        let (Some(border), Some(fill)) = (
            self.tree.texture(ui, gpu, &mount, r"Interface\Tooltips\Nameplate-Border"),
            self.tree.texture(ui, gpu, &mount, r"Interface\TargetingFrame\UI-TargetingFrame-BarFill"),
        ) else {
            return;
        };
        for p in plates {
            let a = if !has_target || p.targeted { 1.0 } else { 127.0 / 255.0 };
            // The frame stays whole on the screen.
            let (cx, cy) = (p.x.clamp(w / 2.0, screen.0 as f32 - w / 2.0), p.y.clamp(h / 2.0, screen.1 as f32 - h / 2.0));
            let (left, bottom) = (cx - w / 2.0, cy + h / 2.0);
            let color = [p.color[0], p.color[1], p.color[2], a];
            let (bar_w, bar_h) = (0.0804 * k, 0.007 * k);
            let bar = [left + 0.0031 * k, bottom - 0.003125 * k - bar_h, bar_w * p.fraction, bar_h];
            ui.quad(crate::ui::Quad { texture: fill, rect: bar, uv: [0.0, 0.0, p.fraction, 1.0], color, additive: false });
            ui.quad(crate::ui::Quad { texture: border, rect: [left, cy - h / 2.0, w, h], uv: crate::ui::FULL_UV, color: [1.0, 1.0, 1.0, a], additive: false });
            self.tree.draw_text_world(ui, gpu, &mount, screen, FONT, &p.name, (cx, cy), 0.01 * k, color, true);
            let (lx, ly) = (left + w - 0.0092 * k, bottom - 0.0071 * k);
            if levels::is_skull(player, p.level) {
                let side = 0.01 * k;
                self.tree.draw_icon_tinted(ui, gpu, &mount, r"Interface\TargetingFrame\UI-TargetingFrame-Skull", [lx - side / 2.0, ly - side / 2.0, side, side], [1.0, 1.0, 1.0, a]);
            } else {
                let [r, g, b, _] = levels::difficulty_color(player, p.level);
                let line = 0.00898 * k;
                self.tree.draw_text_world(ui, gpu, &mount, screen, FONT, &p.level.to_string(), (lx, ly + line / 2.0), line, [r, g, b, a], true);
            }
        }
    }

    /// Queue the open panels, the carried item and the tooltip for drawing.
    pub fn draw(&mut self, ui: &mut Ui, gpu: &Gpu, screen: (u32, u32)) {
        self.update_buffs();
        if !self.any_open() && !self.target_shown() && !self.cast_bar_visible() && !self.breath_shown && self.player_auras.0.is_empty() && self.player_auras.1.is_empty() {
            return;
        }
        let mount = self.mount.clone();
        self.tree.measure(ui, gpu, &mount, screen);
        if self.book_open() {
            book::place_runs(&mut self.tree);
        }
        self.update_scroll_bars(screen);
        if self.popup_open() {
            // The question is as tall as its text and one row of buttons (`StaticPopup_Resize`).
            self.tree.measure(ui, gpu, &mount, screen);
            let room = self.sleep_room();
            let height = 16.0 + self.tree.text_height("StaticPopup1Text") + room + 8.0 + 20.0 + 16.0;
            self.tree.set_size(POPUP, None, Some(height));
            self.tree.set_point("StaticPopup1Button1", Point::TopRight, Some("StaticPopup1Text"), Some(Point::Bottom), -6.0, -8.0 - room);
        }
        self.tree.resize_tab(ui, gpu, &mount, "CharacterFrameTab1", 0.0);
        self.tree.resize_tab(ui, gpu, &mount, "CharacterFrameTab4", 0.0);
        self.tree.resize_tab(ui, gpu, &mount, "MerchantFrameTab1", 0.0);
        for tab in 1..=3 {
            self.tree.resize_tab(ui, gpu, &mount, &format!("TalentFrameTab{tab}"), 0.0);
        }
        if self.menu_open() || self.options_open() {
            // The world dims behind the menu, as behind the map and the level-up panel.
            levelup::quad(ui, crate::ui::WHITE, [0.0, 0.0, screen.0 as f32, screen.1 as f32], [0.0, 0.0, 0.0, 0.38], false);
        }
        self.tree.draw(ui, gpu, &mount, screen);
                let s =screen.1 as f32 / ui_height();
        // The carried item hangs off the mouse.
        if let (Some(c), Some((x, y))) = (self.inv.cursor, self.cursor) {
            let size = 37.0 * s;
            self.tree.draw_icon(ui, gpu, &mount, crate::items::get(c.item).icon, [x - size / 2.0, y - size / 2.0, size, size]);
        } else if let (Some(lines), Some(hover)) = (self.tooltip(), self.tree.hover) {
            if let Some(owner) = self.tree.rect_of(&self.tree.name(hover).to_string(), screen) {
                self.tree.draw_tooltip(ui, gpu, &mount, screen, owner, &lines);
            }
        }
    }
}
