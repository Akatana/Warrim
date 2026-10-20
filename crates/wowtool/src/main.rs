use anyhow::{bail, Context, Result};
use mpq::Mount;
use std::path::PathBuf;
use wowfmt::{adt::Adt, blp::Blp, dbc::Dbc, m2::M2, wdt::Wdt, wmo};

const USAGE: &str = "\
wowtool [--wow <dir>] <command>

  archives                       mounted archives in priority order
  where <path>                   which archive serves a file
  list [substring]               names from all (listfile)s, optionally filtered
  extract <path> [out]           write a file from the archives to disk
  dbc <Name> [rows]              dump a DBFilesClient table (Name without .dbc)
  blp <path> <out.png> [level]   convert a texture to PNG
  wdt <Map>                      which tiles a map has
  adt <Map> <x> <y>              summarise a terrain tile
  verify [substring]             decode every listed file and report failures by cause
  m2 <path>                      summarise a model (.m2)
  m2all                          parse every .m2 and report failures
  wmo <path>                     summarise a WMO and all its groups
  blpall                         decode every .blp and report failures
  wmo-near <wmo> <posx> <posy> <posz> <rot_y> <wx> <wy> <wz> <r>   raw triangles (with flags) of a placed WMO near a WoW-coordinate point
  probe <path>                   per-archive block info (flags, sizes) for a path
";

