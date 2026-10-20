# WoW.exe 1.12.1 (build 5875): reverse-engineering findings

Everything here was read out of the `WoW.exe` in this folder with Ghidra 12.1.4
(headless; see `re/analyze.ps1`, `re/query.ps1`, `re/ghidra-scripts/`). Addresses are
the image addresses Ghidra shows (image base `0x400000`). Function names are Ghidra's
auto-names (`FUN_<addr>`); nothing has been renamed in the project.

Status legend: **confirmed** = read from the binary and cross-checked against the game data
or the Rust implementation. **open** = looked for, not found or not understood.

## The binary

| | |
|---|---|
| Version string | `WoW [Release] Build 5875 (Sep 19 2006 20:32:39)` (`0x80a364`) |
| Size / analysis | 4,775,986 bytes, x86 32-bit PE; 19,806 functions, 11,185 strings found by auto-analysis |
| Original source layout | 129 source paths survive in assertion strings: `re/export/source_files.txt` |

Source modules named by those strings: `Ui` (50 files), `Object` (25), `UIUtil` (9), `WorldClient` (8),
`Glue` (6), `WowServices` (6), `Game`, `WowSvcs` (4 each), `Net` (3), `Console`, `DB`, `Magic`,
`ObjectAlloc`, `SoundInterface` (2 each), plus `Client.cpp`, `LoadingScreen.cpp`, `ObjectMgrClient`.
The terrain/world code is `WorldClient/{Map,MapMem,MapObj,MapObjGroup,MapObjRead,MapWeather,World,DetailDoodad}.cpp`.
`Net/*` is the server protocol layer that this offline project deliberately does not reimplement.

## Archives (confirmed list, open priority direction)

