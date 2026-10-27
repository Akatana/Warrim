mod actionbar;
mod anim;
mod animator;
mod app;
mod audio;
mod brain;
mod camera;
mod cinematic;
mod casting;
mod special;
mod chest;
mod character;
mod collision;
mod consume;
mod combattext;
mod compass;
mod glue;
mod console;
mod cursor;
mod coords;
mod creature;
mod editor;
mod gallery;
mod effects;
mod fires;
mod foes;
mod font;
mod items;
mod levels;
mod loading;
mod headless;
mod models;
mod particles;
mod petbar;
mod keys;
mod lifts;
mod physics;
mod ragdoll;
mod nodes;
mod fishing;
mod pickups;
mod recipes;
mod procs;
mod profile;
mod preview;
mod propedit;
mod lockpick;
mod loot;
mod movement;
mod content;
mod panels;
mod quests;
mod player;
mod renderer;
mod ripples;
mod selection;
mod save;
mod skills;
mod talents;
mod steed;
mod interface;
mod video;
mod visuals;
mod sky;
mod clouds;
mod skygpu;
mod weather;
mod weathergpu;
mod heavens;
mod terrain;
mod ui;
mod uixml;
mod wmovis;
mod world;
mod worn;

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::sync::Arc;

const USAGE: &str = "\
rsworld - explore the WoW 1.12 world offline

