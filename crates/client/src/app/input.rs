//! Keys pressed in the game.

use super::*;

impl State {
    pub(super) fn key(&mut self, code: KeyCode, pressed: bool, event_loop: &ActiveEventLoop) {
        log::debug!("key {code:?} pressed={pressed}");
        // F1 opens and closes the editor.
        if let (KeyCode::F1, true, Some(editor)) = (code, pressed, &mut self.editor) {
            editor.toggle();
            let open = editor.open;
            self.looking = false;
            self.rotating = false;
            if open {
                self.set_grab(false);
            } else if !self.panels.as_ref().is_some_and(|p| p.any_open()) {
                self.set_grab(true);
            }
            return;
        }
        // F2 opens and closes the console; the mouse is free while it is open and the keys held down are let go.
        if let (KeyCode::F2, true, Some(console)) = (code, pressed, &self.console) {
            self.set_console(!console.open);
            return;
        }
        // F5 quicksaves into a second file beside the character's; F9 puts it back as the save and starts the game again from it.
        if pressed && matches!(code, KeyCode::F5 | KeyCode::F9) && self.free.is_none() {
            if let Some(path) = self.save_path.clone() {
                let quick = path.with_extension("quick");
                let note = if code == KeyCode::F5 {
                    match self.capture().map(|saved| save::write(&quick, &saved)) {
                        Some(Ok(())) => "Quicksaved",
                        _ => "Cannot quicksave",
                    }
                } else {
                    match save::read(&quick).and_then(|q| q.map(|q| save::write(&path, &q)).transpose()) {
                        // Nothing may save over the loaded game while the game starts again.
                        Ok(Some(())) => {
                            (self.save_path, self.restart) = (None, true);
                            "Quickloading"
                        }
                        Ok(None) => "No quicksave",
                        Err(e) => {
                            log::warn!("cannot quickload: {e:#}");
                            "Cannot quickload"
                        }
                    }
                };
                if let Some(panels) = &mut self.panels {
                    panels.notice(note);
                }
            }
            return;
        }
        // With the editor open: G drops what is selected (or put down) to the ground, Ctrl + S saves and applies.
        if let (true, true, Some(editor)) = (pressed, self.free.is_some(), &mut self.editor) {
            match code {
                KeyCode::KeyG => {
                    let world = &self.world;
                    editor.snap_to_ground(|x, z| world.height_at(x, z));
                    return;
                }
                KeyCode::KeyS if self.ctrl => {
                    editor.request_save();
                    return;
                }
                _ => {}
            }
        }
        // With the editor's camera out the keys fly it; the game's own keys are not heard.
        if self.free.is_some() && !matches!(code, KeyCode::KeyW | KeyCode::KeyA | KeyCode::KeyS | KeyCode::KeyD | KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::Space | KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::ShiftLeft | KeyCode::ShiftRight) {
            return;
        }
        // While a lock is picked the keys turn the lock, and Esc leaves; nothing else is heard.
        if self.lock.is_some() {
            match code {
                KeyCode::Escape if pressed => self.end_lock(),
                KeyCode::KeyA | KeyCode::KeyD | KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::Space => self.lock_turn = pressed,
                _ => {}
            }
            return;
        }
        if let (Some(slot), Some(bar), Some(player)) = (slot_of_key(code), &mut self.actionbar, &mut self.player) {
            if pressed {
                self.cast_request = bar.press(slot, player).or(self.cast_request);
            } else {
                bar.release(slot, player);
            }
            return;
        }
        match code {
            KeyCode::KeyW | KeyCode::ArrowUp => self.input.forward = pressed,
            KeyCode::KeyS | KeyCode::ArrowDown => self.input.back = pressed,
            KeyCode::KeyA | KeyCode::ArrowLeft => self.input.left = pressed,
            KeyCode::KeyD | KeyCode::ArrowRight => self.input.right = pressed,
            KeyCode::Space => {
                self.input.up = pressed;
                self.input.jump_pressed |= pressed;
            }
            KeyCode::ControlLeft | KeyCode::ControlRight => {
                // Flying down, or on foot: sneaking on and off.
                self.input.down = pressed;
                if let (true, Mode::Walk, None, Some(p)) = (pressed, self.controller.mode, &self.free, &mut self.player) {
                    p.toggle_sneak();
                    if let (true, Some(entry), Some(panels)) = (p.is_sneaking(), self.stealth_sound, &mut self.panels) {
                        panels.emit(crate::audio::Sfx::Entry(entry));
                    }
                }
            }
            KeyCode::ShiftLeft | KeyCode::ShiftRight => self.input.fast = pressed,
            KeyCode::KeyE if pressed => self.interact(),
            KeyCode::KeyE => {
                // Let go of what is carried or dragged; a corpse the key was only tapped on is looted.
                if let Some((corpse, since)) = self.pending_loot.take() {
                    if since.elapsed() < super::interact::DRAG_HOLD {
                        self.loot_corpse(corpse);
                    }
                }
                self.bodies.iter_mut().for_each(|b| b.release());
            }
            KeyCode::Tab if pressed => {
                // The nearest enemy ahead; again, the next.
                let feet = self.camera.pos - Vec3::Y * EYE_HEIGHT;
                if let Some(wolves) = &mut self.wolves {
                    wolves.cycle_target(feet, self.camera.yaw);
                }
            }
            KeyCode::KeyC if pressed => self.toggle_panel(true),
            KeyCode::KeyB if pressed => self.toggle_panel(false),
            KeyCode::KeyK if pressed => {
                if let Some(panels) = &mut self.panels {
                    panels.toggle_character_tab(true);
                    let open = panels.any_open();
                    self.set_grab(!open);
                }
            }
            KeyCode::KeyP if pressed => {
                if let Some(panels) = &mut self.panels {
                    panels.toggle_spellbook();
                    let open = panels.any_open();
                    self.set_grab(!open);
                }
            }
            KeyCode::KeyN if pressed => {
                if let Some(panels) = &mut self.panels {
                    panels.toggle_talents();
                    let open = panels.any_open();
                    self.set_grab(!open);
                }
            }
            KeyCode::KeyU if pressed => {
                if let Some(panels) = &mut self.panels {
                    panels.toggle_level();
                    let open = panels.any_open();
                    self.set_grab(!open);
                }
            }
            KeyCode::KeyM if pressed => {
                if let Some(panels) = &mut self.panels {
                    panels.toggle_map();
                    let open = panels.any_open();
                    self.set_grab(!open);
                }
            }
            KeyCode::KeyL if pressed => {
                if let Some(panels) = &mut self.panels {
                    panels.toggle_quest_log();
                    let open = panels.any_open();
                    self.set_grab(!open);
                }
            }
            KeyCode::KeyR if pressed => {
                if let Some(p) = &mut self.player {
                    p.toggle_sheath();
                }
            }
            KeyCode::KeyV if pressed => {
                if let Some(p) = &mut self.player {
                    p.toggle_view();
                }
            }
            // Stand-in for an enemy: a 10 damage hit, blocked (at a stamina cost) if the shield is up.
            KeyCode::KeyH if pressed => {
                if let Some(p) = &mut self.player {
                    match p.take_hit(10.0) {
                        None => log::info!("test hit: blocked"),
                        Some(n) => log::info!("test hit: took {n} damage"),
                    }
                }
            }
            KeyCode::KeyF if pressed => self.toggle_fly(),
            KeyCode::Escape if pressed => {
                if self.panels.as_ref().is_some_and(|p| p.any_open()) {
                    let panels = self.panels.as_mut().unwrap();
                    panels.escape();
                    // The mouse goes back to the game once nothing is open.
                    if !panels.any_open() {
                        self.set_grab(true);
                    }
                } else if self.wolves.as_ref().is_some_and(|w| w.target().is_some()) {
                    // The first Esc lets go of the target.
                    self.wolves.as_mut().unwrap().clear_target();
                } else if let Some(panels) = &mut self.panels {
                    // Nothing else to close: the game menu.
                    panels.open_menu();
                    self.set_grab(false);
                } else if self.grabbed {
                    self.set_grab(false);
                } else {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
}
