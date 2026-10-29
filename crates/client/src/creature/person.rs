//! How a creature is drawn: a beast with its own model, a person as the dressed human male.

use super::*;

/// Where the name of a model floats, from its feet: its `NAME_ATTACHMENT` posed by `bones` (model space), turned by `orient`; without one, `NAME_FALLBACK` yards up per yard of `scale`.
pub(super) fn name_offset(m2: &M2, bones: &[Mat4], orient: Mat4, scale: f32) -> Vec3 {
    let at = m2.attachment_lookup.get(NAME_ATTACHMENT).and_then(|&i| m2.attachments.get(i as usize)).and_then(|a| Some(bones.get(a.bone as usize)?.transform_point3(Vec3::from(a.position))));
    at.map_or(Vec3::Y * NAME_FALLBACK * scale, |p| orient.transform_point3(crate::character::MODEL_TO_RENDER.transform_point3(p)))
}

pub(super) struct BeastRig {
    pub(super) m2: M2,
    pub(super) template: Arc<ModelCpu>,
    /// The plain standing sequence; the other idle sequences are fidgets that come with a growl.
    pub(super) idle_base: usize,
    pub(super) scale: f32,
}

/// The model of a kind of beast with its skins (none for a person).
pub(super) fn beast_rig(def: &CreatureDef, mount: &Mount, cache: &ModelCache) -> Result<Option<BeastRig>> {
    let Body::Beast { model, skins, scale } = &def.body else { return Ok(None) };
    let m2 = M2::parse(&mount.read(model)?).with_context(|| format!("{} model", def.name))?;
    let skins: Vec<(u32, &str)> = skins.iter().map(|(slot, file)| (*slot, file.as_str())).collect();
    let template = cache.creature(mount, model, &skins)?;
    let idle_base = m2.sequences.iter().position(|s| s.id == STAND).unwrap_or(0);
    Ok(Some(BeastRig { m2, template, idle_base, scale: *scale }))
}

/// A person on screen: a character of some race and sex dressed in his outfit, standing.
pub(crate) struct Person {
    pub(super) character: Character,
    /// The body, dressed, and what hangs on it.
    pub(super) worn: Worn,
    /// Whether it was swinging last frame, to start the swing once; and whether it was holding the stance of a spell being cast, or playing its release.
    pub(super) swinging: bool,
    pub(super) casting: bool,
    pub(super) released: bool,
    /// How tall the model is, yards (a tauren is more than a gnome).
    pub(super) height: f32,
    /// The animation it holds while it stands (a state emote), and whether it runs when it strolls: set by its creature before each pose.
    pub(super) emote: Option<u16>,
    pub(super) run: bool,
}

/// How tall a human man is, yards: what the heights below (the quest mark, the name, the click) were made for.
pub(super) const HUMAN_HEIGHT: f32 = 2.0;

impl Person {
    /// The same dressing as the player's: item textures on the skin, a weapon and a shield in the hands, a helmet, shoulders.
    pub(crate) fn load(outfit: &[usize], look: &Appearance, mount: &Mount, renderer: &mut Renderer, gpu: &Gpu, key: &str) -> Result<Person> {
        let mut equipment: [Option<&'static items::ItemDef>; SLOTS] = [None; SLOTS];
        for &i in outfit {
            let def = items::get(i);
            let places: Vec<usize> = (0..SLOTS).filter(|&s| def.wear.fits(s)).collect();
            if let Some(&slot) = places.iter().find(|&&s| equipment[s].is_none()).or(places.first()) {
                equipment[slot] = Some(def);
            }
        }
        Person::dress(&equipment, look, mount, renderer, gpu, key)
    }

    /// A character of this look in what is worn.
    pub(crate) fn dress(equipment: &[Option<&'static items::ItemDef>; SLOTS], look: &Appearance, mount: &Mount, renderer: &mut Renderer, gpu: &Gpu, key: &str) -> Result<Person> {
        // The textures and models of the people made so far are kept: most of them wear what others wear.
        static CACHE: std::sync::LazyLock<ModelCache> = std::sync::LazyLock::new(ModelCache::default);
        let cache = &*CACHE;
        let mut character = Character::load(mount, cache, key, look)?;
        let mut worn = Worn::new(&character, renderer, gpu);
        worn.dress(&mut character, look, mount, cache, renderer, gpu, equipment)?;
        let height = character.model.vertices.iter().map(|v| v.pos[1]).fold(0.0, f32::max).max(0.5);
        Ok(Person { character, worn, swinging: false, casting: false, released: false, height, emote: None, run: false })
    }

    /// Stand, run after the player, swing the weapon for a melee attack, or fall and lie (the model's own death animation) when dead.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pose(&mut self, dt: f32, pos: Vec3, yaw: f32, state: State, moving: bool, anims: (u16, u16), forward: Vec3, viewer: Vec3, renderer: &mut Renderer, gpu: &Gpu) {
        let swinging = matches!(state, State::Melee(..));
        if swinging && !self.swinging {
            self.character.attack(MELEE_TIME);
        }
        self.swinging = swinging;
        // A spell: the stance is held while it is cast, and the cast is played once when it goes off.
        let casting = matches!(state, State::Cast(_, _, _, false));
        if casting != self.casting {
            self.character.set_precast(casting.then_some(anims.0));
            self.casting = casting;
        }
        let released = matches!(state, State::Cast(_, _, _, true));
        if released && !self.released {
            self.character.play_once(anims.1, CAST_AFTER + 0.3);
        }
        self.released = released;
        let speed = match state {
            State::Chase if moving => RUN_SPEED,
            State::Flee(..) | State::Kite(_) => RUN_SPEED,
            State::Return(_) => RUN_SPEED,
            State::Wander(_) if self.run => RUN_SPEED,
            State::Wander(_) => WALK_SPEED,
            _ => 0.0,
        };
        let t = crate::profile::scope("character.update (bones, skin)");
        match state {
            State::Dead(seconds) => self.character.lie(seconds * 1000.0),
            _ => self.character.update(dt, &Motion { speed, forward_speed: speed, emote: self.emote.filter(|_| state == State::Idle), ..Motion::default() }, true),
        }
        drop(t);
        let t = crate::profile::scope("body vertex upload");
        renderer.update_entity_vertices(gpu, self.worn.body, &self.character.vertices);
        drop(t);
        let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - yaw);
        let t = crate::profile::scope("worn.place");
        self.worn.place(renderer, &self.character, pos, orient, false, false);
        drop(t);
        let _t = crate::profile::scope("worn.animate");
        self.worn.animate(dt, forward, viewer, renderer, gpu, &self.character, pos, orient, false, false);
    }

