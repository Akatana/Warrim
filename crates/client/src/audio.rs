//! Music, ambience and sound effects of the original client, played from its own data: zone music with
//! day / night sets, silences between tracks and zone intros (`AreaTable` -> `ZoneMusic` ->
//! `SoundEntries`), looping zone ambience (`SoundAmbience`), and the player's footsteps on the surface
//! under the feet (`FootstepTerrainLookup`), sword swing and attack grunt.
//!
//! The original played everything through FMOD; here `rodio` decodes the WAV and MP3 files. Volumes are
//! the client's default cvars (music 0.4, ambience 0.6, sound effects 1.0) times each sound entry's own.

use anyhow::{Context, Result};
use glam::Vec3;
use mpq::Mount;
use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::mpsc::{channel, Receiver};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use wowfmt::dbc::Dbc;

const MUSIC_VOLUME: f32 = 0.4;
const AMBIENCE_VOLUME: f32 = 0.6;
const WEATHER_VOLUME: f32 = 0.7;
const SFX_VOLUME: f32 = 1.0;
/// Seconds music and ambience take to fade in or out when the zone changes.
const FADE_SECONDS: f32 = 2.0;
/// The player's model: its sound data gives the footstep set and the attack grunt.
const PLAYER_MODEL: &str = r"character\human\male\humanmale.mdx";
/// `WeaponSwingSounds2`: swing weight 1 (medium), not a critical hit.
const SWING_MEDIUM: (u32, u32) = (1, 0);
/// The cries of a creature; its model's sound data (`CreatureSoundData.dbc`) gives them: it notices the player, snarls
/// as it attacks, its bite lands, it is hit, it dies, it growls while idle, its feet hit the ground.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cry {
    Aggro,
    Attack,
    Bite,
    Wound,
    Death,
    Fidget,
    Footstep,
}

/// The sound entries of one kind of creature, by `Cry` (footsteps excepted) and its footstep set.
struct Cries {
    entries: [u32; 6],
    footstep_set: u32,
}

