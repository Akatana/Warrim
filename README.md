# Warrim

Warrim is an offline, single-player client written in Rust that loads the **original WoW 1.12.1 (build 5875) data files**
and lets you walk around the world. It is an MVP: terrain, textures, animated water, trees and props (M2 models),
buildings and caves (WMOs), zone names and walking physics with collision work (walls, stairs, ledges, slopes, standing
on object surfaces). There are no characters or creatures and no gameplay yet.

No server code: there is no login, realm list or network layer by design.

## Run it

Needs Rust (stable) and a GPU with Vulkan / DX12 / Metal. The game data is **not** included; it is read from your own
install (this folder already sits inside one, and the Data folder is found automatically).

```powershell
cd rsworld
cargo run --release -p client                  # Northshire Valley, walking
cargo run --release -p client -- --fly         # free-fly camera
cargo run --release -p client -- --map Kalimdor --pos -618,-4251    # Durotar
cargo run --release -p client -- --help
```

Start: the original's character list (`GlueXML`) comes first; **Create New Character** is the original's creation screen (race, sex, skin, face, hair, facial hair, name; no classes). Each character is its own save, `%APPDATA%
sworld\characters\<Name>.ron` (a deleted one is renamed `.ron.deleted`); the old `save.ron` became the character Adventurer. A new character first sees the introduction of its race, the original's fly-by over its homeland with the narrator (any key or click skips it; `--intro <race>` plays one by itself). `--character <name>` skips the list, the console command `logout` returns to it.

Controls: `C` character sheet, `B` backpack, `L` quest log, `E` loot a corpse or talk to a friendly creature within 5 yards, click = capture mouse, `Esc` = close what is open, else the game menu (the original's frames, with the options this game has: Video Options (display mode (windowed, fullscreen or windowed fullscreen), resolution and refresh rate (the monitor's modes), interface scale, terrain distance, environment detail (how far trees and props are drawn), anisotropic filtering (from the next start), spell detail, weather intensity, gamma, shadows, vertical sync), Sound Options (all sound, ambience, music, four volumes), Interface Options (invert mouse, mouse look speed, max camera distance, NPC names, buff durations), Logout, Exit Game, Return to Game; key bindings and macros are there but greyed out; the options are kept in `settings.ron`, `video.ron` and `interface.ron` in the `rsworld` folder of `%APPDATA%`), `WASD` move, `Space` jump (costs 10 stamina, not possible with less; fly: up), `Ctrl` down (fly),
`Shift` fast (fly) / sprint (ground: 1.5x run speed, drains 20 stamina per second while moving, comes back after a second's rest; blocked when worn out until 25 are back), `Ctrl` toggle sneaking (ground only: 45% of run speed, the original stealth stand/walk animations, silent footsteps, icon at the top right; the aiming dot at the screen centre (shown while the mouse looks around; the soft glow of the spells' lights) turns into a smaller cool dot with a ring that opens and reddens as creatures near noticing you; creatures notice you from only 35% of the distance and your first blow on one that has not noticed you does triple damage ("Sneak attack!"); jumping, flying or sprinting is not possible while sneaking; not saved), a compass at the top of the screen shows the way you face with the places of `content/pois.ron` within 400 yards: a "?" until you walk within the place's radius (then "Discovered: name", a sound, saved), a gold diamond with its name after; `H` test hit of 10 damage (a blocked one costs 20 stamina, an unblocked one health), `F3` the profiler (CPU milliseconds of the parts of a frame, average and worst over the last second; `--profile` also logs them; `profile::scope("name")` adds a part), `F` toggle fly/walk, mouse wheel = fly speed, `V` first <-> third person,
left click = sword attack, hold right click = raise the shield.
The window title shows zone, coordinates (WoW coordinates), fps and mode.

`--pos x,y[,z]` uses WoW coordinates (x north, y west); omit z to stand on the ground.
Data location: auto-detected from the current directory / exe location, or `--wow <dir>` / `$WOW_DIR`.

Headless screenshot (no window; handy for checking rendering changes):

```powershell
cargo run --release -p client -- --headless shot.png --pos -9120,-175 --alt 18 --pitch -22 --yaw 0
cargo run --release -p client -- --headless p.png --alt 2 --view third --pose run --pose-turn 75   # the player
```

## What is in the workspace

| Crate | What it does |
|---|---|
| `crates/mpq` | MPQ v1 reader written from scratch: hash/block tables, the MPQ cipher, zlib / bzip2 / PKWARE, patch-chain mounting with delete markers. |
| `crates/wowfmt` | Parsers: `dbc` (WDBC tables), `blp` (BLP2: DXT1/3/5, palette+alpha, raw), `adt` (terrain tiles incl. alpha maps, liquids, doodad/WMO placements), `wdt`, `m2` (models v256, static bind pose), `wmo` (world objects v17). |
| `crates/client` | The game (`rsworld`): wgpu renderer, tile + model streaming on worker threads, height-field queries, player controller. |
| `crates/wowtool` | Inspect game data: `list`, `extract`, `dbc`, `blp` -> PNG, `wdt`, `adt`, `m2`, `wmo`, `probe`, plus bulk checks `verify`, `m2all`, `blpall`. |
| `re/` | Ghidra scripts and the exported analysis of `WoW.exe`. |
| `docs/RE-FINDINGS.md` | What was learned from the binary, what is confirmed vs. still open. |

Rendering: 25 tiles (radius 2) are streamed around you; each tile is one draw call with 4-layer alpha-blended
ground textures, then instanced models (trees, props, and every WMO group; only within one tile of you), then a
translucent animated-water pass; geometry is camera-relative so precision holds across the whole 34000-yard map.
Colours are gamma-space like the original. Both a 5x5 area and Stormwind City load in under a second and run at the
display's refresh rate (165 fps here).

