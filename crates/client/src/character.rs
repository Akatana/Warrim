//! The player character: an animated, skinned M2 (Human male) with its default skin, played with the
//! original client's animations: the arms, the sword swing and the shield block all come from the
//! animation sequences, with closed fists around the weapon grip and the shield on the forearm.

use crate::anim::{self, Mix};
use crate::animator::{Animator, Motion, STAND};
use crate::items::Look;
use crate::models::{BatchCpu, ModelCache, ModelCpu, ModelVertex, RawTexture};
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mpq::Mount;
use std::sync::Arc;
use wowfmt::blp::Blp;
use wowfmt::dbc::Dbc;
use wowfmt::m2::{Blend, M2};

/// The races of the player's side and the other (the ones a character can be made as in 1.12).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum Race {
    #[default]
    Human,
    Orc,
    Dwarf,
    NightElf,
    Undead,
    Tauren,
    Gnome,
    Troll,
}

impl Race {
    pub const ALL: [Race; 8] = [Race::Human, Race::Orc, Race::Dwarf, Race::NightElf, Race::Undead, Race::Tauren, Race::Gnome, Race::Troll];

    /// Its row of `ChrRaces.dbc`, which the tables of character customization go by.
    fn id(self) -> u32 {
        Race::ALL.iter().position(|&r| r == self).map_or(1, |i| i as u32 + 1)
    }

    /// The folder of its models in `Character\`.
    fn folder(self) -> &'static str {
        match self {
            Race::Undead => "Scourge",
            Race::NightElf => "NightElf",
            Race::Human => "Human",
            Race::Orc => "Orc",
            Race::Dwarf => "Dwarf",
            Race::Tauren => "Tauren",
            Race::Gnome => "Gnome",
            Race::Troll => "Troll",
        }
    }

    /// The bit of the race in the masks of the original's item tables (1 human, 2 orc, 4 dwarf...).
    pub fn mask(self) -> u32 {
        1 << (self.id() - 1)
    }

    /// The name the original's texts and icons go by (`RACE_INFO_SCOURGE`).
    pub fn key(self) -> String {
        self.folder().to_ascii_uppercase()
    }

    /// Whether it is on the Horde's side.
    pub fn horde(self) -> bool {
        matches!(self, Race::Orc | Race::Undead | Race::Tauren | Race::Troll)
    }

    /// A race by its name, spaces and case ignored (`nightelf`, `Night Elf`).
    pub fn parse(text: &str) -> Option<Race> {
        let plain = |s: &str| s.replace(' ', "").to_ascii_lowercase();
        Race::ALL.iter().copied().find(|r| plain(r.name()) == plain(text) || plain(r.folder()) == plain(text))
    }

    pub fn name(self) -> &'static str {
        match self {
            Race::NightElf => "Night Elf",
            Race::Undead => "Undead",
            other => other.folder(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum Sex {
    #[default]
    Male,
    Female,
}

impl Sex {
    fn id(self) -> u32 {
        self as u32
    }

    pub fn name(self) -> &'static str {
        if self == Sex::Male { "Male" } else { "Female" }
    }
}

/// How a person looks: the race and sex, and indexes into the tables of character customization (`CharSections.dbc`,
/// `CharHairGeosets.dbc`, `CharacterFacialHairStyles.dbc`) for them. The default is the player's own look, the human male.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Appearance {
    pub race: Race,
    pub sex: Sex,
    /// Skin colour (0-9), face (0-11), hair style (0-11, 0 is bald), hair colour (0-9; the beard has it too), beard (0-8, 0 none).
    pub skin: u32,
    pub face: u32,
    pub hair_style: u32,
    pub hair_color: u32,
    pub beard: u32,
    /// A row of `CreatureDisplayInfoExtra.dbc` (0 for none) whose baked texture is the whole skin: the original paints the face and the clothes of most
    /// of its people into one picture, which replaces the skin, face, facial hair, underwear and the textures of worn items.
    pub baked: u32,
}

impl Default for Appearance {
    fn default() -> Appearance {
        Appearance { race: Race::Human, sex: Sex::Male, skin: 0, face: 0, hair_style: 4, hair_color: 0, beard: 0, baked: 0 }
    }
}

impl Appearance {
    /// The model of the race and sex.
    pub fn model(&self) -> String {
        let (race, sex) = (self.race.folder(), self.sex.name());
        format!(r"Character\{race}\{sex}\{race}{sex}.m2")
    }
}

/// How big a character is to the water: its height (`CreatureModelData.dbc`: the collision height) and the scale of the model, which is the
/// display's times the model's. The depth of water it swims in is a share of the height, its ripples go by both. (The hull it walks with stays
/// the client's defaults, which the walking was checked against.)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub height: f32,
    pub scale: f32,
}