    /// Lie as a ragdoll poses the body (`bones`, none: as it lay), with what hangs on it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tumble(&mut self, bones: Option<Vec<Mat4>>, dt: f32, pos: Vec3, orient: Mat4, forward: Vec3, viewer: Vec3, renderer: &mut Renderer, gpu: &Gpu) {
        if let Some(bones) = bones {
            self.character.pose_with(bones);
            renderer.update_entity_vertices(gpu, self.worn.body, &self.character.vertices);
        }
        self.worn.place(renderer, &self.character, pos, orient, false, false);
        self.worn.animate(dt, forward, viewer, renderer, gpu, &self.character, pos, orient, false, false);
    }

    /// Standing, turned north, for the editor's picture; returns the entities to draw.
    pub(crate) fn stand(&mut self, dt: f32, pos: Vec3, forward: Vec3, renderer: &mut Renderer, gpu: &Gpu) -> Vec<usize> {
        self.pose(dt, pos, 0.0, State::Idle, false, (READY_SPELL, CAST_SPELL), forward, pos - forward * 5.0, renderer, gpu);
        self.worn.entities()
    }

    /// Standing and facing `yaw` (radians, 0 north, clockwise), for the editor's ghost; returns the entities to draw.
    pub(crate) fn stand_facing(&mut self, dt: f32, pos: Vec3, yaw: f32, renderer: &mut Renderer, gpu: &Gpu) -> Vec<usize> {
        let forward = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
        self.pose(dt, pos, yaw, State::Idle, false, (READY_SPELL, CAST_SPELL), forward, pos - forward * 5.0, renderer, gpu);
        self.worn.entities()
    }

    /// Give the entities back to the renderer.
    pub(crate) fn retire(self, renderer: &mut Renderer) {
        self.worn.retire(renderer);
    }

    /// How much taller than a human man.
    pub(super) fn tall(&self) -> f32 {
        self.height / HUMAN_HEIGHT
    }

    pub(crate) fn hide(&self, pos: Vec3, renderer: &mut Renderer) {
        self.worn.hide(renderer, pos);
    }
}

pub(super) enum Visual {
    Beast {
        entity: usize,
        anim: Channel,
        /// The idle sequence last seen, to notice when it changes to a fidget.
        last_seq: usize,
        /// The kind whose model it shows (a waypoint script can change it).
        shown: usize,
    },
    Person(Person),
    /// Not made yet (it is near, but its turn has not come) or no longer there.
    None,
}

/// A quest mark's model, shared by every creature that shows it.
pub(super) struct MarkerRig {
    pub(super) m2: M2,
    pub(super) template: Arc<ModelCpu>,
}

pub(super) struct Creature {
    pub(super) mob: Mob,
    /// The creature whose script summons this one (it waits in the pool of summoned creatures until then).
    pub(super) parent: Option<usize>,
    /// The place it stands at, while it is in the world (none for a summoned one, or for a slot that is free).
    pub(super) site: Option<usize>,
    pub(super) visual: Visual,
    /// The entities of its three quest marks (those creatures that can be talked to have them), and the one shown.
    pub(super) marks: Option<[usize; 3]>,
    pub(super) want: Option<Marker>,
    /// It fell as a ragdoll when it died, and lies as that until it is back; whether it was dead at the last frame.
    pub(super) ragdoll: Option<Ragdoll>,
    pub(super) was_dead: bool,
    /// Where its name floats, from its feet (the model's `NAME_ATTACHMENT` as of the last pose).
    pub(super) name_at: Vec3,
    /// Whether a wall hid its name when last looked at, and how many label frames ago (a look costs a ray through the world).
    pub(super) hidden: std::cell::Cell<(bool, u8)>,
    /// Seconds not posed yet and the frame it is at of the few it is posed every (far creatures are posed less often).
    pub(super) owed: f32,
    pub(super) beat: u8,
}


impl Creature {
    /// Red for a hostile creature, yellow for a neutral one, green for a friendly one (its name and the circle under it).
    pub(super) fn reaction(&self) -> [f32; 4] {
        let m = &self.mob;
        if m.is_ally() {
            [0.1, 1.0, 0.1, 1.0]
        } else if m.def.is_hostile() || m.forced_hostile {
            [1.0, 0.1, 0.1, 1.0]
        } else if m.def.is_neutral() {
            [1.0, 0.9, 0.1, 1.0]
        } else {
            [0.1, 1.0, 0.1, 1.0]
        }
    }
}