## Movement and collision

Walking is a port of the WoW 1.12.1 client's own movement code (`crates/client/src/movement.rs`, with the original
function addresses in its comments; see `docs/RE-FINDINGS.md`). The unit is the client's collision hull: a box with a pointed
foot (radius 1/3 yd, height 2.03 yd, reach 1.0 yd). Moving is a collide-and-slide loop; hitting a wall triggers the client's
step probe (up by the available clearance, forward, down, accept if it lands on walkable ground), walkable slopes are
followed along their plane, and after every move the unit casts down to stay on the ground (further if it just stepped), so it
only falls when nothing is found below. Falling uses the client's gravity and terminal velocity. The 50 degree walkable slope,
the foot shape and the cast distances are all constants read from `WoW.exe`.

Each tile builds a grid-bucketed triangle mesh on its worker thread from every WMO group triangle (except pure BSP hints, see
`wowfmt::wmo::is_solid`; the client's own collision list covers all of them) and every M2 collision mesh. Terrain cells are
generated from the height field on demand, like the client does.

Not a transcription: the swept hull-versus-triangle test (the client's is x87 plane clipping; this uses an exact
separating-axis sweep of the same hull). Not ported: swimming, flying, hovering, vehicles, and one extra check inside the step
probe. Run speed and jump velocity remain the documented vanilla values.

Developer flags for checking it against the real world (headless): `--walk <s>` runs the real controller forward;
`--floor-map` / `--floor-scale` print ASCII maps of walkable floors; `--find-holes`, `--find-ramps`, `--dump-tris` inspect
collision geometry; `--probe-walk` and `--stress-walk` walk from grids/random starts and report where the player ends up or falls.

## Verified

- `cargo test` (37 tests): the ported movement against synthetic walls, diagonal walls, stairs with risers (up and down), cracks between treads, ledges up to the reach, slopes and jumps; MPQ hash known-answers; height-field interpolation (a plane is reproduced exactly at >8000
  points); hole handling; movement physics (jump apex 1.64 yd, terminal-velocity cap, 7 yd/s run speed); BLP/DXT decoding
  incl. a regression for short tail mips; chunk reader bounds; ADT alpha-map fix; placement convention; and tests that load
  real models/WMOs from your install (they skip themselves if no game data is found).
- `wowtool verify`: decodes **all 80,980 listed files (6.9 GB) in ~26 s with 0 errors**; a further 5,708 listed files are
  deliberately removed by delete markers in `patch.MPQ`.
- `wowtool blpall`: **all 53,192 textures** decode. `wowtool m2all`: 9,677 models parse; 488 have no renderable geometry
  (cameras etc.) and 5 use another M2 version.
- Gravity (19.2911) and terminal velocity (60.148) are the values found in `WoW.exe`; the map coordinate constants
  (chunk size, 17066.67 origin, tile index `y*64+x`) match the client's loader (see `docs/RE-FINDINGS.md`).

## Not done yet

In rough order of visual payoff:

1. **Movement gaps.** No swimming, flying or mounts, no camera collision, no door logic (doors are as stored), and no
   creature or player collision.
2. Interiors have their furniture (doodad sets), baked vertex-colour lighting and liquids, and which building groups draw
   follows the client's portal traversal (`wmovis.rs`, ported from `FUN_006b3b20/3dd0/41c0`), so e.g. Stormwind's cathedral
   spires only show where the original shows them. Not done: furniture is not tied to its group (always drawn), the
   camera's group is found by a floor ray instead of the client's own test, outdoor groups use sun lighting only, and
   per-doodad tint colours (`MODD`) and WMO lights (`MOLR`) are ignored.
3. Sky and weather: sun, ambient, fog and sky colours come from `Light.dbc` (zone blending by distance, keyframed over the day;
   `--time <hours>` fixes the clock, otherwise it runs 1 game minute per second). The sky itself (`heavens.rs`, `skygpu.rs`) is the
   original's dome with its five sky bands and the sunrise and sunset glow, `Textures\sunCenter/sunGlare/moon.blp` on the original's
   paths (the sun stands in a vertical plane towards the north-west, the moon is up from 22:00 to 4:00), `Environments\Stars\Stars.m2`
   turning once a day with its own meteor animation, and clouds made as the original makes them (`clouds.rs`: its four octaves of noise and its permutation table, drifting and changing with its counter, bump-lit from the sun or moon, laid over a cloud dome shaped like the original's). World light and shadows come from where the sun (by night the moon) is drawn. Weather (`weather.rs`,
   `weathergpu.rs`, `weather.wgsl`) is the original's rain, snow and sandstorm: its particle numbers and the ports of its vertex programs
   (`rain.bls`, `snowpoint.bls`, `sand.bls`, `patter.bls` read from the game files), rain splashes while you stand still, mist, the
   zone's storm light set (`Light.dbc` field 9), the loops of `Sound\Ambience\Weather`. Offline there is no server to send it: it comes and
   goes by the zone's climate (`climate_of`: rain nearly everywhere, snow in Dun Morogh and the like, sandstorms in Tanaris and Silithus,
   none in the driest zones). `--weather rain|snow|sand|none[,strength]` fixes it, `--no-weather` turns it off.
   Not done: underwater light sets, moon phases, lightning, the glare's change as you face the sun, the second moon (disabled in the original, too).