/// Sounds the game asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sfx {
    Footstep,
    /// A hoof of the player's steed, whose model's sound data names this footstep set.
    SteedFoot(u32),
    Land,
    Swing,
    Exertion,
    /// A cry of a kind of creature (an index into the content's creatures).
    Cry(Cry, usize),
    /// The shield stops a hit, the sword hits flesh, the player is hurt, dies, jumps, presses an action button.
    Block,
    SwordHit,
    Wound,
    Death,
    Jump,
    ActionButton,
    /// An item (by its `handling`) picked up or put down.
    PickUp(&'static str),
    PutDown(&'static str),
    /// Any sound entry of the original by name: `igBackPackOpen`, `LOOTWINDOWCOINSOUND`, ...
    Named(&'static str),
    /// A sound entry by its number in `SoundEntries.dbc` (what the spell visuals name).
    Entry(u32),
    /// Like `Entry`, but kept until `Release` (or the next `Hold`) cuts it off: the sound of a cast while it goes on.
    Hold(u32),
    Release,
}

#[derive(Debug)]
pub struct SoundEntry {
    pub name: String,
    dir: String,
    files: Vec<String>,
    pub volume: f32,
    /// Within this many yards the sound is at full volume; beyond `cutoff` it is not heard (3D sounds).
    min_distance: f32,
    cutoff: f32,
}

impl SoundEntry {
    fn path(&self, file: usize) -> String {
        format!("{}\\{}", self.dir, self.files[file % self.files.len()])
    }
}

struct ZoneMusic {
    /// Silence between tracks in milliseconds, for day and night.
    silence_min: [u32; 2],
    silence_max: [u32; 2],
    /// Sound entries for day and night.
    sounds: [u32; 2],
}

struct Area {
    parent: u32,
    ambience: u32,
    music: u32,
    intro: u32,
}

/// The audio tables of the data files.
pub struct SoundBank {
    entries: HashMap<u32, SoundEntry>,
    zone_music: HashMap<u32, ZoneMusic>,
    ambience: HashMap<u32, [u32; 2]>,
    /// Zone intro id -> (sound entry, minutes before it may play again).
    intros: HashMap<u32, (u32, u32)>,
    areas: HashMap<u32, Area>,
    /// Terrain type (`TerrainType.dbc` id) -> its footstep sound id.
    terrain_sounds: HashMap<u32, u32>,
    /// (creature footstep set, terrain sound id) -> sound entry.
    footsteps: HashMap<(u32, u32), u32>,
    /// (creature footstep set, terrain sound id) -> the sound of a foot in water over that ground (the splash column of the same table).
    splashes: HashMap<(u32, u32), u32>,
    /// (liquid type, flow 0 still / 1 slow / 2 fast) -> the sound entry the liquid makes (`SoundWaterType`).
    water: HashMap<(u32, u32), u32>,
    /// (swing weight, critical) -> sound entry.
    swings: HashMap<(u32, u32), u32>,
    footstep_set: u32,
    exertion: u32,
    /// The cries of each kind of creature of the content that has any (beasts with sound data).
    cries: Vec<Option<Cries>>,
    /// Sound entries of the player's cries and of the sounds of combat and the interface.
    /// Sound entries by lower case name.
    names: HashMap<String, u32>,
    player_wound: u32,
    player_death: u32,
    block: u32,
    sword_hit: u32,
    jump: u32,
    button: u32,
}

/// The stereo WAVs of the original carry a wrong "block align" in their header (2 for 16-bit stereo,
/// which needs 4). FMOD did not mind; the decoder does, so correct the field before decoding.
fn repaired(mut bytes: Vec<u8>) -> Vec<u8> {
    let le16 = |b: &[u8], at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
    if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        let mut at = 12;
        while at + 8 <= bytes.len() {
            let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
            if &bytes[at..at + 4] == b"fmt " && size >= 16 && at + 24 <= bytes.len() && le16(&bytes, at + 8) == 1 {
                let align = le16(&bytes, at + 10) * le16(&bytes, at + 22).div_ceil(8);
                bytes[at + 20..at + 22].copy_from_slice(&align.to_le_bytes());
                break;
            }
            at += 8 + size + (size & 1);
        }
    }
    bytes
}

/// Day runs from 6:00 to 18:00; music and ambience have a night variant.
pub fn is_night(hour: f32) -> bool {
    !(6.0..18.0).contains(&hour)
}

impl SoundBank {
    /// The cries of each creature kind of the content: 1 exertion (the attack snarl), 3 injury, 6 death, 9 footstep set, 10 aggro, 14 fidget,
    /// 18 custom attack (the bite) of `CreatureSoundData.dbc`, found through the model of each beast. A person cries with the voice of the
    /// player's model.
    fn cries(mount: &Mount) -> Result<Vec<Option<Cries>>> {
        let dbc = |name: &str| -> Result<Dbc> { Ok(Dbc::parse(&mount.read(&format!("DBFilesClient\\{name}.dbc"))?)?) };
        let model_data = dbc("CreatureModelData")?;
        let sounds = dbc("CreatureSoundData")?;
        let player = model_data.iter().find(|r| r.string(2).eq_ignore_ascii_case(PLAYER_MODEL)).map(|r| r.u32(13)).context("player model has no creature model data")?;
        let row = sounds.find_by_id(player).context("player model has no creature sound data")?;
        Ok(crate::content::get()
            .creatures
            .iter()
            .map(|c| {
                let sound_row = |model: &str| {
                    let mdx = format!("{}.mdx", model.trim_end_matches(".m2"));
                    let data = model_data.iter().find(|r| r.string(2).eq_ignore_ascii_case(&mdx)).map(|r| r.u32(13))?;
                    sounds.find_by_id(data)
                };
                // A person cries with the voice of the model of his race and sex (the human male's when it has none).
                let crate::content::Body::Beast { model, .. } = &c.body else {
                    let crate::content::Body::Humanoid { look, .. } = &c.body else { return None };
                    let voice = sound_row(&look.model()).unwrap_or(row);
                    return Some(Cries { entries: [voice.u32(10), voice.u32(1), voice.u32(1), voice.u32(3), voice.u32(6), 0], footstep_set: voice.u32(9) });
                };
                let row = sound_row(model)?;
                Some(Cries { entries: [row.u32(10), row.u32(1), row.u32(18), row.u32(3), row.u32(6), row.u32(14)], footstep_set: row.u32(9) })
            })
            .collect())
    }

    /// The content changed: the cries by kind of creature are found again.
    pub fn reload_cries(&mut self, mount: &Mount) -> Result<()> {
        self.cries = SoundBank::cries(mount)?;
        Ok(())
    }

    pub fn load(mount: &Mount) -> Result<SoundBank> {
        let dbc = |name: &str| -> Result<Dbc> { Ok(Dbc::parse(&mount.read(&format!("DBFilesClient\\{name}.dbc"))?)?) };
        // SoundEntries: 0 id, 2 name, 3..13 files, 23 directory, 24 volume, 26 minimum distance, 27 cutoff distance.
        let entries: HashMap<u32, SoundEntry> = dbc("SoundEntries")?
            .iter()
            .map(|r| {
                let files = (3..13).map(|i| r.string(i)).filter(|s| !s.is_empty()).map(String::from).collect();
                (r.u32(0), SoundEntry { name: r.string(2).into(), dir: r.string(23).into(), files, volume: r.f32(24), min_distance: r.f32(26), cutoff: r.f32(27) })
            })
            .collect();
        // ZoneMusic: 0 id, 2..4 silence min, 4..6 silence max, 6..8 day / night sounds.
        let zone_music = dbc("ZoneMusic")?
            .iter()
            .map(|r| (r.u32(0), ZoneMusic { silence_min: [r.u32(2), r.u32(3)], silence_max: [r.u32(4), r.u32(5)], sounds: [r.u32(6), r.u32(7)] }))
            .collect();
        let ambience = dbc("SoundAmbience")?.iter().map(|r| (r.u32(0), [r.u32(1), r.u32(2)])).collect();
        // ZoneIntroMusicTable: 0 id, 2 sound entry, 4 minutes between plays.
        let intros = dbc("ZoneIntroMusicTable")?.iter().map(|r| (r.u32(0), (r.u32(2), r.u32(4)))).collect();
        // AreaTable: 0 id, 2 parent, 7 ambience, 8 zone music, 9 intro.
        let areas = dbc("AreaTable")?
            .iter()
            .map(|r| (r.u32(0), Area { parent: r.u32(2), ambience: r.u32(7), music: r.u32(8), intro: r.u32(9) }))
            .collect();
        // TerrainType: 0 id, 4 footstep sound id.
        let terrain_sounds = dbc("TerrainType")?.iter().map(|r| (r.u32(0), r.u32(4))).collect();
        // FootstepTerrainLookup: 1 creature footstep set, 2 terrain sound id, 3 sound entry.
        let footsteps = dbc("FootstepTerrainLookup")?.iter().map(|r| ((r.u32(1), r.u32(2)), r.u32(3))).collect();
        let splashes = dbc("FootstepTerrainLookup")?.iter().map(|r| ((r.u32(1), r.u32(2)), r.u32(4))).collect();
        // SoundWaterType: 1 liquid type, 2 flow (0, 4 or 8), 3 sound entry.
        let water = dbc("SoundWaterType")?.iter().map(|r| ((r.u32(1), (r.u32(2) >> 2).min(2)), r.u32(3))).collect();
        // WeaponSwingSounds2: 1 swing weight, 2 critical, 3 sound entry.
        let swings = dbc("WeaponSwingSounds2")?.iter().map(|r| ((r.u32(1), r.u32(2)), r.u32(3))).collect();

        // The player's sound data: CreatureModelData (2 model path, 13 sound data) -> CreatureSoundData
        // (1 exertion sound, 9 footstep set).
        let sound_data = dbc("CreatureModelData")?
            .iter()
            .find(|r| r.string(2).eq_ignore_ascii_case(PLAYER_MODEL))
            .map(|r| r.u32(13))
            .context("player model has no creature model data")?;
        let sounds = dbc("CreatureSoundData")?;
        let row = sounds.find_by_id(sound_data).context("player model has no creature sound data")?;
        let cries = SoundBank::cries(mount)?;
        // The rest are found by their names in `SoundEntries`: the sword's impact on flesh and the impact on a wooden
        // shield (the columns of `WeaponImpactSounds`), the male human's jump grunt and the action button click.
        let names: HashMap<String, u32> = entries.iter().map(|(&id, e)| (e.name.to_ascii_lowercase(), id)).collect();
        let named = |name: &str| entries.iter().find(|(_, e)| e.name.to_ascii_lowercase().ends_with(name)).map_or(0, |(&id, _)| id);
        let (block, sword_hit, jump, button) = (named("shieldwoodimpact"), named("sword1h_armorflesh"), named("humanmalemainjump"), named("actionbarbuttondown"));
        Ok(SoundBank { entries, zone_music, ambience, intros, areas, terrain_sounds, footsteps, splashes, water, swings, footstep_set: row.u32(9), exertion: row.u32(1), cries, names, player_wound: row.u32(3), player_death: row.u32(6), block, sword_hit, jump, button })
    }

    /// The first non-zero `pick` of the area or, failing that, of its parent zone.
    fn inherited(&self, mut area: u32, pick: impl Fn(&Area) -> u32) -> Option<u32> {
        for _ in 0..4 {
            let a = self.areas.get(&area)?;
            match pick(a) {
                0 if a.parent != 0 && a.parent != area => area = a.parent,
                0 => return None,
                id => return Some(id),
            }
        }
        None
    }

    /// Zone music set of an area.
    pub fn music_for(&self, area: u32) -> Option<u32> {
        self.inherited(area, |a| a.music).filter(|m| self.zone_music.contains_key(m))
    }

    /// Ambience (day and night sound entries) of an area.
    pub fn ambience_for(&self, area: u32) -> Option<[u32; 2]> {
        self.inherited(area, |a| a.ambience).and_then(|a| self.ambience.get(&a).copied())
    }

    /// Intro (id, sound entry, minutes between plays) played when entering exactly this area.
    pub fn intro_for(&self, area: u32) -> Option<(u32, u32, u32)> {
        let id = self.areas.get(&area)?.intro;
        self.intros.get(&id).map(|&(entry, delay)| (id, entry, delay))
    }

    /// The player's footstep on a terrain type (`TerrainType.dbc` ids).
    pub fn footstep(&self, terrain: u32) -> Option<u32> {
        self.footstep_in(self.footstep_set, terrain)
    }

    /// The footstep of a footstep set (`CreatureSoundData` field 9) on a terrain type.
    /// Set 0 means "none" (chickens): the table still has rows for it, all the treant's stomp.
    pub fn footstep_in(&self, set: u32, terrain: u32) -> Option<u32> {
        if set == 0 {
            return None;
        }
        self.footsteps.get(&(set, *self.terrain_sounds.get(&terrain)?)).copied()
    }

    /// A foot of the player in water over the ground of terrain type `terrain`.
    pub fn splash_footstep(&self, terrain: u32) -> Option<u32> {
        self.splashes.get(&(self.footstep_set, *self.terrain_sounds.get(&terrain)?)).copied().filter(|&id| id != 0)
    }

    /// The sound of a liquid of `kind` (0 water, 1 ocean, 2 magma, 3 slime) flowing at `speed` (0 still, 1 slow, 2 fast).
    pub fn water_sound(&self, kind: usize, speed: usize) -> Option<u32> {
        self.water.get(&(kind as u32, speed as u32)).copied().filter(|&id| id != 0)
    }

    pub fn entry(&self, id: u32) -> Option<&SoundEntry> {
        self.entries.get(&id).filter(|e| !e.files.is_empty())
    }

    pub fn swing(&self) -> Option<u32> {
        self.swings.get(&SWING_MEDIUM).copied()
    }

    pub fn exertion(&self) -> u32 {
        self.exertion
    }

    /// The sound entry of a cry of creature kind `kind` (0 when it has none).
    pub fn cry(&self, kind: usize, cry: Cry) -> u32 {
        let Some(Some(c)) = self.cries.get(kind) else { return 0 };
        match cry {
            Cry::Aggro => c.entries[0],
            Cry::Attack => c.entries[1],
            Cry::Bite => c.entries[2],
            Cry::Wound => c.entries[3],
            Cry::Death => c.entries[4],
            Cry::Fidget => c.entries[5],
            Cry::Footstep => 0,
        }
    }

    /// The sound entry of the player's own cries and of combat and interface sounds (0 for any other sound).
    pub fn effect(&self, sfx: Sfx) -> u32 {
        match sfx {
            Sfx::Block => self.block,
            Sfx::SwordHit => self.sword_hit,
            Sfx::Wound => self.player_wound,
            Sfx::Death => self.player_death,
            Sfx::Jump => self.jump,
            Sfx::ActionButton => self.button,
            _ => 0,
        }
    }

    /// A sound entry by its name in `SoundEntries` (case does not matter).
    pub fn named(&self, name: &str) -> Option<u32> {
        self.names.get(&name.to_ascii_lowercase()).copied()
    }

    /// The foot of creature kind `kind` on a terrain type.
    pub fn cry_footstep(&self, kind: usize, terrain: u32) -> Option<u32> {
        self.footstep_in(self.cries.get(kind)?.as_ref()?.footstep_set, terrain)
    }
}

/// A playing sound that fades in and out.
struct Voice {
    sink: Sink,
    /// Volume at full level.
    volume: f32,
    level: f32,
    fading_out: bool,
}

/// Voices of one kind (music or ambience): normally one, plus the ones fading out.
#[derive(Default)]
struct Channel {
    voices: Vec<Voice>,
}

impl Channel {
    fn fade_out(&mut self) {
        self.voices.iter_mut().for_each(|v| v.fading_out = true);
    }

    fn live(&self) -> bool {
        self.voices.iter().any(|v| !v.fading_out)
    }

    /// Move the fades along. Returns whether a voice that was playing at full run ended by itself.
    fn tick(&mut self, dt: f32, gain: f32) -> bool {
        let mut ended = false;
        self.voices.retain_mut(|v| {
            v.level = (v.level + dt / FADE_SECONDS * if v.fading_out { -1.0 } else { 1.0 }).clamp(0.0, 1.0);
            v.sink.set_volume(v.volume * v.level * gain);
            let done = if v.fading_out { v.level <= 0.0 } else { v.sink.empty() };
            ended |= done && !v.fading_out;
            if done {
                v.sink.stop();
            }
            !done
        });
        ended
    }
}

/// A file being read in the background.
struct Load {
    rx: Receiver<Result<Vec<u8>>>,
    /// Volume of the entry at full level.
    volume: f32,
}

/// The sound options (`SoundOptionsFrame`), kept between runs in `settings.ron`: three switches and four volumes from 0 to 1.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SoundSettings {
    pub all: bool,
    pub ambience: bool,
    pub music: bool,
    pub master: f32,
    pub sound: f32,
    pub music_volume: f32,
    pub ambience_volume: f32,
}