Pointer table at `0x82e12c`, read entry by entry: `model, texture, terrain, wmo, sound, misc, interface,
fonts, speech, dbc` (all `*.MPQ` under `Data\`). `speech2.MPQ` is an eleventh pointer (`0x82e154`) that the
start-up code uses separately. Also referenced: `patch.MPQ`, `patch-?.MPQ` (glob), `Data\base.MPQ` (`0x85d484`),
`wow-patch.mpq` (`0x8389d8`).

- `FUN_00403740` enumerates the patch archives and opens everything, passing an incrementing
  priority number that starts at `0x40`; `FUN_00402350` is the start-up routine that calls it.
- **open:** which direction wins (low or high priority number). The Rust `Mount` uses the standard
  rule (`patch-N` > `patch` > base archives > `base.MPQ` > `backup.MPQ`). It is consistent with the data:
  `patch.MPQ` carries delete markers (`flags 0x82000000`) for 5,708 files that still exist in older archives,
  and every one of them is intentional (see `wowtool verify`).

## World / map loading (confirmed)

| Address | Role |
|---|---|
| `FUN_0066fbe0` | caller of the map loader |
| `FUN_006941f0` | map loader: builds `World\Maps\<map>\<map>.wdt` from `"World\Maps\"` (`0x86a610`) and `"%s\%s.wdt"` (`0x86a604`) |
| `FUN_006c2720` | per-tile setup: formats `"%s\%s_%d_%d.adt"` (`0x86c368`) with `(x, y)`, tile index **`y * 64 + x`** |
| `FUN_006c1e70` | called last with the path buffer built above; presumably opens and reads the ADT (not traced further) |

Coordinates, from the constants `FUN_006c2720` uses:

- chunk size = **-33.3333** (`0x8105b0`, bytes `55 55 05 c2`), map origin = **17066.666** (`0x7ffab4`, `55 55 85 46`) = 32 × 533.333.
- Tile `(x, y)` is a 16 × 16 chunk block at chunk index `(x*16, y*16)`; a chunk's world position is `17066.666 - index * 33.3333`.

This is exactly what the data shows (`Azeroth_32_48` chunk (0,0) has `x = -8533.33, y = 0`) and what `crates/client/src/coords.rs` implements:
world `(x north, y west, z up)` -> render `(X east, Y up, Z south)`.

## Movement (partly confirmed)

| Constant | Value | Where |
|---|---|---|
| gravity | **19.2911 yd/s^2** (`0x419a542f`) | `0x81da58`, used by `FUN_007c5d20` and `FUN_007c5e70` |
| terminal fall speed | **60.148 yd/s** | `0x87d894` |
| alternate terminal fall speed | **7.0 yd/s** | `0x87d898`, selected when bit `0x20000000` of the flags at `this+0x40` is set (probably slow fall; the flag's meaning is inferred, not traced) |
| g/2, 2g, 1/g | 9.6455, 38.5822, 0.051837 | `0x81da60`, `0x81da64`, `0x80e020` |

- `FUN_007c5d20(this, dt)` = fall velocity after `dt`: `min(terminal, v0 + dt * g)`, `v0 = *(float*)(this+0xa0)`.
- `FUN_007c5e70(this, dt)` = distance fallen in `dt`, accelerating until terminal speed and then linear.

`crates/client/src/camera.rs` uses these values and has tests for them (jump apex = v^2/2g = 1.64 yd, terminal-velocity cap).

**open:** run speed (7.0) and jump velocity (7.9555) were *not* found as float constants (searched exact and
negated bit patterns). The client probably takes them from per-object movement state rather than literals.
They are the commonly documented vanilla values, flagged as such in the code.

## Collision (partly confirmed)

The client's collision lives next to the `Movement_C.cpp` assertions, at `0x630xxx`-`0x637xxx`. It is a **swept-volume**
test against a per-query list of triangle records (`0x34` bytes each, array pointer `0xc4e534`, count `0xc4e530`,
normal at `+0`, plane distance at `+0xc`), not point sampling.

| Item | Value / address |
|---|---|
| walkable slope | `normal.z >= cos 50° = 0.642788` (`0x80dffc`); a looser `0.173648` = sin 10° (`0x80e000`) is used in another movement mode |
| `tan 50°` | `1.19175` (`0x80e008`): the query box is extended by `distance * tan 50°` so a 50° climb is covered |
| default vertical reach | `2.0` yd (`0x801628`), else a per-unit field at `+0xb8`; the query box reaches `reach + distance*tan50` below the feet and `max(2*reach, distance)` above the head |
| skin margin / epsilon | `0.001389` (`0x80dfec`), `0.01` (`0x8029d0`, used as the length margin of the downward ground cast) |
| collision size | per unit: radius at `+0xb0`, height at `+0xb4` |
| functions | `FUN_00633840` builds the query box and fetches triangles; `FUN_00636100` is the move step (sweep, raise-and-retry, then a downward cast to land); `FUN_006334a0` classifies a triangle as walkable; `FUN_00632ba0` is the sweep |

**Ported.** The movement is now implemented from these routines in `crates/client/src/movement.rs`: `FUN_00634040` (integrator),
`FUN_006367b0` (ground move), `FUN_00636100` (step probe), `FUN_00635c00` / `FUN_00635d80` (slope / wall slide), `FUN_006334a0`
(walkability), `FUN_00635b00` / `FUN_00635600` (falling). Findings that mattered: the hull is a box with a **pyramid foot**
(apex at the feet, ring at `radius * 1.8494`, `0x80c740`); unit defaults radius 1/3, height 2.0278, reach 1.0 (`FUN_00616fa0`);
the post-move ground cast is `distance * 1.8494 + 1/36` (plus `reach` right after stepping), and falling only starts when it
finds nothing; flag `0x4000000` marks "stepped" (steep upward surfaces then count as walkable), `0x40000000` is hover,
`0x2000` is falling. WMO collision uses all group triangles (every triangle is in the BSP face list); terrain collision is
built from the chunk height grid, skipping hole cells. Unresolved: `FUN_00635f80` (a fall simulation inside the step probe, not ported)
and the exact hull/triangle clipping (`FUN_00631be0`, `FUN_00632280`, `FUN_00632700`), replaced by an exact separating-axis sweep.

**WMO triangle flags** (`MOPY`): rendered faces carry `0x20`, "detail" `0x04`, invisible collision geometry `0x08`,
BSP hints `0x10`. An earlier rule that took `0x04` as "collision" wrongly excluded most interior floors (all `0x20`/`0x60` faces).
Everything except pure hints is now solid (`wowfmt::wmo::is_solid`).

## Liquids (confirmed)

Texture sets are hard-coded (`0x86a128`-`0x86a1dc`): `XTextures\river\lake_a.%d.blp`, `...\river\fast_a.%d.blp`,
`...\ocean\ocean_h.%d.blp`, `...\lava\lava.%d.blp`, `...\slime\slime.%d.blp` (30 frames each, `fast_a` has 16),
plus `splash\splash.blp`, `splash\wake.blp`. `LiquidType.dbc` only has `ID, name, flags, spell`, no paths.
Water and ocean frames are black RGB with the ripple pattern in the **alpha** channel; lava and slime are normal RGB.

## Not found (open questions)

- **ADT chunk tags.** `MCNK`, `MCVT`, `MCLY`, `MCAL`, `MCLQ` (either byte order) occur neither as 4-byte
  data nor as 32-bit instruction immediates anywhere in this `WoW.exe`, although `WDBC` and other literals
  are found by the same search. How the client identifies chunks is unresolved, so the
  chunk layouts in `wowfmt` were validated against the **game data** (they render correct terrain) instead of
  against this binary's parser.
- Archive priority direction (above), run speed and jump velocity (above).

## Format quirks found while building the readers

These are facts about the data, found by testing against the shipped files:

- **MCLQ**: the sub-chunk's own size field is `0`; the length comes from `sizeLiquid` in the MCNK header
  (812 = 8 header + 804 data: min/max, 81 x (u32 flow, f32 height), 64 tile flags, flow vectors).
  Vertices without liquid hold `FLT_MAX`. Tile flag `0x0F` = no liquid.
- **Alpha maps**: 4-bit, 64 x 64, low nibble first; unless MCNK flag `0x8000` is set, the last row/column
  are junk and must be copied from the previous ones (otherwise visible seams on every chunk edge).
- **MCNK offsets** are relative to the chunk *header*, i.e. 8 bytes before the payload.
- **Chunk orientation**: `index_x` runs east and `index_y` runs south. Inside a chunk, grid rows run south and
  columns run east (world `x` decreases with the row, world `y` decreases with the column).
- **Holes**: `MCNK.holes` is a 4 x 4 bit grid; each bit hides a 2 x 2 block of cells. Northshire has one near the
  start area that renders as a gap showing the sky; presumably an object (cave/mine entrance) fills it in the real
  game, which this project does not draw yet.
- **Tile index** in `WDT.MAIN` is `y * 64 + x`, where `x` is the *first* number in `Map_x_y.adt` (confirmed in `FUN_006c2720`).

## Models and world objects (derived from the data, not from WoW.exe)

None of this was read out of the binary's parsers (see the open question about chunk tags above); it comes from decoding
the shipped files and checking the results (all 9,677 M2s and 53,192 BLPs parse; Northshire, the Abbey and Stormwind render correctly).

**M2 v256 header.** Arrays are `(count, offset)` pairs, offsets absolute. The 1.12 header carries arrays that most
format write-ups for other versions do not show, and skipping either shifts every field after it:

| Offset | Array | | Offset | Array |
|---|---|---|---|---|
| `0x08` | name | | `0x5c` | textures (16 B: kind, flags, name len, name ofs) |
| `0x14` | global sequences | | `0x64` | transparency tracks |
| `0x1c` | sequences (68 B) | | `0x6c` | **extra array (empty in the models looked at)** |
| `0x24` | animation lookup (u16) | | `0x74` | texture transforms (84 B: translation, rotation, scale tracks) |
| `0x2c` | **playable-animation lookup (4 B entries)** | | `0x7c` | replaceable-texture lookup (u32) |
| `0x34` | bones (108 B, see below) | | `0x84` | render flags (u16 flags, u16 blend) |
| `0x3c` | key-bone lookup | | `0x8c` | bone lookup |
| `0x44` | vertices (48 B) | | `0x94` | texture lookup (u16) |
| `0x4c` | views (44 B each, **embedded**; there are no `.skin` files) | | `0x9c` / `0xa4` / `0xac` | texture-unit / transparency / UV lookups |

The header's bounding box is not at the offset you would expect, so bounds are computed from the vertices.
View 0 holds the vertex lookup, triangle list, submeshes (32 B: id, level, vstart, vcount, **istart, icount**, ...) and
batches (24 B). Batch fields: `flags, priority, shader, submesh@4, geoset, colour, material@10, layer, count, texture combo@16, ...`
(`material` indexes the render-flags array, `texture combo` the texture lookup, which indexes the textures).
Render flags: `0x01` unlit, `0x04` two-sided; blend `0` opaque, `1` alpha key, `2` alpha, `3/4` additive.
The mistake that cost the most time: using the wrong field for `material`/`texture combo` still gave plausible textures
for models whose indices were all 0, and only showed up as opaque olive canopies on trees.

**M2 animation (v256).** Verified by posing the Human male: bones are 108 bytes (key bone `i32`, flags `u32`, parent `i16`,
mesh part `u16`, then three 28-byte animation blocks for translation, rotation and scale, then the pivot `vec3`; the older
note of 112 bytes was wrong). An animation block is `type u16, global sequence i16, ranges (n, ofs), times (n, ofs), keys (n, ofs)`.
`times` are absolute milliseconds on the model's timeline and `ranges[sequence]` is the inclusive `(first, last)` key index
for that sequence (a sequence's keys lie between its start and end); rotation keys are four **floats** (x, y, z, w), not
compressed. Sequences are 68 bytes (`id u16, variation u16, start u32, end u32, move speed f32, flags u32, ...`); ids used:
0 stand, 4 walk (2.5 yd/s), 5 run (6.94 yd/s, close to the 7.0 run speed), 13 walk back, 17 attack 1H, 24 shield block,
37/38 jump start/loop. Vertices carry 4 bone indices (global bone ids) and weights (out of 255). Bone matrix:
`T(pivot) T(translation) R S T(-pivot)`, times the parent's. Attachments (48 B, after the collision arrays at `0x104`):
id 1 right hand, 0 shield (left forearm), 2 left hand, 11 head; position is in model space, the matrix is `bone * T(position)`.
Arm bones (Human male): each bone's pivot is the joint at its own base, so the chain is shoulder 66/65 (turns the upper arm)
-> elbow 79/78 (turns the forearm; the shield mount bone 98 is its child) -> wrist 97/95 (turns the hand), with the fingers
and the grip bone 104/105 hanging off the wrist; there are no separate forearm or hand bones. Sequence 15 (`HandsClosed`, one
frame) curls the finger bones and is what the client plays on a hand holding an item. The default look also needs part 1501 (the
"no cape" variant), which closes the neck and upper back; without it the back is open.
Character meshes use submesh ids `group * 100 + variant` (hair styles are 1..13); the default look shows variant 01 of
groups 1-5 and 7, plus one hair style. The body skin atlas (256x256) takes the face-lower at (0,192), face-upper at
(0,160) and underwear (pelvis) at (128,96) from `CharSections`-named files. Item models take their object skin (texture
type 2) from a file named like the model.

**Audio (derived from the data; the original used FMOD, so there is no code to port).** `AreaTable` fields 7 / 8 / 9 are the area's
ambience id (`SoundAmbience`: day and night `SoundEntries` ids), zone music id (`ZoneMusic`: silence min / max in ms for day and night,
day and night entries) and intro id (`ZoneIntroMusicTable`: entry, minutes between plays); a subzone with 0 inherits its parent's
music and ambience. `SoundEntries`: name at 2, ten file names from 3, directory at 23, volume (float) at 24. Footsteps:
`CreatureModelData` (model path at 2, sound data id at 13) -> `CreatureSoundData` (footstep set at 9, exertion sound at 1; Human
male = set 7, the "CharacterMediumLarge*" entries); `FootstepTerrainLookup` (set, terrain **sound id**, entry) is keyed by
`TerrainType.SoundID` (field 4), not by the terrain type id, which is off by one for most surfaces. Sword swing: `WeaponSwingSounds2`
(weight 1 = medium). Default volumes are the cvar defaults in WoW.exe: `MusicVolume` 0.4, `AmbienceVolume` 0.6. Files: music is MP3,
effects and ambience are PCM WAV, but the stereo WAVs write a wrong block align (2 for 16-bit stereo, which needs 4) that decoders
reject; correcting that field fixes them.

**BLP2.** Some DXT3/5 files store their smallest mip levels at half size; decoding must end the chain there instead of
failing the texture (level 0 is still required). Liquid textures (`lake_a`, `ocean_h`) are black RGB with the pattern in alpha.

**WMO v17.** `MOHD` -> material and group counts; `MOMT` 64 B per material (`flags`, `blend` at +8, texture name offset into
`MOTX` at +12); each group file `name_NNN.wmo` has `MOGP` = 68-byte header followed by `MOVT`/`MONR`/`MOTV`/`MOVI`/`MOBA`
(24 B batches: start index at +12, count at +16, material at +23). Triangles not covered by a batch are collision-only.

**Collision data.** WMO `MOPY` flags per triangle: `0x20` render, `0x04` collision, `0x08` no-material helper (invisible),
`0x40` extra bit seen on rendered triangles; in the gold mine, rendered solid surfaces are `0x24`/`0x64`, decoration is
`0x20`/`0x60`. The collision rule used is "bit `0x04` set"; this follows the observed combinations and the usual community
description, it was not read from the client. M2 collision mesh: header offsets `0xb4` (bounding box, 24 B + radius),
`0xd0` (collision box + radius), `0xec` triangle indices (u16), `0xf4` vertices (vec3), `0xfc` normals.

**Placements (`MDDF`/`MODF`).** Positions are (east, height, south) in yards from the map's north-west corner: render
position = file position - 17066.67 on x and z. Rotation is in degrees, applied as translate, `Ry(heading - 90)`, `Rz(-rx)`,
`Rx(rz)`, scale. Objects that cross a tile border are listed in every ADT they touch, so a tile only spawns placements whose
position lies inside it (otherwise doodads and WMOs appear several times; Stormwind is listed in neighbouring tiles).
Model name case and extension differ from the file names (`.mdx`/`.mdl` in ADTs, `.m2` in the archives).

**Animation engine (read from WoW.exe, `FUN_007121a0` and the resolvers `FUN_005fd100/005fd3f0/005fd830`).** M2 sequence
records (68 B) hold, besides id/variation/range/speed/flags: `probability` (+20, u16, weight among the sequences of one id),
`replayMin/Max` (+24/+28, how often a played sequence repeats before a sibling is picked), `blendTime` (+32 ms) and
`nextVariation` (+64, i16). A change of animation fades over the *new* sequence's blend time with a smoothstep of the old
one's weight. The unit has two channels, the whole body and an upper body (bone 22, the chest, and its descendants) that carries
attacks and the ready stance. Walk/run is chosen by speed against twice the walk speed, backing up is 13, strafing uses walk/run,
turning in place (11 left / 12 right) only when not moving, otherwise stand; the ready stance layers only in combat.

**Sky (WoW.exe `0x6cf000..0x6d7a00`, `0x7e5000..`).** `FUN_006d64d0` builds the light state of a `LightParams` row: int bands 0..17 go to
direct, ambient, sky top, middle, three lower sky bands (4, 5, 6), fog (7), band 8 (unused here), sun and moon tint (9), cloud colours (10 lit,
11 shade, 12 ~black), then water; float bands 0..5 are fog end, fog start scale, ?, **cloud cover (3)**, ?, ?. `Light.dbc` field 7 is the clear
params, 8 under water, **9 storm**, 10 storm under water, 11 death; `LightParams` field 1 is `highlightSky` (the sunset glow), field 2 the
`LightSkybox` row (only some dungeons use one; the open world has none). The sky dome (`FUN_006d0d10`) is a cap: apex, five rings of 24
at 0.17/0.2/0.23/0.24/0.25 pi from the apex (the horizon ring at 0.25 pi, apex 0.29 above it) and a nadir; vertex colours (`FUN_006d0f50`) are
top, middle, bands 4-6 and the fog colour at the horizon, with the glow (`T(day) * highlightSky`; `T` peaks at 6:30 and 21:30) keeping the
colours towards the sun's bearing and fading the rest to the middle band. The bearing profile (table `0xce9af8`: 1 under the sun, 0 at 90
degrees, -0.5 to -0.7 opposite) is in fractions of the circle with the sun at 0.125 = 45 degrees. The cloud dome (`FUN_006d0530`) is 12 rings
of 16 up to the horizon (alpha 1 down to 36 degrees, 0.5 at 41, 0 at 44 and 45), the cloud map laid flat over it, filled by `FUN_006cffc0`
with 4 octaves of value noise and shaded in `FUN_006cfb00` as `band11 * b + band12 + band10 * lit`. Sun and moon: `FUN_006d3b80` (key
tables built there and in `FUN_006ce4a0..006ce560`, day fraction `t = hour / 24`): direction `(sin th cos ph, sin th sin ph, cos th)`
in world space (x north, y west, z up) times 12 from the camera, both with bearing 45 degrees; sun angle from the zenith 100 degrees at 5:30,
5 at noon, 100 at 21:30; moon 100 at 22:00, 35 at midnight, 100 at 4:00; sun disc side 2 near the horizon and 1 by day (`Textures\sunCenter.blp`),
moon side 1.75 x (1..1.5) (`Textures\moon.blp`), both tinted with band 9; the sprite is a quad `FUN_006d1710`, fading over the 0.4 units above
the horizon (`FUN_006d1960`). `Textures\moonGlare.blp`, `sunGlare.blp` are lens-flare sprites (`FUN_006cf490`, which depend on the camera
facing the sun) and `Textures\moon02.blp` a second moon whose colour is never set (not drawn). Stars: `Environments\Stars\Stars.m2` (the model
`FUN_007e60f0` makes), drawn when its alpha `254 * T + 1` exceeds 1 (`T`: 1 from 22:30 to 3:00, 0 from 4:30 to 22:30, table `0xce9a98`).
The cloud picture (128 or 256 texels, bands of 32 rows every 0.1 s; `clouds.rs` ports it): 4 octaves of 3D value noise, steps 16/32/64/128 (LOD 0) or 8/16/32/64 (LOD 1) 256ths of a cell per texel, x and z both start at a counter that goes up by one per full picture, lattice = 256 random floats in (-1, 1], permutation at `0x86f2d0`, fade `(1 - cos(i pi / 256)) / 2`; `index = floor(sum * 64 + 128)`, `density = table[index - (1 - cover) * 255]` with `table[j] = 255 - 255 * 0.96^(j * 0.5977)`; colour `band11 * b + band12 + band10 * lit`, `b = (64 + (255 - density) / 2) / 255`, `lit` the cosine between the bump normal (slope of the first 3 octaves, 1) and the vector from the texel to the sun's (by night the moon's) place on the picture at height `64 + 192 * storm`, times `(1 - 0.75 * storm)`. Cloud LOD cvar `SkyCloudLOD` (0 or 1; texture 128 or 256), draw order in `FUN_006d4940`.

**Weather (`WorldClient/MapWeather.cpp`, `0x674130..0x67e300`).** `FUN_0067baf0(type, strength, instant)` (called for the server's weather
packet, `FUN_0048f690` case `0x2f4`) with types 1 rain, 2 snow, 3 sandstorm; `FUN_0067bc70` moves the strength over `(|to - from| + 0.001) * 10`
seconds; the sky gets `min(strength, 0.25) * 4` (the sun, moon, stars fade out and the storm light params blend in, `FUN_006d2210`) and the
emitters `(strength - 0.25) * 4/3`. Cvars `weatherDensity`, `useWeatherShaders` (without vertex programs the weather is off), `force-weather-type-on`.
Each type is a box round the camera (rain 130 x 130 x 75, snow 90 x 90 x 60, sand 40 x 40 x 25) plus a layer of mist puffs (`FUN_0067a5b0`, 12-yard
`SnowMist01` / `WeatherMistGrainy01` pictures, 2.7 s, count `(density - 0.5) * 2 * 0.66 * 38 / 48 / 64`, faded out nearer than 6 to 18 yards).
Particles per second at full density: `0.66 * 35000` rain, `0.66 * 14000` snow, `0.66 * 32000` sand; each has position, velocity, birth and death
(a death time from a ray to the ground or a roof), made in `FUN_00674c50` (rain: heading `-1.57 +- (0.052 + 0.209 d) / 2`, horizontal speed
`(9.49 +- 1) d + 0.01`, vertical `-28 - 4d - 2d r`), `FUN_00677750` (snow: heading spread `2 pi - 5.93 d`, horizontal `(5.985 +- 0.5) d`, vertical
`-2 - 3.5 d - d r`) and `FUN_006793f0` (sand: from 34 to 40 yards ahead, 18.7 yards a second towards the camera, alive at most one second);
positions are shifted by the camera's velocity times 1.75 s and leaned towards the way you go by `lerp(0, 65 deg, speed / 18)`; rain landing while the
camera stands nearly still (speed squared <= 2) leaves a splash record (`FUN_00675280`: 0.25 s). The vertex programs are ARB text inside
`Shaders\Vertex\rain.bls`, `snowpoint.bls`, `sand.bls`, `patter.bls` (read them with `wowtool extract`, then look for `!!ARBvp1.0`): rain = triangle,
corners at the head (+-0.05 yards across) and 2 yards up the trajectory, tail still falling 2 yards of fall time after the head's death, texture factor
`0x80`; snow = point of `clamp(1 - 0.02 d) * 14` pixels, fade in over the first second and out over 0.25 s; sand = point of
`clamp(1.5 - 0.05 d) * width * 0.0025` pixels, fades 0.2 s each way; splash = 4 x 4 atlas, column by `age * 4 * 3.99`, row by the viewing angle.

## Reproducing

```powershell
cd rsworld\re
.\analyze.ps1                          # import + auto-analyse WoW.exe, export functions/strings (~3 min)
python extract_sources.py              # list the original source files
.\query.ps1 queries\12_physics.txt     # run a batch of Ghidra queries (see ghidra-scripts\Query.java for the commands)
```

`Query.java` commands: `xrefs`, `decomp`, `scalar` (instruction immediates), `strxrefs`, `callers`, `callees`,
`bytes`, `findbytes` (ASCII, or `hex:` with `??` wildcards). The batches that produced the facts above are in `re/queries/`.

## Swimming (read from WoW.exe, 2026-10-19)

`FUN_006030c0` (per-update water state): swimming starts when the water is deeper than `height * 0.75` (0x8012cc; the unit's collision height, 2.03 for a human = 1.52 yd) and stops 1/36 yd under that (0x7ff9d0), or when there is no liquid. Movement while swimming flag `0x200000` is set is `FUN_00634640`: a straight 3-D sweep along the move direction (z included, no gravity). Animation ids: 41 SwimIdle, 42 Swim, 43 SwimLeft, 44 SwimRight, 45 SwimBackwards (the human swim pose is horizontal, back at ~1.6 yd over the origin, so feet at surface - 1.52 puts the back at the water line). Footstep spray: `FUN_005fbf70` (a footstep event of the animation) spawns the hard-coded effect "Footstep Water Run/Walk Spray" (`Particles\FootStepSprayWater[Walk].m2`, run when faster than twice the walking speed 2.5) at the surface when the water is less than `height * 0.5` deep.

## Ripples (water decals; `FUN_005fa760` makes, `FUN_0068f5c0` builds, `FUN_0068fae0` updates and draws)

`FUN_005fa760(state)` runs every update of a unit in liquid; it does nothing unless `depth = level - feetZ < max(2 * height, 1)` and its timer (+0xc78, ms) has passed. State: 0 idle, 1 turning (keys 0x30), 2 moving (flags 0xf), 3 = the splash `FUN_006030c0` asks for when the depth crosses `height * 0.4` either way (clears the timer). A ripple: radius0 = clamp(scale/3 * (0.9 + 0.2 rand), 1/3, 5/3); life = 0.6 + 0.1 rand; growth (yd/s) = (1 + 0.5 rand) * pace, with pace = min(speed, 20) / 2.5 when moving; peak alpha 1 (a4 = 1/6; below 1/6 the alpha is 6 * a4). Deeper than the height: factor f = (H2 - depth) / (H2 - H2/2) * 0.5 + 0.5 (H2 = 2 * height) scales radius, life and alpha. Idle (state 0): alpha x 0.8, radius x 0.6, growth x 0.25. Moving: angle = facing turned by the strafe keys (left +45/135/90 deg, right the same negative, backing +180), next ripple after `f * 0.25 / pace` s; otherwise a random angle and the next after 0.4 + rand(0..50 ms). Ripples that move are group 1 (`wake.blp`, a V), the others group 0 (`splash.blp`, a ring); pools of 32 (the local player) and 96 (others). `FUN_0068f5c0`: the decal is the liquid triangles within the box of the final radius (`radius0 + growth * life`) round the unit, in its own vertex list; `FUN_0068fae0`: radius += growth * dt; alpha rises to the peak in 0.4 of the life (table 0x810348) and falls to 0 over the rest; drawn white with that alpha, textured by planar projection (rotated by the angle, 2 * radius wide, centred), additive blend (render state 7 = 3), depth bias `footstepBias` 0.125. The textures are dark gray (max 47/255) with a binary alpha, so the effect is faint by design. Mode `DAT_00c89db8` is always 0 (the "WaterRadWave" objects of mode 1 are never used).

Swim speeds (4.7222 / 2.5 back) are server values, no constants in the client.

## Water sounds, blend mode, unit size (2026-10-19)

**Swimming has no stroke sound.** No swim animation (ids 41-45) of any race or creature model has sound events ($FSD/$CSD...), `SoundEntries` has no swim entry, and the client has no code that plays one. What a swimmer hears: the splash when the water crosses `height * 0.4` in either direction (`FUN_006030c0` -> `FUN_006237c0`, plays the unit's own splash entry; `SoundEntries` "CharacterSplashSoundSmall/Medium/large" 1090/1096/1097, chosen here by height < 1.6 / < 2.7 / more, the choice itself was not found), the splash of footsteps when wading (`FUN_00623390`, the $FSD events of walk and run), the ambient liquid loops (`SoundWaterType`), and, with the camera under the surface, the "Underwater (DONOTRENAME)" entry (`UndwaterLoop.wav`) instead: `FUN_00511a80` sets `DAT_00835a44` from the liquid at the camera (`DAT_00c7f288`, `FUN_006809c0`: the lowest corner of the near plane in liquid), `FUN_00460bd0` then picks it as the music in place of the zone's, and `FUN_00462b50` stops looking for the liquids round the ears.

**Blend mode** (verified in the GL backend, `FUN_0059d350`): state 7 value v >= 2 is `glBlendFunc(src[v], dst[v])` with src = {2: SRC_ALPHA, 3: SRC_ALPHA, 4,5,6: DST_COLOR} and dst = {2: 1-SRC_ALPHA, 3: ONE, 4: ZERO, 5: SRC_COLOR, 6: ONE}; 0 and 1 (alpha key, ref 0xe0) do not blend. So the ripples' 3 is (SRC_ALPHA, ONE).

**Unit size**: the water code reads the unit's height (+0xa5c) and its scale (vfunc 0x1c); both come from the model: `CreatureModelData.dbc` collision height (field 15; width field 14) times the display scale (`CreatureDisplayInfo` field 4) times the model scale (field 4). A human man: 2.031 and 1.0; the movement hull keeps the constructor's 0.3333 / 2.0278.

**Buildings' liquids** (`MLIQ`) take part in swimming, wading, ripples and the sounds of water like the terrain's.
