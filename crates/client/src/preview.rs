//! The picture beside a creature's form in the editor: its model with the skins it has now, or the dressed person with his
//! look, drawn into an offscreen picture, and the lists to choose from — every creature model of the game and the skins the
//! game has for it (`CreatureModelData.dbc`, `CreatureDisplayInfo.dbc`).

use crate::anim;
use crate::camera::Camera;
use crate::content::RawBody;
use crate::creature::Person;
use crate::items::{self, Wear};
use crate::models::{ModelCache, ModelCpu, ModelVertex};
use crate::renderer::{Gpu, Renderer};
use crate::world::World;
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mpq::Mount;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use wowfmt::dbc::Dbc;
use crate::character::{self, Appearance, Race, Ranges, Sex};
use wowfmt::m2::M2;

/// The picture's size in pixels.
pub const SIZE: (u32, u32) = (240, 288);

/// The texture slot of the first skin of a creature model; the second and third follow.
const FIRST_SKIN: u32 = 11;

/// A set of skins of a model: (texture slot, file).
pub type Skins = Vec<(u32, String)>;

/// Every creature model of the game and the sets of skins the game has for each.
pub struct Catalogue {
    pub models: Vec<String>,
    /// By the lower-case file of the model.
    skins: HashMap<String, Vec<Skins>>,
}

impl Catalogue {
    fn load(mount: &Mount) -> Result<Catalogue> {
        let dbc = |name: &str| -> Result<Dbc> { Ok(Dbc::parse(&mount.read(&format!(r"DBFilesClient\{name}.dbc"))?)?) };
        let (data, display) = (dbc("CreatureModelData")?, dbc("CreatureDisplayInfo")?);
        // The file of every model id, as the archives have it (`.m2`), when it is there.
        let files: HashMap<u32, String> = data
            .iter()
            .filter_map(|r| {
                let file = r.string(2).rsplit_once('.').map(|(stem, _)| format!("{stem}.m2"))?;
                mount.exists(&file).then_some((r.u32(0), file))
            })
            .collect();
        let mut skins: HashMap<String, Vec<Skins>> = HashMap::new();
        let mut seen = BTreeSet::new();
        for r in display.iter() {
            let Some(model) = files.get(&r.u32(1)) else { continue };
            let folder = model.rsplit_once('\\').map_or("", |(f, _)| f);
            let set: Skins = (0..3).filter_map(|i| Some(r.string(6 + i)).filter(|n| !n.is_empty()).map(|n| (FIRST_SKIN + i as u32, format!(r"{folder}\{n}.blp")))).collect();
            if !set.is_empty() && set.iter().all(|(_, f)| mount.exists(f)) && seen.insert((model.to_ascii_lowercase(), format!("{set:?}").to_ascii_lowercase())) {
                skins.entry(model.to_ascii_lowercase()).or_default().push(set);
            }
        }
        let mut models: Vec<String> = files.into_values().collect::<BTreeSet<_>>().into_iter().collect();
        models.sort_by_key(|m| m.to_ascii_lowercase());
        models.dedup_by_key(|m| m.to_ascii_lowercase());
        Ok(Catalogue { models, skins })
    }

    /// The sets of skins the game has for a model.
    pub fn skins_of(&self, model: &str) -> &[Skins] {
        self.skins.get(&model.to_ascii_lowercase()).map_or(&[], |v| v)
    }
}

/// Whether two sets of skins are the same files.
pub fn same_skins(a: &[(u32, String)], b: &[(u32, String)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.0 == y.0 && x.1.eq_ignore_ascii_case(&y.1))
}

/// What the picture shows, made once for each model with its skins and each person with his look.
enum Shown {
    Beast { entity: usize, m2: M2, template: Arc<ModelCpu>, seq: Option<usize>, low: Vec3, high: Vec3 },
    Person(Person),
}

/// What the picture is of: a creature's body, or the character wearing or holding the item with this row of `ItemDisplayInfo.dbc`.
pub enum Subject {
    Body(RawBody),
    Display(u32, Wear),
}

