//! `Creatures`: every creature of the world, streamed in near the player, drawn, picked and targeted.

use super::*;

/// The rectangle (x0, y0, x1, y1 in pixels) that a thing `height` yards tall standing at `feet` covers on a `size` screen seen
/// through `camera`; `width` is half its width as a share of its height on the screen.
pub fn screen_box(camera: &Camera, size: (u32, u32), feet: Vec3, height: f32, width: f32) -> Option<[f32; 4]> {
    let (fx, fy) = Creatures::on_screen(camera, size, feet)?;
    let (_, hy) = Creatures::on_screen(camera, size, feet + Vec3::Y * height)?;
    let half = ((fy - hy).abs() * width).max(8.0);
    Some([fx - half, hy.min(fy) - 4.0, fx + half, hy.max(fy) + 4.0])
}

/// What the target frame shows about the target.
pub struct TargetView {
    /// Which creature it is (for noticing a change of target).
    pub id: usize,
    pub name: String,
    pub level: u32,
    /// Health as a share of the most it has.
    pub fraction: f32,
    pub dead: bool,
    pub hostile: bool,
    pub neutral: bool,
    /// Its harmful auras, and the icons of its helpful ones.
    pub debuffs: Vec<super::Aura>,
    pub buffs: Vec<super::Aura>,
}

/// How far the target is let go at (yards), and how far the next one is looked for.
const TARGET_LOSE: f32 = 60.0;
const TARGET_RANGE: f32 = 40.0;

/// A name floating over a creature: where on the screen (pixels, the middle of the text) and in which colour.
pub struct Label {
    pub text: String,
    /// The line under the name, `<Weapon Master>` (`FUN_00608f50`: a creature's subname).
    pub sub: Option<String>,
    pub color: [f32; 4],
    pub x: f32,
    pub y: f32,
    /// How tall its line of text is on the screen, pixels.
    pub line: f32,
}

/// A place of the map where a creature of some kind stands (a spawn of the content): while the player is far it is only this.
struct Site {
    kind: usize,
    pos: Vec3,
    yaw: f32,
    wander: f32,
    respawn: Option<f32>,
    route: Vec<Stop>,
    /// Its place among the creatures while it is in the world.
    slot: Option<usize>,
}

pub(super) fn cell_of(at: Vec3) -> (i32, i32) {
    ((at.x / CELL).floor() as i32, (at.z / CELL).floor() as i32)
}

pub struct Creatures {
    /// By kind (index into the content's creatures): the model of every kind of beast in the world.
    rigs: Vec<Option<BeastRig>>,
    /// The creatures in the world now (those near the player); a slot that is free holds an unborn creature and is in `free`.
    creatures: Vec<Creature>,
    sites: Vec<Site>,
    /// The sites by cell and by kind.
    grid: HashMap<(i32, i32), Vec<usize>>,
    by_kind: HashMap<usize, Vec<usize>>,
    free: Vec<usize>,
    /// What a save says of a creature whose site is not in the world now, by site.
    pending: HashMap<usize, MobSave>,
    mount: Arc<Mount>,
    cache: ModelCache,
    budget: usize,
    scan: f32,
    markers: Vec<MarkerRig>,
    viewer: Vec3,
    rng: Rng,
    clock_ms: f32,
    vertices: Vec<ModelVertex>,
    /// Kinds of the creatures killed since `take_deaths`.
    deaths: Vec<usize>,
    /// The creature the player has selected.
    target: Option<usize>,
    /// What the pet bar has told the pet.
    orders: ally::PetOrders,
    /// The spell whose harm is being put on creatures now (its icon and dispel kind), so that the target frame can list it; none for harm that comes from gear.
    source: (String, String, u32),
    /// The damage of the ticks of damage over time and where the creature stood, since `take_ticks`.
    ticks: Vec<(Vec3, f32)>,
    /// The quest items that the active quests want (`QuestLog::wanted`): what dies now may drop them, and no other quest item.
    wanted: Vec<usize>,
    /// The tomes that the character can learn and does not have (`Panels::learnable_tomes`): any kill may drop one of them.
    tomes: Vec<usize>,
    /// Whether the creatures that die fall as ragdolls (the interface options).
    pub ragdolls: bool,
    /// The animations the creatures' spells ask for.
    anims: SpellAnims,
    /// How fast the missiles of spells fly (yards a second), for the spells that have one: the time a pet's bolt takes to arrive.
    speeds: HashMap<u32, f32>,
    /// The spells of pets in the air: who cast them, which spell, at which creature, and the seconds before they land.
    bolts: Vec<(usize, u32, usize, f32)>,
    /// What the pet's spells cast by the player's order show (they are told with the creatures' acts of the next update).
    shows: Vec<Foe>,
}

/// How far a taming rod reaches, yards, and how long the beast it tames follows, seconds.
const TAME_REACH: f32 = 20.0;
const TAME_SECONDS: f32 = 60.0;

/// Most creatures of one kind that a creature has waiting to be summoned.
const MOST_SUMMONED: usize = 6;

impl Creatures {
    /// The creatures at `spawns`: (kind, feet position in render space, heading in radians, yards it strolls from there). They are made as the player comes near.
    pub fn load(mount: &Arc<Mount>, cache: &ModelCache, renderer: &mut Renderer, gpu: &Gpu, spawns: &[(usize, Vec3, f32, f32)]) -> Result<Creatures> {
        let defs = &content::get().creatures;
        let mut rigs: Vec<Option<BeastRig>> = defs.iter().map(|_| None).collect();
        let rng = Rng(0x5EED_1234);
        // The models of the marks; without them (a missing file) the creatures simply show none.
        let markers: Vec<MarkerRig> = MARKER_MODELS
            .iter()
            .map_while(|path| {
                let m2 = M2::parse(&mount.read(path).ok()?).ok()?;
                Some(MarkerRig { m2, template: cache.plain(mount, path).ok()? })
            })
            .collect();
        if markers.len() < MARKER_MODELS.len() {
            log::warn!("the quest mark models are missing");
        }
        let markers = if markers.len() == MARKER_MODELS.len() { markers } else { Vec::new() };
        // The looks that the waypoint scripts turn creatures into.
        let morphs = content::get().spawns.iter().flat_map(|s| &s.path).flat_map(|w| &w.steps).filter_map(|(_, step)| if let content::Step::Morph(id) = step { defs.iter().position(|d| d.id == *id) } else { None });
        // (A polymorphed creature looks like a sheep.)
        for kind in morphs.chain(content::sheep_kind()) {
            if rigs[kind].is_none() {
                rigs[kind] = beast_rig(&defs[kind], mount, cache)?;
            }
        }
        let sites: Vec<Site> = spawns.iter().map(|&(kind, pos, yaw, wander)| Site { kind, pos, yaw, wander, respawn: None, route: Vec::new(), slot: None }).collect();
        let (mut grid, mut by_kind): (HashMap<(i32, i32), Vec<usize>>, HashMap<usize, Vec<usize>>) = Default::default();
        for (i, site) in sites.iter().enumerate() {
            grid.entry(cell_of(site.pos)).or_default().push(i);
            by_kind.entry(site.kind).or_default().push(i);
        }
        let _ = renderer;
        let _ = gpu;
        Ok(Creatures { rigs, creatures: Vec::new(), sites, grid, by_kind, free: Vec::new(), pending: HashMap::new(), mount: mount.clone(), cache: ModelCache::default(), budget: MAKE_PER_FRAME, scan: SCAN, markers, viewer: Vec3::ZERO, rng, clock_ms: 0.0, vertices: Vec::new(), deaths: Vec::new(), target: None, orders: Default::default(), source: (String::new(), String::new(), 0), ticks: Vec::new(), wanted: Vec::new(), tomes: Vec::new(), ragdolls: true, anims: SpellAnims::new(), speeds: HashMap::new(), bolts: Vec::new(), shows: Vec::new() })
    }

    /// The creatures at their places that have a model: (kind id, north and west of the place, where it is now in render space).
    pub fn standing(&self) -> Vec<(String, f32, f32, Vec3)> {
        self.creatures
            .iter()
            .filter(|c| c.site.is_some() && !matches!(c.visual, Visual::None) && !c.mob.is_dead())
            .map(|c| (c.mob.def.id.clone(), -c.mob.home.z, -c.mob.home.x, c.mob.pos))
            .collect()
    }

    /// How many creatures are in the world now, and how many of them have their model made (the log of a picture taken from the command line says it).
    pub fn counts(&self) -> (usize, usize) {
        let live = self.creatures.iter().filter(|c| c.site.is_some());
        (live.clone().count(), live.filter(|c| !matches!(c.visual, Visual::None)).count())
    }

    /// How many creatures get their model in one frame at most (the first frames of a picture taken from the command line want them all).
    pub fn set_budget(&mut self, most: usize) {
        self.budget = most;
    }

    /// Put a creature into a free slot, or a new one.
    fn alloc(&mut self, creature: Creature) -> usize {
        match self.free.pop() {
            Some(i) => {
                self.creatures[i] = creature;
                i
            }
            None => {
                self.creatures.push(creature);
                self.creatures.len() - 1
            }
        }
    }

