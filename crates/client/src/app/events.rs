//! The window's events while the game is played: keys, the mouse, redraws.

use super::*;
use winit::event::KeyEvent;

impl State {
    /// A mouse button on the open panels: true when they took it (the game must not act on it).
    pub(super) fn panel_mouse(&mut self, down: bool, left: bool) -> bool {
        let (Some(panels), Some(pos)) = (&mut self.panels, self.cursor) else { return false };
        let size = (self.config.width, self.config.height);
        if !panels.any_open() {
            // The hint over the bars is a button while the mouse is free.
            if down && panels.level_hint_at(pos, size) {
                panels.toggle_level();
                return true;
            }
            return false;
        }
        if down {
            panels.is_over(pos, size) && panels.mouse_down(pos, size)
        } else {
            panels.mouse_up(pos, size, left);
            true
        }
    }

    /// One event of the game's window.
    pub(super) fn event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resize(size.width, size.height),
            WindowEvent::KeyboardInput { event, .. } => self.keyboard(event_loop, event),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => self.left_press(),
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => self.left_release(),
            WindowEvent::CursorMoved { position, .. } => {
                if let (true, Some(old), Some(editor)) = (self.rotating, self.cursor, &mut self.editor) {
                    editor.turn_by(position.x as f32 - old.0);
                }
                self.cursor = (!self.grabbed).then_some((position.x as f32, position.y as f32));
                if let Some(editor) = &mut self.editor {
                    editor.mouse_moved((position.x as f32, position.y as f32));
                }
                if let Some(bar) = &mut self.actionbar {
                    bar.set_cursor(self.cursor);
                }
                let size = (self.config.width, self.config.height);
                if let Some(panels) = &mut self.panels {
                    panels.hover(self.cursor, size);
                }
            }
            // Middle button: holding it looks round with the editor's camera.
            WindowEvent::MouseInput { state: button_state, button: MouseButton::Middle, .. } => {
                let over = self.cursor.zip(self.editor.as_ref().filter(|e| e.open)).is_some_and(|(pos, e)| e.over(pos, self.config.height));
                self.editor_look(button_state == ElementState::Pressed && !over);
            }
            // Right button with the editor open: held, it turns the model that is put down (the pointer stays), else it looks round.
            WindowEvent::MouseInput { state: button_state, button: MouseButton::Right, .. } if self.editor.as_ref().is_some_and(|e| e.open) => {
                let down = button_state == ElementState::Pressed;
                let over = self.cursor.zip(self.editor.as_ref()).is_some_and(|(pos, e)| e.over(pos, self.config.height));
                if !down || !over {
                    if self.editor.as_ref().is_some_and(|e| e.placing()) {
                        self.rotating = down;
                    } else {
                        self.editor_look(down);
                    }
                }
            }
            WindowEvent::MouseInput { state: button_state, button: MouseButton::Right, .. } => {
                let down = button_state == ElementState::Pressed;
                if self.panel_mouse(down, false) {
                } else if !self.grabbed {
                    if down {
                        self.world_click(true);
                    }
                } else if let (Some(p), true) = (&mut self.player, self.grabbed) {
                    p.set_block(down);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                // The pointer left the window: nothing to draw.
                self.cursor = None;
                if let Some(bar) = &mut self.actionbar {
                    bar.set_cursor(None);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => self.wheel(delta),
            WindowEvent::Focused(false) => {
                self.input = Input::default();
                self.rotating = false;
                self.ctrl = false;
                if let (Some(bar), Some(p)) = (&mut self.actionbar, &mut self.player) {
                    bar.release_all(p);
                }
                if let Some(p) = &mut self.player {
                    p.set_block(false);
                }
                self.set_grab(false);
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    /// A key went down or up: the console, the editor's fields and the game's keys, in that order.
    fn keyboard(&mut self, event_loop: &ActiveEventLoop, event: KeyEvent) {
        if let PhysicalKey::Code(KeyCode::ControlLeft | KeyCode::ControlRight) = event.physical_key {
            self.ctrl = event.state == ElementState::Pressed;
        }
        // The console takes everything that is typed (F2 is heard below, to close it).
        if let Some(console) = self.console.as_mut().filter(|c| c.open) {
            use crate::console::Key;
            use winit::keyboard::{Key as Logical, NamedKey};
            if event.physical_key != PhysicalKey::Code(KeyCode::F2) {
                if event.state == ElementState::Pressed {
                    if let Some(text) = &event.text {
                        console.typed(text);
                    }
                    let line = match event.logical_key {
                        Logical::Named(NamedKey::Backspace) => console.key(Key::Backspace),
                        Logical::Named(NamedKey::Enter) => console.key(Key::Enter),
                        Logical::Named(NamedKey::Tab) => console.key(Key::Tab),
                        Logical::Named(NamedKey::ArrowUp) => console.key(Key::Up),
                        Logical::Named(NamedKey::ArrowDown) => console.key(Key::Down),
                        Logical::Named(NamedKey::Escape) => {
                            self.set_console(false);
                            None
                        }
                        _ => None,
                    };
                    if let Some(line) = line {
                        self.run_line(&line);
                    }
                }
                return;
            }
        }
        // What is typed goes to the field of the editor that is being edited, and nothing else hears it.
        if let Some(editor) = self.editor.as_mut().filter(|e| e.open) {
            if event.state == ElementState::Pressed {
                if let Some(text) = &event.text {
                    editor.typed(text);
                }
                match event.logical_key {
                    winit::keyboard::Key::Named(winit::keyboard::NamedKey::Backspace) => editor.key(crate::editor::Key::Backspace),
                    winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => editor.key(crate::editor::Key::Enter),
                    winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => editor.key(crate::editor::Key::Escape),
                    _ => {}
                }
            }
            // (F1 is not typed: it always gets to close the editor.)
            // Ctrl + S saves even in a field: what is typed is kept first.
            let save_chord = self.ctrl && event.state == ElementState::Pressed && event.physical_key == PhysicalKey::Code(KeyCode::KeyS);
            if save_chord && editor.typing() {
                editor.key(crate::editor::Key::Enter);
            }
            if editor.typing() && event.physical_key != PhysicalKey::Code(KeyCode::F1) && !save_chord {
                return;
            }
        }
        if let PhysicalKey::Code(code) = event.physical_key {
            if !event.repeat {
                self.key(code, event.state == ElementState::Pressed, event_loop);
            }
        }
    }

    /// The left button went down: the editor, the lock, the windows, the action bar, the world, or the sword.
    fn left_press(&mut self) {
        if let (Some(editor), Some(pos)) = (self.editor.as_mut().filter(|e| e.open), self.cursor) {
            if editor.over(pos, self.config.height) {
                editor.click();
                return;
            }
            // A click in the world is a click away from the field being typed in: it keeps what is typed and lets the keys go.
            editor.click();
            // A click on the gizmo takes hold of it; a click on a prop selects it.
            let world = &self.world;
            if editor.world_press(&|x, z| world.height_at(x, z)) {
                return;
            }
        }
        // Holding the mouse to look round, a click does not swing the sword.
        if self.looking {
            return;
        }
        // Putting props down: the click stands the chosen model where the pointer touches the world.
        if let Some(pos) = self.cursor.filter(|_| self.editor.as_ref().is_some_and(|e| e.placing())) {
            if let Some(hit) = self.cursor_hit(pos) {
                if let Some(editor) = &mut self.editor {
                    editor.place(hit);
                }
            }
            return;
        }
        if self.lock.is_some() {
            self.lock_turn = true;
        } else if self.panel_mouse(true, true) {
        } else if !self.grabbed {
            // A click on an action button uses it; anywhere else it hands the mouse back to the game.
            let size = (self.config.width, self.config.height);
            let slot = self.actionbar.as_ref().and(self.cursor).and_then(|pos| ActionBar::slot_at(pos, size));
            // An item picked up with a click is put on the button clicked.
            if let (Some(slot), Some(bar), Some(panels)) = (slot, &mut self.actionbar, &mut self.panels) {
                if let Some(action) = panels.take_carried_action() {
                    bar.place(slot, action);
                    return;
                }
            }
            match (slot, &mut self.actionbar, &mut self.player) {
                // Shift lifts the action off its button to move or drop it instead of using it.
                (Some(slot), Some(bar), Some(_)) if self.input.fast => bar.pick_up(slot),
                (Some(slot), Some(bar), Some(player)) => {
                    self.cast_request = bar.press(slot, player).or(self.cast_request);
                    self.mouse_slot = Some(slot);
                }
                _ if self.panels.as_ref().is_some_and(|p| p.any_open()) => {}
                // A click on a creature selects it; a click on nothing hands the mouse back to the game.
                _ => {
                    if !self.world_click(false) {
                        self.set_grab(true);
                    }
                }
            }
        } else if let Some(p) = &mut self.player {
            p.attack_press();
        }
    }

    /// The left button went up.
    fn left_release(&mut self) {
        self.lock_turn = false;
        if let Some(editor) = &mut self.editor {
            editor.release();
        }
        if let Some(p) = &mut self.player {
            p.attack_release();
        }
        // An action dragged from the spellbook or off the bar lands on the button under the pointer; dropped anywhere
        // else it is gone from the bar.
        if let Some(bar) = &mut self.actionbar {
            let from_book = self.panels.as_mut().and_then(|p| p.take_drag());
            if let Some(action) = bar.take_drag().or(from_book) {
                let size = (self.config.width, self.config.height);
                if let Some(slot) = self.cursor.and_then(|pos| ActionBar::slot_at(pos, size)) {
                    bar.place(slot, action);
                }
            }
        }
        self.panel_mouse(false, true);
        if let (Some(slot), Some(bar), Some(player)) = (self.mouse_slot.take(), &mut self.actionbar, &mut self.player) {
            bar.release(slot, player);
        }
    }

    /// The wheel: scrolls the window under the pointer, sizes the editor's model, or sets the speed of flying.
    fn wheel(&mut self, delta: MouseScrollDelta) {
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => y,
            MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
        };
        // Over an open window the wheel scrolls it; otherwise it sets the speed of flying.
        let size = (self.config.width, self.config.height);
        let shift = self.input.fast;
        if let (Some(editor), Some(pos)) = (self.editor.as_mut().filter(|e| e.open), self.cursor) {
            if editor.over(pos, size.1) {
                editor.wheel(lines);
                return;
            }
            // In the world it sizes (and with Shift, raises) the model that is put down.
            if editor.scroll_world(lines, shift) {
                return;
            }
        }
        let scrolled = match (&mut self.panels, self.cursor) {
            (Some(panels), Some(pos)) => panels.scroll(pos, size, lines),
            _ => false,
        };
        if !scrolled {
            let controller = self.free.as_mut().map_or(&mut self.controller, |f| &mut f.controller);
            controller.fly_speed = (controller.fly_speed * 1.2f32.powf(lines)).clamp(2.0, 2000.0);
        }
    }

    /// A frame, and what the frame asked for: the editor's content taken into use, the end of the game, a new start.
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        self.frame();
        if self.apply_after.is_some_and(|t| self.start.elapsed().as_secs_f32() > t) {
            self.apply_after = None;
            self.apply = true;
        }
        if std::mem::take(&mut self.apply) {
            self.apply_content();
        }
        if self.quit {
            return event_loop.exit();
        }
        // The editor saved what it changed and asked for the game to begin again with it, standing where it stood.
        if self.restart {
            self.save_game();
            // The same character goes on (not the list again).
            let mut args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--new").collect();
            if self.logout {
                // Back to the list: the character is not named, nor is its look.
                let at = args.iter().position(|a| a == "--character");
                if let Some(at) = at {
                    args.drain(at..(at + 2).min(args.len()));
                }
            } else if !args.iter().any(|a| a == "--character" || a == "--save" || a == "--no-save") {
                args.extend(["--character".to_string(), self.name.clone()]);
            }
            match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).args(args).spawn()) {
                Ok(_) => event_loop.exit(),
                Err(e) => {
                    log::warn!("cannot start the game again: {e}");
                    self.restart = false;
                }
            }
        }
    }
}