pub struct Preview {
    mount: Arc<Mount>,
    cache: ModelCache,
    /// The offscreen picture and the interface texture that shows it.
    target: (usize, usize),
    pub catalogue: Option<Result<Catalogue, String>>,
    shown: HashMap<String, Result<Shown, String>>,
    /// What the form asks to be drawn this frame, and why it cannot be.
    want: Option<Subject>,
    pub error: Option<String>,
    /// What the picture shows of an item's look: its icon and model, for a line of text.
    pub note: String,
    display: Option<Result<Dbc, String>>,
    notes: HashMap<String, String>,
    looks: HashMap<(usize, usize, u32, u32), Ranges>,
    /// How far the model is turned round (radians).
    pub turn: f32,
    clock_ms: f32,
    vertices: Vec<ModelVertex>,
}

impl Preview {
    pub fn new(renderer: &mut Renderer, gpu: &Gpu, mount: Arc<Mount>) -> Preview {
        let target = renderer.add_offscreen(gpu, SIZE);
        Preview { mount, cache: ModelCache::default(), target, catalogue: None, shown: HashMap::new(), want: None, error: None, note: String::new(), display: None, notes: HashMap::new(), looks: HashMap::new(), turn: 0.6, clock_ms: 0.0, vertices: Vec::new() }
    }

    /// The interface texture that shows the picture.
    pub fn texture(&self) -> usize {
        self.target.1
    }

    /// Read the models and skins from the tables, once, when the form first asks for them.
    pub fn load_catalogue(&mut self) {
        let mount = &self.mount;
        self.catalogue.get_or_insert_with(|| Catalogue::load(mount).map_err(|e| format!("{e:#}")));
    }

    /// What the game has for a race and sex to choose from (the usual numbers when the tables cannot be read).
    pub fn ranges(&mut self, race: Race, sex: Sex, skin: u32, hair_style: u32) -> Ranges {
        let mount = &self.mount;
        *self.looks.entry((race as usize, sex as usize, skin, hair_style)).or_insert_with(|| {
            character::ranges(mount, race, sex, skin, hair_style).unwrap_or_else(|e| {
                log::warn!("no looks for {} {}: {e:#}", race.name(), sex.name());
                Ranges { skin: 9, face: 11, hair_style: 11, hair_color: 9, beard: 8 }
            })
        })
    }

    /// The content was replaced: every picture is made again when it is next asked for.
    pub fn forget(&mut self, renderer: &mut Renderer) {
        for (_, shown) in self.shown.drain() {
            match shown {
                Ok(Shown::Beast { entity, .. }) => renderer.remove_entity(entity),
                Ok(Shown::Person(person)) => person.retire(renderer),
                Err(_) => {}
            }
        }
        self.notes.clear();
    }

    /// Draw this body in the picture this frame.
    pub fn show(&mut self, body: &RawBody) {
        self.want = Some(Subject::Body(body.clone()));
    }

    /// Draw the character wearing or holding this look of an item.
    pub fn show_display(&mut self, id: u32, wear: Wear) {
        self.want = Some(Subject::Display(id, wear));
    }

    fn display_dbc(&mut self) -> Result<&Dbc, String> {
        let mount = &self.mount;
        self.display.get_or_insert_with(|| mount.read(r"DBFilesClient\ItemDisplayInfo.dbc").map_err(anyhow::Error::from).and_then(|d| Ok(Dbc::parse(&d)?)).map_err(|e| format!("{e:#}"))).as_ref().map_err(|e| e.clone())
    }

    /// The looks of items the game has that fit `wear`: (row, text to search and read).
    pub fn displays(&mut self, wear: Wear) -> Vec<(u32, String)> {
        let mount = self.mount.clone();
        let Ok(dbc) = self.display_dbc() else { return Vec::new() };
        let held = |row: &wowfmt::dbc::Record, folder: &str| {
            let model = row.string(1).rsplit_once('.').map_or(row.string(1), |(stem, _)| stem);
            !model.is_empty() && !row.string(3).is_empty() && mount.exists(&format!(r"Item\ObjectComponents\{folder}\{model}.m2"))
        };
        dbc.iter()
            .filter(|r| !r.string(5).is_empty())
            .filter(|r| match (wear.holds(), wear) {
                (Some(folder), _) => held(r, folder),
                (_, Wear::Chest | Wear::Shirt | Wear::Tabard) => (14..20).any(|i| !r.string(i).is_empty()),
                (_, Wear::Feet) => !r.string(21).is_empty() && (14..19).all(|i| r.string(i).is_empty()),
                (_, Wear::Legs) => !r.string(19).is_empty() || !r.string(20).is_empty(),
                (_, Wear::Waist) => !r.string(19).is_empty(),
                (_, Wear::Wrist) => !r.string(15).is_empty(),
                (_, Wear::Hands) => !r.string(16).is_empty(),
                (_, Wear::Head | Wear::Shoulder) => !r.string(1).is_empty(),
                (_, Wear::Back) => !r.string(3).is_empty(),
                _ => true,
            })
            .map(|r| (r.u32(0), format!("{}  {}  {}", r.u32(0), r.string(5), r.string(1))))
            .collect()
    }