impl Default for SoundSettings {
    fn default() -> Self {
        SoundSettings { all: true, ambience: true, music: true, master: 1.0, sound: 1.0, music_volume: 1.0, ambience_volume: 1.0 }
    }
}

impl SoundSettings {
    pub fn load() -> SoundSettings {
        crate::save::load_settings("settings.ron")
    }

    pub fn save(&self) {
        crate::save::write_settings("settings.ron", self);
    }
}

pub struct Audio {
    /// Scales everything (the client's master volume, 1.0 by default).
    pub master_volume: f32,
    /// The sound, music and ambience volumes of the options (0 to 1), under the master volume.
    pub sfx_gain: f32,
    pub music_gain: f32,
    pub ambience_gain: f32,
    _stream: OutputStream,
    handle: OutputStreamHandle,
    mount: Arc<Mount>,
    pub bank: SoundBank,
    clock: f64,
    rng: u64,
    area: Option<u32>,
    music: Channel,
    music_set: Option<u32>,
    music_load: Option<Load>,
    /// When the next track starts (seconds on `clock`).
    music_next: f64,
    last_track: Option<String>,
    /// An intro entry waiting to play instead of the next track, and when each intro last played.
    intro: Option<u32>,
    intro_played: HashMap<u32, f64>,
    ambience: Channel,
    ambience_entry: Option<u32>,
    ambience_load: Option<Load>,
    /// The loop of the weather (rain, snow, sand): its file, while it plays.
    weather_sound: Channel,
    weather_file: Option<&'static str>,
    weather_load: Option<Load>,
    sfx_cache: HashMap<String, Arc<[u8]>>,
    /// The ears: the camera's position and yaw.
    listener: (Vec3, f32),
    /// The sound of the cast that goes on (`Sfx::Hold`).
    held: Option<Sink>,
    /// Sounds that loop where they are (a stream, a waterfall), by what the game calls them: they are heard from their place.
    places: HashMap<u64, Place>,
    /// Places that have gone out of earshot, still dying down; and when they may be stopped.
    ended: Vec<(f64, Sink)>,
    /// Whether the player's feet are in water: the footsteps splash.
    wading: bool,
    /// The camera is under the water: the loop of the sea replaces the music, the ambience and the sounds of the water about (`FUN_004609b0`'s
    /// "Underwater (DONOTRENAME)" entry, `FUN_00460bd0`, `FUN_00462b50`).
    underwater: bool,
    under: Channel,
    under_load: Option<Load>,
    /// The theme of the screens before the game has been asked for.
    menu_started: bool,
}

/// The music of the screens before the game (the character list and the creation), which plays over and over.
const MENU_THEME: &str = r"Sound\Music\GlueScreenMusic\wow_main_theme.mp3";

/// A sound that loops at a place: what the source reads to pan and quieten it, and the sink it plays on.
struct Place {
    sink: Sink,
    ears: Arc<[AtomicU32; 2]>,
}

impl Audio {
    /// Take the sound options into use (the sounds playing change at once).
    pub fn apply(&mut self, o: &SoundSettings) {
        self.master_volume = if o.all { o.master } else { 0.0 };
        self.sfx_gain = o.sound;
        self.music_gain = if o.music { o.music_volume } else { 0.0 };
        self.ambience_gain = if o.ambience { o.ambience_volume } else { 0.0 };
    }