fn main() -> Result<()> {
    env_logger::init();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut wow = None;
    if args.first().map(String::as_str) == Some("--wow") {
        if args.len() < 2 {
            bail!("--wow needs a directory");
        }
        wow = Some(PathBuf::from(args.remove(1)));
        args.remove(0);
    }
    let Some(cmd) = args.first().cloned() else {
        print!("{USAGE}");
        return Ok(());
    };
    let wow = wow
        .or_else(mpq::find_wow_dir)
        .context("could not find WoW directory (run inside it, pass --wow, or set WOW_DIR)")?;
    let mount = Mount::open_wow_dir(&wow).with_context(|| format!("mounting {}", wow.display()))?;
    let rest = &args[1..];
    let arg = |i: usize| rest.get(i).map(String::as_str).with_context(|| format!("missing argument\n\n{USAGE}"));

    match cmd.as_str() {
        "archives" => {
            for a in mount.archives() {
                println!("{:>8} files  {}", a.file_count(), a.path().display());
            }
        }
        "where" => match mount.locate(arg(0)?) {
            Some(p) => println!("{}", p.display()),
            None => bail!("not found"),
        },
        "list" => {
            let filter = rest.first().map(|s| s.to_ascii_lowercase());
            for name in mount.list() {
                if filter.as_ref().is_none_or(|f| name.to_ascii_lowercase().contains(f)) {
                    println!("{name}");
                }
            }
        }
        "extract" => {
            let name = arg(0)?;
            let data = mount.read(name)?;
            let out = rest
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(name.rsplit(['\\', '/']).next().unwrap()));
            std::fs::write(&out, &data)?;
            println!("{} bytes -> {}", data.len(), out.display());
        }
        "dbc" => {
            let dbc = Dbc::parse(&mount.read(&format!("DBFilesClient\\{}.dbc", arg(0)?))?)?;
            println!("{} records, {} fields, record size {}", dbc.record_count, dbc.field_count, dbc.record_size);
            let rows: usize = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
            for rec in dbc.iter().take(rows) {
                let cells: Vec<String> = (0..dbc.field_count)
                    .map(|f| {
                        let v = rec.u32(f);
                        let s = rec.string(f);
                        // Heuristic: show a field as text when it is a plausible string offset.
                        if v != 0 && s.len() >= 3 && s.chars().all(|c| !c.is_control()) {
                            format!("{f}:\"{s}\"")
                        } else {
                            format!("{f}:{v}")
                        }
                    })
                    .collect();
                println!("{}", cells.join(" "));
            }
        }
        "blp" => {
            let blp = Blp::parse(&mount.read(arg(0)?)?)?;
            let level: usize = rest.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let level = level.min(blp.mips.len() - 1);
            let (w, h) = blp.level_size(level);
            println!("{}x{} alpha={} mips={}", blp.width, blp.height, blp.has_alpha, blp.mips.len());
            let file = std::fs::File::create(arg(1)?)?;
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header()?.write_image_data(&blp.mips[level])?;
        }
        "wdt" => {
            let map = arg(0)?;
            let wdt = Wdt::parse(&mount.read(&format!("World\\Maps\\{map}\\{map}.wdt"))?)?;
            println!("flags {:#x}, {} tiles", wdt.flags, wdt.tile_count());
            for y in 0..64 {
                let row: String = (0..64).map(|x| if wdt.has_tile(x, y) { '#' } else { '.' }).collect();
                if row.contains('#') {
                    println!("{y:02} {row}");
                }
            }
        }
        "adt" => {
            let map = arg(0)?;
            let (x, y): (u32, u32) = (arg(1)?.parse()?, arg(2)?.parse()?);
            let adt = Adt::parse(&mount.read(&format!("World\\Maps\\{map}\\{map}_{x}_{y}.adt"))?)?;
            println!("textures: {}", adt.textures.len());
            for (i, t) in adt.textures.iter().enumerate() {
                println!("  [{i}] {t}");
            }
            println!("m2 models: {}  placements: {}", adt.m2_names.len(), adt.doodads.len());
            for n in adt.m2_names.iter().take(1000) {
                println!("  {n}");
            }
            if let Some(d) = adt.doodads.first() {
                println!("  first placement: model {:?} pos {:?} rot {:?} scale {}", adt.m2_names.get(d.name_index), d.position, d.rotation, d.scale);
            }
            for d in adt.doodads.iter().filter(|d| adt.m2_names.get(d.name_index).is_some_and(|n| n.to_ascii_lowercase().contains("waterfall"))) {
                println!("  waterfall {:?} pos {:?} rot {:?} scale {}", adt.m2_names.get(d.name_index), d.position, d.rotation, d.scale);
            }
            for w in adt.wmos.iter() {
                println!("  wmo {:?} pos {:?} rot {:?} flags {:#x} set {}", adt.wmo_names.get(w.name_index), w.position, w.rotation, w.flags, w.doodad_set);
            }
            println!("wmos: {}  placements: {}", adt.wmo_names.len(), adt.wmos.len());
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for c in &adt.chunks {
                for &h in &c.heights {
                    lo = lo.min(h);
                    hi = hi.max(h);
                }
            }
            println!("height range: {lo:.1} .. {hi:.1}");
            for (ix, iy) in [(0usize, 0usize), (15, 0), (0, 15), (15, 15)] {
                let c = &adt.chunks[iy * 16 + ix];
                println!(
                    "chunk[{ix},{iy}] header idx=({},{}) pos=({:.2},{:.2},{:.2}) area={} layers={} liquid={}",
                    c.index_x, c.index_y, c.position[0], c.position[1], c.position[2], c.area_id, c.layers.len(), c.liquid.is_some()
                );
            }
            let mut kinds = std::collections::BTreeMap::new();
            for c in adt.chunks.iter().filter_map(|c| c.liquid.as_ref().map(|l| (c, l))) {
                for f in c.1.cell_flags {
                    *kinds.entry((c.0.flags >> 2 & 0xf, f)).or_insert(0) += 1;
                }
            }
            println!("liquid cells by (MCNK liquid flags, cell flag): {kinds:02x?}");
            let water = adt.chunks.iter().filter(|c| c.liquid.is_some()).count();
            println!("chunks with liquid: {water}");
            for c in adt.chunks.iter().filter(|c| c.liquid.as_ref().is_some_and(|l| l.max_height - l.min_height > 1.0)) {
                let l = c.liquid.as_ref().unwrap();
                let cells = l.cell_flags.iter().filter(|&&f| f & 0x0f != 0x0f).count();
                let mut histogram = std::collections::BTreeMap::new();
                l.cell_flags.iter().for_each(|f| *histogram.entry(*f).or_insert(0) += 1);
                println!("    cell flags {:02x?}", histogram);
                println!("    depths {:?}", l.depths);
                println!("    flows {:?}", l.flows);
                println!("  sloped liquid chunk ({},{}) at x {:.0} y {:.0}: kind {:#x} heights {:.1}..{:.1}, {cells} cells", c.index_x, c.index_y, c.position[0], c.position[1], l.kind, l.min_height, l.max_height);
            }
        }
        "m2" => {
            let m = M2::parse(&mount.read(arg(0)?)?)?;
            println!("{:?}: {} verts, {} indices, bounds {:?}..{:?}", m.name, m.vertices.len(), m.indices.len(), m.bounds_min, m.bounds_max);
            for (i, t) in m.textures.iter().enumerate() {
                println!("  texture[{i}] kind={} flags={:#x} {:?}", t.kind, t.flags, t.name);
            }
            for b in &m.batches {
                println!(
                    "  batch idx {}+{} part {} tex={:?} {:?} two_sided={} unlit={} flags={:#x} mode={} priority={} color={:?} transparency={:?} uv={:?}",
                    b.index_start, b.index_count, b.submesh_id, b.texture, b.blend, b.two_sided, b.unlit, b.render_flags, b.blend_mode, b.priority, b.color, b.transparency, b.uv_anim
                );
            }
            for (i, c) in m.cameras.iter().enumerate() {
                println!("  camera[{i}] {c:?}");
            }
            for (i, c) in m.colors.iter().enumerate() {
                println!("  color[{i}] rgb {} keys {:?}, alpha {} keys {:?}", c.color.keys.len(), c.color.keys.first(), c.alpha.keys.len(), c.alpha.keys.first());
            }
            for (i, t) in m.transparency.iter().enumerate() {
                println!("  transparency[{i}] global {} ranges {:?} times {:?} keys {:?}", t.global_sequence, t.ranges, t.times, t.keys);
            }
            for (i, u) in m.uv_anims.iter().enumerate() {
                println!("  uv[{i}] translation: global {} interp {} ranges {:?} times {:?} keys {:?}", u.translation.global_sequence, u.translation.interpolation, u.translation.ranges, u.translation.times, u.translation.keys);
                println!("        rotation keys {:?}, scale keys {:?}", u.rotation.keys, u.scale.keys);
            }
            println!("  global sequences {:?}", m.global_sequences);
            for (i, q) in m.sequences.iter().enumerate() {
                println!("  sequence[{i}] id {} {}..{} flags {:#x}", q.id, q.start, q.end, q.flags);
            }
            for e in &m.events {
                println!("  event {:?} data {} bone {} at {:?}", String::from_utf8_lossy(&e.id), e.data, e.bone, e.position);
            }
            println!("  particles {} bones {} attachments {}", m.particles.len(), m.bones.len(), m.attachments.len());
            let first = |t: &wowfmt::m2::M2Track<f32>| t.keys.first().copied();
            for (i, p) in m.particles.iter().enumerate() {
                println!(
                    "  particle[{i}] flags {:#x} at {:?} bone {} texture {} blend {} emitter {} type {} head/tail {} grid {}x{} speed {:?} variation {:?} vertical {:?} horizontal {:?} gravity {:?} life {:?} rate {:?} area {:?}x{:?}",
                    p.flags, p.position, p.bone, p.texture, p.blend, p.emitter_type, p.particle_type, p.head_or_tail, p.rows, p.cols, first(&p.speed), first(&p.variation), first(&p.vertical_range), first(&p.horizontal_range), first(&p.gravity), first(&p.lifespan), first(&p.rate), first(&p.area_length), first(&p.area_width)
                );
                println!("        colours {:08x?} sizes {:?} scales {:?} mid {} slowdown {} head {:?} tail {:?} {}", p.colors, p.sizes, p.scales, p.mid, p.slowdown, p.head_cells, p.tail_cells, p.tail_length);
            }
            for (i, b) in m.bones.iter().enumerate() {
                if !b.rotation.keys.is_empty() {
                    println!("    rotation: global {} interp {} ranges {:?} times {:?} keys {:?}", b.rotation.global_sequence, b.rotation.interpolation, b.rotation.ranges, b.rotation.times, b.rotation.keys);
                }
                println!("  bone[{i}] parent {} flags {:#x} pivot {:?} translation keys {} rotation keys {} scale keys {}", b.parent, b.flags, b.pivot, b.translation.keys.len(), b.rotation.keys.len(), b.scale.keys.len());
            }
        }
        "m2all" => {
            let (mut ok, mut failed) = (0usize, std::collections::BTreeMap::<String, (usize, String)>::new());
            for name in mount.list().into_iter().filter(|n| n.to_ascii_lowercase().ends_with(".m2")) {
                match mount.read(&name) {
                    Ok(d) => match M2::parse(&d) {
                        Ok(m) if !m.batches.is_empty() => ok += 1,
                        Ok(_) => {
                            let e = failed.entry("no renderable batches".into()).or_insert((0, name.clone()));
                            e.0 += 1;
                        }
                        Err(e) => {
                            let key = e.to_string().split(':').next().unwrap_or("").to_string();
                            let ent = failed.entry(key).or_insert((0, name.clone()));
                            ent.0 += 1;
                        }
                    },
                    Err(_) => {} // removed by a patch
                }
            }
            println!("{ok} models parsed");
            for (cause, (n, example)) in failed {
                println!("{n:>6} x {cause}   e.g. {example}");
            }
        }
        "wmo" => {
            let path = arg(0)?;
            let root = wmo::WmoRoot::parse(&mount.read(path)?)?;
            println!("{} groups, {} materials", root.group_count, root.materials.len());
            for (i, m) in root.materials.iter().enumerate().take(8) {
                println!("  material[{i}] flags={:#x} blend={} {:?}", m.flags, m.blend, m.texture);
            }
            let (mut verts, mut tris, mut batches) = (0, 0, 0);
            let mut hist = std::collections::BTreeMap::<(u8, bool, char), usize>::new();
            let mut slopes = [0usize; 19]; // solid triangles by angle from horizontal, 5-degree bins
            for g in 0..root.group_count {
                let gp = wmo::group_path(path, g);
                match mount.read(&gp).map_err(anyhow::Error::from).and_then(|d| Ok(wmo::WmoGroup::parse(&d)?)) {
                    Ok(grp) => {
                        verts += grp.vertices.len();
                        tris += grp.indices.len() / 3;
                        batches += grp.batches.len();
                        if let Some(l) = &grp.liquid {
                            let mut tiles = std::collections::BTreeMap::<u8, usize>::new();
                            l.tiles.iter().for_each(|&t| *tiles.entry(t).or_default() += 1);
                            println!(
                                "  group {g} liquid {}x{} material {} group flags {:#x} base {:?} h {:.1}..{:.1} tiles {tiles:x?}",
                                l.x_verts, l.y_verts, l.material, grp.flags, l.base,
                                l.heights.iter().cloned().fold(f32::MAX, f32::min), l.heights.iter().cloned().fold(f32::MIN, f32::max)
                            );
                        }
                        for (t, &(f, m)) in grp.tri_info.iter().enumerate() {
                            let Some(tri) = grp.indices.chunks_exact(3).nth(t) else { continue };
                            let v = |i: u16| grp.vertices[i as usize];
                            let (a, b, c) = (v(tri[0]), v(tri[1]), v(tri[2]));
                            let (e1, e2) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
                            // model space is z-up
                            let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
                            let class = if (n[2] / len).abs() >= 0.6428 { 'F' } else { 'W' };
                            *hist.entry((f, m == 0xFF, class)).or_default() += 1;
                            if wmo::is_solid(f) {
                                let deg = (n[2] / len).abs().clamp(0.0, 1.0).acos().to_degrees(); // 0 = flat, 90 = vertical
                                slopes[((deg / 5.0) as usize).min(18)] += 1;
                            }
                        }
                    }
                    Err(e) => println!("  group {g} ({gp}): {e}"),
                }
            }
            println!("total: {verts} vertices, {tris} triangles, {batches} batches");
            println!("solid triangles by slope (degrees from horizontal):");
            for (i, n) in slopes.iter().enumerate().filter(|(_, n)| **n > 0) {
                println!("  {:>2}-{:<2} {n}", i * 5, i * 5 + 5);
            }
            println!("triangle flags (flags, no-material, F=floor-like/W=wall-like): count");
            for ((f, nomat, class), n) in hist {
                println!("  {f:#04x} {nomat:<5} {class} {n}");
            }
        }
        "blpall" => {
            let (mut ok, mut failed) = (0usize, std::collections::BTreeMap::<String, (usize, String)>::new());
            for name in mount.list().into_iter().filter(|n| n.to_ascii_lowercase().ends_with(".blp")) {
                let Ok(data) = mount.read(&name) else { continue };
                match Blp::parse(&data) {
                    Ok(_) => ok += 1,
                    Err(e) => {
                        let key = e.to_string().split('(').next().unwrap_or("").trim().to_string();
                        let ent = failed.entry(key).or_insert((0, name.clone()));
                        ent.0 += 1;
                    }
                }
            }
            println!("{ok} textures decoded");
            for (cause, (n, example)) in failed {
                println!("{n:>6} x {cause}   e.g. {example}");
            }
        }
        "m2anim" => {
            let m2 = M2::parse(&mount.read(arg(0)?)?)?;
            println!("{} bones, {} sequences, {} global sequences", m2.bones.len(), m2.sequences.len(), m2.global_sequences.len());
            for (i, s) in m2.sequences.iter().enumerate() {
                println!("  seq {i:>3} id {:>3}.{} {}..{} ms speed {:.2} flags {:#x} prob {} replay {}..{} blend {} ms next {:?}", s.id, s.variation, s.start, s.end, s.move_speed, s.flags, s.probability, s.replay_min, s.replay_max, s.blend_ms, s.next_variation);
            }
            for (i, a) in m2.attachments.iter().enumerate() {
                println!("  attachment {i} id {} bone {} at {:?}", a.id, a.bone, a.position);
            }
            println!("  attachment lookup {:?}", m2.attachment_lookup);
            for (i, b) in m2.bones.iter().enumerate().filter(|(_, b)| b.rotation.keys.len() > 30).take(2) {
                println!("  bone {i} rotation: type {} seq {} ranges {:?} times {:?} keys {:?}", b.rotation.interpolation, b.rotation.global_sequence,
                    b.rotation.ranges.iter().take(6).collect::<Vec<_>>(), b.rotation.times.iter().take(40).collect::<Vec<_>>(), b.rotation.keys.iter().take(2).collect::<Vec<_>>());
            }
            for (i, b) in m2.bones.iter().enumerate().take(0) {
                println!(
                    "  bone {i} parent {} pivot {:?} T {}/{} keys R {}/{} S {}/{} ranges {:?}",
                    b.parent, b.pivot, b.translation.keys.len(), b.translation.interpolation, b.rotation.keys.len(), b.rotation.interpolation,
                    b.scale.keys.len(), b.scale.interpolation, b.rotation.ranges.iter().take(3).collect::<Vec<_>>()
                );
            }
        }
        "wmo-groups" => {
            let path = arg(0)?;
            let root = wmo::WmoRoot::parse(&mount.read(path)?)?;
            let groups: Vec<_> = (0..root.group_count)
                .map(|g| mount.read(&wmo::group_path(path, g)).map_err(anyhow::Error::from).and_then(|d| Ok(wmo::WmoGroup::parse(&d)?)).ok())
                .collect();
            for (g, grp) in groups.iter().enumerate() {
                let Some(grp) = grp else { continue };
                let (a, b) = grp.bbox;
                println!(
                    "group {g:>3} flags {:#010x} portals {}+{} bbox ({:.0},{:.0},{:.0})..({:.0},{:.0},{:.0}) tris {}",
                    grp.flags, grp.portal_start, grp.portal_count, a[0], a[1], a[2], b[0], b[1], b[2], grp.indices.len() / 3
                );
            }
        }
        "wmo-near" => {
            let path = arg(0)?;
            let f: Vec<f32> = (1..9).map(|i| arg(i)?.parse::<f32>().map_err(anyhow::Error::from)).collect::<Result<_>>()?;
            let (pos, rot_y, target, radius) = ([f[0], f[1], f[2]], f[3], [f[4], f[5], f[6]], f[7]);
            // target WoW (x north, y west, z up) -> render (east, up, south)
            let t = [-target[1], target[2], -target[0]];
            let theta = (rot_y - 90.0).to_radians();
            let (s, c) = theta.sin_cos();
            let to_render = |m: [f32; 3]| {
                let v = [m[0], m[2], -m[1]]; // model (x east, y north, z up) -> render
                let r = [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c];
                [pos[0] - 17066.667 + r[0], pos[1] + r[1], pos[2] - 17066.667 + r[2]]
            };
            let root = wmo::WmoRoot::parse(&mount.read(path)?)?;
            for g in 0..root.group_count {
                let grp = wmo::WmoGroup::parse(&mount.read(&wmo::group_path(path, g))?)?;
                for (ti, tri) in grp.indices.chunks_exact(3).enumerate() {
                    let v: Vec<[f32; 3]> = tri.iter().map(|&i| to_render(grp.vertices[i as usize])).collect();
                    let cen = [(v[0][0] + v[1][0] + v[2][0]) / 3.0, (v[0][1] + v[1][1] + v[2][1]) / 3.0, (v[0][2] + v[1][2] + v[2][2]) / 3.0];
                    if ((cen[0] - t[0]).powi(2) + (cen[2] - t[2]).powi(2)).sqrt() > radius || (cen[1] - t[1]).abs() > 1.5 {
                        continue;
                    }
                    let (flags, mat) = grp.tri_info.get(ti).copied().unwrap_or((0, 0));
                    let e1 = [v[1][0] - v[0][0], v[1][1] - v[0][1], v[1][2] - v[0][2]];
                    let e2 = [v[2][0] - v[0][0], v[2][1] - v[0][1], v[2][2] - v[0][2]];
                    let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
                    println!(
                        "group {g:>2} tri {ti:>5} flags {flags:#04x} mat {mat:>3} solid={:<5} slope {:>3.0} deg  centre wow ({:.2}, {:.2}, {:.2})",
                        wmo::is_solid(flags),
                        (n[1] / len).abs().clamp(0.0, 1.0).acos().to_degrees(),
                        -cen[2],
                        -cen[0],
                        cen[1]
                    );
                }
            }
        }
        "probe" => {
            let name = arg(0)?;
            for a in mount.archives() {
                match a.find(name) {
                    Some(b) => println!(
                        "{:<12} pos={:>10} packed={:>9} size={:>9} flags={:#010x}{}",
                        a.path().file_name().unwrap().to_string_lossy(),
                        b.pos,
                        b.packed_size,
                        b.file_size,
                        b.flags,
                        if b.flags & mpq::FLAG_DELETE_MARKER != 0 { "  DELETE-MARKER" } else { "" }
                    ),
                    None => println!("{:<12} -", a.path().file_name().unwrap().to_string_lossy()),
                }
            }
        }
        "verify" => {
            use std::collections::BTreeMap;
            let filter = rest.first().map(|s| s.to_ascii_lowercase());
            let started = std::time::Instant::now();
            let (mut ok, mut bytes) = (0usize, 0u64);
            let mut failures: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for name in mount.list() {
                if filter.as_ref().is_some_and(|f| !name.to_ascii_lowercase().contains(f)) {
                    continue;
                }
                match mount.read(&name) {
                    Ok(d) => {
                        ok += 1;
                        bytes += d.len() as u64;
                    }
                    // Listed in a (listfile) but shadowed/removed: not a decoder problem.
                    Err(mpq::Error::NotFound(_)) => {
                        let removed = mount.archives().iter().any(|a| a.is_deleted(&name));
                        let cause = if removed { "removed by a patch delete marker" } else { "absent from every archive" };
                        failures.entry(cause.into()).or_default().push(name)
                    }
                    Err(e) => failures.entry(e.to_string()).or_default().push(name),
                }
            }
            println!("{ok} files decoded ({:.0} MB) in {:.1}s", bytes as f64 / 1e6, started.elapsed().as_secs_f32());
            for (cause, files) in &failures {
                println!("{:>6} x {cause}   e.g. {}", files.len(), files[0]);
            }
        }
        other => bail!("unknown command '{other}'\n\n{USAGE}"),
    }
    Ok(())
}
