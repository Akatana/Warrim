//! The developer console's commands, and the other ways the player is moved about.

use super::*;

impl State {
    /// Open or close the console; the mouse is free while it is open and the keys held down are let go.
    pub(super) fn set_console(&mut self, open: bool) {
        let Some(console) = &mut self.console else { return };
        console.open = open;
        if open {
            self.input = Input::default();
            self.set_grab(false);
        } else if !self.panels.as_ref().is_some_and(|p| p.any_open()) {
            self.set_grab(true);
        }
    }

    /// Fly (F, `tcl`): nothing stops the character, and back on foot it lands on the terrain.
    pub(super) fn toggle_fly(&mut self) {
        self.controller.toggle_mode();
        if let Some(p) = &mut self.player {
            p.set_sneaking(false);
        }
        if self.controller.mode == Mode::Walk {
            if let Some(g) = self.world.height_at(self.camera.pos.x, self.camera.pos.z) {
                self.camera.pos.y = self.camera.pos.y.max(g + EYE_HEIGHT);
            }
        }
    }

    /// Run a line of the console and print the answer.
    pub(super) fn run_line(&mut self, line: &str) {
        if line.trim().eq_ignore_ascii_case("clear") {
            return self.console.iter_mut().for_each(|c| c.clear());
        }
        let result = crate::console::parse(line).and_then(|command| self.run_command(command));
        if let Some(console) = &mut self.console {
            console.reply(result);
        }
    }

    pub(super) fn run_command(&mut self, command: Command) -> Result<String, String> {
        let (Some(panels), Some(player)) = (&mut self.panels, &mut self.player) else { return Err("the game is not ready".into()) };
        let content = content::get();
        Ok(match command {
            Command::Help => crate::console::Console::help(),
            Command::Logout => {
                (self.logout, self.restart) = (true, true);
                "logging out".into()
            }
            Command::GodMode => {
                self.god = !self.god;
                format!("god mode {}", if self.god { "on" } else { "off" })
            }
            Command::Collision => {
                self.toggle_fly();
                format!("collision {}", if self.controller.mode == Mode::Walk { "on" } else { "off (flying)" })
            }
            Command::FreeCamera => {
                self.detached = !self.detached;
                format!("free camera {}", if self.detached { "on" } else { "off" })
            }
            Command::Level(n) => {
                panels.set_level(n);
                format!("level {n}")
            }
            Command::Xp(n) => {
                panels.gain_xp(n);
                format!("gained {n} experience")
            }
            Command::Gold(n) => {
                panels.inv.money = (panels.inv.money as i64 + n * 10_000).clamp(0, u32::MAX as i64) as u32;
                panels.refresh();
                format!("{} gold, {} silver, {} copper", panels.inv.money / 10_000, panels.inv.money / 100 % 100, panels.inv.money % 100)
            }
            Command::Skill(skill, rank) => {
                let all = skills::ALL;
                let chosen = skill.as_slice().iter().chain(skill.is_none().then_some(&all[..]).into_iter().flatten());
                chosen.for_each(|&s| panels.skills.set(s, rank));
                panels.refresh();
                format!("{} set to {rank}", skill.map_or("every skill", |s| s.name()))
            }
            Command::Item(item, count) => {
                let def = crate::items::get(item);
                if !panels.inv.add(crate::items::Stack { item, count }) {
                    return Err("no room in the backpack".into());
                }
                panels.refresh();
                format!("{count} x {}", def.name)
            }
            Command::Spell(spell) => {
                let all: Vec<usize> = (0..content.spells.len()).collect();
                let learned = spell.as_slice().iter().chain(spell.is_none().then_some(&all[..]).into_iter().flatten()).filter(|&&s| panels.learn(s)).count();
                format!("learned {learned} spells")
            }
            Command::Teleport(north, west) => {
                self.teleport(north, west, None);
                format!("teleported to {north:.1}, {west:.1}")
            }
            Command::Heal => {
                for r in [&mut player.vitals.health, &mut player.vitals.mana, &mut player.vitals.stamina] {
                    r.current = r.max;
                }
                "healed".into()
            }
            Command::Time(hour) => {
                self.fixed_time = hour.is_some();
                self.hour = hour.unwrap_or(self.hour);
                format!("it is {:02}:{:02}{}", self.hour as u32, (self.hour.fract() * 60.0) as u32, if self.fixed_time { "" } else { ", and time runs" })
            }
            Command::Pos => {
                let [north, west, _] = render_to_wow(self.camera.pos);
                format!("north {north:.1}, west {west:.1}, height {:.1}", self.camera.pos.y)
            }
        })
    }

    /// Put the player at a place (north, west) and on the ground there once the terrain has streamed in; with a feet `height`
    /// on the floor at that height (a building, a bridge), as a loaded save does.
    pub(super) fn teleport(&mut self, north: f32, west: f32, height: Option<f32>) {
        (self.camera.pos.x, self.camera.pos.z) = (-west, -north);
        self.snap_to_ground = Some(0.0);
        self.saved_feet = height;
        self.controller.reset();
        // The loading screen is up until the terrain there has streamed in (at least a moment, for a place close by).
        if let (None, Some(mut loading)) = (&self.loading, self.spare_loading.take()) {
            loading.restart(1.0);
            self.loading = Some(loading);
        }
    }
}