    pub fn new(mount: Arc<Mount>) -> Result<Audio> {
        let (stream, handle) = OutputStream::try_default().context("no audio output device")?;
        let bank = SoundBank::load(&mount)?;
        Ok(Audio {
            master_volume: 1.0,
            sfx_gain: 1.0,
            music_gain: 1.0,
            ambience_gain: 1.0,
            _stream: stream,
            handle,
            mount,
            bank,
            clock: 0.0,
            rng: 0x9E37_79B9_7F4A_7C15,
            area: None,
            music: Channel::default(),
            music_set: None,
            music_load: None,
            music_next: 0.0,
            last_track: None,
            intro: None,
            intro_played: HashMap::new(),
            ambience: Channel::default(),
            ambience_entry: None,
            ambience_load: None,
            weather_sound: Channel::default(),
            weather_file: None,
            weather_load: None,
            sfx_cache: HashMap::new(),
            listener: (Vec3::ZERO, 0.0),
            held: None,
            places: HashMap::new(),
            ended: Vec::new(),
            wading: false,
            underwater: false,
            under: Channel::default(),
            under_load: None,
            menu_started: false,
        })
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Read a sound file on another thread, so entering a zone does not stall the frame.
    fn read_in_background(&self, path: String, volume: f32) -> Load {
        let (tx, rx) = channel();
        let mount = self.mount.clone();
        std::thread::spawn(move || {
            let _ = tx.send(mount.read(&path).map_err(anyhow::Error::from).with_context(|| path));
        });
        Load { rx, volume }
    }

    fn start(&self, channel: &mut Channel, bytes: Vec<u8>, volume: f32, looped: bool) {
        let Ok(sink) = Sink::try_new(&self.handle) else { return };
        let bytes = repaired(bytes);
        let ok = if looped {
            Decoder::new_looped(Cursor::new(bytes)).map(|d| sink.append(d)).is_ok()
        } else {
            Decoder::new(Cursor::new(bytes)).map(|d| sink.append(d)).is_ok()
        };
        if ok {
            sink.set_volume(0.0);
            channel.voices.push(Voice { sink, volume, level: 0.0, fading_out: false });
        }
    }

    /// Advance the music and ambience for the player being in `area` at `hour` (0..24).
    /// A room of a building with music or ambience of its own overrides the area's.
    pub fn update(&mut self, dt: f32, area: Option<u32>, room: Option<crate::world::Room>, hour: f32) {
        self.clock += dt as f64;
        let night = is_night(hour) as usize;

        // Entering an area: its intro, if it has one that has not played lately, replaces the next track.
        if area != self.area {
            self.area = area;
            if let Some((id, entry, delay)) = area.and_then(|a| self.bank.intro_for(a)) {
                if self.intro_played.get(&id).is_none_or(|t| self.clock - t >= delay as f64 * 60.0) {
                    self.intro_played.insert(id, self.clock);
                    self.intro = Some(entry);
                    self.restart_music();
                }
            }
        }

        // Zone music: a new set fades the old one out and starts right away; after a track comes a silence.
        let room = room.unwrap_or_default();
        let set = if self.underwater {
            None
        } else {
            Some(room.music).filter(|m| self.bank.zone_music.contains_key(m)).or_else(|| area.and_then(|a| self.bank.music_for(a)))
        };
        if set != self.music_set {
            self.music_set = set;
            self.restart_music();
        }
        if self.music.tick(dt, self.master_volume * self.music_gain) {
            if let Some(zone) = self.music_set.and_then(|s| self.bank.zone_music.get(&s)) {
                let (lo, hi) = (zone.silence_min[night] as f64, zone.silence_max[night].max(zone.silence_min[night]) as f64);
                self.music_next = self.clock + (lo + (hi - lo) * self.random() as f64) / 1000.0;
            }
        }
        if let Some((result, volume)) = self.music_load.as_ref().and_then(|l| l.rx.try_recv().ok().map(|r| (r, l.volume))) {
            self.music_load = None;
            match result {
                Ok(bytes) => {
                    let mut channel = std::mem::take(&mut self.music);
                    self.start(&mut channel, bytes, volume * MUSIC_VOLUME, false);
                    self.music = channel;
                }
                Err(e) => log::warn!("music: {e:#}"),
            }
        }
        if self.music_load.is_none() && !self.music.live() && self.clock >= self.music_next {
            let roll = self.random();
            let entry = self.intro.take().or_else(|| self.music_set.and_then(|s| self.bank.zone_music.get(&s)).map(|z| z.sounds[night]));
            if let Some(e) = entry.and_then(|id| self.bank.entry(id)) {
                // A random track, not the one that just played.
                let mut pick = (roll * e.files.len() as f32) as usize;
                if e.files.len() > 1 && self.last_track.as_deref() == Some(e.files[pick % e.files.len()].as_str()) {
                    pick += 1;
                }
                let (path, volume) = (e.path(pick), e.volume);
                log::info!("music: {} ({path})", e.name);
                self.last_track = Some(e.files[pick % e.files.len()].clone());
                self.music_load = Some(self.read_in_background(path, volume));
            } else {
                // Nothing to play here: look again when the zone changes.
                self.music_next = f64::MAX;
            }
        }
        let own = self.bank.ambience.get(&room.ambience).copied();
        self.update_ambience(if self.underwater { None } else { own.or_else(|| area.and_then(|a| self.bank.ambience_for(a))) }, night, dt);
        self.update_underwater(dt);
    }

    /// Keep the music of the screens before the game going (call every frame there): the main theme, over and over.
    pub fn update_menu(&mut self, dt: f32) {
        if !std::mem::replace(&mut self.menu_started, true) {
            self.music_load = Some(self.read_in_background(MENU_THEME.to_string(), 1.0));
        }
        self.music.tick(dt, self.master_volume * self.music_gain);
        if let Some((result, volume)) = self.music_load.as_ref().and_then(|l| l.rx.try_recv().ok().map(|r| (r, l.volume))) {
            self.music_load = None;
            match result {
                Ok(bytes) => {
                    let mut channel = std::mem::take(&mut self.music);
                    log::info!("music: {MENU_THEME}");
                    self.start(&mut channel, bytes, volume * MUSIC_VOLUME, true);
                    self.music = channel;
                }
                Err(e) => log::warn!("music: {e:#}"),
            }
        }
    }

    /// The camera goes under the water or comes up out of it.
    pub fn set_underwater(&mut self, underwater: bool) {
        if underwater != self.underwater {
            self.underwater = underwater;
            if underwater {
                self.places.clear();
                if let Some(e) = self.bank.named("Underwater (DONOTRENAME)").and_then(|id| self.bank.entry(id)) {
                    let (path, volume) = (e.path(0), e.volume);
                    self.under_load = Some(self.read_in_background(path, volume));
                }
            } else {
                self.under_load = None;
                self.under.fade_out();
            }
        }
    }

    fn update_underwater(&mut self, dt: f32) {
        self.under.tick(dt, self.master_volume * self.music_gain);
        if let Some((result, volume)) = self.under_load.as_ref().and_then(|l| l.rx.try_recv().ok().map(|r| (r, l.volume))) {
            self.under_load = None;
            match result {
                Ok(bytes) => {
                    let mut channel = std::mem::take(&mut self.under);
                    log::info!("music: {MENU_THEME}");
                    self.start(&mut channel, bytes, volume * MUSIC_VOLUME, true);
                    self.under = channel;
                }
                Err(e) => log::warn!("underwater sound: {e:#}"),
            }
        }
    }

    /// Fade the current track out and let the next one start now.
    fn restart_music(&mut self) {
        self.music.fade_out();
        self.music_load = None;
        self.music_next = self.clock;
        self.last_track = None;
    }

    /// Ambience: the zone's day or night loop, crossfading when either changes.
    fn update_ambience(&mut self, sounds: Option<[u32; 2]>, night: usize, dt: f32) {
        let want = sounds.map(|e| e[night]).filter(|&e| e != 0);
        if want != self.ambience_entry {
            self.ambience_entry = want;
            self.ambience.fade_out();
            self.ambience_load = None;
            if let Some(e) = want.and_then(|id| self.bank.entry(id)) {
                let (path, volume) = (e.path(0), e.volume);
                self.ambience_load = Some(self.read_in_background(path, volume));
            }
        }
        self.ambience.tick(dt, self.master_volume * self.ambience_gain);
        if let Some(load) = self.ambience_load.as_ref().and_then(|l| l.rx.try_recv().ok().map(|r| (r, l.volume))) {
            self.ambience_load = None;
            match load.0 {
                Ok(bytes) => {
                    let mut channel = std::mem::take(&mut self.ambience);
                    self.start(&mut channel, bytes, load.1 * AMBIENCE_VOLUME, true);
                    self.ambience = channel;
                }
                Err(e) => log::warn!("ambience: {e:#}"),
            }
        }
    }

    /// The sound of the weather: `sound` is the file and the strength of what it makes now (`Weather::sound`), `indoors` muffles it.
    pub fn set_weather(&mut self, sound: Option<(&'static str, f32)>, indoors: bool, dt: f32) {
        let file = sound.map(|s| s.0);
        if file != self.weather_file {
            self.weather_file = file;
            self.weather_sound.fade_out();
            self.weather_load = file.map(|f| self.read_in_background(f.to_string(), 1.0));
        }
        let volume = sound.map_or(0.0, |(_, strength)| (0.35 + 0.65 * strength) * WEATHER_VOLUME) * if indoors { 0.3 } else { 1.0 };
        self.weather_sound.voices.iter_mut().filter(|v| !v.fading_out).for_each(|v| v.volume = volume);
        self.weather_sound.tick(dt, self.master_volume * self.ambience_gain);
        if let Some(result) = self.weather_load.as_ref().and_then(|l| l.rx.try_recv().ok()) {
            self.weather_load = None;
            match result {
                Ok(bytes) => {
                    let mut channel = std::mem::take(&mut self.weather_sound);
                    self.start(&mut channel, bytes, volume, true);
                    self.weather_sound = channel;
                }
                Err(e) => log::warn!("weather sound: {e:#}"),
            }
        }
    }

    /// Play a one-shot sound effect; `terrain` is the footstep surface (`TerrainType.dbc` id).
    pub fn play(&mut self, sfx: Sfx, terrain: u32) {
        self.play_sound(sfx, terrain, None);
    }

    /// Like `play`, for a sound made at `source` (render space): quieter with distance, panned between the ears.
    pub fn play_3d(&mut self, sfx: Sfx, terrain: u32, source: Vec3) {
        self.play_sound(sfx, terrain, Some(source));
    }

    /// Sets the sounds that loop at places (`key`, sound entry, where): each is heard from there, quieter and panned as the ears move; one that is not
    /// asked for any more dies down. Call it every frame.
    pub fn set_places(&mut self, wanted: &[(u64, u32, Vec3)]) {
        // Under the water there is only the loop of the sea.
        let wanted = if self.underwater { &[] } else { wanted };
        let (listener, yaw) = self.listener;
        let mut seen = Vec::new();
        for &(key, entry, at) in wanted {
            let Some(e) = self.bank.entry(entry) else { continue };
            let gains = spatial_gains(listener, yaw, at, e.min_distance, e.cutoff);
            seen.push(key);
            if let Some(place) = self.places.get(&key) {
                place.ears[0].store(gains[0].to_bits(), Ordering::Relaxed);
                place.ears[1].store(gains[1].to_bits(), Ordering::Relaxed);
            } else if gains != [0.0, 0.0] {
                let (path, volume) = (e.path(0), e.volume * SFX_VOLUME * self.master_volume * self.sfx_gain);
                if let Some(place) = self.start_place(&path, volume, gains) {
                    self.places.insert(key, place);
                }
            }
        }
        let gone: Vec<u64> = self.places.keys().filter(|k| !seen.contains(k)).copied().collect();
        for key in gone {
            if let Some(place) = self.places.remove(&key) {
                place.ears.iter().for_each(|g| g.store(0f32.to_bits(), Ordering::Relaxed));
                self.ended.push((self.clock + 0.5, place.sink));
            }
        }
        let now = self.clock;
        self.ended.retain(|(until, sink)| {
            let alive = *until > now;
            if !alive {
                sink.stop();
            }
            alive
        });
    }

    fn start_place(&mut self, path: &str, volume: f32, gains: [f32; 2]) -> Option<Place> {
        let bytes = match self.sfx_cache.get(path) {
            Some(b) => b.clone(),
            None => match self.mount.read(path) {
                Ok(b) => self.sfx_cache.entry(path.to_string()).or_insert_with(|| repaired(b).into()).clone(),
                Err(e) => {
                    log::warn!("sound {path}: {e}");
                    return None;
                }
            },
        };
        let decoder = Decoder::new_looped(Cursor::new(bytes)).ok()?;
        let sink = Sink::try_new(&self.handle).ok()?;
        let ears = Arc::new([AtomicU32::new(gains[0].to_bits()), AtomicU32::new(gains[1].to_bits())]);
        sink.append(Positioned::new(decoder.convert_samples::<f32>(), ears.clone()));
        sink.set_volume(volume);
        Some(Place { sink, ears })
    }

    /// Whether the player's feet are in water: the steps splash.
    pub fn set_wading(&mut self, wading: bool) {
        self.wading = wading;
    }

    /// The splash of a character that is waist deep in water or has come out of it (`FUN_006030c0`, at 0.4 of its height): its size says which.
    pub fn splash(&mut self, height: f32) {
        self.play(Sfx::Named(if height < 1.6 { "CharacterSplashSoundSmall" } else if height < 2.7 { "CharacterSplashSoundMedium" } else { "CharacterSplashSoundlarge" }), 0);
    }

    /// Where the ears are: the camera's position and heading.
    pub fn set_listener(&mut self, pos: Vec3, yaw: f32) {
        self.listener = (pos, yaw);
    }

    fn play_sound(&mut self, sfx: Sfx, terrain: u32, source: Option<Vec3>) {
        if sfx == Sfx::Release {
            return drop(self.held.take());
        }
        let entry = match sfx {
            Sfx::Footstep | Sfx::Land => self.wading.then(|| self.bank.splash_footstep(terrain)).flatten().or_else(|| self.bank.footstep(terrain)),
            Sfx::SteedFoot(set) => self.bank.footstep_in(set, terrain),
            Sfx::Swing => self.bank.swing(),
            Sfx::Exertion => Some(self.bank.exertion()),
            Sfx::Cry(Cry::Footstep, kind) => self.bank.cry_footstep(kind, terrain),
            Sfx::PickUp(h) => self.bank.named(&format!("PickUp{h}")),
            Sfx::PutDown(h) => self.bank.named(&format!("PutDown{}", put_down_name(h))),
            Sfx::Named(name) => self.bank.named(name),
            Sfx::Entry(id) | Sfx::Hold(id) => Some(id),
            Sfx::Release => None,
            Sfx::Block | Sfx::SwordHit | Sfx::Wound | Sfx::Death | Sfx::Jump | Sfx::ActionButton => Some(self.bank.effect(sfx)),
            Sfx::Cry(cry, kind) => Some(self.bank.cry(kind, cry)),
        };
        let roll = self.random();
        let Some(e) = entry.and_then(|id| self.bank.entry(id)) else { return };
        let (path, volume) = (e.path((roll * e.files.len() as f32) as usize), e.volume * SFX_VOLUME * self.master_volume * self.sfx_gain);
        let gains = source.map(|at| spatial_gains(self.listener.0, self.listener.1, at, e.min_distance, e.cutoff));
        if gains == Some([0.0, 0.0]) {
            return;
        }
        let bytes = match self.sfx_cache.get(&path) {
            Some(b) => b.clone(),
            None => match self.mount.read(&path) {
                Ok(b) => self.sfx_cache.entry(path).or_insert_with(|| repaired(b).into()).clone(),
                Err(e) => return log::warn!("sound {path}: {e}"),
            },
        };
        let Ok(sink) = Sink::try_new(&self.handle) else { return };
        let Ok(decoder) = Decoder::new(Cursor::new(bytes)) else { return };
        match gains {
            Some(gains) => sink.append(panned(decoder, gains)),
            None => sink.append(decoder),
        }
        sink.set_volume(volume);
        if matches!(sfx, Sfx::Hold(_)) {
            self.held = Some(sink);
        } else {
            sink.detach();
        }
    }
}

/// The name of an item sound when it is put down: the original spells a few of them the other way round.
pub fn put_down_name(handling: &str) -> &str {
    match handling {
        "MetalLarge" => "LargeMetal",
        "MetalSmall" => "SmallMetal",
        "Cloth_Leather" => "ClothLeather",
        "Rocks_Ore" => "Rock_Ore",
        other => other,
    }
}

/// How loud a sound at `source` is in the left and right ear of a listener at `listener` heading `yaw` (0 =
/// north, like the camera): full volume within `min` yards, then falling off like 1/distance (FMOD's inverse
/// roll-off the client used), fading out over the last fifth before `cutoff`, and panned by its direction.
pub fn spatial_gains(listener: Vec3, yaw: f32, source: Vec3, min: f32, cutoff: f32) -> [f32; 2] {
    let (min, cutoff) = (min.max(0.1), if cutoff > min { cutoff } else { 45.0 });
    let to = source - listener;
    let distance = to.length();
    if distance >= cutoff {
        return [0.0, 0.0];
    }
    let fade = 1.0 - ((distance - cutoff * 0.8) / (cutoff * 0.2)).clamp(0.0, 1.0);
    let gain = if distance <= min { 1.0 } else { min / distance } * fade;
    // Constant-power pan from the direction across the listener's right (x = cos yaw, z = sin yaw), centre = 1.
    let flat = Vec3::new(to.x, 0.0, to.z).normalize_or_zero();
    let pan = flat.dot(Vec3::new(yaw.cos(), 0.0, yaw.sin()));
    let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    [gain * (angle.cos() * std::f32::consts::SQRT_2).min(1.0), gain * (angle.sin() * std::f32::consts::SQRT_2).min(1.0)]
}

/// A looping sound for the two ears: the sound's own channels spread over a left and a right ear whose volumes are read from `ears` as it plays and
/// move to the new ones over a tenth of a second, so that a sound that is panned or put out does not click.
struct Positioned<I> {
    inner: I,
    channels: u16,
    rate: u32,
    ears: Arc<[AtomicU32; 2]>,
    now: [f32; 2],
    frame: [f32; 2],
    right: bool,
}

impl<I: Source<Item = f32>> Positioned<I> {
    fn new(inner: I, ears: Arc<[AtomicU32; 2]>) -> Self {
        let (channels, rate) = (inner.channels(), inner.sample_rate());
        let now = [f32::from_bits(ears[0].load(Ordering::Relaxed)), f32::from_bits(ears[1].load(Ordering::Relaxed))];
        Positioned { inner, channels, rate, ears, now, frame: [0.0; 2], right: true }
    }
}

impl<I: Source<Item = f32>> Iterator for Positioned<I> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        self.right = !self.right;
        if !self.right {
            // The next frame of the sound: one sample spread over both ears, or the first two of its channels.
            let left = self.inner.next()?;
            self.frame = if self.channels == 1 {
                [left, left]
            } else {
                let right = self.inner.next()?;
                for _ in 2..self.channels {
                    self.inner.next()?;
                }
                [left, right]
            };
            let step = 10.0 / self.rate as f32;
            for (ear, now) in self.now.iter_mut().enumerate() {
                let target = f32::from_bits(self.ears[ear].load(Ordering::Relaxed));
                *now += (target - *now).clamp(-step, step);
            }
        }
        let ear = usize::from(self.right);
        Some(self.frame[ear] * self.now[ear])
    }
}

