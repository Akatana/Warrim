//! The video options (`OptionsFrame`), kept between runs in `video.ron`: the window (borderless full screen or not), vertical sync and how far the
//! terrain is seen.

use std::sync::Arc;
use winit::dpi::PhysicalSize;
use winit::window::{Fullscreen, Window};

/// `OPTIONS_FARCLIP_MIN` and `_MAX` of the original: the yards the slider runs over.
const FAR_MIN: f32 = 177.0;
const FAR_MAX: f32 = 777.0;

/// How the game takes the screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum DisplayMode {
    /// A window of the chosen size.
    #[default]
    Windowed,
    /// The monitor is set to the chosen video mode.
    Fullscreen,
    /// A window without borders over the whole screen, which leaves the monitor's mode alone.
    Borderless,
}

fn yes() -> bool {
    true
}

impl DisplayMode {
    pub const ALL: [(DisplayMode, &'static str); 3] = [(DisplayMode::Windowed, "Windowed"), (DisplayMode::Fullscreen, "Fullscreen"), (DisplayMode::Borderless, "Windowed Fullscreen")];
}

#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct VideoSettings {
    pub mode: DisplayMode,
    /// The switch the mode replaced, read from files of before (`false` was the borderless full screen); never written.
    #[serde(skip_serializing, default = "yes")]
    windowed: bool,
    pub vsync: bool,
    /// How far the terrain is seen, from 0 (`FAR_MIN`) to 1 (`FAR_MAX`).
    pub terrain: f32,
    /// `weatherDensity` 0 to 3: how much rain, snow and sand there is.
    pub weather: f32,
    /// The anisotropic filtering level 1 to 5 (2, 4, 8, 16 times; the first is none). It takes effect when the game starts.
    pub anisotropy: f32,
    /// The interface is drawn at `ui_scale` (0.64 to 1) of its size when `use_ui_scale` is on.
    pub use_ui_scale: bool,
    pub ui_scale: f32,
    /// `gamma` -0.5 to 0.5: darker or brighter.
    pub gamma: f32,
    /// `WorldDetail` 0 to 2: how far the props (trees, fences, rocks) are drawn; and `spellEffectLevel` 0 to 2: how many particles effects make.
    pub env_detail: f32,
    pub spell_detail: f32,
    /// The size of the window (or the video mode of the full screen) and the refresh rate (Hz) of the full screen; 0 is whatever the monitor has.
    pub resolution: (u32, u32),
    /// The sun's shadows (the one thing here that is drawn by a pixel shader the options can switch off).
    pub shadows: bool,
    pub refresh: u32,
}

impl Default for VideoSettings {
    fn default() -> Self {
        VideoSettings { mode: DisplayMode::Windowed, windowed: true, vsync: true, terrain: 1.0, weather: 3.0, anisotropy: 4.0, use_ui_scale: false, ui_scale: 1.0, gamma: 0.0, env_detail: 2.0, spell_detail: 2.0, resolution: (0, 0), shadows: true, refresh: 0 }
    }
}

impl VideoSettings {
    pub fn load() -> VideoSettings {
        let mut v: VideoSettings = crate::save::load_settings("video.ron");
        if !v.windowed {
            (v.mode, v.windowed) = (DisplayMode::Borderless, true);
        }
        v
    }

    pub fn save(&self) {
        crate::save::write_settings("video.ron", self);
    }

    /// The fog's furthest distance (yards).
    pub fn distance(&self) -> f32 {
        FAR_MIN + self.terrain.clamp(0.0, 1.0) * (FAR_MAX - FAR_MIN)
    }

    /// The share of the full weather that falls.
    pub fn weather_scale(&self) -> f32 {
        self.weather.clamp(0.0, 3.0) / 3.0
    }

    /// The sampler's anisotropy clamp: 1, 2, 4, 8 or 16.
    pub fn anisotropy_clamp(&self) -> u16 {
        1 << (self.anisotropy.round().clamp(1.0, 5.0) as u32 - 1)
    }

    pub fn present_mode(&self) -> wgpu::PresentMode {
        if self.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync }
    }

    /// How big the interface is drawn (1 is the original's 768 units to the height of the screen).
    pub fn interface_scale(&self) -> f32 {
        if self.use_ui_scale { self.ui_scale.clamp(0.64, 1.0) } else { 1.0 }
    }

    /// How far props are drawn (yards).
    pub fn prop_reach(&self) -> f32 {
        [100.0, 250.0, f32::INFINITY][self.env_detail.round().clamp(0.0, 2.0) as usize]
    }

    /// The share of the particles of the file that effects make.
    pub fn particle_scale(&self) -> f32 {
        [0.3, 0.65, 1.0][self.spell_detail.round().clamp(0.0, 2.0) as usize]
    }

    /// Put the window at its size, or take the screen: in the video mode that is asked for (the best the monitor has when none is), or, when
    /// there is no such mode, as a window without borders. (The surface follows with the resize this causes.)
    pub fn apply_window(&self, window: &Arc<Window>) {
        let wanted = self.resolution != (0, 0);
        match self.mode {
            DisplayMode::Windowed => {
                window.set_fullscreen(None);
                if wanted {
                    let _ = window.request_inner_size(PhysicalSize::new(self.resolution.0, self.resolution.1));
                }
            }
            DisplayMode::Borderless => window.set_fullscreen(Some(Fullscreen::Borderless(None))),
            DisplayMode::Fullscreen => {
                let mode = window.current_monitor().and_then(|m| {
                    m.video_modes()
                        .filter(|v| !wanted || (v.size().width, v.size().height) == self.resolution)
                        .filter(|v| self.refresh == 0 || (v.refresh_rate_millihertz() + 500) / 1000 == self.refresh)
                        .max_by_key(|v| (v.size().width * v.size().height, v.refresh_rate_millihertz()))
                });
                window.set_fullscreen(Some(mode.map_or(Fullscreen::Borderless(None), Fullscreen::Exclusive)));
            }
        }
    }

    /// The video modes of the window's monitor: width, height and refresh rate (Hz), the biggest first.
    pub fn modes(window: &Arc<Window>) -> Vec<(u32, u32, u32)> {
        let mut modes: Vec<_> = window.current_monitor().into_iter().flat_map(|m| m.video_modes()).map(|v| (v.size().width, v.size().height, (v.refresh_rate_millihertz() + 500) / 1000)).collect();
        modes.sort_by(|a, b| (b.0 * b.1, b.2).cmp(&(a.0 * a.1, a.2)));
        modes.dedup();
        modes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_distance_runs_over_the_originals_range_and_options_fill_in_defaults() {
        assert_eq!((VideoSettings { terrain: 0.0, ..Default::default() }.distance(), VideoSettings::default().distance()), (177.0, 777.0));
        let low = VideoSettings { env_detail: 0.0, spell_detail: 0.0, use_ui_scale: true, ui_scale: 0.7, ..Default::default() };
        assert_eq!((low.prop_reach(), low.particle_scale(), low.interface_scale(), VideoSettings::default().interface_scale()), (100.0, 0.3, 0.7, 1.0));
        assert_eq!((VideoSettings::default().anisotropy_clamp(), VideoSettings { anisotropy: 1.0, ..Default::default() }.anisotropy_clamp()), (8, 1));
        // A file of before the display mode: the switch that is read, and not written again.
        let old: VideoSettings = ron::from_str("(windowed: false)").unwrap();
        assert!(!old.windowed);
        let o: VideoSettings = ron::from_str("(vsync: false)").unwrap();
        assert_eq!(o, VideoSettings { vsync: false, ..Default::default() });
    }
}