4. The player is a Human male (`player.rs`, `character.rs`, `animator.rs`, `anim.rs`) with the default skin, face, hair, underwear, legs and
   back, a long sword in the right hand and a round shield on the left, in first or third person. M2 skeletal animation is
   implemented (CPU skinning) with the client's animation engine in `animator.rs`: sequences fade into each other over their
   blend time, idles and sword swings pick a weighted-random variation (idle fidgets after a random number of repeats),
   walk/run/back keep the stride phase, a smoothed speed with hysteresis keeps direction changes from flipping the state,
   turning on the spot plays the shuffle animations, jumps play take-off / air loop / fall / landing, and the swing and the
   combat-ready stance play on the upper body while the legs keep walking. The third-person camera collides with the terrain and with object collision meshes (walls, roofs, trunks): it moves in at once and eases back out. The arms are the original animations' (no
   hand-made poses): the sword swing, and the shield block, which plays the `ShieldBlock` animation up to its raised pose and holds
   it there for as long as the button is down, then lowers it. The fists are closed with the `HandsClosed` animation on the
   finger bones and the shield rides the forearm bone as in the original. In first person the camera is fixed to the head bone, just in front of the face (the head and hair are behind it) and the
   whole body is shown with the same animations as in third person, so looking down shows your chest, arms and legs. Not done:
   character creation or other races and sexes, armour and other equipment (item display ids), no combat or damage (the
   swing is only an animation), and other
   animated models (creatures) are still drawn in their bind pose.
5. Action bar (`actionbar.rs`, `ui.rs`): the original main bar drawn from the client's own interface art (`MainMenuBar.xml` /
   `ActionButtonTemplate.xml` layout: the left half of the bar art (512 units) centred on the screen with the gryphon end caps the experience bar frame and, above it between the gryphons, health (red), mana (blue) and stamina (yellow) bars (`Player::vitals`, all full: nothing changes them yet), twelve 36-unit buttons with the
   slot frames, icons, pressed / hover / active states and hotkey text rendered from `ARIALN.TTF`), scaled with the screen height
   like the original (768 units). Keys 1-0, - and = use the slots (a click does too, while the cursor is free after Esc); slot 1 is
   the sword swing, slot 2 the shield block (held as long as the key or button is down). Not done: dragging actions, other
   bars and pages, spells, cooldowns, tooltips, the right half of the bar with the bag and menu buttons, experience.
