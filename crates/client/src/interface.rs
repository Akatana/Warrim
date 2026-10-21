//! The interface options (`UIOptionsFrame`), kept between runs in `interface.ron`: how the mouse looks round, how far the camera stands from the
//! character in third person, and what the screen shows.

#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct InterfaceSettings {
    pub invert_mouse: bool,
    pub npc_names: bool,
    pub buff_durations: bool,
    /// Whether dying creatures fall as ragdolls (off: they keep their death animation).
    pub ragdolls: bool,
    /// `cameraYawMoveSpeed`: 90 to 270 degrees a second of the original; the game's own look speed is that of 180.
    pub look_speed: f32,
    /// `cameraDistanceMaxFactor`: 1 to 2 times the usual distance behind the character.
    pub follow: f32,
}

impl Default for InterfaceSettings {
    fn default() -> Self {
        InterfaceSettings { invert_mouse: false, npc_names: true, buff_durations: false, ragdolls: true, look_speed: 180.0, follow: 1.0 }
    }
}

impl InterfaceSettings {
    pub fn load() -> InterfaceSettings {
        crate::save::load_settings("interface.ron")
    }

    pub fn save(&self) {
        crate::save::write_settings("interface.ron", self);
    }

    /// How much faster than usual the mouse turns the view.
    pub fn look_scale(&self) -> f32 {
        self.look_speed / 180.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usual_look_speed_is_one_and_missing_options_take_their_defaults() {
        assert_eq!(InterfaceSettings::default().look_scale(), 1.0);
        let o: InterfaceSettings = ron::from_str("(invert_mouse: true)").unwrap();
        assert_eq!(o, InterfaceSettings { invert_mouse: true, ..Default::default() });
    }
}