    /// Nothing is shown this frame (the form is not on screen).
    pub fn clear(&mut self) {
        self.want = None;
    }

    fn build(&mut self, subject: &Subject, renderer: &mut Renderer, gpu: &Gpu, key: &str) -> Result<Shown> {
        let body = match subject {
            Subject::Body(body) => body,
            Subject::Display(id, wear) => {
                let dbc = self.display_dbc().map_err(anyhow::Error::msg)?;
                let row = dbc.find_by_id(*id).with_context(|| format!("no ItemDisplayInfo row {id}"))?;
                let (note, model, skin): (String, String, String) = (format!("icon {}, model {}", row.string(5), row.string(1)), row.string(1).to_string(), row.string(3).to_string());
                let textures: Vec<String> = (14..22).map(|i| row.string(i).to_string()).collect();
                self.notes.insert(key.to_string(), note);
                let _ = (&textures, &model, &skin);
                let slot = wear.default_slot().filter(|_| *wear != Wear::None).context("Not worn: it has no picture on the character.")?;
                // An item made of this look alone (the kind of armor is not known: cloth).
                let row = self.display_dbc().map_err(anyhow::Error::msg)?.find_by_id(*id).context("no row")?;
                let mut def = items::ItemDef::plain(*wear);
                (def.look, def.held, def.visual) = crate::content::look_of(&row, *wear);
                let def: &'static items::ItemDef = Box::leak(Box::new(def));
                let mut equipment = [None; items::SLOTS];
                equipment[slot] = Some(def);
                return Ok(Shown::Person(Person::dress(&equipment, &Appearance::default(), &self.mount, renderer, gpu, key)?));
            }
        };
        match body {
            RawBody::Beast { model, skins, .. } => {
                let m2 = M2::parse(&self.mount.read(model)?).context("model")?;
                let skins: Vec<(u32, &str)> = skins.iter().map(|(slot, file)| (*slot, file.as_str())).collect();
                let template = self.cache.creature(&self.mount, model, &skins)?;
                let entity = renderer.add_entity(gpu, &template);
                let seq = m2.sequences.iter().position(|s| s.id == 0);
                let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
                for v in &template.vertices {
                    for i in 0..3 {
                        (low[i], high[i]) = (low[i].min(v.pos[i]), high[i].max(v.pos[i]));
                    }
                }
                // A model with nothing to see (effects only) gets a yard-sized frame.
                let (low, high) = if template.vertices.is_empty() { (Vec3::splat(-0.5), Vec3::splat(0.5)) } else { (Vec3::from(low), Vec3::from(high)) };
                Ok(Shown::Beast { entity, m2, template, seq, low, high })
            }
            RawBody::Humanoid { outfit, look } => {
                // What is not in the game yet (an item made in this session) is not worn until the game is restarted with it.
                let worn: Vec<usize> = outfit.iter().filter_map(|id| items::all().iter().position(|i| i.id == *id)).collect();
                Ok(Shown::Person(Person::load(&worn, look, &self.mount, renderer, gpu, key)?))
            }
        }
    }