impl Default for Size {
    /// A human man: what the movement constructor of the client starts every unit with.
    fn default() -> Size {
        Size { height: crate::movement::DEFAULT_HEIGHT, scale: 1.0 }
    }
}

impl Size {
    pub fn of(mount: &Mount, look: &Appearance) -> Result<Size> {
        let dbc = |name: &str| -> Result<Dbc> { Ok(Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc"))?)?) };
        let (models, displays) = (dbc("CreatureModelData")?, dbc("CreatureDisplayInfo")?);
        let file = look.model().replace(".m2", ".mdx");
        let data = models.iter().find(|r| r.string(2).eq_ignore_ascii_case(&file)).context("the race has no model data")?;
        let display = displays.iter().find(|r| r.u32(1) == data.u32(0)).map_or(1.0, |r| r.f32(4));
        let scale = display.max(0.01) * data.f32(4).max(0.01);
        Ok(Size { height: data.f32(15) * scale, scale })
    }
}

/// The highest value each part of an appearance can have for a race and sex (0 is the lowest, and for the hair style and the beard none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ranges {
    pub skin: u32,
    pub face: u32,
    pub hair_style: u32,
    pub hair_color: u32,
    pub beard: u32,
}

/// What the tables of character customization have for a race and sex: the faces that exist for this skin colour, and the hair colours that
/// exist for this hair style.
pub fn ranges(mount: &Mount, race: Race, sex: Sex, skin: u32, hair_style: u32) -> Result<Ranges> {
    let dbc = |name: &str| -> Result<Dbc> { Ok(Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc"))?)?) };
    let sections = dbc("CharSections")?;
    let rows = || sections.iter().filter(|r| r.u32(1) == race.id() && r.u32(2) == sex.id());
    // The highest `field` of the rows of a kind (0 skin, 1 face, 3 hair) for which `keep(variation, colour)` holds.
    let highest = |kind: u32, field: usize, keep: &dyn Fn(u32, u32) -> bool| rows().filter(|r| r.u32(3) == kind && keep(r.u32(4), r.u32(5))).map(|r| r.u32(field)).max().unwrap_or(0);
    let beard = dbc("CharacterFacialHairStyles")?.iter().filter(|r| r.u32(0) == race.id() && r.u32(1) == sex.id()).map(|r| r.u32(2)).max().unwrap_or(0);
    Ok(Ranges {
        skin: highest(0, 5, &|_, _| true),
        face: highest(1, 4, &|_, colour| colour == skin),
        hair_style: highest(3, 4, &|_, _| true),
        hair_color: highest(3, 5, &|style, _| hair_style == 0 || style == hair_style),
        beard,
    })
}

/// The textures an appearance is made of, and the geosets (mesh parts) it shows.
#[derive(Clone, Debug)]
struct Parts {
    skin: String,
    /// Lower and upper face, then the lower and upper facial hair painted over them.
    face: [Option<String>; 2],
    /// The second texture of the skin, which the models of some races paint another part of the body with.
    skin_extra: Option<String>,
    facial: [Option<String>; 2],
    underwear: Option<String>,
    hair: Option<String>,
    /// The hair style's mesh part (0 for none), and the facial hair's three mesh parts (100s, 200s, 300s).
    hair_geoset: u16,
    facial_geosets: [u16; 3],
    /// The skin is a baked picture of the person (its face and clothes are in it).
    baked: bool,
}

impl Parts {
    /// Look the textures and geosets of an appearance up in the original's tables.
    fn of(mount: &Mount, a: &Appearance) -> Result<Parts> {
        let dbc = |name: &str| -> Result<Dbc> { Ok(Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc"))?)?) };
        // CharSections: 1 race, 2 sex, 3 kind (0 skin, 1 face, 2 facial hair, 3 hair, 4 underwear), 4 variation, 5 colour,
        // 6-8 textures.
        let sections = dbc("CharSections")?;
        let (race, sex) = (a.race.id(), a.sex.id());
        let section = |kind: u32, variation: u32, color: u32| {
            sections.iter().find(|r| r.u32(1) == race && r.u32(2) == sex && r.u32(3) == kind && r.u32(4) == variation && r.u32(5) == color)
        };
        let text = |s: &str| (!s.is_empty()).then(|| s.to_string());
        let skin_row = section(0, 0, a.skin).context("no such skin colour")?;
        let mut skin = Some(skin_row.string(6).to_string()).filter(|s| !s.is_empty()).context("no such skin colour")?;
        // CreatureDisplayInfoExtra: 18 the name of the baked picture (in Textures\BakedNpcTextures).
        let baked = (a.baked != 0).then(|| dbc("CreatureDisplayInfoExtra")).transpose()?.and_then(|t| Some(t.find_by_id(a.baked)?.string(18).to_string())).filter(|n| !n.is_empty());
        if let Some(name) = &baked {
            skin = format!(r"Textures\BakedNpcTextures\{name}");
        }
        // A face that the tables do not have for this skin is left out (the editor offers only those that exist).
        let face = section(1, a.face, a.skin);
        let facial = (a.beard > 0).then(|| section(2, a.beard, a.hair_color)).flatten();
        let underwear = section(4, 0, a.skin).and_then(|r| text(r.string(6)));
        let hair = (a.hair_style > 0).then(|| section(3, a.hair_style, a.hair_color)).flatten().and_then(|r| text(r.string(6)));
        // CharHairGeosets: 1 race, 2 sex, 3 style, 4 mesh part. CharacterFacialHairStyles: 0 race, 1 sex, 2 style, 6-8 the
        // parts of the three groups (the jaw, then the moustache, then the sideburns).
        let hair_geoset = dbc("CharHairGeosets")?.iter().find(|r| r.u32(1) == race && r.u32(2) == sex && r.u32(3) == a.hair_style).map_or(0, |r| r.u32(4) as u16);
        let styles = dbc("CharacterFacialHairStyles")?;
        let beard = styles.iter().find(|r| r.u32(0) == race && r.u32(1) == sex && r.u32(2) == a.beard);
        let facial_geosets = beard.map_or([1, 1, 1], |r| [r.u32(6) as u16, r.u32(7) as u16, r.u32(8) as u16]);
        Ok(Parts {
            skin,
            face: [face.and_then(|r| text(r.string(6))), face.and_then(|r| text(r.string(7)))],
            skin_extra: text(skin_row.string(7)),
            facial: [facial.and_then(|r| text(r.string(6))), facial.and_then(|r| text(r.string(7)))],
            underwear,
            hair,
            hair_geoset: if a.hair_style == 0 { 0 } else { hair_geoset },
            facial_geosets: [100 + facial_geosets[0], 300 + facial_geosets[1], 200 + facial_geosets[2]],
            baked: baked.is_some(),
        })
    }
}

/// Animation id of the closed-fist pose that the hands hold the weapon grip in.
const HANDS_CLOSED: u16 = 15;

/// Attachment ids: right hand (the sword grip), left hand.
const RIGHT_HAND: u32 = 1;
const LEFT_HAND: u32 = 2;
const HEAD: u32 = 11;
/// The first-person eye in the bind pose (model space): just in front of the nose, which reaches x = 0.18.
const EYE: Vec3 = Vec3::new(0.26, 0.0, 1.90);

/// An arm's bones that matter here: the shoulder (to find the chest) and everything below the wrist
/// (fingers, thumb, grip), which hold the closed-fist pose.
struct Arm {
    shoulder: usize,
    hand_bones: Vec<usize>,
}

pub struct Character {
    pub m2: M2,
    /// Bind-pose model with the geosets and textures shown; its vertices are re-skinned every frame.
    pub model: ModelCpu,
    pub vertices: Vec<ModelVertex>,
    pub bones: Vec<Mat4>,
    animator: Animator,
    clock_ms: f32,
    /// Bones that play the upper-body animation (the chest and everything on it).
    upper_body: Vec<bool>,
    right: Arm,
    left: Arm,
    /// The head bone (the helmet attachment hangs off one of its children), which the eye is fixed to.
    head: usize,
    /// Names its model and its skin textures (they are cached by name, so every character needs its own).
    key: String,
    /// What the person looks like (see `Appearance`), for painting the skin again.
    parts: Parts,
    /// The textures of the hair and of the second skin of some races, and what is worn now.
    hair: Option<(String, Arc<RawTexture>)>,
    extra: Option<(String, Arc<RawTexture>)>,
    gear: Gear,
    sex: Sex,
    /// How often the skin was redone, for the name of its texture.
    skin_generation: u32,
    /// The closed-fist pose: (sequence, time).
    closed: Option<(usize, f32)>,
}

fn find_seq(m2: &M2, id: u16) -> Option<usize> {
    m2.sequences.iter().position(|s| s.id == id && s.variation == 0).or_else(|| m2.sequences.iter().position(|s| s.id == id))
}

/// What the character wears, as far as the body shows it: the textures painted on the skin, the mesh parts (geosets) that are put on or taken
/// off (sleeves, a robe's skirt, a glove, a boot, a cape), the cape's texture, and what a helmet hides of the hair and face.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gear {
    pub looks: Vec<Look>,
    pub show: Vec<u16>,
    pub hide: Vec<u16>,
    /// The file of the cape's texture.
    pub cape: Option<String>,
    /// From a row of `HelmetGeosetVisData.dbc`: flags (a bit for each mesh part number) of what is hidden of the hair, the jaw, the sideburns,
    /// the moustache and the ears.
    pub helm: Option<[u32; 5]>,
}

impl Gear {
    fn hidden_by_helm(&self, group: usize, part: u16) -> bool {
        self.helm.is_some_and(|flags| part < 32 && flags[group] >> part & 1 == 1)
    }
}

/// Geosets shown: the body, the hair style, the facial hair, the ears (`ears`: 701, or 702 for a model that has only that), the default variant (01) of hands, feet, legs and
/// the back piece that closes the neck and upper back when no cape is worn; and what the gear puts on and takes off.
fn geoset_shown(id: u16, parts: &Parts, gear: &Gear, ears: u16) -> bool {
    if gear.hide.contains(&id) {
        return false;
    }
    if gear.show.contains(&id) {
        return true;
    }
    match id {
        0 => true,
        1..=99 => id == parts.hair_geoset && !gear.hidden_by_helm(0, id),
        100..=199 => parts.facial_geosets.contains(&id) && !gear.hidden_by_helm(1, id - 100),
        200..=299 => parts.facial_geosets.contains(&id) && !gear.hidden_by_helm(2, id - 200),
        300..=399 => parts.facial_geosets.contains(&id) && !gear.hidden_by_helm(3, id - 300),
        701..=799 => id == ears && !gear.hidden_by_helm(4, id - 700),
        _ => id % 100 == 1 && matches!(id / 100, 4 | 5 | 13 | 15),
    }
}

/// Where the item textures go on the 256 x 256 skin (the original's character texture regions): the folder of the
/// item texture and the position of its top left corner.
const ITEM_REGIONS: [(&str, usize, usize); 8] = [
    ("ArmUpperTexture", 0, 0),
    ("ArmLowerTexture", 0, 64),
    ("HandTexture", 0, 128),
    ("TorsoUpperTexture", 128, 0),
    ("TorsoLowerTexture", 128, 64),
    ("LegUpperTexture", 128, 96),
    ("LegLowerTexture", 128, 160),
    ("FootTexture", 128, 224),
];

/// Body skin with the face and underwear textures blitted onto it at the client's atlas positions, and the
/// textures of what is worn over them.
fn composite_skin(mount: &Mount, parts: &Parts, sex: Sex, looks: &[Look]) -> Result<RawTexture> {
    let decode = |path: &str| -> Result<Blp> { Ok(Blp::parse(&mount.read(path)?)?) };
    let base = decode(&parts.skin)?;
    let (w, h) = (base.width as usize, base.height as usize);
    let mut pixels = base.mips[0].clone();
    let scale = w as f32 / 256.0;
    let mut blit = |layer: &Blp, x: usize, y: usize| {
        let (ox, oy) = ((x as f32 * scale) as usize, (y as f32 * scale) as usize);
        for ly in 0..layer.height as usize {
            for lx in 0..layer.width as usize {
                let (dx, dy) = (ox + lx, oy + ly);
                if dx >= w || dy >= h {
                    continue;
                }
                let s = &layer.mips[0][(ly * layer.width as usize + lx) * 4..][..4];
                let d = &mut pixels[(dy * w + dx) * 4..][..4];
                let a = s[3] as u32;
                for c in 0..3 {
                    d[c] = ((s[c] as u32 * a + d[c] as u32 * (255 - a)) / 255) as u8;
                }
            }
        }
    };
    // A baked skin has all of this in it already.
    let layers = if parts.baked { vec![] } else { vec![(&parts.face[0], 0, 192), (&parts.face[1], 0, 160), (&parts.facial[0], 0, 192), (&parts.facial[1], 0, 160), (&parts.underwear, 128, 96)] };
    for (path, x, y) in layers {
        if let Some(layer) = path.as_ref().and_then(|p| decode(p).ok()) {
            blit(&layer, x, y);
        }
    }
    for look in looks.iter().filter(|_| !parts.baked) {
        for (name, (dir, x, y)) in look.parts.iter().zip(ITEM_REGIONS) {
            let Some(name) = name else { continue };
            // The textures for the sex, then unisex ones: `Leather_A_01Dark_Chest_TU_M.blp`, `..._U.blp`.
            let layer = [if sex == Sex::Male { "M" } else { "F" }, "U"].iter().find_map(|g| decode(&format!(r"Item\TextureComponents\{dir}\{name}_{g}.blp")).ok());
            match layer {
                Some(layer) => blit(&layer, x, y),
                None => log::warn!("item texture {dir}\\{name} not found"),
            }
        }
    }
    Ok(RawTexture { width: base.width, height: base.height, mips: vec![pixels] })
}

fn attachment_bone(m2: &M2, id: u32) -> Option<usize> {
    m2.attachments.get(*m2.attachment_lookup.get(id as usize)? as usize).map(|a| a.bone as usize)
}

fn parent(m2: &M2, b: usize) -> usize {
    m2.bones[b].parent.max(0) as usize
}

/// Whether `b` is `ancestor` or one of its descendants.
fn is_below(m2: &M2, mut b: usize, ancestor: usize) -> bool {
    for _ in 0..m2.bones.len() {
        if b == ancestor {
            return true;
        }
        match m2.bones[b].parent {
            p if p >= 0 => b = p as usize,
            _ => return false,
        }
    }
    false
}

fn depth(m2: &M2, mut b: usize) -> usize {
    let mut d = 0;
    while m2.bones[b].parent >= 0 && d < m2.bones.len() {
        b = m2.bones[b].parent as usize;
        d += 1;
    }
    d
}

/// The arm whose grip (hand attachment) hangs off `grip_bone`: grip -> wrist -> elbow -> shoulder.
fn arm_of(m2: &M2, grip_bone: usize) -> Arm {
    let wrist = parent(m2, grip_bone);
    let shoulder = parent(m2, parent(m2, wrist));
    let hand_bones = (0..m2.bones.len()).filter(|&b| b != wrist && is_below(m2, b, wrist)).collect();
    Arm { shoulder, hand_bones }
}

impl Character {
    pub fn load(mount: &Mount, cache: &ModelCache, key: &str, look: &Appearance) -> Result<Character> {
        let m2 = M2::parse(&mount.read(&look.model())?).context("character model")?;
        let parts = Parts::of(mount, look)?;
        let hair = parts.hair.as_ref().and_then(|path| cache.texture(mount, path));
        let extra = parts.skin_extra.as_ref().and_then(|path| cache.texture(mount, path));
        let vertices: Vec<ModelVertex> = m2
            .vertices
            .iter()
            .map(|v| ModelVertex { pos: [v.pos[0], v.pos[2], -v.pos[1]], normal: [v.normal[0], v.normal[2], -v.normal[1]], uv: v.uv, color: [255, 255, 255, 0] })
            .collect();
        let model = ModelCpu {
            key: key.into(),
            vertices: vertices.clone(),
            indices: m2.indices.clone(),
            batches: Vec::new(),
            col_vertices: Vec::new(),
            col_indices: Vec::new(),
            tracks: None,
        };

        let seq = find_seq(&m2, STAND).context("no stand animation")?;
        let bones = anim::bone_matrices(&m2, seq, m2.sequences[seq].start as f32, 0.0);
        // Seeded by the clock so the idle fidgets and sword swings differ from run to run.
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos());
        let animator = Animator::new(&m2, seed);
        let right = arm_of(&m2, attachment_bone(&m2, RIGHT_HAND).context("no right hand")?);
        let left = arm_of(&m2, attachment_bone(&m2, LEFT_HAND).context("no left hand")?);
        // The chest: the deepest bone above both shoulder girdles.
        let torso = (0..m2.bones.len())
            .filter(|&b| is_below(&m2, right.shoulder, b) && is_below(&m2, left.shoulder, b) && b != right.shoulder && b != left.shoulder)
            .max_by_key(|&b| depth(&m2, b))
            .context("no torso bone")?;
        let closed = find_seq(&m2, HANDS_CLOSED).map(|i| (i, m2.sequences[i].start as f32));

        let head = parent(&m2, attachment_bone(&m2, HEAD).context("no head")?);
        let upper_body = (0..m2.bones.len()).map(|b| is_below(&m2, b, torso)).collect();
        let mut character = Character { m2, model, vertices, bones, animator, clock_ms: 0.0, upper_body, right, left, head, key: key.into(), parts, hair, extra, gear: Gear::default(), sex: look.sex, skin_generation: 0, closed };
        character.dress(mount, cache, &Gear::default())?;
        Ok(character)
    }

    /// Dress the body: paint the skin with the textures of what is worn and choose the mesh parts. The model's batches are made again, so it
    /// must be uploaded again (a new entity) to show.
    pub fn dress(&mut self, mount: &Mount, cache: &ModelCache, gear: &Gear) -> Result<()> {
        let skin = Arc::new(composite_skin(mount, &self.parts, self.sex, &gear.looks)?);
        self.skin_generation += 1;
        let skin = Some((format!("{}/skin/{}", self.key, self.skin_generation), skin));
        let cape = gear.cape.as_ref().and_then(|path| cache.texture(mount, path));
        let mut batches: Vec<BatchCpu> = Vec::new();
        // The ears of the model: the first variant, but the night elves and the trolls have only the second (their long ears).
        let ears = if self.m2.batches.iter().any(|b| b.submesh_id == 701) { 701 } else { 702 };
        for b in self.m2.batches.iter().filter(|b| geoset_shown(b.submesh_id, &self.parts, gear, ears)) {
            let texture = match b.texture.map(|i| self.m2.textures[i].kind) {
                Some(1) => skin.clone(),
                Some(6) => self.hair.clone(),
                Some(8) => self.extra.clone(),
                // The cape's parts: shown only with a cape, in its texture.
                Some(2) if cape.is_some() => cape.clone(),
                _ => continue,
            };
            batches.push(BatchCpu {
                index_start: b.index_start as u32,
                index_count: b.index_count as u32,
                texture,
                wrap: false,
                blend: b.blend,
                unlit: b.unlit,
                refs: Default::default(),
                particle: false,
            });
        }
        batches.sort_by_key(|b| b.blend != Blend::Opaque && b.blend != Blend::AlphaKey);
        self.model.batches = batches;
        self.gear = gear.clone();
        Ok(())
    }

    /// Start a sword swing lasting `seconds`.
    pub fn attack(&mut self, seconds: f32) {
        self.animator.start_attack(&self.m2, seconds);
    }

    /// Play animation `id` once on the upper body, over `seconds` (the cast of a spell).
    pub fn play_once(&mut self, id: u16, seconds: f32) {
        self.animator.start_upper(&self.m2, id, seconds);
    }

    /// The sheathing animation, over `seconds`.
    pub fn sheath(&mut self, seconds: f32) {
        self.animator.start_sheath(&self.m2, seconds);
    }

    /// Draw a bow over `seconds` with animation `id`, and let the string go.
    pub fn draw(&mut self, id: u16, seconds: f32) {
        self.animator.start_draw(&self.m2, id, seconds);
    }

    pub fn loose(&mut self) {
        self.animator.loose();
    }

    /// Hold animation `id` on the upper body (a spell being cast), or let go of it.
    pub fn set_precast(&mut self, id: Option<u16>) {
        self.animator.precast = id;
    }

    /// Raise the shield (`true`) and hold it up, or lower it.
    pub fn set_block(&mut self, on: bool) {
        self.animator.set_block(on);
    }

    /// Advance `dt` seconds of the animation `motion` calls for (`vary`: idle fidgets allowed).
    pub fn update(&mut self, dt: f32, motion: &Motion, vary: bool) {
        self.animator.update(&self.m2, dt, motion, vary);
        self.clock_ms += dt * 1000.0;
        let (body, upper) = self.animator.mixes(&self.m2);
        let mut poses: Vec<Option<Mix>> = self.upper_body.iter().map(|&u| u.then_some(upper)).collect();
        if let Some((seq, time)) = self.closed {
            for &b in self.right.hand_bones.iter().chain(&self.left.hand_bones) {
                poses[b] = Some(Mix::one(seq, time));
            }
        }
        self.bones = anim::bone_matrices_with(&self.m2, body, self.clock_ms, &[], &poses);
        anim::skin(&self.m2, &self.bones, &self.model.vertices, &mut self.vertices);
    }

    /// Pose the character `ms` milliseconds into its death animation (it stays lying at the end of it).
    pub fn lie(&mut self, ms: f32) {
        const DEATH: u16 = 1;
        let Some(seq) = self.m2.sequences.iter().position(|s| s.id == DEATH) else { return self.update(0.0, &Motion::default(), false) };
        let s = &self.m2.sequences[seq];
        self.bones = anim::bone_matrices(&self.m2, seq, s.start as f32 + ms.min((s.end - s.start) as f32 - 1.0).max(0.0), self.clock_ms);
        anim::skin(&self.m2, &self.bones, &self.model.vertices, &mut self.vertices);
    }

    /// Where the first-person camera is, in model space: fixed to the head, so it follows the head through
    /// every animation (a jump's crouch and lean) and the face never moves into it.
    pub fn eye(&self) -> Vec3 {
        self.bones[self.head].transform_point3(EYE)
    }

    /// Whether a bow or a gun is being drawn or loosed.
    pub fn aiming(&self) -> bool {
        self.animator.aiming()
    }

    /// How far the head is turned to the left of the way the body faces, in radians (the archer's stance turns it a long way).
    pub fn head_yaw(&self) -> f32 {
        let forward = self.bones[self.head].transform_vector3(Vec3::X);
        forward.y.atan2(forward.x)
    }

    /// Model-space matrix of an attachment point (1 right hand, 2 left hand, 0 shield, 11 head).
    pub fn attachment(&self, id: u32) -> Option<Mat4> {
        let a = self.m2.attachments.get(*self.m2.attachment_lookup.get(id as usize)? as usize)?;
        Some(*self.bones.get(a.bone as usize)? * Mat4::from_translation(Vec3::from(a.position)))
    }
}

/// WMO / M2 model space (x east, y north, z up) to render space (x east, y up, z south).
pub const MODEL_TO_RENDER: Mat4 = crate::wmovis::MODEL_TO_RENDER;

/// An attachment matrix in model space expressed in render space.
pub fn to_render(m: Mat4) -> Mat4 {
    MODEL_TO_RENDER * m * MODEL_TO_RENDER.transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every look the content gives a person (and the extremes of what can be asked for) exists in the tables.
    #[test]
    fn the_appearances_of_the_content_and_the_extremes_exist() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let d = Appearance::default();
        let mut looks = vec![d, Appearance { skin: 9, face: 11, hair_style: 11, hair_color: 9, beard: 8, ..d }, Appearance { skin: 5, face: 6, hair_style: 0, hair_color: 5, beard: 4, ..d }];
        looks.extend(crate::content::get().creatures.iter().filter_map(|c| match &c.body {
            crate::content::Body::Humanoid { look, .. } => Some(*look),
            _ => None,
        }));
        for look in looks {
            let parts = Parts::of(&mount, &look).unwrap_or_else(|e| panic!("{look:?}: {e:#}"));
            assert!(parts.face.iter().all(Option::is_some), "{look:?} has a face");
            // A hair style shows a mesh part and a texture, bald shows neither; a beard paints facial hair over the face.
            assert_eq!((parts.hair_geoset > 0, parts.hair.is_some()), (look.hair_style > 0, look.hair_style > 0), "{look:?}");
            assert_eq!(parts.facial[0].is_some(), look.beard > 0, "{look:?}");
        }
    }

    /// Every race and sex the editor offers has a model, and every look up to what the tables have for it can be made.
    #[test]
    fn every_race_and_sex_can_be_made_with_the_highest_values_of_its_tables() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let cache = ModelCache::default();
        for race in Race::ALL {
            for sex in [Sex::Male, Sex::Female] {
                let max = ranges(&mount, race, sex, 0, 0).unwrap();
                let top = ranges(&mount, race, sex, max.skin, max.hair_style).unwrap();
                assert!(max.skin > 0, "{race:?} {sex:?}: {max:?} {top:?}");
                for look in [
                    Appearance { race, sex, skin: 0, face: 0, hair_style: 0, hair_color: 0, beard: 0, baked: 0 },
                    Appearance { race, sex, skin: max.skin, face: top.face, hair_style: top.hair_style, hair_color: top.hair_color, beard: top.beard, baked: 0 },
                ] {
                    let c = Character::load(&mount, &cache, "race-test", &look).unwrap_or_else(|e| panic!("{look:?}: {e:#}"));
                    let height = c.model.vertices.iter().map(|v| v.pos[1]).fold(0.0, f32::max);
                    eprintln!("{race:?} {sex:?}: {height:.2} yd, {max:?}");
                    assert!((0.8..4.0).contains(&height));
                }
            }
        }
    }

    fn load() -> Option<Character> {
        let mount = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok())?;
        Some(Character::load(&mount, &ModelCache::default(), "player", &Appearance::default()).expect("character loads"))
    }