6. Combat (`creature.rs`): two wolves (the original `Wolf.m2` with its Timber skin and its own animations) lie in wait ahead of
   the spawn point. Until then hostile creatures do not stand still: they stroll (walk animation, 2.2 yd/s, footsteps) to places within 8 yards of home and rest 4-10 seconds between walks. A wolf notices you within 14 yards, runs at you (6.94 yd/s, so only a sprint outruns it), bites for 8 about
   every 2 seconds (blocked by the raised shield at a cost of 20 stamina, otherwise health), flinches when hit, dies at 0 of its
   50 health and comes back after 5 minutes (the captains after 15). Your swing hits, at the end of its cut, everything within 3.5 yards and 60 degrees in front, for the damage of the weapon in your main hand
   (plus what attack power adds) times the share of the weapon's speed that has passed since your last swing began: you may swing as often as the
   animation allows, but a swing that follows the last after a quarter of the weapon's speed does a quarter of the damage (at least 15%), and one that
   comes after the whole speed does all of it, so a faster click does not make more damage in a second. Every swing costs stamina (8 for a weapon of
   speed 2, more for a slow one and less for a fast one), so swinging in a hurry soon leaves you winded: a swing without the stamina is refused with
   "Not enough stamina." The attack button darkens with the damage that a swing would lack. A tap on it (left mouse, or key 1) is a
   light swing; holding it for 0.4 seconds, or the heavy attack button (key 3), is a heavy one: a long wind-up, twice the damage and
   three times the stamina (25 at speed 2), by the same rule of time. A bow, crossbow or gun in the ranged slot is shot with key 4: the string is drawn for as long as the button is held (the weapon's speed to draw it fully,
   held there after that), and letting go sooner does only that share of the damage (at least 15%); each shot uses up a piece of the ammo in the ammo slot
   (arrows for bows and crossbows, bullets for guns) and flies as an arrow (the second model of the ammo's display, `ArrowFlight_01` or `BulletFlight_01`, at 40 yards a second) from the hand that holds the weapon to the selected creature, or the nearest one in line, within 35 yards, where it does its damage when it lands; with nothing in line it flies on and hits nothing. The animations and sounds of the draw, the release, the flight and the hit are those of the weapon's `SpellVisual.dbc` row (`ItemDisplayInfo.dbc` field 10). While it is drawn, the sword and the shield are put away. The world's own fires burn too (`fires.rs`): the particle emitters of every placed model within 250 yards of the camera (the 34 candelabras in the abbey, the stone pyres, torches, the chandelier, fountains, waterfalls, light shafts), including those of models inside buildings (the WMO's doodad sets), and the models that move (flames and glows that turn to the viewer as billboards, a flickering lamp) are shown by it instead of the still ones of the tile. Models' own tracks play: the colour and opacity of their batches and the movement of their textures (`anim::LookTracks`, in the model shader's per-batch uniform) for the effects, the placed doodads and the worn items; blends 5 and 6 (modulate, twice) are drawn; particles of types 1 and 2 have tails, and emitters flagged for it are sorted far to near. The lamps of a building (`MOLT`, the four nearest to a doodad) light the doodads in its rooms. Fires and moving doodads burn within 350 yards, the original's default `farclip`. Particles follow the original's emitter code (read from WoW.exe): the plane and sphere emitters' angles and areas, the cells of the texture stepped through over a particle's life, the age they begin with, and emission at a quarter of the rate beyond 87 yards from the camera. A model in a room of a building is lit by the colour of its place (`MODD`) as the original lights it: that colour held down to 96 for the ambient and up to 112 for the light from the sun's direction. Weapons that burn or glow show it: the enchantment visuals of `ItemVisuals.dbc` (a glow model at each of the five points along the blade, e.g. the flames of a fiery axe) and the particle emitters of the item models themselves (torches, burning swords) are `Effects` owned by `Worn`, moving with the part (only weapons and shields get the glows of an enchantment visual: the original client passes no visual for helmets, shoulders or quivers, and nothing reads it for armor). A wounded wolf shows a health bar over its head, and each hit shows its damage in white, centred on the wolf, scrolling up
   and fading like the original's combat text (`combattext.rs`, constants from `Blizzard_CombatText.lua`: FRIZQT at height 25 with
   a shadow, 1.9 s scroll, fade from 1.3 s, stagger of 10 units). What happens to you scrolls up from the middle of the screen the
   same way, stacked so lines do not overlap: "-8" in red for a bite, "Entering Combat" / "Leaving Combat" (5 seconds after the
   last wolf), "Health Low" at a fifth of your health. A blocked bite shows nothing there, as in the original, whose "Show Damage
   Reduction" option is off by default (the shield rings and your stamina bar drops instead). At 0 health you reappear at the spawn point restored.
   The wolves make their original sounds (aggro growl, attack snarl, bite impact, wound, death, an idle growl when they
   fidget, paws on the surface underfoot) from `CreatureSoundData`. They are positioned in 3D: heard from the camera, at full volume
   within the sound entry's minimum distance (8 yards), falling off like 1/distance (FMOD's inverse roll-off), gone at its cutoff
   (45), panned between the ears by direction (`spatial_gains`; the player's own sounds stay plain stereo). `--wolf` puts one in
   the headless player previews. Not done: other creatures, loot and experience, doppler, occlusion by walls, wolves
   going around walls (they run straight at you), threat or groups.
7. Interface from the original's own files (`uixml.rs`, `font.rs`, `panels.rs`, `items.rs`): a loader for the XML of
   `Interface\FrameXML` (templates and inheritance, anchors, texture coordinates, button states, backdrops, scaling, frame
   levels, the fonts of `Fonts.xml`, global strings, colour codes) draws the real character sheet and backpack; there is no Lua,
   so the game fills in what the scripts would. `C` opens the character sheet (paper doll with the character's 3D model drawn
   into the frame, portrait, 20 equipment slots, attributes, armor, melee and ranged blocks, resistances), `B` the backpack
   (16 slots, money), `Esc` closes them; the mouse is free while one is open. Click an item to pick it up and click a slot to
   put it down (swap, stack, wear), right-click to wear it from the pack or take it off, hover for the tooltip. The sword and the
   shield on the character show only while worn; the stats follow the original's formulas for a level 1 human paladin (armor
   mitigation, attack power, weapon damage, health from stamina) and drive the combat: your swing damage, the bites that get
   through armor. The status bars above the action bar are the original's player frame bars: green health, blue mana, and
   stamina in the energy yellow, with the 50% black backing and "Health 100 / 100" text while the mouse is over them or the
   sheet is open. Not done: the Pet, Reputation, Skills and Honor tabs (hidden), bank and other bags, vendors, item
   links, and the stat tooltips.
   Looting: a dead wolf drops copper and items rolled from its table (`loot.rs`: meat, pelt, linen cloth); "Press E to loot"
   shows within 5 yards, `E` opens the original `LootFrame` with the coin and the items, a click on an icon takes it (an
   "Inventory is full." error in red when it does not fit), the window closes when emptied, on Esc, or when you walk 8 yards
   away, and what is left stays on the corpse until it respawns. The interface makes its sounds (panels opening, items picked
   up and put down by kind, coins). `--panel loot,error,hint` previews them.
8. Game data (`content.rs`, the `content/` folder): items, drop tables, creature kinds, where creatures stand and the starting kit are
   RON files embedded in the program, not code. Things refer to each other by string id (`"tough_wolf_meat"`), every reference is
   resolved and checked at startup (and by a test), and an item names a row of `ItemDisplayInfo.dbc`, which gives its icon, the
   skin textures it paints on the character and the model it holds, so a worn item always shows on the character. To add an item,
   a creature or a drop, edit the files and run `cargo test`; spawns are in WoW coordinates (north, west) and heading degrees.
9. Creatures (`creature.rs`): every kind in `creatures.ron` is either a Beast (any model with its skins and display scale, like
   the wolf; its cries and footsteps come from the model's `CreatureSoundData` automatically) or a Humanoid (a person of any of the eight races, male or female, dressed
   in items, with weapon and shield in hand, like the player; the race decides the model, the textures of skin, face and hair, the voice and the height). Hostile ones chase and bite; the others stand and can be talked to.
   Names float over creatures (red hostile, green friendly). Talking opens the original `GossipFrame`: the greeting and the lines to
   choose from `gossip` in the creatures file (`Say` answers with text; vendors and quests will be more actions). `--npc` previews
   Deputy Willem, `--panel gossip` the window. The interface engine learned `ScrollChild` frames, virtual texture inheritance (button
   art), frames anchored to font strings, wrapped text sizing, and button text alignment for it.
