//! Sleeping in a bed: the question of the original's `StaticPopup1` asks for the hour to wake up at, which a slider under its text sets (made
//! from `OptionsSliderTemplate`, the template of the options' sliders); "Sleep" hands the hour to the game (`take_sleep`), which sets the
//! clock and fills health, mana and stamina.

use super::*;
use menu::SLIDER_WIDTH;

pub(super) const SLIDER: &str = "StaticPopup1Slider";
/// The room the slider and its labels take under the text of the question.
const ROOM: f32 = 42.0;
/// The last hour that can be chosen (the slider goes in steps of half an hour).
const LAST: f32 = 23.5;

pub(super) fn setup(tree: &mut Tree) -> Result<()> {
    tree.instantiate_template("OptionsSliderTemplate", SLIDER, POPUP)?;
    tree.set_point(SLIDER, Point::Top, Some("StaticPopup1Text"), Some(Point::Bottom), 0.0, -12.0);
    tree.set_text(&format!("{SLIDER}Text"), "");
    tree.set_text(&format!("{SLIDER}Low"), "00:00");
    tree.set_text(&format!("{SLIDER}High"), "23:30");
    tree.show(SLIDER, false);
    Ok(())
}

impl Panels {
    /// Ask until which hour the character sleeps (it begins at six in the morning).
    pub fn ask_sleep(&mut self) {
        self.close_all();
        self.sleep = Some(6.0);
        self.tree.set_text("StaticPopup1Button1Text", "Sleep");
        self.tree.set_text("StaticPopup1Button2Text", &self.string("CANCEL"));
        self.sleep_changed();
        self.tree.show(SLIDER, true);
        self.tree.show(POPUP, true);
    }

    /// The text and the thumb show the hour.
    fn sleep_changed(&mut self) {
        let hour = self.sleep.unwrap_or(0.0);
        self.tree.set_text("StaticPopup1Text", &format!("Sleep until {:02}:{:02}", hour as u32, (hour.fract() * 60.0) as u32));
        self.tree.set_region_point(&format!("{SLIDER}Thumb"), Point::Center, Some(SLIDER), Some(Point::Left), hour / LAST * SLIDER_WIDTH, 0.0);
    }

    /// The room the popup makes under its text for the slider (none for the question of the quest log).
    pub(super) fn sleep_room(&self) -> f32 {
        if self.sleep.is_some() { ROOM } else { 0.0 }
    }

    /// The wake-up hour goes `by` hours on (not round the clock). False when nobody is asked.
    pub(super) fn sleep_step(&mut self, by: f32) -> bool {
        let Some(hour) = &mut self.sleep else { return false };
        *hour = (*hour + by).clamp(0.0, LAST);
        self.sleep_changed();
        true
    }

    /// The answer "Sleep".
    pub(super) fn sleep_chosen(&mut self) {
        self.slept = self.sleep.take();
        self.tree.show(POPUP, false);
        self.after_close();
    }

    /// The hour the character went to sleep until, once.
    pub fn take_sleep(&mut self) -> Option<f32> {
        self.slept.take()
    }

    /// While the button is held on the slider, the thumb follows the pointer (pixels).
    pub(super) fn sleep_drag(&mut self, pos: (f32, f32), screen: (u32, u32)) {
        if !self.sleep_dragging || self.sleep.is_none() {
            return;
        }
        let Some(r) = self.tree.rect_of(SLIDER, screen) else { return };
        let at = ((pos.0 / (screen.1 as f32 / ui_height()) - r.x) / r.w).clamp(0.0, 1.0);
        let hour = (at * LAST * 2.0).round() / 2.0;
        if self.sleep != Some(hour) {
            self.sleep = Some(hour);
            self.sleep_changed();
        }
    }
}