    const STILL: Motion = Motion { speed: 0.0, forward_speed: 0.0, vertical_speed: 0.0, yaw_rate: 0.0, airborne: false, mounted: false, sneaking: false, swimming: false, side_speed: 0.0 };

    #[test]
    fn real_character_animates_and_has_a_complete_body() {
        let Some(mut c) = load() else { return eprintln!("skipped: no game data") };
        assert!(c.model.batches.iter().all(|b| b.texture.is_some()));
        // The default pelvis / legs (group 13) and the back piece that closes the neck and upper back (group 15).
        for part in [1301, 1501] {
            let b = c.m2.batches.iter().find(|b| b.submesh_id == part).unwrap_or_else(|| panic!("model has part {part}"));
            assert!(c.model.batches.iter().any(|m| m.index_start as usize == b.index_start), "part {part} is not drawn");
        }
        // Hands for the sword and shield.
        assert!(c.attachment(1).is_some() && c.attachment(0).is_some());
        c.update(0.2, &STILL, false);
        let stand = c.vertices.clone();
        c.update(0.4, &Motion { speed: 7.0, forward_speed: 7.0, ..STILL }, false);
        let moved = stand.iter().zip(&c.vertices).filter(|(a, b)| (Vec3::from(a.pos) - Vec3::from(b.pos)).length() > 0.05).count();
        assert!(moved > 100, "running moved only {moved} vertices");
        // Skinned vertices stay within a human-sized box (x east, y up, z south): nothing explodes.
        for v in &c.vertices {
            assert!(v.pos.iter().all(|p| p.is_finite() && p.abs() < 4.0), "{:?}", v.pos);
        }
    }