USAGE: rsworld [options]

  --wow <dir>        WoW install containing Data/*.MPQ (default: auto-detect / $WOW_DIR)
  --map <name>       map to load (default Azeroth; try Kalimdor)
  --pos <x,y[,z]>    start position in WoW coordinates (x north, y west).
                     Default: Northshire Abbey. Omit z to stand on the ground.
  --yaw <deg>        initial heading, 0 = north, 90 = east (default 200)
  --pitch <deg>      initial pitch (default -8)
  --radius <tiles>   tiles to keep loaded around you (default 2)
  --fly              start in free-fly mode (toggle in game with F)
  --headless <png>   render one frame to a PNG and exit (no window)
  --alt <yards>      headless: camera height above the ground (default 3)
  --floor-map        headless: print an ASCII map of floors above the terrain around the camera
  --probe-walk       headless: walk from a grid of starts/headings around --pos; report runs that end on raised floors
  --find-ramps       headless: list clusters of 35-50 degree collision surfaces within 40 yd of --pos
  --floor-profile    headless: print floor height along a line from --pos heading --yaw for 20 yd
  --dump-tris        headless: print collision triangles within 3 yd of --pos (WoW coordinates)
  --stress-walk      headless: steer randomly from random starts near --pos and report falls after climbing
  --floor-scale <yd> headless: yards per character for --floor-map (default 1)
  --find-holes       headless: find gaps in the walkable surface (no floor at --pos height but floor on both sides)
  --walk <seconds>   headless: walk forward (heading = --yaw) with real collision before the shot
  --lifts <seconds>  headless: the lifts, ships and zeppelins of the map as they are this many seconds into the game
  --time <hours>     fix the time of day (0-24); otherwise it starts at 12 and runs 1 game minute per second
  --view <first|third>  headless: draw the player (character, weapon and shield) in this view
  --pose <name>      headless, with --view: stand (default), walk, run, jump, attack, block or sheath
  --pose-turn <deg>  headless: turn the body this far from the view direction (to see it from the side)
  --pose-time <s>    headless: seconds the pose has been playing (default 0.4)
  --size <WxH>       headless image size (default 1280x720)
  --character <name> play this character of the character list, without the list (a new one is made by the list)
  --race <name>      headless: the player's race (human, orc, dwarf, nightelf, undead, tauren, gnome, troll); --female for a woman
  --new              start a new game instead of loading the saved one (the old save is replaced when the game ends)
  --no-save          neither load nor write a save (and show no character list)
  --save <file>      play the character kept in this file, without the character list (the list keeps its characters in rsworld/characters/ in the application data folder)
  --no-audio         play no music, ambience or sound effects
  --weather <kind[,strength]>  fix the weather: rain, snow, sand or none, with a strength of 0 to 1 (default 0.8); otherwise it comes and goes as the server's weather of the zone does
  --no-weather       no weather at all
  --loading <0-1>    headless: draw the loading screen of --map with the bar that full
  --show <frames>    headless: draw these original interface frames (comma separated, the first is built, e.g. CharacterFrame,PaperDollFrame)
  --intro <race>     show the introduction of this race (what a new character of it sees), then quit
  --cinema <race>[,s]  headless: the fly-by of this race's introduction, s seconds into it
  --glue             headless, with --show: the frames come from the screens before the game (e.g. CharacterCreateFrame)
  --glue-screen <select|create>  headless: the character list or the creation, with --clicks x,y;x,y on it, --glue-type text for the name, --glue-dir for the folder of characters
  --panel <names>    headless, with --view: open the character sheet and/or backpack (character,bag,loot,gossip,trainer,merchant,quest,quest-progress,quest-reward,tracker,log,abandon,levelup,levelhint,taken,ready,lockpick; error and hint preview the messages; breath the breath bar)
  --clicks <x,y;x,y>  headless: left-click these pixel positions on the open panels, in order (a negative x is a right click)
  --equip <slots>    headless: wear the items in these backpack slots first (0-based, e.g. 1,2)
  --cursor <x,y>     headless: the mouse position in pixels for tooltips
  --ripples <idle|walk|run>  headless, with --view: the ripples of a character standing, walking or running in knee-deep water at the position
  --levelfx <ms>     headless, with --view: the level up effect at this many milliseconds into it (it lasts 1867)
  --chests <open|closed>  headless, with --view: the chests of the world, with their lids open or shut
  --reticle          headless: the aiming dot at the middle of the screen (with --sneak, as the eye of a sneaking player; --alert 0..1 sets how near they are to noticing)
  --check-spawns     headless: warn about content (creatures, chests) standing in a wall or tree
  --effect model,ms  headless: a model of effects (a file of Spells) ms after it began, six yards ahead
  --cast a,b         headless, with --creature or --spawns: cast these spells (ids of content/spells.ron) at the first creature
  --editor tab,n,steps  headless: the editor at a tab (places, chests, discoveries, creatures, quests, items or loot) with entry n selected;
                     steps: x:y clicks at a pixel, t:text types it (the window of a ... button, its search)
  --apply-after s    take the content files into use live after s seconds, as the editor Apply does (to try it)
  --wear a,b         headless: put on the items with these ids (all levels), for the player previews
  --kill             headless, with --creature: the creature straight ahead dies
  --spawns           headless: every creature of content/spawns.ron
  --profile          show the profiler (F3 toggles it) and write its table to the log every second
  --creature a,b     headless: these creatures (ids of content/creatures.ron) stand in a row ahead
  --compass          headless: the compass at the top (with --discover N places already found)
  --sneak            headless, with --view: the character sneaks (crouched, with the sneaking icon)
  --mount <display>[,<percent>]  headless, with --view: the character rides the steed of this CreatureDisplayInfo row (default speed bonus 60)
  --target           headless, with --wolf or --npc: the first creature is the target (the target frame)
  --level <n>        headless, with --view: the character is of level n (its stats and experience bar)
  --xp <n>           headless: experience gathered towards the next level
  --npc              headless, with --view: Deputy Willem stands 5 yards ahead, 2 to the left, facing the player
  --wolf             headless, with --view: a wolf stands 4 yards ahead, 1.5 to the right, and goes for the player

CONTROLS  click = capture mouse, Esc = release/quit, WASD move, Space jump/up,
          Ctrl down (fly), Shift fast (fly) / walk (ground), F fly<->walk, wheel = fly speed,
          V first<->third person, R sheathe/draw weapons, left click = attack, hold right click = block
";

pub struct Args {
    pub wow_dir: Option<PathBuf>,
    pub map: String,
    /// WoW coordinates; `None` for z means "on the ground".
    pub pos: (f32, f32, Option<f32>),
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub radius: i32,
    pub fly: bool,
    pub headless: Option<PathBuf>,
    pub alt: f32,
    pub walk: f32,
    pub lifts: Option<f32>,
    pub floor_map: bool,
    pub floor_scale: f32,
    pub probe_walk: bool,
    pub find_ramps: bool,
    pub dump_tris: bool,
    pub stress_walk: bool,
    pub find_holes: bool,
    pub floor_profile: bool,
    pub size: (u32, u32),
    pub time: Option<f32>,
    /// `--weather`: fixed weather (kind, strength 0 to 1), `None` kind for clear; otherwise it comes and goes as the server's weather of the zone does.
    pub weather: Option<(Option<weather::Kind>, f32)>,
    pub no_weather: bool,
    pub audio: bool,
    pub new_game: bool,
    /// The character to play by its name (the list is not shown), and how the player looks (a character's own look is in its save).
    pub character: Option<String>,
    pub look: character::Appearance,
    pub save: bool,
    pub save_path: Option<std::path::PathBuf>,
    pub view: Option<String>,
    pub pose: String,
    pub pose_time: f32,
    pub pose_turn: f32,
    pub wolf: bool,
    pub npc: bool,
    pub target: bool,
    pub sneak: bool,
    pub mount: Option<(u32, i32)>,
    pub compass: bool,
    pub creatures: Vec<String>,
    /// How far ahead `--creature` stands, yards.
    pub creature_at: f32,
    pub spawns: bool,
    pub kill: bool,
    pub editor: Option<(String, usize, Vec<String>)>,
    pub apply_after: Option<f32>,
    pub cast: Vec<String>,
    pub effect: Option<(String, f32)>,
    pub check_spawns: bool,
    pub discover: usize,
    pub reticle: bool,
    pub alert: Option<f32>,
    pub chests: Option<String>,
    pub levelfx: Option<f32>,
    pub ripples: Option<String>,
    pub level: Option<u32>,
    pub xp: u32,
    pub panel: Vec<String>,
    pub cursor: Option<(f32, f32)>,
    pub clicks: Vec<(f32, f32)>,
    pub equip: Vec<usize>,
    pub wear: Vec<String>,
    pub show: Vec<String>,
    pub glue: bool,
    pub intro: Option<character::Race>,
    pub cinema: Option<(character::Race, f32)>,
    pub fov: Option<f32>,
    pub glue_screen: Option<String>,
    pub glue_dir: Option<PathBuf>,
    pub glue_type: String,
    pub loading: Option<f32>,
}

/// Where the game starts when the command line says nothing: the human starting area, Northshire Valley (a new character of another race starts in its own homeland).
pub const START_POS: (f32, f32, Option<f32>) = (-8949.95, -132.49, None);

fn parse_args() -> Result<Args> {
    let mut a = Args {
        wow_dir: None,
        map: "Azeroth".into(),
        // Human starting area, Northshire Valley.
        pos: START_POS,
        yaw_deg: 200.0,
        pitch_deg: -8.0,
        radius: 2,
        fly: false,
        headless: None,
        alt: 3.0,
        walk: 0.0,
        lifts: None,
        floor_map: false,
        floor_scale: 1.0,
        probe_walk: false,
        find_ramps: false,
        dump_tris: false,
        stress_walk: false,
        find_holes: false,
        floor_profile: false,
        size: (1280, 720),
        time: None,
        weather: None,
        no_weather: false,
        audio: true,
        new_game: false,
        character: None,
        look: character::Appearance::default(),
        save: true,
        save_path: None,
        view: None,
        pose: "stand".into(),
        pose_time: 0.4,
        pose_turn: 0.0,
        wolf: false,
        npc: false,
        target: false,
        sneak: false,
        mount: None,
        compass: false,
        creatures: Vec::new(),
        creature_at: 6.0,
        spawns: false,
        kill: false,
        editor: None,
        apply_after: None,
        cast: Vec::new(),
        effect: None,
        check_spawns: false,
        discover: 0,
        reticle: false,
        alert: None,
        chests: None,
        levelfx: None,
        ripples: None,
        level: None,
        xp: 0,
        panel: Vec::new(),
        cursor: None,
        clicks: Vec::new(),
        equip: Vec::new(),
        wear: Vec::new(),
        show: Vec::new(),
        glue: false,
        intro: None,
        cinema: None,
        fov: None,
        glue_screen: None,
        glue_dir: None,
        glue_type: String::new(),
        loading: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().with_context(|| format!("{name} needs a value"));
        match flag.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--wow" => a.wow_dir = Some(value("--wow")?.into()),
            "--map" => a.map = value("--map")?,
            "--pos" => {
                let v = value("--pos")?;
                let n: Vec<f32> = v.split(',').map(|s| s.trim().parse()).collect::<Result<_, _>>()?;
                a.pos = match n.as_slice() {
                    [x, y] => (*x, *y, None),
                    [x, y, z] => (*x, *y, Some(*z)),
                    _ => bail!("--pos expects x,y or x,y,z"),
                };
            }
            "--yaw" => a.yaw_deg = value("--yaw")?.parse()?,
            "--pitch" => a.pitch_deg = value("--pitch")?.parse()?,
            "--radius" => a.radius = value("--radius")?.parse::<i32>()?.clamp(1, 6),
            "--fly" => a.fly = true,
            "--headless" => a.headless = Some(value("--headless")?.into()),
            "--alt" => a.alt = value("--alt")?.parse()?,
            "--find-holes" => a.find_holes = true,
            "--stress-walk" => a.stress_walk = true,
            "--dump-tris" => a.dump_tris = true,
            "--find-ramps" => a.find_ramps = true,
            "--floor-profile" => a.floor_profile = true,
            "--probe-walk" => a.probe_walk = true,
            "--floor-scale" => a.floor_scale = value("--floor-scale")?.parse()?,
            "--floor-map" => a.floor_map = true,
            "--walk" => a.walk = value("--walk")?.parse()?,
            "--lifts" => a.lifts = Some(value("--lifts")?.parse()?),
            "--no-audio" => a.audio = false,
            "--no-weather" => a.no_weather = true,
            "--weather" => {
                let text = value("--weather")?;
                let (name, strength) = text.split_once(',').map_or((text.as_str(), "0.8"), |(n, s)| (n, s));
                let kind = match name {
                    "none" | "clear" => None,
                    other => Some(weather::Kind::parse(other).ok_or_else(|| anyhow::anyhow!("--weather: rain, snow, sand or none, not {other}"))?),
                };
                a.weather = Some((kind, strength.parse()?));
            }
            "--new" => a.new_game = true,
            "--character" => a.character = Some(value("--character")?),
            "--race" => {
                let name = value("--race")?;
                a.look.race = character::Race::parse(&name).with_context(|| format!("--race: no race {name}"))?;
            }
            "--female" => a.look.sex = character::Sex::Female,
            "--no-save" => a.save = false,
            "--save" => a.save_path = Some(value("--save")?.into()),
            "--wolf" => a.wolf = true,
            "--npc" => a.npc = true,
            "--target" => a.target = true,
            "--sneak" => a.sneak = true,
            "--mount" => {
                let v = value("--mount")?;
                let (display, percent) = v.split_once(',').unwrap_or((&v, "60"));
                a.mount = Some((display.parse()?, percent.parse()?));
            }
            "--compass" => a.compass = true,
            "--spawns" => a.spawns = true,
            "--profile" => {
                profile::set_enabled(true);
                profile::log_to_file(true);
            }
            "--kill" => a.kill = true,
            "--apply-after" => a.apply_after = Some(value("--apply-after")?.parse()?),
            "--editor" => {
                let v = value("--editor")?;
                let mut parts = v.split(',').map(String::from);
                let tab = parts.next().unwrap_or_default();
                let n = parts.next().map_or(Ok(0), |n| n.parse())?;
                a.editor = Some((tab, n, parts.collect()));
            }
            "--cast" => a.cast = value("--cast")?.split(',').map(String::from).collect(),
            "--effect" => {
                let v = value("--effect")?;
                let (path, ms) = v.rsplit_once(',').unwrap_or((&v, "400"));
                a.effect = Some((path.to_string(), ms.parse()?));
            }
            "--check-spawns" => a.check_spawns = true,
            "--creature" => a.creatures = value("--creature")?.split(',').map(String::from).collect(),
            "--creature-at" => a.creature_at = value("--creature-at")?.parse()?,
            "--discover" => a.discover = value("--discover")?.parse()?,
            "--reticle" => a.reticle = true,
            "--alert" => a.alert = Some(value("--alert")?.parse::<f32>()?.clamp(0.0, 1.0)),
            "--chests" => a.chests = Some(value("--chests")?),
            "--ripples" => a.ripples = Some(value("--ripples")?),
            "--levelfx" => a.levelfx = Some(value("--levelfx")?.parse()?),
            "--level" => a.level = Some(value("--level")?.parse()?),
            "--xp" => a.xp = value("--xp")?.parse()?,
            "--panel" => a.panel = value("--panel")?.split(',').map(String::from).collect(),
            "--wear" => a.wear = value("--wear")?.split(',').map(String::from).collect(),
            "--equip" => a.equip = value("--equip")?.split(',').map(|v| v.parse()).collect::<Result<_, _>>()?,
            "--clicks" => {
                for pair in value("--clicks")?.split(';') {
                    let (x, y) = pair.split_once(',').context("--clicks wants x,y;x,y")?;
                    a.clicks.push((x.parse()?, y.parse()?));
                }
            }
            "--cursor" => {
                let v = value("--cursor")?;
                let (x, y) = v.split_once(',').context("--cursor wants x,y")?;
                a.cursor = Some((x.parse()?, y.parse()?));
            }
            "--glue" => a.glue = true,
            "--intro" => {
                let name = value("--intro")?;
                a.intro = Some(character::Race::parse(&name).with_context(|| format!("--intro: no race {name}"))?);
            }
            "--cinema" => {
                let v = value("--cinema")?;
                let (name, s) = v.split_once(',').unwrap_or((&v, "0"));
                a.cinema = Some((character::Race::parse(name).with_context(|| format!("--cinema: no race {name}"))?, s.parse()?));
            }
            "--glue-screen" => a.glue_screen = Some(value("--glue-screen")?),
            "--glue-dir" => a.glue_dir = Some(value("--glue-dir")?.into()),
            "--glue-type" => a.glue_type = value("--glue-type")?,
            "--show" => a.show = value("--show")?.split(',').map(String::from).collect(),
            "--loading" => a.loading = Some(value("--loading")?.parse()?),
            "--view" => a.view = Some(value("--view")?),
            "--pose" => a.pose = value("--pose")?,
            "--pose-turn" => a.pose_turn = value("--pose-turn")?.parse()?,
            "--pose-time" => a.pose_time = value("--pose-time")?.parse()?,
            "--time" => a.time = Some(value("--time")?.parse::<f32>()?.rem_euclid(24.0)),
            "--size" => {
                let v = value("--size")?;
                let (w, h) = v.split_once('x').context("--size expects WxH")?;
                a.size = (w.parse()?, h.parse()?);
            }
            other => bail!("unknown option '{other}'\n\n{USAGE}"),
        }
    }
    Ok(a)
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"))
        .init();
    let args = parse_args()?;
    let wow = args
        .wow_dir
        .clone()
        .or_else(mpq::find_wow_dir)
        .context("could not find the WoW data (run from inside the WoW folder, pass --wow, or set WOW_DIR)")?;
    log::info!("game data: {}", wow.display());
    let mount = Arc::new(mpq::Mount::open_wow_dir(&wow)?);
    content::init(&mount)?;

    // The view of a fly-by: its map, and the camera at the moment asked for.
    let mut args = args;
    if let Some((race, seconds)) = args.cinema {
        let flyby = cinematic::Flyby::load(&mount, race)?;
        let camera = flyby.camera_at(seconds * 1000.0, args.size.0 as f32 / args.size.1 as f32);
        let [north, west, _] = coords::render_to_wow(camera.pos);
        (args.map, args.pos, args.yaw_deg, args.pitch_deg, args.fov) = (cinematic::map_of(race).into(), (north, west, Some(camera.pos.y)), camera.yaw.to_degrees(), camera.pitch.to_degrees(), Some(camera.fov_y));
    }
    match args.headless.clone() {
        Some(out) => headless::run(&args, mount, &out),
        None => app::run(args, mount),
    }
}