    /// Draw what the form asked for into the picture. `base` is where in the world to put it: anywhere on a loaded tile.
    pub fn render(&mut self, renderer: &mut Renderer, gpu: &Gpu, world: &World, base: Vec3, dt: f32) {
        let Some(subject) = self.want.take() else { return };
        self.clock_ms += dt * 1000.0;
        let key = match &subject {
            Subject::Body(RawBody::Beast { model, skins, .. }) => format!("beast|{model}|{skins:?}"),
            Subject::Body(RawBody::Humanoid { outfit, look }) => format!("person|{outfit:?}|{look:?}"),
            Subject::Display(id, wear) => format!("display|{id}|{wear:?}"),
        }
        .to_ascii_lowercase();
        if !self.shown.contains_key(&key) {
            // shortcut: every look tried in a session keeps its entity; the renderer cannot drop them, and an editing session is short.
            let built = self.build(&subject, renderer, gpu, &key).map_err(|e| format!("{e:#}"));
            self.shown.insert(key.clone(), built);
        }
        self.note = self.notes.get(&key).cloned().unwrap_or_default();
        let clock = self.clock_ms;
        let turned = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let (orient, low, high, ids) = match self.shown.get_mut(&key) {
            Some(Ok(Shown::Beast { entity, m2, template, seq, low, high })) => {
                let scale = if let Subject::Body(RawBody::Beast { scale, .. }) = &subject { scale.clamp(0.05, 20.0) } else { 1.0 };
                if let Some(seq) = *seq {
                    let (start, length) = (m2.sequences[seq].start as f32, (m2.sequences[seq].end - m2.sequences[seq].start).max(1) as f32);
                    let bones = anim::bone_matrices(m2, seq, start + clock % length, clock);
                    anim::skin(m2, &bones, &template.vertices, &mut self.vertices);
                    renderer.update_entity_vertices(gpu, *entity, &self.vertices);
                }
                let orient = turned * Mat4::from_scale(Vec3::splat(scale));
                renderer.set_entity(*entity, base, orient, true);
                (orient, *low, *high, vec![*entity])
            }
            Some(Ok(Shown::Person(person))) => (turned, Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 2.0, 0.5), person.stand(dt, base, Vec3::new((std::f32::consts::PI + self.turn).sin(), 0.0, -(std::f32::consts::PI + self.turn).cos()), renderer, gpu)),
            Some(Err(e)) => {
                self.error = Some(e.clone());
                return;
            }
            _ => return,
        };
        self.error = None;
        // Looking at the middle of it from the front, a little turned, far enough that all of it fits in the picture.
        let (aspect, fov) = (SIZE.0 as f32 / SIZE.1 as f32, 25f32.to_radians());
        let around = std::f32::consts::PI + self.turn;
        let (forward, right) = (Vec3::new(around.sin(), 0.0, -around.cos()), Vec3::new(around.cos(), 0.0, around.sin()));
        let corners = (0..8).map(|i| orient.transform_point3(Vec3::new(if i & 1 == 0 { low.x } else { high.x }, if i & 2 == 0 { low.y } else { high.y }, if i & 4 == 0 { low.z } else { high.z })));
        let (mut half_w, mut half_h, mut half_d, mut middle) = (0f32, 0f32, 0f32, Vec3::ZERO);
        let corners: Vec<Vec3> = corners.collect();
        for c in &corners {
            middle += *c / 8.0;
        }
        for c in &corners {
            let c = *c - middle;
            (half_w, half_h, half_d) = (half_w.max(c.dot(right).abs()), half_h.max(c.y.abs()), half_d.max(c.dot(forward).abs()));
        }
        let tan = (fov / 2.0).tan();
        let distance = (half_h / tan).max(half_w / (tan * aspect)) * 1.08 + half_d;
        let mut camera = Camera::new(base + middle - forward * distance, around, 0.0);
        (camera.fov_y, camera.aspect, camera.near, camera.far) = (fov, aspect, 0.1, distance + half_d + 20.0);
        renderer.render_offscreen(gpu, self.target.0, &camera, &ids, world.tiles());
        // Only the picture shows them: the world's frame must not.
        for id in ids {
            renderer.set_entity(id, base, Mat4::IDENTITY, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_knows_the_models_and_the_skins_of_the_wolf() {
        let Some(dir) = mpq::find_wow_dir() else { return };
        let catalogue = Catalogue::load(&Mount::open_wow_dir(dir).unwrap()).unwrap();
        assert!(catalogue.models.iter().any(|m| m.eq_ignore_ascii_case(r"Creature\Wolf\Wolf.m2")));
        let sets = catalogue.skins_of(r"Creature\Wolf\Wolf.m2");
        assert!(sets.len() > 1 && sets.iter().all(|s| !s.is_empty()));
        // The wolves of the content use skins the game has.
        let timber = vec![(11, r"Creature\Wolf\WolfSkinTimber.blp".to_string()), (12, r"Creature\Wolf\WolfSkinTimberAlpha.blp".to_string())];
        assert!(sets.iter().any(|s| same_skins(s, &timber)));
    }
}