10. Vendors and quests (`panels.rs`, `quests.rs`, `content/quests.ron`): a creature with `vendor` wares and a `Vendor` gossip line opens
   the original `MerchantFrame` with the backpack: click a ware to buy one, right-click a backpack item to sell the stack, the last
   thing sold can be bought back for what it fetched; prices are `buy` and `sell` in `items.ron`. Quests come from the talk window
   (a yellow `!` over the giver's head and a line with the same icon): the original `QuestFrame` shows the offer (Accept or
   Decline), the progress (Continue is greyed until the goals are met) and the reward. Goals are Kill(creature, count) and
   Collect(item, count) (carried items, taken at the turn in); a yellow `?` marks a quest ready to turn in (and a grey `?` one taken but not done; the marks are the original's own small 3D models, `TalkToMe*.m2`, which bob and, for the `?`, turn to face you), and a turn in that
   leads on to the next quest of the chain opens it at once. `L` opens the original `QuestLogFrame` (select, details, abandon),
   and the quests being worked on are listed at the top right like `QuestWatchFrame`. Messages ("Wolf slain: 1/2", "Quest accepted")
   stack at the top. Money frames follow the original's `MoneyFrame_Update` rules. `--panel quest,quest-progress,quest-reward,log,
   tracker,merchant` preview it. A quest can offer a choice of rewards ("Choose your reward:": click one, the highlight moves to it, Complete Quest asks for a choice if there is none; the Wolves quest offers one of two jerkins that show on the character), and abandoning a quest in the log asks first (the original's `StaticPopup1`, Esc closes it before anything else). Windows with long texts have the original's scroll frames: what is in them is cut to the frame, the wheel scrolls it, and the scroll bar (arrows and knob) shows when there is more to see. Not done: zone headers in the log, dragging the scroll knob.
11. Experience and levels (`levels.rs`): kills (`45 + 5 x level` for a creature of the player's level, 5% more per level above, less per
    level below until grey; the original's formula) and quest turn ins (`xp` in `quests.ron`) fill the purple experience bar over the
    action bar ("XP 1800 / 4500" on hover or while the sheet is open); crossing a level says "Congratulations, you have reached level N!",
    plays the level up sound and grows the attributes, attack power, armor, health and weapon skill (a creature's `level` in
    `creatures.ron` sets what it is worth; armor also stops less of the damage of higher level creatures). The table of experience per
    level (to 60) is the original's as remembered, the growth per level (`levels.rs`) is the game's own rough paladin. Not done: talent points, rested experience, creature levels shown in the interface. `--level N --xp N` preview a character.
13. Targeting (`creature.rs`, `panels.rs`): `Tab` selects the nearest living enemy ahead within 40 yards, again the next one; the creature you
    hit becomes the target when you have none, talking to someone targets him, and the first `Esc` lets go. The original `TargetFrame`
    shows it at the top (its portrait is the creature's head drawn offscreen and masked round, the name plate red for enemies and green for
    friends, its level in the colour of how hard it is for you or a skull when it is ten or more above, the health bar, "Dead" for a corpse);
    a target further than 60 yards is let go. The interface engine got status bars (`BarTexture`/`BarColor`), round pictures, texture tints
    and frame levels for it. `--target` previews it. Not done: clicking a creature to target it, target of target, auras, the player frame.
14. Faces (`character.rs`): a Humanoid's `look` in `creatures.ron` (skin 0-9, face 0-11, hair_style 0-11 with 0 bald, hair_color 0-9, beard 0-8)
    is looked up in the original's tables (`CharSections.dbc` for the skin, face, hair and facial hair textures, `CharHairGeosets.dbc` and
    `CharacterFacialHairStyles.dbc` for the hair and beard meshes), so people no longer look alike; the player has the default look.
    Whoever you talk to shows his face (the target frame's portrait) in the talk, trade and quest windows. The level up effect (`effects.rs`) is the original's `Spells\LevelUp\LevelUp.m2`, played once round the character with its own bone animation (golden light rising, fading in and out as a whole since the model's own opacity tracks are not read); `--levelfx <ms>` catches it at one moment. 15. Chests and lockpicking (`chest.rs`, `lockpick.rs`, `content/chests.ron`): chests stand in the world (the original's `Chest01`/`Chest03` models, lid swinging
    up with their own animation) with a drop table and, optionally, a lock of one of five levels. `E` opens an unlocked chest into the loot window; a
    locked one starts the lockpicking in the manner of Skyrim: the mouse turns the pick to find the lock's hidden sweet spot, holding A, D, Space or
    the left button turns the lock, and the nearer the pick is to the sweet spot the further it turns (all the way opens the chest); held against
    its stop the pick shudders, strains (the bar under the lock) and breaks, using up a lockpick (`lockpick` item: bought from Trader Aldric,
    found in chests; the start kit has five). It is drawn with the original's own pictures: the keyhole plate of a door turning with the lock (`World\Goober\G_UldamanMapKeyHole`), the glowing runed bezel (`Spells\AuraRune256`, `DoubleRing128`) and the Pick Lock spell icon; the interface now draws turned and round quads (`Ui::set_turn`, `set_round`). Harder locks have a narrower sweet spot and weaker picks; the pick cannot be moved once the lock
    turns; `Esc` leaves. Chest state is not saved (they are locked again after a restart). `--chests open|closed` and `--panel lockpick` preview them.
    16. The mouse pointer (`cursor.rs`): the system pointer is hidden and the game draws the original's own (`Interface\Cursor`, 32 pixels, the tip at the top left)
    wherever the mouse is free: the gauntlet over nothing, the sword over an enemy, the speech bubble over someone to talk to, the bag over a
    vendor, a ware to buy or a corpse (or open chest) with loot, the gear over a chest to open, the padlock over a locked chest. With the mouse free
    (after `Esc`, or with a window open) the left button selects the creature under the pointer and the right button uses it (talks, loots, opens, or
    "You are too far away."); a left click on nothing still hands the mouse back to the game. An item on the mouse is drawn instead of the pointer.
    `--cursor x,y` shows the pointer at a position in the previews. Not done: the `Unable` pointers, a pointer for the shield or spell targeting.
    Not done: lockpicking skill and perks, locked doors, other races and
    women, a way for the player to choose a look.
12. Saved game (`save.rs`): the game saves itself every 30 seconds and when it ends, and loads the save at the start: position and
    heading, health, level and experience, money, what is worn and carried, the quests being worked on (with their kills) and the finished
    ones. Items and quests are saved by id, so a save survives changes to `content/`: what no longer exists is dropped, a stack is held to
    its limit, and an item that no longer fits its slot goes in the backpack (all logged). The file is written beside the old one and
    renamed over it; a file that cannot be read is kept and the game then starts new and does not save. It is `rsworld/save.ron` in the
    application data folder. `--new` starts afresh (replacing the save when the game ends), `--no-save` neither loads nor writes, `--save
    <file>` uses another file. The world is not saved: creatures, corpses and loot start afresh, and dying still returns to the spawn point. `--panel character,bag,loot`, `--cursor x,y` and `--show Frame`
   draw them in the headless previews.
8. Sound (`audio.rs`): the original's zone music (day / night sets, silences between tracks, zone intros such as Stormwind's),
   looping zone ambience, and the player's footsteps on the surface underfoot, sword swing and attack grunt, all from the
   archives, with the client's default volumes (music 0.4, ambience 0.6, effects 1.0; `--no-audio` turns it off). Not done:
   reverb for interiors, WMO doodad sounds, and any sound for things that do not exist yet (spells, interface). Water is
   heard as in the original: the nearest liquid within nine yards loops from its place (`SoundWaterType`: still, slow and fast
   water, ocean, magma, slime; at most two kinds at once), models that play a sound all the time (waterfalls, fountains: the
   `$DSL` event of the model) loop from where they stand, and wading through water splashes each step (`FootstepTerrainLookup`'s
   splash column) with a splash on entering. Not done: the visual splashes and wakes, underwater sound, swimming. Footsteps are timed by distance walked (matching the walk and run strides), not
   by the animation events, and the surface comes from the ground texture's file name, since the tiles carry no surface data
   (building floors count as stone).
6. Content: creature spawns, quests and loot live on the *server* side of the original game, so an offline game needs its own data for those.
   The interface (`Interface\FrameXML`, Lua + XML) and sound (`sound.MPQ` decodes fine) are in the archives.

Known limitations of what exists: no frustum culling, no slope limit when walking, model/WMO uploads happen on the main
thread (a heavy tile like Stormwind can cause a brief hitch when it streams in), the model rotation convention is
taken from community documentation and verified only by eye (trees and buildings line up with the terrain and roads),
water is a flat translucent tint with ripples (no depth-based colour), river flow direction is ignored.

## Re-running the binary analysis

```powershell
cd re
.\analyze.ps1                      # needs Ghidra 12.x; pass -Ghidra <dir> if it is not under %LOCALAPPDATA%\ghidra-cli
.\query.ps1 queries\12_physics.txt
```

## Legal

The repository contains no Blizzard assets and the program only reads files from an installation you already own.
The game data and `WoW.exe` remain Blizzard's property; if you want to *distribute* your own game, plan to replace
the art and sound with your own: the original data cannot be shipped with it.

## The story of the valley

`content/STORY_NORTHSHIRE.md` tells "The Dark Beacon": the abbey's lens was stolen by its own steward and sold to the kobolds of Echo
Ridge Mine. The valley is populated to match: the abbey's three brothers (Abbot Hollis, Brother Tolan, Brother Corwin), cellar rats,
valley boars, the Redhand cutpurses and their captain Garrick Thorne at the Stone Bridge, and the kobold miners and Foreman Gribble
at Echo Ridge Mine, with ten quests in all (`content/quests.ron`), a locked chest at the camp and one at the mine, and gear to
earn. Hostile people (the Redhand) run at you, swing their weapons and fall on their backs when dead. Headless checks:
`--spawns` shows every creature of the content, `--creature a,b` a row of chosen ones, `--check-spawns` warns about content
standing in a wall or tree.

The whole world is populated: `tools/import_world.py` (run `import_creatures.py` first) makes `classic_spawns.ron` (about 62,700 spawns in the 34 maps the client has, with their respawn times, strolls and patrols), `classic_quests.ron` (about 1,500 quests with their chains, levels, races, rewards and experience), `classic_gossip.ron`, `classic_pickups.ron` and `classic_goal_items.ron` (the objects that quests ask to pick up or use), `classic_objects.ron` (notice boards, statues and corpses that give quests, as creatures too small to see) and `classic_morphs.ron`. `WHY=nobody py -3 tools/import_world.py` says why quests were left out. Only the creatures near the player are in the world (`creature.rs`: a place becomes a creature within 200 yards, gets its model within 150 and is put away beyond 260; far creatures are posed less often), and each map has its own (`Spawn.map`). A new character starts in the homeland of its race, and the Horde and the Alliance are each other's enemies (`CreatureDef::is_hostile` follows the race). Quests are for a level and a race (`min_level`, `races`), and can ask to kill, to collect, to use an object, or to walk to a place (`Reach`). Left out: quests of classes and professions, quests that begin with an item, escorts and other quests a script finishes, creatures that sit, kneel or sleep, flight masters and trainers (they only greet).

## The items of the original

`content/classic_items.ron` holds the original's items: 16,204 of the 17,718 of the 1.12.1 item table (what is left out are the placeholders
of its developers: "(test)", "Monster - ...", "[PH]", "zz", "Deprecated" and the like), with the name, quality, class and kind, where it is worn,
the level it needs, binding, uniqueness, durability, armor, block, stats, health, mana, resistances, weapon damage and speed (and elemental damage),
stack, prices, lore, the skill it asks for, and its spells with their descriptions as the original writes them (`Spell.dbc`). The client files hold
only the looks of items, not their data, so `tools/import_items.py` makes the file from the CMaNGOS classic database
(`tools/import_loot.py` the original's loot tables of `classic_loot.ron`); both read their data from `tools/data` (the database dump and the client tables, kept there and not shipped; the head of each tool says where it comes from), so `py -3 tools/import_items.py` and `py -3 tools/import_loot.py` make the files again. `content/classic_props.ron` holds the original's objects that stand for looks (signs, banners, mailboxes, benches, forges, anvils, braziers...), 4,429 in Azeroth and 2,575 in Kalimdor, made by `tools/import_props.py` (it also needs `GameObjectDisplayInfo.dbc`, `AreaTable.dbc` and `files.txt` in `tools/data`, and the built `wowtool`): chests, herbs and ores, traps, boats and everything a game event puts out (Hallow's End, Winter Veil, Darkmoon Faire...) are left out, and only the turn about the vertical is taken. They are props of the editor like the others (each has its `map` and its `zone`, shown in the Props list): what the editor changes of them goes to `props.ron` (moved or new ones) and `editor.ron` (those taken away), and the file itself is never written by it. A prop can be one to pick up (`pickup: Some((item: "id", respawn: 120.0, vanish: true))` in `props.ron`, or "Gives item" in the editor's prop form): it is drawn apart from the map (`pickups.rs`), can only be used (E, or the pointer) while a quest being worked on still wants the item, gives the item into the bag (the quest counts it) and, with `vanish`, is gone for `respawn` seconds (not kept in the saved game); one that does not vanish stays and can be used again until the quest has what it needs. The compass and the map point to them like to chests. `content/classic_creatures.ron` (+ `classic_creature_loot.ron`) holds the original's 9,678 drawable creatures (where they stand is in `classic_spawns.ron`), made by `tools/import_creatures.py` (it also needs `files.txt` = `wowtool list ""` in `tools/data`); most people (4,896 of 4,899) wear the original's baked skin texture (`Appearance.baked`, a CreatureDisplayInfoExtra row) and their helmets, shoulders and the rest of their outfit as made-up pieces (`content/classic_pieces.ron`, ids `p<Slot><display>`, 8,098 of them: most of it is no item of the original), and drops below 0.2% are cut. The ids are `i<entry>`
(the original's number), except the three the game had before (`hearthstone`, `linen_cloth`, `tough_wolf_meat`); the game's own items stay in
`items.ron` (the tomes, the story's items and the starting kit). The tooltip is the original's (`GlobalStrings.lua` templates): binding, unique,
the place and kind, damage with speed and damage per second, armor, block, stats, resistances, durability, classes and races, the level and the skill
it needs (red when the character falls short), the spells and the lore.

Wearing: every place of the paper doll takes what the original puts there (a weapon of both hands takes the off hand too; rings and trinkets go in
the free one of two), asks for its level (`ERR_CANT_EQUIP_LEVEL_I`), and adds its stats, armor, health and resistances. What the worn items look like
is the original's, on the player and on every person of the world (`worn.rs`): textures painted on the skin, the sleeves, gloves, boots and robes
of its mesh (gloves and boots take the shape of their armor: cloth and leather, mail, plate), helmets and shoulder pads as models on the head and the
shoulders (of the race and sex of whoever wears them, hiding the hair and face as the helmet does), capes, tabards, shields, weapons in the hands,
a bow, gun or crossbow on the back. A test (`every_wearable_item_has_a_look_on_the_character`) checks that every wearable item of the game has
all of that and its files in the game data (jewelry, relics, ammunition, wands and thrown weapons are not drawn by the original either).
The skills have all the weapon families (29 skills).

Using (right click in the backpack): potions heal and restore mana at once (and wait for their cooldown), food, drink and bandages restore over their
seconds while the character keeps still out of a fight, elixirs, scrolls and oils raise a stat, the armor or the health for a while (shown as icons
top right, `BuffFrame`), worn items with such spells do the same while worn, and the hearthstone takes the character home after ten seconds
(the cast bar; moving stops it). `consume.rs` has the logic and its tests. Items with other spells (damage, summons, trade skills) show them in
the tooltip only. Merchants show their wares ten to a page with the original's page buttons, and sell the number the original sells at a time.

What is placed where the original has it, as far as the game has the same creature: the loot tables of the Young Wolf, Timber Wolf, Stonetusk Boar,
Kobold Laborer, Kobold Miner, Defias Cutpurse and Garrick Padfoot (the wolves, the black wolf, the boars, the kobolds, the foreman, the cutpurses
and Garrick of the story) and the chests of the valley (`loot.ron` rolls them with `also`, besides the game's own tomes and quest items), Trader Aldric's wares (the
original's Northshire merchants: armor, weapons, bread, water and ammunition) and the rewards of three quests (those of the original's
"Wolves Across the Border", "Grape Manifest" and "Bounty on Garrick Padfoot"). The rest of the catalogue is in the game, in the editor
(F1, Items: look, class, level, stats, spells; Loot, Creatures, Quests) for placing it, but the world has no more creatures to drop it. Not done: bags and
quivers (no extra slots), ammunition and ranged combat, professions and recipes, keys and lockboxes, readable books, items with a random suffix,
item sets, durability and repair, binding and unique rules, armor and weapon proficiency by class (the classes and races of an item are shown, not enforced).

## Skills, spells and tomes

Twenty-nine skills, trained by use like Skyrim's (rank 1 to 100, `skills.rs`): the weapon families of the original (swords, axes, maces and daggers; the
two-handed ones; polearms, staves, fist weapons, spears, bows, guns, crossbows, thrown weapons and wands), unarmed, blocking, sneaking, lockpicking and one for each of the nine classes. A hit trains the skill of the
weapon in your hand (or your fists), a blow taken on the shield trains blocking, sneaking about trains sneaking, an opened lock (and, less, a
broken pick) trains lockpicking and every spell cast trains its class. A rank is worth something: weapons hit up to 50% harder, class
spells up to twice as hard, blocks cost up to half less stamina, creatures notice a sneaking player from up to half less far away, and the
sweet spot of a lock is up to half wider. `C` then the Skills tab shows them in the original's skills window (headings fold, the wheel
scrolls, a click shows what a skill is for). The skills are saved.

`content/spells.ron` has two spells for each class, a basic one and a greater one that needs rank 20 of the class skill. `P` opens the
spellbook (the original's, twelve to a page; clicking a spell casts it), and every spell learned goes on the first free button of the
action bar. A spell costs mana (which comes back 4 a second) or stamina, has a cooldown, and either hurts the selected creature within its
range (Tab selects), hurts every enemy around you, or heals you. Spells are shown the way the original shows them, from its own tables
(`visuals.rs`: `Spell.dbc`, `SpellVisual.dbc`, `SpellVisualKit.dbc`, `SpellVisualEffectName.dbc`, `SpellCastTimes.dbc`): a spell with a
cast time (Fireball 1.5 s, Holy Light 2.5 s, ...) holds the character in the ready animation, with the glows of its school in the hands
and the sound of its start, while the original's cast bar fills (`CastingBarFrame`, with its spark, its green flash and its red
"Interrupted" if the character moves); when it goes off the cast animation, the hand effects and the cast sound play, a missile flies
to the target at its own speed and, where it lands, the burst of the spell's impact, its sound and the damage happen. The effects
are the original's models of `Spells\`, whose particle emitters (sparks, smoke, glows, rings) are simulated and drawn
(`particles.rs`, `effects.rs`). Casting breaks the sneak. The Paladin knows Holy Light to begin with.

Every other spell is learned from a tome (`tome_...` items): right-click it in the backpack. A basic tome asks for nothing; a greater one
asks for rank 20 of its class skill ("Requires Mage (20)" in red on the tooltip, and the spell is not learned until the skill is
trained by casting that class's basic spell). They are found in the four chests of the valley and as rare drops (`content/loot.ron`);
the two captains carry the greater tomes of the Rogue and the Mage. Headless: `--panel spellbook`, `--panel skills`, `--cast a,b` (with
`--creature wolf`, `--panel hint` for the cast bar), `--effect model,ms`.

## The editor (F1)

`F1` opens an editor over the game, for the lists that people change: **Places** (where every creature and chest stands: put a new creature
down where you stand, move one here, go to one, delete it; chests with their lock and loot table and what they always hold (copper and items, besides the table); and the places to discover on the compass, with their radius), **Creatures** (the NPCs and the
enemies: name, level, health, bite, respawn, loot, wares, model or outfit and look, what they say and the lines you can choose; a picture of the creature stands at the top of its form, turnable: a beast has a window with a search and a picture of each of the 407 creature models of the game and one for the skins the game has for the model (from `CreatureModelData`/`CreatureDisplayInfo`); a person has arrows for race and sex (what exists for them limits the rest), skin, face, hair, beard and for putting on or taking off what he wears, and the picture follows each change),
**Quests** (texts, who gives and takes them, what they need, goals, rewards) and **Items** (a picture of the look worn or held, chosen from a window over every `ItemDisplayInfo` row that fits where it is worn, with a search and the picture of each; quality, where worn, stats, weapon,
prices, the spell a tome teaches) and **Loot** (the drop tables: copper and the items with count and chance; an item with `quest: true` (the **Quest item** switch) is a quest item: it drops, from creatures and chests alike, only while a quest being worked on still wants it, says "Quest Item" in its tooltip and cannot be sold). Every list has a find field (all the words typed must be in the line); every field that names a creature, item, quest, loot table or spell has a `...` button that chooses from a window with a search, and an id that does not exist is red; what is changed and not saved stays when F1 is closed and opened again. Click a field, type, `Enter` keeps it and `Esc` throws it away; the arrows step through choices; the wheel
scrolls lists and forms. Long forms (an item, a creature, a patrol) fold into sections that a click opens and closes (the advanced ones start closed); long texts have fields of several lines; numbers have their unit in the label (deg, yd, s) and prices show their gold, silver and copper; what cannot be taken back (Delete, Clear the patrol, Discard changes) asks once more ("Sure? Click again"). In the world the editor's help is at the bottom right. `Save` checks that the content with the change is valid (a quest that names a creature that does not exist is
refused, with the reason in the status line) and writes it to `content/editor.ron`, an overlay that is put on the other content files
(they keep their comments; delete the overlay to undo it all). `Save and apply` saves and takes the change into use while the game runs, with no restart: the character (items, level, quests, skills, discoveries) is kept by ids, the creatures and chests are put in the world again from the new content (corpses, loot and wounds are forgotten, like after a respawn) and the windows that showed creatures or loot close. If that cannot be done (the spells changed on disk) the game starts again instead, standing where you stood. The game reads the content files from the `content` folder of the source tree (or `RSWORLD_CONTENT`), so nothing has to be built again. `--apply-after s` does the same Apply by itself after s seconds (a smoke test). Not in the editor: spells and the starting kit (edit those files by hand). Headless: `--editor places|chests|discoveries|creatures|quests|items|loot,n`.