    /// The place at `site` becomes a creature in the world (without a model yet), with the creatures that its script summons waiting behind it.
    fn activate(&mut self, site: usize) {
        let defs = &content::get().creatures;
        let s = &self.sites[site];
        let (kind, pos, yaw, wander) = (s.kind, s.pos, s.yaw, s.wander);
        let mut mob = Mob::new(kind, pos, yaw);
        mob.wander = wander;
        mob.respawn = s.respawn.unwrap_or(mob.def.respawn);
        mob.route = s.route.clone();
        if let Some(saved) = self.pending.remove(&site) {
            mob.restore(&saved);
        }
        let slot = self.alloc(Creature { mob, parent: None, site: Some(site), visual: Visual::None, marks: None, want: None, ragdoll: None, was_dead: false, name_at: Vec3::Y * NAME_FALLBACK, hidden: std::cell::Cell::new((true, u8::MAX)), owed: 0.0, beat: 0 });
        self.sites[site].slot = Some(slot);
        // Behind it wait the creatures that its script summons (twice as many as there are summon actions, so that a summon that repeats finds some).
        let mut waiting: Vec<(usize, usize)> = Vec::new();
        for act in defs[kind].brain.events.iter().flat_map(|e| &e.acts).filter(|a| a.0 == brain::act::SUMMON) {
            let id = format!("c{}", act.1);
            let Some(summoned) = content::get().creature_numbers.get(&id).copied() else { continue };
            match waiting.iter_mut().find(|w| w.0 == summoned) {
                Some(w) => w.1 = (w.1 + 2).min(MOST_SUMMONED),
                None => waiting.push((summoned, 2)),
            }
        }
        for (summoned, count) in waiting {
            for _ in 0..count {
                let mut mob = Mob::new(summoned, pos, yaw);
                (mob.wander, mob.summoned, mob.state) = (wander, true, State::Unborn);
                self.alloc(Creature { mob, parent: Some(slot), site: None, visual: Visual::None, marks: None, want: None, ragdoll: None, was_dead: false, name_at: Vec3::Y * NAME_FALLBACK, hidden: std::cell::Cell::new((true, u8::MAX)), owed: 0.0, beat: 0 });
            }
        }
    }

    /// Take creature `i` and what it summons out of the world (its models go back), leaving free slots.
    fn deactivate(&mut self, i: usize, renderer: &mut Renderer) {
        let site = self.creatures[i].site;
        let kids: Vec<usize> = (0..self.creatures.len()).filter(|&j| self.creatures[j].parent == Some(i)).collect();
        for j in kids.into_iter().chain([i]) {
            let c = &mut self.creatures[j];
            match std::mem::replace(&mut c.visual, Visual::None) {
                Visual::Beast { entity, .. } => renderer.remove_entity(entity),
                Visual::Person(p) => p.retire(renderer),
                Visual::None => {}
            }
            c.marks.take().into_iter().flatten().for_each(|e| renderer.remove_entity(e));
            (c.parent, c.site, c.want, c.ragdoll, c.was_dead) = (None, None, None, None, false);
            c.mob.state = State::Unborn;
            self.free.push(j);
        }
        if let Some(site) = site {
            self.sites[site].slot = None;
        }
        if self.target == Some(i) {
            self.target = None;
        }
    }