impl<I: Source<Item = f32>> Source for Positioned<I> {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        2
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

/// The decoded sound as stereo with the gains of the two ears applied (a mono sound is spread over both).
fn panned(decoder: Decoder<Cursor<Arc<[u8]>>>, gains: [f32; 2]) -> SamplesBuffer<f32> {
    let (channels, rate) = (decoder.channels() as usize, decoder.sample_rate());
    let samples: Vec<f32> = decoder.convert_samples::<f32>().collect();
    let out: Vec<f32> = if channels == 1 {
        samples.iter().flat_map(|&s| [s * gains[0], s * gains[1]]).collect()
    } else {
        samples.iter().enumerate().map(|(i, &s)| s * gains[(i % channels).min(1)]).collect()
    };
    // Always two channels: a mono sound was spread over both above.
    SamplesBuffer::new(2, rate, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wowfmt::m2::M2;

    fn bank() -> Option<(Mount, SoundBank)> {
        let mount = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok())?;
        let bank = SoundBank::load(&mount).expect("sound tables load");
        Some((mount, bank))
    }

    #[test]
    fn panning_keeps_the_pitch_and_length_of_mono_and_stereo_sounds() {
        // 1000 frames at 22050 Hz, 16-bit, mono and stereo.
        let wav = |channels: u16| {
            let data: Vec<u8> = (0..1000 * channels as usize).flat_map(|i| (((i % 50) as i16 - 25) * 400).to_le_bytes()).collect();
            let mut w = b"RIFF".to_vec();
            w.extend((36 + data.len() as u32).to_le_bytes());
            w.extend(b"WAVEfmt ");
            w.extend(16u32.to_le_bytes());
            w.extend([1, 0]);
            w.extend(channels.to_le_bytes());
            w.extend(22050u32.to_le_bytes());
            w.extend((22050 * 2 * channels as u32).to_le_bytes());
            w.extend((2 * channels).to_le_bytes());
            w.extend(16u16.to_le_bytes());
            w.extend(b"data");
            w.extend((data.len() as u32).to_le_bytes());
            w.extend(data);
            let bytes: Arc<[u8]> = w.into();
            Decoder::new(Cursor::new(bytes)).unwrap()
        };
        for channels in [1, 2] {
            let out = panned(wav(channels), [1.0, 0.5]);
            // Played as stereo at the file's rate, the same 1000 frames: the same duration, not slower or lower.
            assert_eq!((out.channels(), out.sample_rate()), (2, 22050), "{channels} channels");
            assert!((out.total_duration().unwrap().as_secs_f64() - 1000.0 / 22050.0).abs() < 1e-6);
        }
    }

    #[test]
    fn sounds_fall_off_with_distance_and_pan_towards_their_side() {
        let at = |x: f32, z: f32| spatial_gains(Vec3::ZERO, 0.0, Vec3::new(x, 0.0, z), 8.0, 45.0);
        // Facing north (-Z): straight ahead and behind are centred; east is the right ear, west the left.
        assert!(at(0.0, -5.0).iter().all(|g| (g - 1.0).abs() < 1e-5));
        let (ahead, behind) = (at(0.0, -20.0), at(0.0, 20.0));
        assert!((ahead[0] - 0.4).abs() < 1e-3 && ahead[0] == ahead[1] && ahead == behind, "{ahead:?}");
        let (east, west) = (at(20.0, 0.0), at(-20.0, 0.0));
        assert!(east[1] > east[0] * 3.0 && west[0] > west[1] * 3.0 && (east[1] - west[0]).abs() < 1e-5, "{east:?} {west:?}");
        // Farther is quieter, the last fifth fades out, and beyond the cutoff nothing is heard.
        assert!(at(0.0, -30.0)[0] < at(0.0, -20.0)[0] && at(0.0, -40.0)[0] < 8.0 / 40.0 * 0.6);
        assert_eq!(at(0.0, -45.0), [0.0, 0.0]);
        // Turning to face east puts the east source ahead of the listener.
        let turned = spatial_gains(Vec3::ZERO, std::f32::consts::FRAC_PI_2, Vec3::new(20.0, 0.0, 0.0), 8.0, 45.0);
        assert!((turned[0] - turned[1]).abs() < 1e-5);
    }

    #[test]
    fn a_wrong_block_align_in_a_wav_header_is_corrected() {
        // 16-bit stereo, block align written as 2 instead of 4.
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend(44u32.to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        // PCM, 2 channels, 22050 Hz, 88200 bytes/s, block align 2 (wrong), 16 bits.
        for v in [1u16, 2] {
            wav.extend(v.to_le_bytes());
        }
        wav.extend(22050u32.to_le_bytes());
        wav.extend(88200u32.to_le_bytes());
        for v in [2u16, 16] {
            wav.extend(v.to_le_bytes());
        }
        wav.extend(b"data");
        wav.extend(8u32.to_le_bytes());
        wav.extend([0u8; 8]);
        assert_eq!(&wav[32..34], [2, 0]);
        let fixed = repaired(wav);
        assert_eq!(&fixed[32..34], [4, 0]);
        assert!(Decoder::new(Cursor::new(fixed)).is_ok());
    }

    /// Run the engine for `seconds` of game time in `area`, giving the background reads real time to finish.
    fn run(audio: &mut Audio, area: Option<u32>, hour: f32, seconds: f32) {
        for _ in 0..(seconds * 20.0) as usize {
            audio.update(0.05, area, None, hour);
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
    }

    #[test]
    fn saved_sound_options_fall_back_to_the_defaults_for_what_is_missing() {
        let o: SoundSettings = ron::from_str("(music: false, master: 0.5)").unwrap();
        assert_eq!(o, SoundSettings { music: false, master: 0.5, ..Default::default() });
    }

    #[test]
    fn music_and_ambience_follow_the_area_with_intro_fades_and_silences() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let Ok(mut audio) = Audio::new(Arc::new(mount)) else { return eprintln!("skipped: no audio device") };
        audio.master_volume = 0.0;
        // Elwynn Forest: forest music and its ambience start.
        run(&mut audio, Some(12), 12.0, 1.0);
        assert!(audio.music.live() && audio.ambience.live(), "nothing playing in Elwynn");
        assert!(audio.last_track.as_deref().is_some_and(|t| t.contains("Forest")), "track {:?}", audio.last_track);
        // The track ends: the next one waits out a 3-5 minute silence.
        audio.music.voices.iter_mut().for_each(|v| v.sink.stop());
        run(&mut audio, Some(12), 12.0, 0.2);
        assert!(!audio.music.live() && audio.music_next > audio.clock + 170.0 && audio.music_next < audio.clock + 310.0);
        // Walking into Stormwind City: its intro plays once, at once, ahead of the silence.
        run(&mut audio, Some(1519), 12.0, 1.0);
        assert!(audio.music.live() && audio.last_track.as_deref().is_some_and(|t| t.contains("stormwind_intro")), "track {:?}", audio.last_track);
        // Leaving and coming back within the hour: no second intro.
        audio.music.voices.iter_mut().for_each(|v| v.sink.stop());
        run(&mut audio, Some(12), 12.0, 1.0);
        run(&mut audio, Some(1519), 12.0, 1.0);
        assert!(!audio.last_track.as_deref().is_some_and(|t| t.contains("stormwind_intro")), "intro repeated: {:?}", audio.last_track);
        // No area (or one without music): everything fades out within the fade time.
        run(&mut audio, None, 12.0, FADE_SECONDS + 0.5);
        assert!(audio.music.voices.is_empty() && audio.ambience.voices.is_empty());
        // Every effect plays on every surface without trouble.
        let wolf = crate::content::creature_index("wolf");
        for sfx in [Sfx::Block, Sfx::SwordHit, Sfx::Wound, Sfx::Death, Sfx::Jump, Sfx::ActionButton, Sfx::Footstep, Sfx::Land, Sfx::Swing, Sfx::Exertion, Sfx::Cry(Cry::Aggro, wolf), Sfx::Cry(Cry::Attack, wolf), Sfx::Cry(Cry::Bite, wolf), Sfx::Cry(Cry::Wound, wolf), Sfx::Cry(Cry::Death, wolf)] {
            (0..=10).for_each(|surface| audio.play(sfx, surface));
        }
    }

    #[test]
    fn day_runs_from_six_to_eighteen() {
        assert!(!is_night(6.0) && !is_night(12.0) && !is_night(17.9));
        assert!(is_night(18.0) && is_night(0.0) && is_night(5.9));
    }

    #[test]
    fn real_areas_resolve_to_their_music_ambience_and_intro() {
        let Some((_, bank)) = bank() else { return eprintln!("skipped: no game data") };
        // Elwynn Forest (12) and Northshire Valley (9, whose music is its parent's).
        for area in [12, 9] {
            let zone = bank.music_for(area).expect("Elwynn plays music");
            let day = bank.entry(bank.zone_music[&zone].sounds[0]).unwrap();
            assert_eq!(day.name, "Zone-Forest Day");
            assert!(bank.ambience_for(area).is_some());
        }
        // Stormwind City: its own music set and the Valley of Heroes intro (played at most hourly).
        let (id, entry, delay) = bank.intro_for(1519).expect("Stormwind has an intro");
        assert_eq!((id, delay), (61, 60));
        assert!(bank.entry(entry).unwrap().files[0].contains("stormwind_intro"));
        assert_ne!(bank.music_for(1519), bank.music_for(12));
        // The player: Human male footsteps on grass and stone are different sound entries, a swing and a grunt exist.
        let (grass, stone) = (bank.footstep(5).unwrap(), bank.footstep(2).unwrap());
        assert_eq!(bank.entry(grass).unwrap().name, "CharacterMediumLargeGrass");
        assert_eq!(bank.entry(stone).unwrap().name, "CharacterMediumLargeStone");
        assert_eq!(bank.entry(bank.swing().unwrap()).unwrap().name, "MediumWeaponNormal");
        assert_eq!(bank.entry(bank.exertion()).unwrap().name, "HumanMalePlayerAttack");
    }

    /// Every file the game can ask for exists and decodes: all zone music, intros, zone ambience, footsteps, swings.
    #[test]
    fn water_and_waterfalls_have_sounds_that_decode() {
        let Some((mount, bank)) = bank() else { return eprintln!("skipped: no game data") };
        // The streams and lakes: still, slow and fast water, the ocean, magma and slime.
        let wanted = [(0, 0), (0, 1), (0, 2), (1, 0), (2, 0), (3, 0)];
        let mut ids: Vec<u32> = wanted.iter().map(|&(kind, speed)| bank.water_sound(kind, speed).unwrap_or_else(|| panic!("no sound for liquid {kind} at {speed}"))).collect();
        // The waterfalls' own sound, from their model's $DSL event.
        let falls = M2::parse(&mount.read(r"World\Azeroth\Elwynn\PassiveDoodads\Waterfall\ElwynnTallWaterfall01.m2").unwrap()).unwrap();
        ids.extend(falls.events.iter().filter(|e| &e.id == b"$DSL").map(|e| e.data));
        assert!(ids.len() > wanted.len(), "the waterfall plays a sound all the time");
        // A foot in water over each kind of ground has a splash.
        ids.extend((0..=10).filter_map(|t| bank.splash_footstep(t)));
        assert!(bank.splash_footstep(5).is_some());
        for id in ids {
            let e = bank.entry(id).unwrap_or_else(|| panic!("no entry {id}"));
            let bytes = mount.read(&e.path(0)).unwrap_or_else(|_| panic!("no file {}", e.path(0)));
            assert!(Decoder::new(Cursor::new(repaired(bytes))).is_ok(), "{} does not decode", e.path(0));
        }
        for name in ["CharacterSplashSoundSmall", "CharacterSplashSoundMedium", "CharacterSplashSoundlarge", "Underwater (DONOTRENAME)"] {
            let e = bank.named(name).and_then(|id| bank.entry(id)).unwrap_or_else(|| panic!("no entry {name}"));
            let bytes = mount.read(&e.path(0)).unwrap_or_else(|_| panic!("no file {}", e.path(0)));
            assert!(Decoder::new(Cursor::new(repaired(bytes))).is_ok(), "{} does not decode", e.path(0));
        }
    }

    #[test]
    fn a_positioned_sound_follows_its_ears_and_keeps_its_length() {
        let ears = Arc::new([AtomicU32::new(1f32.to_bits()), AtomicU32::new(0f32.to_bits())]);
        let mono = SamplesBuffer::new(1, 100, vec![0.5f32; 100]);
        let out: Vec<f32> = Positioned::new(mono, ears.clone()).collect();
        // Mono becomes stereo: twice the samples, the left ear heard and the right silent.
        assert_eq!(out.len(), 200);
        assert!(out.chunks(2).all(|f| (f[0] - 0.5).abs() < 1e-6 && f[1] == 0.0));
        // The ears move to new volumes over a tenth of a second (10 frames at 100 Hz): the first frame is a tenth of the way.
        let ears = Arc::new([AtomicU32::new(1f32.to_bits()), AtomicU32::new(0f32.to_bits())]);
        let mut sound = Positioned::new(SamplesBuffer::new(2, 100, vec![1.0f32; 200]), ears.clone());
        ears[0].store(0f32.to_bits(), Ordering::Relaxed);
        ears[1].store(1f32.to_bits(), Ordering::Relaxed);
        let (left, right) = (sound.next().unwrap(), sound.next().unwrap());
        assert!((left - 0.9).abs() < 1e-4 && (right - 0.1).abs() < 1e-4, "{left} {right}");
    }

    #[test]
    fn every_weather_loop_decodes() {
        let Some((mount, _)) = bank() else { return eprintln!("skipped: no game data") };
        for kind in [crate::weather::Kind::Rain, crate::weather::Kind::Snow, crate::weather::Kind::Sand] {
            for strength in [0.3, 0.5, 0.9] {
                let mut weather = crate::weather::Weather::new(1);
                weather.set(Some(kind), strength, true);
                let (file, _) = weather.sound().unwrap();
                let bytes = mount.read(file).unwrap_or_else(|_| panic!("no {file}"));
                assert!(Decoder::new(Cursor::new(repaired(bytes))).is_ok(), "{file} does not decode");
            }
        }
    }

    #[test]
    fn every_sound_the_game_plays_decodes() {
        let Some((mount, bank)) = bank() else { return eprintln!("skipped: no game data") };
        let mut ids: Vec<u32> = bank.zone_music.values().flat_map(|z| z.sounds).collect();
        ids.extend(bank.ambience.values().flatten());
        ids.extend(bank.intros.values().map(|i| i.0));
        ids.extend((0..=10).filter_map(|t| bank.footstep(t)));
        ids.extend([bank.swing().unwrap(), bank.exertion()]);
        let wolf = crate::content::creature_index("wolf");
        ids.extend([Cry::Aggro, Cry::Attack, Cry::Bite, Cry::Wound, Cry::Death, Cry::Fidget].map(|c| bank.cry(wolf, c)));
        let effects = [Sfx::Block, Sfx::SwordHit, Sfx::Wound, Sfx::Death, Sfx::Jump, Sfx::ActionButton].map(|s| bank.effect(s));
        assert!(effects.iter().all(|&e| bank.entry(e).is_some()), "a combat or interface sound is missing: {effects:?}");
        ids.extend(effects);
        ids.extend((0..=10).filter_map(|t| bank.cry_footstep(wolf, t)));
        // Every sound the interface asks for by name, and the pick up and put down sound of every item.
        for name in ["igCharacterInfoOpen", "igCharacterInfoClose", "igBackPackOpen", "igBackPackClose", "LOOTWINDOWCOINSOUND", "LOOTWINDOWOPENEMPTY", "igQuestListOpen", "igQuestListClose", "QUESTADDED", "QUESTCOMPLETED", "igQuestCancel", "igQuestListSelect", "igQuestListComplete", "igQuestLogOpen", "igQuestLogClose", "igQuestLogAbandonQuest", "igMainMenuOptionCheckBoxOn", "LEVELUPSOUND", "TaxiNodeDiscovered", "ChestOpen", "Item - Lockpick", "igCreatureAggroSelect", "igCharacterNPCSelect", "INTERFACESOUND_LOSTTARGETUNIT", "igSpellBookOpen", "igSpellBookClose", "igSpellBokPageTurn", "igSpellBookSpellIconPickup", "HumanMale_CantLearnSpell", "SpellFizzleHoly", "igCharacterInfoTab"] {
            ids.push(bank.named(name).unwrap_or_else(|| panic!("no sound {name}")));
        }
        // The sounds of the pictures of every spell: when it is begun, cast, when it flies and where it hits.
        for v in crate::visuals::load(&mount).unwrap() {
            ids.extend([v.precast.sound, v.cast.sound, v.impact.sound, v.missile_sound].into_iter().flatten());
        }
        for item in crate::items::all().iter() {
            let (up, down) = (format!("PickUp{}", item.handling), format!("PutDown{}", put_down_name(item.handling)));
            ids.extend([bank.named(&up).unwrap_or_else(|| panic!("no sound {up}")), bank.named(&down).unwrap_or_else(|| panic!("no sound {down}"))]);
        }
        assert!(bank.cry_footstep(wolf, 5).is_some() && bank.cry_footstep(wolf, 0).is_some(), "wolves have footsteps on grass and dirt");
        assert!(bank.footstep_in(0, 5).is_none(), "footstep set 0 is silent, not the treant's stomp");
        ids.sort_unstable();
        ids.dedup();
        let (mut files, mut missing, mut bad) = (0, Vec::new(), Vec::new());
        for entry in ids.into_iter().filter(|&i| i != 0).filter_map(|i| bank.entry(i)) {
            for f in 0..entry.files.len() {
                let path = entry.path(f);
                files += 1;
                match mount.read(&path) {
                    Err(_) => missing.push(path),
                    Ok(bytes) => {
                        if Decoder::new(Cursor::new(repaired(bytes))).is_err() {
                            bad.push(path);
                        }
                    }
                }
            }
        }
        assert!(files > 200, "only {files} files checked");
        // The data has a few entries that point at files the archives do not hold; the ones played must be fine.
        assert!(bad.is_empty(), "undecodable: {bad:?}");
        assert!(missing.len() * 20 < files, "{} of {files} missing, e.g. {:?}", missing.len(), &missing[..missing.len().min(5)]);
    }
}