    #[test]
    fn a_human_man_is_as_big_as_the_clients_defaults_and_the_races_differ() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let human = Size::of(&mount, &Appearance::default()).expect("human size");
        let d = Size::default();
        assert!((human.height - d.height).abs() < 0.01 && human.scale == 1.0, "{human:?}");
        let tauren = Size::of(&mount, &Appearance { race: Race::Tauren, ..Appearance::default() }).expect("tauren size");
        assert!(tauren.height > human.height && tauren.scale > 1.2, "{tauren:?}");
    }

    #[test]
    fn the_eye_stays_fixed_to_the_head() {
        let Some(mut c) = load() else { return eprintln!("skipped: no game data") };
        c.update(0.0, &STILL, false);
        let pivot = |c: &Character| c.bones[c.head].transform_point3(Vec3::from(c.m2.bones[c.head].pivot));
        let (eye, offset) = (c.eye(), c.eye().distance(pivot(&c)));
        // The stand pose leans the head a little from the bind pose.
        assert!(eye.distance(EYE) < 0.1, "{eye:?}");
        // Through a jump the eye moves with the head, at the same distance from it.
        let air = Motion { airborne: true, vertical_speed: 3.0, ..STILL };
        let mut lowest: f32 = eye.z;
        for _ in 0..60 {
            c.update(1.0 / 60.0, &air, false);
            lowest = lowest.min(c.eye().z);
            assert!((c.eye().distance(pivot(&c)) - offset).abs() < 1e-3);
        }
        assert!(c.eye().distance(eye) > 0.02 || lowest < eye.z - 0.02, "the head never moved");
    }

    #[test]
    fn the_fists_close_and_the_items_sit_in_the_hands() {
        let Some(mut c) = load() else { return eprintln!("skipped: no game data") };
        c.update(0.0, &STILL, false);
        // The sword hangs from the right hand and the shield from the left forearm, both beside the body.
        let (grip, shield) = (c.attachment(1).unwrap().transform_point3(Vec3::ZERO), c.attachment(0).unwrap().transform_point3(Vec3::ZERO));
        assert!(grip.y < -0.1 && shield.y > 0.1, "grip {grip:?} shield {shield:?}");
        // A finger bone is posed by the closed-hand animation, not the stand one.
        let finger = c.right.hand_bones[0];
        let stand = find_seq(&c.m2, STAND).unwrap();
        let open = anim::bone_matrices(&c.m2, stand, c.m2.sequences[stand].start as f32, 0.0);
        let local = |m: &[Mat4]| m[parent(&c.m2, finger)].inverse() * m[finger];
        assert!(!local(&c.bones).abs_diff_eq(local(&open), 1e-3), "finger bone {finger} is not curled");
    }

    #[test]
    fn blocking_raises_the_shield_and_holds_it_until_released() {
        let Some(mut c) = load() else { return eprintln!("skipped: no game data") };
        let shield = |c: &Character| c.attachment(0).unwrap().transform_point3(Vec3::ZERO);
        c.update(0.5, &STILL, false);
        let low = shield(&c);
        c.set_block(true);
        for _ in 0..30 {
            c.update(1.0 / 60.0, &STILL, false);
        }
        let up = shield(&c);
        assert!(up.z > low.z + 0.3 && up.x > low.x + 0.2, "shield {low:?} -> {up:?}");
        // Held for seconds: it does not move.
        for _ in 0..180 {
            c.update(1.0 / 60.0, &STILL, false);
        }
        assert!(shield(&c).distance(up) < 0.02);
        // Released: it comes back down.
        c.set_block(false);
        for _ in 0..90 {
            c.update(1.0 / 60.0, &STILL, false);
        }
        assert!(shield(&c).distance(low) < 0.1, "{:?}", shield(&c));
    }

    #[test]
    fn drawing_a_bow_turns_the_head_from_the_body_and_the_game_turns_the_body_back_by_it() {
        let Some(mut c) = load() else { return eprintln!("skipped: no game data") };
        c.update(0.0, &STILL, false);
        assert!(!c.aiming() && c.head_yaw().abs() < 0.05);
        c.draw(crate::animator::ATTACK_BOW, 2.0);
        for _ in 0..60 {
            c.update(1.0 / 60.0, &STILL, false);
        }
        assert!(c.aiming() && c.head_yaw().abs() > 0.5, "{}", c.head_yaw());
    }
}