    /// Look at who is near: the places within `ACTIVATE` yards become creatures, and the ones at rest beyond `DEACTIVATE` are put away.
    fn stream(&mut self, player: Vec3, renderer: &mut Renderer) {
        let (cx, cz) = cell_of(player);
        let reach = (ACTIVATE / CELL).ceil() as i32;
        let mut wake = Vec::new();
        for dx in -reach..=reach {
            for dz in -reach..=reach {
                for &s in self.grid.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    if self.sites[s].slot.is_none() && flat(self.sites[s].pos - player).length() <= ACTIVATE {
                        wake.push(s);
                    }
                }
            }
        }
        for s in wake {
            self.activate(s);
        }
        // Only a creature that is at rest goes: not one that fights, runs home, lies dead (its time to come back is counted), or is the target.
        let gone: Vec<usize> = (0..self.creatures.len())
            .filter(|&i| {
                let c = &self.creatures[i];
                c.site.is_some()
                    && self.target != Some(i)
                    && matches!(c.mob.state, State::Idle | State::Wander(_))
                    && flat(c.mob.pos - player).length() > DEACTIVATE
                    && flat(c.mob.home - player).length() > DEACTIVATE
            })
            .collect();
        for i in gone {
            self.deactivate(i, renderer);
        }
    }

    /// Give the models to the creatures that are near and have none, the nearest first, a few in a frame (a model is read and dressed for each).
    fn make_visible(&mut self, player: Vec3, renderer: &mut Renderer, gpu: &Gpu) {
        let mut need: Vec<(f32, usize)> = (0..self.creatures.len())
            .filter(|&i| matches!(self.creatures[i].visual, Visual::None) && self.creatures[i].mob.state != State::Unborn)
            .map(|i| (flat(self.creatures[i].mob.pos - player).length(), i))
            .filter(|&(d, _)| d <= DRAW_RANGE)
            .collect();
        need.sort_by(|a, b| a.0.total_cmp(&b.0));
        let started = Instant::now();
        for (n, (_, i)) in need.into_iter().enumerate() {
            if n >= self.budget || (n > 0 && started.elapsed() > MAKE_TIME) {
                break;
            }
            let began = Instant::now();
            let made = self.make(i, renderer, gpu);
            log::debug!("model of {} made in {:?}", self.creatures[i].mob.def.name, began.elapsed());
            if let Err(e) = made {
                // It is not in the world then (and is not tried again).
                log::warn!("{}: {e:#}", self.creatures[i].mob.def.name);
                self.creatures[i].mob.state = State::Unborn;
            }
        }
    }

    /// The model of creature `i` (and its quest marks).
    fn make(&mut self, i: usize, renderer: &mut Renderer, gpu: &Gpu) -> Result<()> {
        let (kind, def) = (self.creatures[i].mob.kind, self.creatures[i].mob.def);
        // People of one kind share the picture of their skin (they are dressed alike), under a key of the kind.
        let key = format!("npc/{kind}");
        let visual = match &def.body {
            Body::Beast { .. } => {
                if self.rigs[kind].is_none() {
                    self.rigs[kind] = beast_rig(def, &self.mount, &self.cache)?;
                }
                let rig = self.rigs[kind].as_ref().unwrap();
                Visual::Beast { entity: renderer.add_room_lit_entity(gpu, &rig.template), anim: Channel::new(&rig.m2, STAND, &mut self.rng), last_seq: usize::MAX, shown: kind }
            }
            // A look that does not exist for the race (a face, a hair style) is shown as the default of the race instead.
            Body::Humanoid { outfit, look } => Visual::Person(match Person::load(outfit, look, &self.mount, renderer, gpu, &key) {
                Ok(person) => person,
                Err(e) => {
                    log::warn!("{}: {e:#}; showing the default look", def.name);
                    Person::load(outfit, &Appearance { race: look.race, sex: look.sex, skin: 0, face: 0, hair_style: 0, hair_color: 0, beard: 0, baked: 0 }, &self.mount, renderer, gpu, &key)?
                }
            }),
        };
        // Whoever can be talked to may have a quest mark over its head: one entity for each of the three marks.
        let marks = (def.gossip.is_some() && !self.markers.is_empty()).then(|| [0, 1, 2].map(|m| renderer.add_entity(gpu, &self.markers[m].template)));
        let c = &mut self.creatures[i];
        (c.visual, c.marks) = (visual, marks);
        Ok(())
    }

    /// Where the creatures of the given kinds are now (render space): the living ones, else the corpses (which wait to come back); of those
    /// that are not in the world now, where they stand. The nearest few to the viewer.
    pub fn positions(&self, kinds: &[usize]) -> Vec<Vec3> {
        let of_kind = || self.creatures.iter().filter(|c| c.site.is_some() || c.parent.is_some()).map(|c| &c.mob).filter(|m| kinds.contains(&m.kind));
        let mut all: Vec<Vec3> = of_kind().filter(|m| !m.is_dead()).map(|m| m.pos).collect();
        all.extend(kinds.iter().filter_map(|k| self.by_kind.get(k)).flatten().map(|&s| &self.sites[s]).filter(|s| s.slot.is_none()).map(|s| s.pos));
        if all.is_empty() {
            all = of_kind().map(|m| m.pos).collect();
        }
        all.sort_by(|a, b| flat(*a - self.viewer).length().total_cmp(&flat(*b - self.viewer).length()));
        all.truncate(MOST_LEADS);
        all
    }

    /// The creatures worth keeping in a save (the ones hurt, dead, fighting or carrying loot; the rest start afresh), and the ones kept for places not in the world now.
    pub fn snapshot(&self) -> Vec<MobSave> {
        self.creatures.iter().filter(|c| c.site.is_some() && c.mob.worth_saving()).map(|c| c.mob.snapshot()).chain(self.pending.values().cloned()).collect()
    }

    /// Give the sites their own respawn times and patrols (the n-th is the n-th spawned): the seconds to come back, and the places to walk to.
    pub fn set_routes(&mut self, routes: Vec<(Option<f32>, Vec<content::Waypoint>)>) {
        for (site, (respawn, path)) in self.sites.iter_mut().zip(routes) {
            site.respawn = respawn;
            site.route = path
                .into_iter()
                .map(|w| Stop { at: crate::coords::wow_to_render([w.at.0, w.at.1, 0.0]), wait: w.wait, facing: w.facing.map(f32::to_radians), steps: w.steps })
                .collect();
        }
    }

    /// The site a saved creature stood at (the same kind with the same home: the content may have changed since).
    fn site_of(&self, saved: &MobSave) -> Option<usize> {
        let (cx, cz) = cell_of(saved.home);
        let defs = &content::get().creatures;
        (-1..=1)
            .flat_map(|dx| (-1..=1).map(move |dz| (cx + dx, cz + dz)))
            .flat_map(|cell| self.grid.get(&cell).into_iter().flatten().copied())
            .find(|&s| defs[self.sites[s].kind].id == saved.kind && flat(self.sites[s].pos - saved.home).length() <= 1.0)
    }

    /// Put saved creatures back; those that do not match a place of the world any more start afresh. A creature whose place is not in the world now comes back when it is.
    pub fn restore(&mut self, saved: &[MobSave]) {
        let mut back = 0;
        for s in saved {
            let Some(site) = self.site_of(s) else { continue };
            match self.sites[site].slot {
                Some(slot) => back += self.creatures[slot].mob.restore(s) as usize,
                None => {
                    self.pending.insert(site, s.clone());
                    back += 1;
                }
            }
        }
        log::info!("{back} of {} creatures back as saved", saved.len());
    }

    /// Where an attachment of creature `who` (1 the right hand, 2 the left) is in the world, if it is a person: what the glows of its spells follow.
    pub fn hand(&self, who: usize, id: u32) -> Option<Mat4> {
        let c = self.creatures.get(who)?;
        let Visual::Person(p) = &c.visual else { return None };
        let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - c.mob.yaw);
        Some(Mat4::from_translation(c.mob.pos) * orient * crate::character::to_render(p.character.attachment(id)?))
    }

    /// The animations the spells of creatures ask for (the stance held while casting, the cast).
    pub fn set_spell_anims(&mut self, anims: SpellAnims) {
        self.anims = anims;
    }

    /// How fast the missiles of the spells fly (see `Foes::missile_speeds`).
    pub fn set_missile_speeds(&mut self, speeds: HashMap<u32, f32>) {
        self.speeds = speeds;
    }

    /// How tall creature `c` is, yards (where its spells are shown).
    fn height_of(c: &Creature) -> f32 {
        match (&c.visual, &c.mob.def.body) {
            (Visual::Person(p), _) => HUMAN_HEIGHT * p.tall(),
            (_, Body::Beast { scale, .. }) => scale.max(0.8),
            _ => 1.5,
        }
    }

    /// Advance every creature and pose it. Returns what they did that the game acts on (the swings that landed on the player, the spells begun, stopped
    /// and gone off), and the sounds the creatures made with the place they came from.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, dt: f32, watch: &Watch, ground: impl Fn(f32, f32, f32) -> Option<f32>, blocked: impl Fn(Vec3, Vec3) -> bool, mut physics: Option<&mut Bodies>, renderer: &mut Renderer, gpu: &Gpu) -> (Vec<Foe>, Vec<(Sfx, Vec3)>) {
        self.clock_ms += dt * 1000.0;
        let player = watch.feet;
        self.scan += dt;
        let t = crate::profile::scope("stream (spawns near)");
        if self.scan >= SCAN {
            self.scan = 0.0;
            self.stream(player, renderer);
        }
        drop(t);
        let t = crate::profile::scope("make_visible (models)");
        self.make_visible(player, renderer, gpu);
        drop(t);
        let (mut step_time, mut pose_time) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        let (mut bones_time, mut skin_time, mut upload_time, mut person_time) = (std::time::Duration::ZERO, std::time::Duration::ZERO, std::time::Duration::ZERO, std::time::Duration::ZERO);
        let t = crate::profile::scope("creature loop");
        let ahead = Vec3::new(watch.yaw.sin(), 0.0, -watch.yaw.cos());
        let (mut foes, mut cries) = (Vec::new(), Vec::new());
        // Who is about, for the creatures that look after friends, call them and run to them.
        // (Only a creature that looks after friends and is in a fight reads it: nobody else, nobody makes it.)
        let needed = self.creatures.iter().any(|c| c.mob.social && c.mob.state.fights());
        let roster: Vec<Friend> = self
            .creatures
            .iter()
            .enumerate()
            .filter(|_| needed)
            .filter(|(_, c)| c.mob.attackable() && !c.mob.is_dead() && flat(c.mob.pos - player).length() <= DRAW_RANGE)
            .map(|(index, c)| Friend { index, pos: c.mob.pos, faction: c.mob.def.brain.faction, dist: 0.0, health: c.mob.health, max: c.mob.def.health, auras: c.mob.aura_ids() })
            .collect();
        let (mut casts, mut summons, mut assists) = (Vec::new(), Vec::new(), Vec::new());
        // What the player's allies fight: what he has selected, if it is a foe within reach of him; else what fights him and is the nearest to him. And who of them soak blows.
        let is_foe = |c: &Creature| c.mob.attackable() && !c.mob.is_dead() && flat(c.mob.pos - player).length() <= ally::GO_FOR;
        let nearest_foe = |this: &Self, reach: f32, fighting: bool| {
            (0..this.creatures.len())
                .filter(|&i| is_foe(&this.creatures[i]) && flat(this.creatures[i].mob.pos - player).length() <= reach && (!fighting || this.creatures[i].mob.state.fights()))
                .min_by(|&a, &b| flat(this.creatures[a].mob.pos - player).length().total_cmp(&flat(this.creatures[b].mob.pos - player).length()))
        };
        let focus: Option<(usize, Vec3)> = self.target.filter(|&t| self.creatures.get(t).is_some_and(is_foe)).or_else(|| nearest_foe(self, ally::GO_FOR, true)).map(|i| (i, self.creatures[i].mob.pos));
        // The pet fights what it is sent against; else, by its stance, nothing, what fights the player, or any foe about. (The attack order ends with its foe.)
        if self.orders.attack.is_some_and(|t| !self.creatures.get(t).is_some_and(is_foe)) {
            self.orders.attack = None;
        }
        let orders = self.orders;
        let pet_focus: Option<(usize, Vec3)> = orders.attack.or_else(|| orders.pick().and_then(|(reach, fighting)| nearest_foe(self, reach, fighting))).map(|i| (i, self.creatures[i].mob.pos));
        let helpers: Vec<(usize, Vec3)> = (0..self.creatures.len()).filter(|&i| self.creatures[i].mob.is_pet() && !self.creatures[i].mob.is_dead() && !self.creatures[i].mob.is_phased()).map(|i| (i, self.creatures[i].mob.pos)).collect();
        let (mut strikes, mut spells, mut soaked, mut gone) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (i, c) in self.creatures.iter_mut().enumerate() {
            let mouth = c.mob.pos + Vec3::Y * 0.8;
            let kind = c.mob.kind;
            cries.extend(c.mob.sounds.drain(..).map(|cry| (Sfx::Cry(cry, kind), mouth)));
            if c.mob.state == State::Unborn && (c.mob.is_ally() || (c.mob.summoned && c.parent.is_none())) {
                gone.push(i);
            }
            if c.mob.state == State::Unborn || flat(c.mob.pos - player).length() > DRAW_RANGE {
                // Out of reach nothing is updated: one that was after the player (as the save left it) gives up, or the player would stay in combat.
                if c.mob.state.fights() {
                    c.mob.state = State::Return(0.0);
                }
                c.mob.tick_dead(dt);
                match &c.visual {
                    Visual::Beast { entity, .. } => renderer.set_entity(*entity, c.mob.pos, Mat4::IDENTITY, false),
                    Visual::Person(p) => p.hide(c.mob.pos, renderer),
                    Visual::None => {}
                }
                for e in c.marks.into_iter().flatten() {
                    renderer.set_entity(e, c.mob.pos, Mat4::IDENTITY, false);
                }
                continue;
            }
            // Near, but its model is yet to be made.
            if matches!(c.visual, Visual::None) {
                continue;
            }
            // The quest mark: a small model that spins above the head (its own animation), only the one that is wanted.
            if let Some(marks) = c.marks {
                for (m, &entity) in marks.iter().enumerate() {
                    let shown = c.want.map(|w| w as usize) == Some(m);
                    // The model's front is turned to the viewer (the same heading arithmetic as for a creature that faces him).
                    let tall = if let Visual::Person(p) = &c.visual { p.tall() } else { 1.0 };
                    let at = c.mob.pos + Vec3::Y * MARKER_HEIGHT * tall;
                    let to = flat(self.viewer - at);
                    let facing = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - to.x.atan2(-to.z));
                    renderer.set_entity(entity, at, facing, shown);
                    if shown {
                        let rig = &self.markers[m];
                        let seq = rig.m2.sequences.iter().position(|q| q.id == STAND).unwrap_or(0);
                        let (start, length) = (rig.m2.sequences[seq].start as f32, (rig.m2.sequences[seq].end - rig.m2.sequences[seq].start).max(1) as f32);
                        let bones = anim::bone_matrices(&rig.m2, seq, start + self.clock_ms % length, self.clock_ms);
                        anim::skin(&rig.m2, &bones, &rig.template.vertices, &mut self.vertices);
                        renderer.update_entity_vertices(gpu, entity, &self.vertices);
                    }
                }
            }
            (c.mob.hidden, c.mob.notice) = (watch.sneaking.is_some(), watch.sneaking.unwrap_or(1.0));
            let height = Self::height_of(c);
            let aim = if c.mob.is_pet() { pet_focus } else { focus };
            let acts = {
                let _p = crate::profile::sum(&mut step_time);
                if c.mob.is_ally() {
                    let acts = c.mob.step_ally(dt, player, aim.map(|f| f.1), c.mob.is_pet() && orders.stay, &blocked);
                    if let Some(y) = ground(c.mob.pos.x, c.mob.pos.z, c.mob.pos.y) {
                        c.mob.pos.y = y;
                    }
                    acts
                } else {
                    c.mob.step(dt, watch, i, &roster, &ground, &blocked)
                }
            };
            self.ticks.extend(c.mob.ticked.drain(..).map(|damage| (c.mob.pos + Vec3::Y * 0.9, damage)));
            // What the damage over time slew is a kill of the player's.
            if std::mem::take(&mut c.mob.slain_over) {
                self.deaths.push(kind);
                c.mob.settle_loot(&self.wanted, &self.tomes);
            }
            for act in acts {
                match act {
                    Act::Melee(damage) => {
                        // A pet next to the one that strikes takes some of the blows.
                        let near = helpers.iter().filter(|h| flat(h.1 - c.mob.pos).length() < ally::SOAK_REACH).map(|h| h.0).next();
                        match near {
                            Some(j) if self.rng.next() % 100 < ally::SOAK_PERCENT => soaked.push((j, damage)),
                            _ => foes.push(Foe::Melee(damage, c.mob.def.level)),
                        }
                    }
                    Act::Strike(damage) => strikes.push((aim.map(|f| f.0), damage)),
                    Act::Spell(spell) => spells.push((i, spell, aim.map(|f| f.0))),
                    Act::Begin(spell) => foes.push(Foe::Begin { who: i, spell, at: c.mob.pos, yaw: c.mob.yaw, height }),
                    Act::Stop => foes.push(Foe::Stop { who: i }),
                    Act::Cast(cast) => casts.push((i, cast, height)),
                    Act::Summon { entry, ms } => summons.push((i, entry, ms)),
                    Act::Assist(radius) => assists.push((i, radius)),
                }
            }
            // A creature that dies falls as a ragdoll if the physics has room for it, until it is back.
            let dead = matches!(c.mob.state, State::Dead(_));
            if let Some(r) = c.ragdoll.take() {
                match physics.as_deref_mut() {
                    Some(p) if dead && r.alive(p) => c.ragdoll = Some(r),
                    Some(p) => r.remove(p),
                    None => {}
                }
            }
            let fresh = dead && !c.was_dead && matches!(c.mob.state, State::Dead(t) if t < 0.5);
            c.was_dead = dead;
            let push = flat(c.mob.pos - player).normalize_or_zero() * RAGDOLL_PUSH + Vec3::Y * 1.5;
            let spawn_here = fresh && self.ragdolls && flat(c.mob.pos - player).length() < RAGDOLL_REACH;
            // Far from the player a creature is posed less often (its model is skinned and sent to the graphics card each time): every second frame
            // beyond 30 yards, every fourth beyond 70, every sixth beyond 110, and rarely one that is behind him. One that fights, lies dead or falls is posed at once.
            let (to, away) = (flat(c.mob.pos - player), flat(c.mob.pos - player).length());
            let behind = away > 12.0 && to.dot(ahead) < -0.2 * away;
            c.owed += dt;
            c.beat = (c.beat + 1) % if behind { 8 } else if away < POSE_NEAR { 1 } else if away < POSE_FAR { 2 } else if away < 110.0 { 4 } else { 6 };
            if c.beat != 0 && c.ragdoll.is_none() && !dead && !c.mob.state.fights() && !spawn_here {
                continue;
            }
            let dt = std::mem::take(&mut c.owed);
            let _p = crate::profile::sum(&mut pose_time);
            match &mut c.visual {
                Visual::Beast { entity, anim, last_seq, shown } => {
                    // A script changed how it looks: the model of the new kind takes the place of the old.
                    if *shown != c.mob.look {
                        match self.rigs[c.mob.look].as_ref() {
                            Some(rig) => {
                                renderer.remove_entity(*entity);
                                (*entity, *anim, *last_seq, *shown) = (renderer.add_room_lit_entity(gpu, &rig.template), Channel::new(&rig.m2, STAND, &mut self.rng), usize::MAX, c.mob.look);
                            }
                            None => c.mob.look = *shown,
                        }
                    }
                    let rig = self.rigs[*shown].as_ref().expect("a beast has its rig");
                    let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - c.mob.yaw) * Mat4::from_scale(Vec3::splat(rig.scale));
                    if let (true, Some(p)) = (spawn_here, physics.as_deref_mut()) {
                        let bones = anim::bone_matrices_with(&rig.m2, anim.mix(&rig.m2), self.clock_ms, &[], &[]);
                        c.ragdoll = Ragdoll::spawn(p, &rig.m2, &rig.template, bones, Mat4::from_translation(c.mob.pos) * orient, push);
                    }
                    if let Some(rd) = c.ragdoll.as_mut() {
                        if let Some(bones) = physics.as_deref_mut().and_then(|p| rd.bones(p)) {
                            anim::skin(&rig.m2, &bones, &rig.template.vertices, &mut self.vertices);
                            renderer.update_entity_vertices(gpu, *entity, &self.vertices);
                            // The corpse is where it has been moved to (what is looted and clicked on).
                            if let Some(at) = physics.as_deref().and_then(|p| rd.corpse_at(p)) {
                                c.mob.pos = Vec3::new(at.x, ground(at.x, at.z, at.y).unwrap_or(c.mob.pos.y), at.z);
                            }
                            c.name_at = rd.origin() - c.mob.pos + name_offset(&rig.m2, &bones, orient, rig.scale);
                        }
                        renderer.set_entity(*entity, rd.origin(), orient, true);
                        continue;
                    }
                    anim.request(&rig.m2, c.mob.animation(&self.anims), true, &mut self.rng);
                    anim.advance(&rig.m2, dt * 1000.0, 1.0, true, &mut self.rng);
                    // An idle that turns into a fidget (a look around, a shake) comes with a growl.
                    if c.mob.state == State::Idle && anim.seq() != *last_seq && anim.seq() != rig.idle_base && *last_seq != usize::MAX {
                        cries.push((Sfx::Cry(Cry::Fidget, kind), mouth));
                    }
                    *last_seq = anim.seq();
                    let bones = {
                        let _p = crate::profile::sum(&mut bones_time);
                        anim::bone_matrices_with(&rig.m2, anim.mix(&rig.m2), self.clock_ms, &[], &[])
                    };
                    {
                        let _p = crate::profile::sum(&mut skin_time);
                        anim::skin(&rig.m2, &bones, &rig.template.vertices, &mut self.vertices);
                    }
                    {
                        let _p = crate::profile::sum(&mut upload_time);
                        renderer.update_entity_vertices(gpu, *entity, &self.vertices);
                    }
                    c.name_at = name_offset(&rig.m2, &bones, orient, rig.scale);
                    // Batches that the model shows only in some animations (the logs a peasant carries) fade in and out by its tracks.
                    if let Some(tracks) = &rig.template.tracks {
                        let pose = anim.mix(&rig.m2).to;
                        let looks: Vec<_> = rig.template.batches.iter().map(|b| tracks.look_in(b.refs, pose.seq, pose.time, self.clock_ms)).collect();
                        renderer.set_entity_looks(gpu, *entity, &looks);
                    }
                    let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - c.mob.yaw) * Mat4::from_scale(Vec3::splat(rig.scale));
                    renderer.set_entity(*entity, c.mob.pos, orient, true);
                }
                Visual::Person(person) => {
                    let orient = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2 - c.mob.yaw);
                    if let (true, Some(p)) = (spawn_here, physics.as_deref_mut()) {
                        c.ragdoll = Ragdoll::spawn(p, &person.character.m2, &person.character.model, person.character.bones.clone(), Mat4::from_translation(c.mob.pos) * orient, push);
                    }
                    if let Some(rd) = c.ragdoll.as_mut() {
                        let bones = physics.as_deref_mut().and_then(|p| rd.bones(p));
                        if bones.is_some() {
                            if let Some(at) = physics.as_deref().and_then(|p| rd.corpse_at(p)) {
                                c.mob.pos = Vec3::new(at.x, ground(at.x, at.z, at.y).unwrap_or(c.mob.pos.y), at.z);
                            }
                        }
                        if let Some(b) = &bones {
                            c.name_at = rd.origin() - c.mob.pos + name_offset(&person.character.m2, b, orient, 1.0);
                        }
                        person.tumble(bones, dt, rd.origin(), orient, (c.mob.pos + Vec3::Y - self.viewer).normalize_or_zero(), self.viewer, renderer, gpu);
                        continue;
                    }
                    let anims = if let State::Cast(spell, ..) = c.mob.state { c.mob.spell_anims(spell, &self.anims) } else { (READY_SPELL, CAST_SPELL) };
                    (person.emote, person.run) = (c.mob.emote, c.mob.run);
                    let _q = crate::profile::sum(&mut person_time);
                    person.pose(dt, c.mob.pos, c.mob.yaw, c.mob.state, c.mob.moving, anims, (c.mob.pos + Vec3::Y - self.viewer).normalize_or_zero(), self.viewer, renderer, gpu);
                    c.name_at = name_offset(&person.character.m2, &person.character.bones, orient, 1.0);
                }
                Visual::None => {}
            }
        }
        crate::profile::add("mob.step (AI, movement)", step_time);
        crate::profile::add("pose (skin, upload)", pose_time);
        crate::profile::add("  beasts: bone matrices", bones_time);
        crate::profile::add("  beasts: skinning", skin_time);
        crate::profile::add("  beasts: vertex upload", upload_time);
        crate::profile::add("  people: pose", person_time);
        drop(t);
        // What the allies did: the blows and spells land, the pets take the blows they soaked, the ones that are done are gone.
        for (j, damage) in soaked {
            if let Some(c) = self.creatures.get_mut(j) {
                c.mob.hurt(damage);
            }
        }
        for (target, damage) in strikes {
            if let Some(t) = target {
                self.ally_hit(t, damage);
            }
        }
        // The spells of pets in the air land when their missiles arrive.
        let (due, flying): (Vec<_>, Vec<_>) = std::mem::take(&mut self.bolts).into_iter().map(|b| (b.0, b.1, b.2, b.3 - dt)).partition(|b| b.3 <= 0.0);
        self.bolts = flying;
        for (who, spell, target, _) in due {
            self.ally_cast(who, spell, Some(target));
        }
        for (who, spell, target) in spells {
            let (Some(t), Some(c)) = (target, self.creatures.get(who)) else { continue };
            let Some(tc) = self.creatures.get(t) else { continue };
            let from = c.mob.pos + Vec3::Y * Self::height_of(c) * 0.7;
            let to = tc.mob.pos + Vec3::Y * Self::height_of(tc) * 0.6;
            let delay = self.speeds.get(&spell).map_or(0.0, |&speed| (from.distance(to) / speed).max(0.1));
            foes.push(Foe::Bolt { who, spell, from: c.mob.pos, yaw: c.mob.yaw, height: Self::height_of(c), to, delay });
            if delay > 0.0 {
                self.bolts.push((who, spell, t, delay));
            } else {
                self.ally_cast(who, spell, Some(t));
            }
        }
        for i in gone {
            self.free_ally(i, renderer);
        }
        // Summoned creatures come out of the pool, round whoever summoned them.
        for (parent, entry, ms) in summons {
            let id = format!("c{entry}");
            let (at, yaw) = (self.creatures[parent].mob.pos, self.creatures[parent].mob.yaw);
            if let Some(j) = (0..self.creatures.len()).find(|&j| self.creatures[j].parent == Some(parent) && self.creatures[j].mob.state == State::Unborn && self.creatures[j].mob.def.id == id) {
                let (angle, radius) = (self.rng.next() as f32 / u32::MAX as f32 * std::f32::consts::TAU, 2.0 + 2.0 * (self.rng.next() as f32 / u32::MAX as f32));
                self.creatures[j].mob.summon_at(at + Vec3::new(angle.sin(), 0.0, angle.cos()) * radius, yaw, ms);
            }
        }
        // A call for help: the creatures of the faction round the caller that are not fighting turn on the player.
        for (caller, radius) in assists {
            let (at, faction) = (self.creatures[caller].mob.pos, self.creatures[caller].mob.def.brain.faction);
            for (j, c) in self.creatures.iter_mut().enumerate() {
                if j != caller && c.mob.def.brain.faction == faction && flat(c.mob.pos - at).length() <= radius {
                    c.mob.assist();
                }
            }
        }
        // Spells for friends do their work on the friends they reach.
        for (who, cast, height) in casts {
            let Some(sp) = content::classic_spell(cast.spell) else { continue };
            let faction = self.creatures[who].mob.def.brain.faction;
            let mut allies = Vec::new();
            match cast.aim {
                Aim::Itself => allies.push(self.creatures[who].mob.pos),
                Aim::Friend => {
                    let f = cast.friend.filter(|&f| f < self.creatures.len() && !self.creatures[f].mob.is_dead()).unwrap_or(who);
                    self.creatures[f].mob.apply(sp);
                    allies.push(self.creatures[f].mob.pos);
                }
                Aim::Around => {
                    let radius = sp.effects.iter().filter(|e| e.friendly()).map(|e| e.radius).fold(0.0, f32::max);
                    let at = self.creatures[who].mob.pos;
                    for (j, c) in self.creatures.iter_mut().enumerate() {
                        if j != who && !c.mob.is_dead() && c.mob.def.brain.faction == faction && flat(c.mob.pos - at).length() <= radius {
                            c.mob.apply(sp);
                            allies.push(c.mob.pos);
                        }
                    }
                    allies.push(at);
                }
                Aim::Victim => {}
            }
            foes.push(Foe::Cast { who, cast, height, allies });
        }
        foes.append(&mut self.shows);
        (foes, cries)
    }

    /// The player's sword swing: every hostile creature it reaches (see `Mob::struck_by`) takes `damage`. Returns where
    /// above each the damage should show.
    /// `against(level, kind)` is what the blow does to a creature of that level and type: `None` when it misses (it is listed with no damage), else the damage
    /// the gear adds against its type. Each hit is listed with the creature (its index) it landed on.
    pub fn strike(&mut self, from: Vec3, yaw: f32, damage: f32, unaware_bonus: f32, against: &mut dyn FnMut(u32, u32) -> Option<f32>) -> Vec<(Vec3, f32, bool, usize)> {
        let (mut hits, mut ids) = (Vec::new(), Vec::new());
        for (i, c) in self.creatures.iter_mut().enumerate() {
            // A creature that has not noticed the player takes the blow harder (`unaware_bonus`, 1 for none).
            let sneak = matches!(c.mob.state, State::Idle | State::Wander(_)) && unaware_bonus > 1.0;
            let extra = if c.mob.in_swing(from, yaw, SWORD_REACH) { against(c.mob.def.level, c.mob.def.kind) } else { Some(0.0) };
            let dealt = (damage + extra.unwrap_or(0.0)) * if sneak { unaware_bonus } else { 1.0 };
            if extra.is_none() {
                hits.push((c.mob.pos + Vec3::Y * 0.9, 0.0, false, i));
            } else if c.mob.struck_by(from, yaw, SWORD_REACH, dealt) {
                hits.push((c.mob.pos + Vec3::Y * 0.9, dealt, sneak, i));
                ids.push(i);
                if c.mob.is_dead() {
                    self.deaths.push(c.mob.kind);
                    c.mob.settle_loot(&self.wanted, &self.tomes);
                }
            }
        }
        // Whoever is hit becomes the target, unless a creature that is still alive is selected already.
        if self.target.is_none_or(|t| self.creatures.get(t).is_none_or(|c| c.mob.is_dead())) {
            self.target = ids.first().copied().or(self.target);
        }
        hits
    }

    /// The first living hostile creature that the flight of a missile from `a` to `b` passes through (its body is a cylinder as tall as it is and as wide as a
    /// third of that, at least 0.4 yd): it and where the missile enters it.
    pub fn arrow_hit(&self, a: Vec3, b: Vec3) -> Option<(usize, Vec3)> {
        let d = b - a;
        let hit = |i: usize| {
            let (c, pos) = (&self.creatures[i], self.creatures[i].mob.pos);
            let (h, r) = (Self::height_of(c), (Self::height_of(c) * 0.33).clamp(0.4, 2.0));
            let (f, dd) = ((a.x - pos.x, a.z - pos.z), (d.x, d.z));
            let (qa, qb, qc) = (dd.0 * dd.0 + dd.1 * dd.1, 2.0 * (f.0 * dd.0 + f.1 * dd.1), f.0 * f.0 + f.1 * f.1 - r * r);
            // Where the segment is inside the circle on the ground, as a share of its length; straight up or down it is there all along or never.
            let (t0, t1) = if qa < 1e-9 {
                if qc > 0.0 { return None } else { (0.0, 1.0) }
            } else {
                let disc = qb * qb - 4.0 * qa * qc;
                if disc < 0.0 { return None }
                ((-qb - disc.sqrt()) / (2.0 * qa), (-qb + disc.sqrt()) / (2.0 * qa))
            };
            let (t0, t1) = (t0.max(0.0), t1.min(1.0));
            // And at that height: the share of the way where it is between the feet and the head.
            let (lo, hi) = (pos.y - 0.1, pos.y + h);
            let (ya, yb) = (a.y + d.y * t0, a.y + d.y * t1);
            (t0 <= t1 && ya.max(yb) >= lo && ya.min(yb) <= hi).then(|| (t0, a + d * t0))
        };
        (0..self.creatures.len()).filter(|&i| self.enemy_at(i).is_some_and(|p| flat(p - self.viewer).length() <= DRAW_RANGE)).filter_map(|i| hit(i).map(|(t, at)| (t, i, at))).min_by(|x, y| x.0.total_cmp(&y.0)).map(|(_, i, at)| (i, at))
    }

    pub fn target(&self) -> Option<usize> {
        self.target
    }

    /// Where creature `i` stands, if it is a hostile one that is alive (what a spell can be cast on).
    pub fn enemy_at(&self, i: usize) -> Option<Vec3> {
        let m = &self.creatures.get(i)?.mob;
        (m.attackable() && !m.is_dead()).then_some(m.pos)
    }

    /// A spell hurts creature `i` for `damage`. Returns where above it the damage shows. It becomes the target.
    pub fn spell_hit(&mut self, i: usize, damage: f32) -> Option<Vec3> {
        self.enemy_at(i)?;
        let c = &mut self.creatures[i];
        c.mob.hurt(damage);
        if c.mob.is_dead() {
            self.deaths.push(c.mob.kind);
            c.mob.settle_loot(&self.wanted, &self.tomes);
        }
        self.target = Some(i);
        Some(c.mob.pos + Vec3::Y * 0.9)
    }

    /// Put something on every hostile creature within `radius` yards of `centre`.
    pub fn afflict_area(&mut self, centre: Vec3, radius: f32, status: Status) {
        for i in 0..self.creatures.len() {
            if self.enemy_at(i).is_some_and(|p| flat(p - centre).length() <= radius) {
                self.afflict(i, status);
            }
        }
    }

    // ---- allies

    /// A creature of the original (its `entry`) comes to fight for the player at `at` for `left` seconds (0: for good). `spell` is the spell of the original it casts from
    /// afar (0: it strikes); a `pet` lies dead until it is revived. Its place among the creatures, or none if the content has no such creature.
    pub fn call_ally(&mut self, entry: u32, at: Vec3, yaw: f32, level: u32, left: f32, spell: u32, pet: bool) -> Option<usize> {
        let kind = *content::get().creature_numbers.get(&format!("c{entry}"))?;
        let mut mob = Mob::new(kind, at, yaw);
        mob.make_ally(level, left, spell, pet, false);
        if pet {
            self.orders = Default::default();
        }
        Some(self.alloc(Creature { mob, parent: None, site: None, visual: Visual::None, marks: None, want: None, ragdoll: None, was_dead: false, name_at: Vec3::Y * NAME_FALLBACK, hidden: std::cell::Cell::new((true, u8::MAX)), owed: 0.0, beat: 0 }))
    }

    /// A creature (a kind) is called to be fought at `at` for `seconds`: it is hostile whatever its kind is, and goes when the fight is over.
    pub fn call_foe(&mut self, kind: usize, at: Vec3, yaw: f32, seconds: f32) {
        let mut mob = Mob::new(kind, at, yaw);
        (mob.summoned, mob.forced_hostile) = (true, true);
        mob.summon_at(at, yaw, (seconds * 1000.0) as i32);
        self.alloc(Creature { mob, parent: None, site: None, visual: Visual::None, marks: None, want: None, ragdoll: None, was_dead: false, name_at: Vec3::Y * NAME_FALLBACK, hidden: std::cell::Cell::new((true, u8::MAX)), owed: 0.0, beat: 0 });
    }

    /// The beast selected is of the kind `goal` (by its name) and within reach of `from`: it is tamed for a minute, and counts as killed for the quest that asks for one.
    pub fn tame_for_quest(&mut self, goal: usize, from: Vec3, level: u32) -> bool {
        let wanted = &content::get().creatures[goal].name;
        let Some(i) = self.target.filter(|&t| self.enemy_at(t).is_some_and(|p| flat(p - from).length() <= TAME_REACH) && self.creatures[t].mob.def.name == *wanted) else { return false };
        self.deaths.push(goal);
        self.charm(i, TAME_SECONDS, level)
    }

    /// An ally that is done is taken out of the world: its model goes back and its place is free.
    fn free_ally(&mut self, i: usize, renderer: &mut Renderer) {
        let c = &mut self.creatures[i];
        match std::mem::replace(&mut c.visual, Visual::None) {
            Visual::Beast { entity, .. } => renderer.remove_entity(entity),
            Visual::Person(p) => p.retire(renderer),
            Visual::None => {}
        }
        c.marks.take().into_iter().flatten().for_each(|e| renderer.remove_entity(e));
        (c.parent, c.site, c.want, c.ragdoll, c.was_dead) = (None, None, None, None, false);
        (c.mob.ally, c.mob.summoned, c.mob.forced_hostile) = (None, false, false);
        c.mob.state = State::Unborn;
        if !self.free.contains(&i) {
            self.free.push(i);
        }
        if self.target == Some(i) {
            self.target = None;
        }
    }

    /// An ally's blow lands on creature `i` (a kill is the player's, for the quests and the loot).
    fn ally_hit(&mut self, i: usize, damage: f32) {
        let Some(c) = self.creatures.get_mut(i).filter(|c| c.mob.attackable() && !c.mob.is_dead()) else { return };
        c.mob.hurt(damage);
        // What a pet deals is the player's: it floats up as a number.
        self.ticks.push((c.mob.pos + Vec3::Y * 0.9, damage));
        if c.mob.is_dead() {
            self.deaths.push(c.mob.kind);
            c.mob.settle_loot(&self.wanted, &self.tomes);
        }
    }

    /// An ally casts a spell of the original at creature `target`: what it hurts and puts on it.
    fn ally_cast(&mut self, who: usize, spell: u32, target: Option<usize>) {
        let (Some(target), Some(sp)) = (target, content::classic_spell(spell)) else { return };
        let Some(c) = self.creatures.get(who) else { return };
        let level = c.mob.ally.as_ref().map_or(c.mob.def.level, |a| a.level);
        let mut rng = self.rng.next();
        let outcomes = crate::procs::effects_of(sp, level, &mut || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng
        });
        for o in outcomes {
            match o {
                crate::procs::Outcome::Hurt { amount, .. } => self.ally_hit(target, amount),
                crate::procs::Outcome::Put(status) => self.afflict(target, status),
                _ => {}
            }
        }
    }

    /// Where the living creatures of a type (1 beast ... 7 humanoid) are, within the reach of the creatures that are in the world.
    pub fn of_type(&self, kind: u32) -> Vec<Vec3> {
        self.creatures.iter().map(|c| &c.mob).filter(|m| m.attackable() && !m.is_dead() && m.def.kind == kind).map(|m| m.pos).collect()
    }

    /// The nearest hostile creature within `range` yards of `at` that is alive (what a trap waits for, and a totem shoots at).
    pub fn foe_near(&self, at: Vec3, range: f32) -> Option<usize> {
        (0..self.creatures.len())
            .filter(|&i| self.enemy_at(i).is_some_and(|p| flat(p - at).length() <= range))
            .min_by(|&a, &b| flat(self.creatures[a].mob.pos - at).length().total_cmp(&flat(self.creatures[b].mob.pos - at).length()))
    }

    /// Creature `i`, a beast not more than `top` levels high, is made into a pet: it is taken out of the world (a copy of it is the pet). Its kind's id, if it can be.
    pub fn tame(&mut self, i: usize, top: u32) -> Option<String> {
        self.enemy_at(i)?;
        let m = &self.creatures[i].mob;
        if m.def.kind != 1 || m.def.level > top {
            return None;
        }
        let id = m.def.id.clone();
        let c = &mut self.creatures[i];
        c.mob.health = 0.0;
        c.mob.loot = None;
        c.mob.die();
        if self.target == Some(i) {
            self.target = None;
        }
        Some(id)
    }

    /// The pet of the player's that is in the world, if there is one (alive or dead).
    pub fn pet(&self) -> Option<usize> {
        (0..self.creatures.len()).find(|&i| self.creatures[i].mob.is_pet())
    }

    /// The pet is sent away.
    pub fn dismiss_pet(&mut self) {
        for c in self.creatures.iter_mut().filter(|c| c.mob.is_pet()) {
            c.mob.state = State::Unborn;
        }
    }

    /// A dead pet is on its feet again, half whole. False if there is none that is dead.
    pub fn revive_pet(&mut self) -> bool {
        match self.pet().filter(|&i| self.creatures[i].mob.is_dead()) {
            Some(i) => {
                self.creatures[i].mob.revive_ally();
                true
            }
            None => false,
        }
    }

    /// The pet's orders, and whether it lies dead, if the player has one in the world (what the pet bar shows).
    pub fn pet_orders(&self) -> Option<(ally::PetOrders, bool)> {
        self.pet().map(|i| (self.orders, self.creatures[i].mob.is_dead()))
    }

    /// An order from the pet bar. Err is what is wrong with it.
    pub fn command_pet(&mut self, command: ally::PetCommand) -> Result<(), &'static str> {
        let i = self.pet().ok_or("You have no pet.")?;
        if self.creatures[i].mob.is_dead() {
            return Err("Your pet is dead.");
        }
        match command {
            ally::PetCommand::Attack => match self.target.filter(|&t| self.creatures.get(t).is_some_and(|c| c.mob.attackable() && !c.mob.is_dead())) {
                Some(t) => (self.orders.attack, self.orders.stay) = (Some(t), false),
                None => return Err("You have no target."),
            },
            ally::PetCommand::Follow => (self.orders.attack, self.orders.stay) = (None, false),
            ally::PetCommand::Stay => (self.orders.attack, self.orders.stay) = (None, true),
            ally::PetCommand::Stance(stance) => self.orders.stance = stance,
            // The spells are cast by the game (they need the character), not here.
            ally::PetCommand::Spell(_) => {}
        }
        Ok(())
    }

    /// The pet is healed by this many percent of what health it can have.
    pub fn mend_pet(&mut self, percent: f32) {
        if let Some(i) = self.pet() {
            let amount = self.creatures[i].mob.ally_max() * percent / 100.0;
            self.creatures[i].mob.mend(amount);
        }
    }

    /// The id of the pet's creature, if the player has one in the world (alive or dead).
    pub fn pet_id(&self) -> Option<String> {
        self.pet().map(|i| self.creatures[i].mob.def.id.clone())
    }

    /// The pet as it is, to be saved: its health, whether it lies dead, and its orders.
    pub fn pet_save(&self) -> Option<crate::save::PetSave> {
        let i = self.pet()?;
        let m = &self.creatures[i].mob;
        let stance = match self.orders.stance {
            ally::Stance::Passive => 0,
            ally::Stance::Defensive => 1,
            ally::Stance::Aggressive => 2,
        };
        let mana = m.ally.as_ref().map_or(1.0, |a| a.mana / a.mana_max.max(1.0));
        Some(crate::save::PetSave { health: m.health, dead: m.is_dead(), stance, stay: self.orders.stay, mana })
    }

    /// The pet just called is put as it was saved: its health, the corpse it lay as, and the orders it had.
    pub fn set_pet_save(&mut self, now: crate::save::PetSave) {
        let Some(i) = self.pet() else { return };
        self.orders.stance = match now.stance {
            0 => ally::Stance::Passive,
            2 => ally::Stance::Aggressive,
            _ => ally::Stance::Defensive,
        };
        self.orders.stay = now.stay;
        let m = &mut self.creatures[i].mob;
        if let Some(a) = &mut m.ally {
            a.mana = a.mana_max * now.mana.clamp(0.0, 1.0);
        }
        if now.dead {
            m.health = 0.0;
            m.die();
        } else {
            m.health = now.health.clamp(1.0, m.ally_max());
        }
    }

    /// The level of the pet's master that the pet has, if the player has a pet in the world.
    pub fn pet_level(&self) -> Option<u32> {
        self.pet().and_then(|i| self.creatures[i].mob.ally_level())
    }

    /// The pet's master went up to `level`: the pet has what the original gives a pet of it.
    pub fn set_pet_level(&mut self, level: u32) {
        if let Some(i) = self.pet() {
            self.creatures[i].mob.set_ally_level(level);
        }
    }

    /// The pet is told to cast `spell` of the original at the player (who stands at `to`): a spell that shifts it out of phase works on the pet itself. Err is why it
    /// cannot; Ok is whether the spell is to be put on the player (the pet paid for it and cast it).
    pub fn cast_pet_spell(&mut self, spell: u32, to: Vec3) -> Result<bool, &'static str> {
        let i = self.pet().ok_or("You have no pet.")?;
        let sp = content::classic_spell(spell).ok_or("Your pet cannot do that.")?;
        let m = &mut self.creatures[i].mob;
        if m.is_dead() {
            return Err("Your pet is dead.");
        }
        let Some(a) = &mut m.ally else { return Err("You have no pet.") };
        if a.cooldowns.iter().any(|c| c.0 == spell) {
            return Err("That spell is not ready yet.");
        }
        let on_pet = sp.effects.iter().any(|e| e.aura == 93);
        if !on_pet && flat(to - m.pos).length() > Mob::reach(sp) {
            return Err("Out of range.");
        }
        let cost = sp.mana_cost(a.mana_max);
        if a.mana < cost {
            return Err("Your pet does not have enough mana.");
        }
        (a.mana, a.spent) = (a.mana - cost, 0.0);
        if sp.cooldown > 0 {
            a.cooldowns.push((spell, sp.cooldown as f32 / 1000.0));
        }
        if on_pet {
            a.phased = true;
        }
        // It casts (the cast animation, the glow and the burst on whoever it is cast on).
        m.state = State::Cast(spell, 0.0, 0.0, true);
        let (from, yaw) = (m.pos, m.yaw);
        let height = Self::height_of(&self.creatures[i]);
        self.shows.push(Foe::Bolt { who: i, spell, from, yaw, height, to: if on_pet { from + Vec3::Y * height * 0.5 } else { to + Vec3::Y * 1.0 }, delay: 0.0 });
        Ok(!on_pet)
    }

    /// Whether the pet is out of phase with the world.
    pub fn pet_phased(&self) -> bool {
        self.pet().is_some_and(|i| self.creatures[i].mob.is_phased())
    }

    /// The pet casts this spell of the original from now on (it learned a better rank).
    pub fn set_pet_spell(&mut self, spell: u32) {
        if let Some(a) = self.pet().and_then(|i| self.creatures[i].mob.ally.as_mut()) {
            a.spell = spell;
        }
    }

    /// The pet's kind (the id of the creature) and where it is, if it is in the world and alive.
    pub fn pet_kind(&self) -> Option<(String, Vec3)> {
        self.pet().filter(|&i| !self.creatures[i].mob.is_dead()).map(|i| (self.creatures[i].mob.def.id.clone(), self.creatures[i].mob.pos))
    }

    /// Creature `i`, a hostile one, fights for the player for `seconds` (mind control, enslave); it is itself again when they are over.
    pub fn charm(&mut self, i: usize, seconds: f32, level: u32) -> bool {
        if self.enemy_at(i).is_none() {
            return false;
        }
        self.creatures[i].mob.make_ally(level, seconds, 0, false, true);
        if self.target == Some(i) {
            self.target = None;
        }
        true
    }

    /// A corpse near `from` (the nearest within `range` yards that was not raised before and has nothing left to loot) rises to fight for `left` seconds, half whole. Returns whether
    /// one did.
    pub fn raise(&mut self, from: Vec3, range: f32, level: u32, left: f32) -> bool {
        let found = self.nearest(from, range, |m| m.is_dead() && !m.is_ally() && !m.raised && m.def.attackable() && m.loot.as_ref().is_none_or(|l| l.is_empty()));
        let Some(i) = found else { return false };
        let (kind, at, yaw) = (self.creatures[i].mob.kind, self.creatures[i].mob.pos, self.creatures[i].mob.yaw);
        self.creatures[i].mob.raised = true;
        let mut mob = Mob::new(kind, at, yaw);
        mob.make_ally(level, left, 0, false, false);
        mob.revive_ally();
        self.alloc(Creature { mob, parent: None, site: None, visual: Visual::None, marks: None, want: None, ragdoll: None, was_dead: false, name_at: Vec3::Y * NAME_FALLBACK, hidden: std::cell::Cell::new((true, u8::MAX)), owed: 0.0, beat: 0 });
        true
    }

    /// What the player's spells do to creature `i` besides hurting it: break off its spell, take its auras of some kinds (`mask`) or some of its mana, rob it.
    pub fn interrupt(&mut self, i: usize) {
        if let Some(c) = self.creatures.get_mut(i) {
            c.mob.interrupt();
        }
    }

    pub fn purge(&mut self, i: usize, mask: u32) {
        if let Some(c) = self.creatures.get_mut(i) {
            c.mob.purge(mask);
        }
    }

    pub fn drain_mana(&mut self, i: usize, amount: f32) {
        if let Some(c) = self.creatures.get_mut(i) {
            c.mob.drain(amount);
        }
    }

    pub fn pickpocket(&mut self, i: usize) -> Option<u32> {
        self.creatures.get_mut(i).and_then(|c| c.mob.pickpocket())
    }

    /// Every creature that fights the player loses him (a vanish, a feigned death).
    pub fn lose_player(&mut self) {
        self.creatures.iter_mut().for_each(|c| c.mob.lose_player());
    }

    /// Put something on creature `i`.
    pub fn afflict(&mut self, i: usize, status: Status) {
        if let Some(c) = self.creatures.get_mut(i) {
            c.mob.afflict_from(status, &self.source.0, &self.source.1, self.source.2);
        }
    }

    /// What is put on creatures from now on comes from the spell with this name, icon and dispel kind (an empty icon: from none).
    pub fn set_source(&mut self, name: &str, icon: &str, dispel: u32) {
        self.source = (name.to_string(), icon.to_string(), dispel);
    }

    /// The ticks of damage over time that struck since the last call: where, and for how much (they float up as numbers).
    pub fn take_ticks(&mut self) -> Vec<(Vec3, u32)> {
        std::mem::take(&mut self.ticks).into_iter().map(|(at, damage)| (at, damage.round() as u32)).collect()
    }

    /// The creature that is fighting the player and is nearest to `feet` (what hits back at him is aimed at it), within a short reach.
    pub fn attacker(&self, feet: Vec3) -> Option<usize> {
        (0..self.creatures.len())
            .filter(|&i| self.creatures[i].mob.state.fights() && !self.creatures[i].mob.is_ally() && !self.creatures[i].mob.is_dead() && flat(self.creatures[i].mob.pos - feet).length() <= 12.0)
            .min_by(|&a, &b| flat(self.creatures[a].mob.pos - feet).length().total_cmp(&flat(self.creatures[b].mob.pos - feet).length()))
    }

    /// The level and the type of creature `i`, if it is a hostile one that is alive.
    pub fn enemy_info(&self, i: usize) -> Option<(u32, u32)> {
        self.enemy_at(i).map(|_| (self.creatures[i].mob.def.level, self.creatures[i].mob.def.kind))
    }

    /// A spell hurts every hostile creature within `radius` yards of `centre`, each for what `roll` gives (by its level and type). Returns where above
    /// each the damage shows, and how much it was.
    pub fn spell_area(&mut self, centre: Vec3, radius: f32, mut roll: impl FnMut(u32, u32) -> f32) -> Vec<(Vec3, f32)> {
        let near: Vec<usize> = (0..self.creatures.len()).filter(|&i| self.enemy_at(i).is_some_and(|p| flat(p - centre).length() <= radius)).collect();
        near.into_iter().filter_map(|i| {
            let damage = roll(self.creatures[i].mob.def.level, self.creatures[i].mob.def.kind);
            self.spell_hit(i, damage).map(|at| (at, damage))
        }).collect()
    }

    pub fn clear_target(&mut self) {
        self.target = None;
    }

    /// Select a creature (for previews).
    pub fn set_target(&mut self, creature: usize) {
        self.target = (creature < self.creatures.len()).then_some(creature);
    }

    /// Tab: select the nearest living hostile creature ahead of `from` (facing `yaw`) within range; again, the next
    /// nearest. Nothing changes when there is none.
    pub fn cycle_target(&mut self, from: Vec3, yaw: f32) {
        let ahead = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
        let mut near: Vec<(f32, usize)> = (0..self.creatures.len())
            .filter_map(|i| {
                let m = &self.creatures[i].mob;
                let to = flat(m.pos - from);
                let d = to.length();
                (m.attackable() && !m.is_dead() && d <= TARGET_RANGE && (d < 2.0 || to.normalize().dot(ahead) > 0.0)).then_some((d, i))
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let next = match self.target.and_then(|t| near.iter().position(|&(_, i)| i == t)) {
            Some(at) => near[(at + 1) % near.len()].1,
            None => match near.first() {
                Some(&(_, i)) => i,
                None => return,
            },
        };
        self.target = Some(next);
    }

    /// What the target frame shows. A target that went out of sight (further than `from` can see) is let go.
    pub fn target_view(&mut self, from: Vec3) -> Option<TargetView> {
        let i = self.target?;
        let c = self.creatures.get(i)?;
        if flat(c.mob.pos - from).length() > TARGET_LOSE {
            self.target = None;
            return None;
        }
        let m = &c.mob;
        let (debuffs, buffs) = m.shown_auras();
        Some(TargetView { id: i, name: m.def.name.clone(), level: m.def.level, fraction: (m.health / m.def.health).clamp(0.0, 1.0), dead: m.is_dead(), hostile: (m.def.is_hostile() || m.forced_hostile) && !m.is_ally(), neutral: m.def.is_neutral() && !m.is_ally(), debuffs, buffs })
    }

    /// A camera looking at the head of a creature and the entities to draw, for the portrait of the target frame.
    pub fn portrait_shot(&self, creature: usize) -> Option<(Camera, Vec<usize>)> {
        let c = self.creatures.get(creature)?;
        let (distance, height, entities) = match (&c.visual, &c.mob.def.body) {
            (Visual::Person(p), _) => (1.25 * p.tall(), 1.84 * p.tall(), p.worn.entities()),
            (Visual::Beast { entity, .. }, Body::Beast { scale, .. }) => (1.9 * scale, 0.85 * scale, vec![*entity]),
            (Visual::Beast { entity, .. }, _) => (1.9, 0.85, vec![*entity]),
            (Visual::None, _) => return None,
        };
        // In front of the creature, looking back at it.
        let front = c.mob.yaw + std::f32::consts::PI;
        let (sf, cf) = (front.sin(), front.cos());
        let mut camera = Camera::new(c.mob.pos + Vec3::new(-sf * distance, height, cf * distance), front, 0.0);
        (camera.fov_y, camera.aspect, camera.near, camera.far) = (30f32.to_radians(), 1.0, 0.1, 40.0);
        Some((camera, entities))
    }

    /// Take every creature out of the world, giving its entities back: the world is made again from new content.
    pub fn retire(self, renderer: &mut Renderer) {
        for c in self.creatures {
            match c.visual {
                Visual::Beast { entity, .. } => renderer.remove_entity(entity),
                Visual::Person(p) => p.retire(renderer),
                Visual::None => {}
            }
            c.marks.into_iter().flatten().for_each(|e| renderer.remove_entity(e));
        }
    }

    /// Say which tomes a kill may drop now (every frame, before anything dies).
    pub fn set_tomes(&mut self, tomes: Vec<usize>) {
        self.tomes = tomes;
    }

    /// Say which quest items are wanted by the quests being worked on (every frame, before anything dies).
    pub fn set_wanted(&mut self, wanted: Vec<usize>) {
        self.wanted = wanted;
    }

    /// The kinds of the creatures that died since the last call (for the kill goals of quests).
    pub fn take_deaths(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.deaths)
    }

    /// Say which quest mark each creature (by its kind) shows over its head now, if any, and where the marks are looked at
    /// from (they turn to face the viewer).
    pub fn set_markers(&mut self, viewer: Vec3, marker: impl Fn(usize) -> Option<Marker>) {
        self.viewer = viewer;
        for c in &mut self.creatures {
            c.want = if c.mob.is_dead() { None } else { marker(c.mob.kind) };
        }
    }

    /// The nearest creature within `range` yards of `from` that passes `test`.
    fn nearest(&self, from: Vec3, range: f32, test: impl Fn(&Mob) -> bool) -> Option<usize> {
        let dist = |c: &Creature| flat(c.mob.pos - from).length();
        (0..self.creatures.len()).filter(|&i| dist(&self.creatures[i]) <= range && test(&self.creatures[i].mob)).min_by(|&a, &b| dist(&self.creatures[a]).total_cmp(&dist(&self.creatures[b])))
    }

    /// The nearest corpse within `range` yards of `from` that still has something to loot.
    pub fn lootable_near(&self, from: Vec3, range: f32, seen: impl Fn(Vec3) -> bool) -> Option<usize> {
        self.nearest(from, range, |m| m.is_dead() && m.loot.is_some() && seen(m.pos))
    }

    /// The nearest corpse within `range` yards of `from` that can be skinned: nothing left to loot, not skinned yet, and a beast that gives a hide.
    pub fn skinnable_near(&self, from: Vec3, range: f32, seen: impl Fn(Vec3) -> bool) -> Option<usize> {
        self.nearest(from, range, |m| m.is_dead() && m.loot.is_none() && !m.skinned && content::get().skins.contains_key(&m.def.id) && seen(m.pos))
    }

    /// What a corpse gives to a knife: its table of skinning and its level, if it can be skinned.
    pub fn skin_of(&self, creature: usize) -> Option<(&'static str, u32)> {
        let m = &self.creatures.get(creature)?.mob;
        (m.is_dead() && m.loot.is_none() && !m.skinned).then(|| content::get().skins.get(&m.def.id).map(|t| (t.as_str(), m.def.level)))?
    }

    /// The hide was taken from the corpse.
    pub fn mark_skinned(&mut self, creature: usize) {
        if let Some(c) = self.creatures.get_mut(creature) {
            c.mob.skinned = true;
        }
    }

    /// The nearest living creature within `range` yards of `from` that can be talked to (one that is not hostile) and that
    /// `seen` accepts (given where it stands: the player faces it, with no wall between).
    pub fn talkable_near(&self, from: Vec3, range: f32, seen: impl Fn(Vec3) -> bool) -> Option<usize> {
        self.nearest(from, range, |m| !m.def.attackable() && !m.is_ally() && !m.is_dead() && seen(m.pos))
    }

    pub fn def(&self, creature: usize) -> Option<&'static CreatureDef> {
        self.creatures.get(creature).map(|c| c.mob.def)
    }

    /// Turn a creature to face `target` (feet position).
    pub fn face(&mut self, creature: usize, target: Vec3) {
        if let Some(c) = self.creatures.get_mut(creature) {
            let to = flat(target - c.mob.pos);
            if to.length() > 0.01 {
                c.mob.yaw = to.x.atan2(-to.z);
            }
        }
    }

    pub fn loot(&self, creature: usize) -> Option<&Loot> {
        self.creatures.get(creature)?.mob.loot.as_ref()
    }

    pub fn corpse_pos(&self, creature: usize) -> Option<Vec3> {
        self.creatures.get(creature).map(|c| c.mob.pos)
    }

    /// Leave `loot` on the corpse (what is left after looting); an empty one empties the corpse.
    pub fn set_loot(&mut self, creature: usize, loot: Loot) {
        if let Some(c) = self.creatures.get_mut(creature).filter(|c| c.mob.is_dead()) {
            c.mob.loot = (!loot.is_empty()).then_some(loot);
        }
    }

    /// Whether any creature is after the player (chasing, striking or flinching): the player is in combat.
    pub fn engaged(&self) -> bool {
        self.creatures.iter().any(|c| c.mob.state.fights() && !c.mob.is_ally())
    }

    /// Who is after the player, for the log: the kind, the state and how far from the player.
    pub fn engaged_by(&self, player: Vec3) -> Vec<String> {
        self.creatures.iter().filter(|c| c.mob.state.fights() && !c.mob.is_ally()).map(|c| format!("{} {:?} at {:.1} yards (home {:.1})", c.mob.def.name, c.mob.state, flat(player - c.mob.pos).length(), flat(player - c.mob.home).length())).collect()
    }

    /// Where a point is on a `size` screen seen through `camera` (pixels), if it is in front of it.
    fn on_screen(camera: &Camera, size: (u32, u32), at: Vec3) -> Option<(f32, f32)> {
        let clip = camera.view_proj_relative() * (at - camera.pos).extend(1.0);
        (clip.w > 0.1).then(|| ((clip.x / clip.w * 0.5 + 0.5) * size.0 as f32, (0.5 - clip.y / clip.w * 0.5) * size.1 as f32))
    }

    /// The creature under the pointer at `pos` (pixels) on a `size` screen seen through `camera`, and the pointer that suits it:
    /// the sword over an enemy, a bag over a corpse with loot or a vendor, a speech bubble over someone with something to say.
    pub fn pick(&self, camera: &Camera, size: (u32, u32), pos: (f32, f32)) -> Option<(usize, Kind)> {
        (0..self.creatures.len())
            .filter_map(|i| {
                let c = &self.creatures[i];
                let m = &c.mob;
                if matches!(c.visual, Visual::None) {
                    return None;
                }
                let kind = if m.is_dead() {
                    if m.loot.is_some() {
                        Kind::LootAll
                    } else {
                        (!m.skinned && content::get().skins.contains_key(&m.def.id)).then_some(Kind::Skin)?
                    }
                } else if m.attackable() {
                    Kind::Attack
                } else if !m.def.vendor.is_empty() {
                    Kind::Buy
                } else {
                    m.def.gossip.is_some().then_some(Kind::Speak)?
                };
                // About as tall as it is: a person two yards, a beast by its scale, a corpse lying low.
                let (height, width) = match (&c.visual, &m.def.body) {
                    (Visual::Person(p), _) => (HUMAN_HEIGHT * p.tall(), 0.25),
                    (_, Body::Beast { scale, .. }) => (*scale, 0.8),
                    _ => (1.0, 0.8),
                };
                let [x0, y0, x1, y1] = screen_box(camera, size, m.pos, if m.is_dead() { height * 0.4 } else { height }, width)?;
                (pos.0 >= x0 && pos.0 <= x1 && pos.1 >= y0 && pos.1 <= y1).then(|| ((m.pos - camera.pos).length(), i, kind))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i, kind)| (i, kind))
    }

    /// The most any creature is alert to the player at `feet` (see `Mob::alert`).
    pub fn alert(&self, feet: Vec3, level: u32) -> f32 {
        self.creatures.iter().map(|c| c.mob.alert(feet, level)).fold(0.0, f32::max)
    }

    /// The names floating over creatures: red for hostile ones, yellow for neutral ones, green for friendly ones.
    /// Like the original's unit names (`FUN_006070a0`) they show on the target and, with `npc_names`, on every creature. They are text standing in the
    /// world at the model's name attachment: they shrink with distance and a wall in between hides them (`blocked`).
    pub fn labels(&self, camera: &Camera, size: (u32, u32), npc_names: bool, blocked: impl Fn(Vec3, Vec3) -> bool) -> Vec<Label> {
        let focal = size.1 as f32 / 2.0 / (camera.fov_y / 2.0).tan();
        let mut looks = LOOKS_PER_FRAME;
        self.creatures
            .iter()
            .enumerate()
            .filter(|(i, c)| !matches!(c.visual, Visual::None) && !c.mob.is_dead() && (npc_names || self.target == Some(*i)))
            .filter_map(|(_, c)| {
                let head = c.mob.pos + c.name_at;
                let (x, y) = Self::on_screen(camera, size, head)?;
                if x < 0.0 || y < 0.0 || x > size.0 as f32 || y > size.1 as f32 {
                    return None;
                }
                let depth = (camera.view_proj_relative() * (head - camera.pos).extend(1.0)).w;
                let em = if c.name_at.y > 4.0 { c.name_at.y / 4.0 * 1.5 * NAME_EM } else { NAME_EM };
                let line = em * focal / depth;
                // Under a pixel it shows nothing.
                if line < 1.0 {
                    return None;
                }
                let (hidden, age) = c.hidden.get();
                if age >= LOOK_EVERY && looks > 0 {
                    looks -= 1;
                    c.hidden.set((blocked(camera.pos, head), 0));
                } else {
                    c.hidden.set((hidden, age.saturating_add(1)));
                }
                if c.hidden.get().0 {
                    return None;
                }
                let color = c.reaction();
                Some(Label { text: c.mob.def.name.clone(), sub: c.mob.def.title.as_ref().filter(|t| !t.is_empty()).map(|t| format!("<{t}>")), color, x, y, line })
            })
            .collect()
    }

    /// Where the selected creature stands, the radius of its circle on the ground (`selection::radius` of its model) and the colour of its reaction.
    pub fn target_circle(&self) -> Option<(Vec3, f32, [f32; 3])> {
        let c = self.creatures.get(self.target?)?;
        let radius = match &c.visual {
            Visual::Person(p) => crate::selection::radius(p.character.m2.bounds_min, p.character.m2.bounds_max, p.tall()),
            Visual::Beast { shown, .. } => {
                let rig = self.rigs[*shown].as_ref()?;
                crate::selection::radius(rig.m2.bounds_min, rig.m2.bounds_max, rig.scale)
            }
            Visual::None => return None,
        };
        let [r, g, b, _] = c.reaction();
        Some((c.mob.pos, radius, [r, g, b]))
    }

    /// Health bars over the hostile creatures that are hurt, drawn on a `size` screen seen through `camera`.
    pub fn draw_plates(&self, ui: &mut Ui, camera: &Camera, size: (u32, u32)) {
        let h = size.1 as f32;
        for c in self.creatures.iter().filter(|c| !c.mob.is_dead() && c.mob.health < c.mob.def.health) {
            let Some((x, y)) = Self::on_screen(camera, size, c.mob.pos + Vec3::Y * 1.7) else { continue };
            let (bar_w, bar_h) = (60.0 * h / 768.0, 7.0 * h / 768.0);
            let fill = (c.mob.health / c.mob.def.health).clamp(0.0, 1.0);
            let mut quad = |rect: [f32; 4], color: [f32; 4]| ui.quad(Quad { texture: WHITE, rect, uv: FULL_UV, color, additive: false });
            quad([x - bar_w / 2.0 - 1.0, y - 1.0, bar_w + 2.0, bar_h + 2.0], [0.0, 0.0, 0.0, 0.85]);
            quad([x - bar_w / 2.0, y, bar_w * fill, bar_h], [0.2 + 0.6 * (1.0 - fill), 0.8 * fill + 0.1, 0.1, 1.0]);
        }
    }
}
