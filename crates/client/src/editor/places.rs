use super::*;

/// A creature of `creature` standing at `at` (north, west) and facing `facing` degrees, as a new spawn.
pub(super) fn new_spawn(creature: String, map: &str, at: (f32, f32), facing: f32) -> Spawn {
    Spawn { creature, map: Some(map.to_string()), at, facing, wander: crate::content::DEFAULT_WANDER, height: None, respawn: None, path: Vec::new() }
}

/// A new chest like the one selected (its lock and what it holds), else like the first, else a plain one that holds something.
pub(super) fn chest_like(d: &Data, selected: usize) -> content::ChestDef {
    let mut chest = d.cur.chests.get(selected).or(d.cur.chests.first()).cloned().unwrap_or_else(|| content::ChestDef { id: String::new(), model: CHEST_MODELS[0].0.to_string(), at: (0.0, 0.0), facing: 0.0, lock: Some(Level::Novice), loot: "chest_poor".into(), copper: 0, items: Vec::new() });
    let ids: Vec<&str> = d.cur.chests.iter().map(|c| c.id.as_str()).collect();
    chest.id = fresh(&ids, "chest");
    chest
}

/// The places tab: where creatures and chests stand, the places the compass shows and the props.
#[allow(clippy::too_many_arguments)]
pub(super) fn world(g: &mut Gui, d: &mut Data, pick: &mut [Pick; 10], sub: &mut usize, kind: &mut usize, placer: &mut Placer, me: Place, around: Place, actions: &mut Vec<Action>, top: f32, bottom: f32) {
    for (i, name) in ["Creatures", "Chests", "Discoveries", "Props", "Nodes", "Node kinds"].into_iter().enumerate() {
        if g.button(8.0 + i as f32 * 108.0, top, 102.0, name) {
            *sub = i;
        }
    }
    g.fill(8.0 + *sub as f32 * 108.0, top + ROW, 102.0, 2.0, GOLD);
    let body = top + 28.0;
    let rows = ((bottom - body - 26.0) / ROW) as usize;
    match *sub {
        0 => spawns(g, d, &mut pick[0], kind, placer, me, around, actions, body, bottom, rows),
        1 => chests(g, d, &mut pick[1], placer, me, actions, body, bottom, rows),
        2 => pois(g, d, &mut pick[6], me, actions, body, bottom, rows),
        3 => props(g, d, &mut pick[7], placer, me, actions, body, bottom, rows),
        4 => nodes(g, d, &mut pick[8], placer, me, around, actions, body, bottom, rows),
        _ => node_kinds(g, d, &mut pick[9], placer, body, bottom, rows),
    }
    // Esc ends the putting down, and so does leaving the tab of what is put down.
    if placer.placing && !g.blocked && g.input.keys.contains(&Key::Escape) {
        placer.placing = false;
    }
    placer.placing &= matches!((placer.aim, *sub), (Aim::Creature | Aim::Stop, 0) | (Aim::Chest, 1) | (Aim::Prop, 3) | (Aim::Node, 4));
}

/// The distance in yards from the player to a place (north, west).
fn yards_from(me: Place, north: f32, west: f32) -> f32 {
    ((north - me.north).powi(2) + (west - me.west).powi(2)).sqrt()
}

/// The button that starts putting something down by clicks in the world, or stops it.
fn put_down_button(g: &mut Gui, y: &mut f32, x: f32, placer: &mut Placer, aim: Aim, what: &str, facing: f32) {
    if placer.placing && placer.aim == aim {
        if g.button(x, *y, 260.0, "Stop putting down (Esc)") {
            placer.placing = false;
        }
    } else if g.button(x, *y, 260.0, "Click in the world to put it down") {
        placer.start(aim, what, facing);
    }
    *y += ROW + 6.0;
}

/// The creatures standing in the world.
#[allow(clippy::too_many_arguments)]
fn spawns(g: &mut Gui, d: &mut Data, p: &mut Pick, kind: &mut usize, placer: &mut Placer, me: Place, around: Place, actions: &mut Vec<Action>, body: f32, bottom: f32, rows: usize) {
    // Of the world's tens of thousands only those near the editor's eye are listed (a line left empty is not shown).
    let lines: Vec<String> = d.cur.spawns.iter().map(|s| if near_here(s.at, around) { format!("{} {:.0}yd  ({:.0}, {:.0})", s.creature, yards_from(me, s.at.0, s.at.1), s.at.0, s.at.1) } else { String::new() }).collect();
    let kinds: Vec<String> = d.cur.creatures.iter().map(|c| c.id.clone()).collect();
    *kind = (*kind).min(kinds.len().saturating_sub(1));
    let (made, gone) = list_header(g, body, "delete-spawn", "New here", !d.cur.spawns.is_empty());
    if made {
        if let Some(creature) = kinds.get(*kind) {
            d.cur.spawns.push(new_spawn(creature.clone(), me.map, (me.north, me.west), me.yaw));
            p.selected = d.cur.spawns.len() - 1;
            p.reveal = true;
        }
    }
    list(g, 8.0, body + 26.0, LIST_WIDTH, rows, &lines, p);
    let selected = p.selected;
    // The form scrolls: the patrol can be long.
    form(g, p, PANE_X, body, PANE_W, bottom, |g, y, _| {
        let (x, w) = (PANE_X, PANE_W);
        if let Some(s) = d.cur.spawns.get_mut(selected) {
            g.text(x, *y, w, "The selected creature", GOLD);
            *y += ROW + 4.0;
            text_row_ref(g, y, x, w, "Creature", 4, &mut s.creature, Ref::Creature, None);
            let mut at = [s.at.0, s.at.1];
            if numbers_row(g, y, x, w, "Position", 0, &["North", "West"], &mut at) {
                (s.at, s.height) = ((at[0], at[1]), None);
            }
            value_row(g, y, x, w, "Facing (deg)", 2, &mut s.facing);
            value_row(g, y, x, w, "Wanders (yd)", 3, &mut s.wander);
            s.wander = s.wander.max(0.0);
            hint(g, y, x, "How far from its spot it strolls; 0: it stands still.");
            option_value_row(g, y, x, w, "Back after (s)", 5, &mut s.respawn);
            hint(g, y, x, "Seconds from its death until it is back; empty: as long as its kind says.");
            option_row(g, y, x, w, "Map", 6, &mut s.map);
            hint(g, y, x, "The map it stands in (Azeroth, Kalimdor, a dungeon's name); empty: in every map.");
            if here_and_go(g, y, x, s.at, actions) {
                (s.at, s.facing, s.height, s.map) = ((me.north, me.west), me.yaw, None, Some(me.map.to_string()));
            }
            patrol_form(g, y, x, w, s, me, actions, placer);
            gizmo_row(g, y, x, placer, false);
        } else {
            empty_pane(g, x, *y, w, "No creature is selected. Pick one in the list, or put one down below.");
            *y += ROW + 8.0;
        }
        *y += 12.0;
        g.text(x, *y, w, "Put a new creature down", GOLD);
        *y += ROW + 4.0;
        let mut current = kinds.get(*kind).cloned().unwrap_or_default();
        // What is chosen in the window starts the putting down at once, as for props.
        let chosen = matches!(&g.modal.picked, Some((t, _)) if *t == fid("pick", 5));
        if text_row_ref(g, y, x, w, "Creature", 5, &mut current, Ref::Creature, None) {
            *kind = kinds.iter().position(|k| *k == current).unwrap_or(0);
        }
        if chosen {
            placer.start(Aim::Creature, &current, me.yaw);
        }
        put_down_button(g, y, x, placer, Aim::Creature, &current, me.yaw);
        hint(g, y, x, "\"New here\" over the list puts one of this kind where you stand.");
        *y += 8.0;
    });
    if gone {
        d.cur.spawns.remove(selected);
        p.selected = selected.saturating_sub(1);
    }
}

/// The chests standing in the world.
#[allow(clippy::too_many_arguments)]
fn chests(g: &mut Gui, d: &mut Data, p: &mut Pick, placer: &mut Placer, me: Place, actions: &mut Vec<Action>, body: f32, bottom: f32, rows: usize) {
    let lines: Vec<String> = d.cur.chests.iter().map(|c| format!("{} {:.0}yd  ({:.0}, {:.0})", c.id, yards_from(me, c.at.0, c.at.1), c.at.0, c.at.1)).collect();
    let selected = p.selected;
    let (made, gone) = list_header(g, body, "delete-chest", "Copy to here", !d.cur.chests.is_empty());
    if made {
        let mut chest = chest_like(d, selected);
        (chest.at, chest.facing) = ((me.north, me.west), me.yaw);
        d.cur.chests.push(chest);
        p.selected = d.cur.chests.len() - 1;
        p.scroll = d.cur.chests.len().saturating_sub(rows);
    }
    list(g, 8.0, body + 26.0, LIST_WIDTH, rows, &lines, p);
    let locks: Vec<String> = ["None", "Novice", "Apprentice", "Adept", "Expert", "Master"].iter().map(|s| s.to_string()).collect();
    form(g, p, PANE_X, body, PANE_W, bottom, |g, y, _| {
        let (x, w) = (PANE_X, PANE_W);
        if let Some(c) = d.cur.chests.get_mut(selected) {
            g.text(x, *y, w, "The selected chest", GOLD);
            *y += ROW + 4.0;
            text_row(g, y, x, w, "Id", 10, &mut c.id);
            model_row(g, y, x, w, "Model", 11, &mut c.model, Pickable::Chests);
            let mut lock = c.lock.map_or("None".to_string(), |l| l.name().to_string());
            if choice_row(g, y, x, w, "Lock", &locks, &mut lock) {
                c.lock = [Level::Novice, Level::Apprentice, Level::Adept, Level::Expert, Level::Master].into_iter().find(|l| l.name() == lock);
            }
            hint(g, y, x, "How skilled a lockpicker has to be to open it.");
            text_row_ref(g, y, x, w, "Loot table", 12, &mut c.loot, Ref::Loot, None);
            hint(g, y, x, "What it holds by chance; may be left empty.");
            money_row(g, y, x, w, "Always holds: coins", 16, &mut c.copper);
            stacks_row(g, y, x, w, "Always holds: items", 17, &mut c.items);
            hint(g, y, x, "Write item:count, separated by commas, or choose with the button.");
            let mut at = [c.at.0, c.at.1];
            if numbers_row(g, y, x, w, "Position", 13, &["North", "West"], &mut at) {
                c.at = (at[0], at[1]);
            }
            value_row(g, y, x, w, "Facing (deg)", 15, &mut c.facing);
            if here_and_go(g, y, x, c.at, actions) {
                (c.at, c.facing) = ((me.north, me.west), me.yaw);
            }
            gizmo_row(g, y, x, placer, false);
        } else {
            empty_pane(g, x, *y, w, "No chest is selected. Pick one in the list, or put one down below.");
            *y += ROW + 8.0;
        }
        *y += 12.0;
        g.text(x, *y, w, "Put new chests down", GOLD);
        *y += ROW + 4.0;
        hint(g, y, x, "They are like the selected one: its lock and what it holds.");
        if placer.chest.is_empty() {
            placer.chest = CHEST_MODELS[0].0.to_string();
        }
        let tag = fid("pick", 60);
        g.text(x, *y + 3.0, LABEL_WIDTH, "Model", DIM);
        g.fill(x + LABEL_WIDTH, *y, w - LABEL_WIDTH - BROWSE - 4.0, ROW, BOX);
        g.text(x + LABEL_WIDTH + 4.0, *y + 3.0, w - LABEL_WIDTH - BROWSE - 12.0, file_stem(&placer.chest), TEXT);
        if let Some(model) = g.picked(tag) {
            placer.chest = model.clone();
            placer.start(Aim::Chest, &model, me.yaw);
        }
        if g.button(x + w - BROWSE, *y, BROWSE, "...") {
            g.modal.want = Some((tag, Pickable::Chests, placer.chest.clone()));
        }
        *y += ROW + 6.0;
        let model = placer.chest.clone();
        put_down_button(g, y, x, placer, Aim::Chest, &model, me.yaw);
    });
    if gone {
        d.cur.chests.remove(selected);
        p.selected = selected.saturating_sub(1);
    }
}

/// A line for a model chosen from a gallery: its name, and the button that opens the gallery.
pub(super) fn model_row(g: &mut Gui, y: &mut f32, x: f32, w: f32, label: &str, n: usize, value: &mut String, what: Pickable) {
    let tag = fid("pick", n);
    if let Some(model) = g.picked(tag) {
        *value = model;
    }
    g.text(x, *y + 3.0, LABEL_WIDTH, label, DIM);
    g.fill(x + LABEL_WIDTH, *y, w - LABEL_WIDTH - BROWSE - 4.0, ROW, BOX);
    g.text(x + LABEL_WIDTH + 4.0, *y + 3.0, w - LABEL_WIDTH - BROWSE - 12.0, file_stem(value), TEXT);
    if g.button(x + w - BROWSE, *y, BROWSE, "...") {
        g.modal.want = Some((tag, what, value.clone()));
    }
    *y += ROW + 3.0;
}

/// The buttons that choose what the handles over a thing in the world do (a creature or a chest is moved and turned, a prop also scaled).
pub(super) fn gizmo_row(g: &mut Gui, y: &mut f32, x: f32, placer: &mut Placer, scale: bool) {
    g.text(x, *y + 3.0, LABEL_WIDTH, "Handles", DIM);
    for (i, (mode, name)) in [(Mode::Move, "Move"), (Mode::Turn, "Turn"), (Mode::Scale, "Scale")].into_iter().filter(|(m, _)| scale || *m != Mode::Scale).enumerate() {
        let bx = x + LABEL_WIDTH + i as f32 * 66.0;
        if placer.tool == mode || (!scale && placer.tool == Mode::Scale && mode == Mode::Move) {
            g.fill(bx - 2.0, *y - 2.0, 64.0, ROW + 4.0, SELECTED);
        }
        if g.button(bx, *y, 60.0, name) {
            placer.tool = mode;
        }
    }
    *y += ROW + 3.0;
    hint(g, y, x, "Click it in the world to select it, then drag the arrows, the ring or the knob over it.");
    *y += 4.0;
}

/// The patrol of a creature: the places it walks to one after the other, round and round, and for each the seconds it stands there, the way it
/// turns and what it does there (the original's waypoint script). Stops are added where one stands or by clicking in the world, where the
/// patrol is drawn.
#[allow(clippy::too_many_arguments)]
pub(super) fn patrol_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, s: &mut Spawn, me: Place, actions: &mut Vec<Action>, placer: &mut Placer) {
    let title = if s.path.is_empty() { "Patrol (none: it stays where it is or strolls)".to_string() } else { format!("Patrol ({} stops, then round again)", s.path.len()) };
    if !section(g, y, x, w, "patrol", &title, !s.path.is_empty()) {
        return;
    }
    let count = s.path.len();
    let (mut remove, mut swap) = (None, None);
    for (i, stop) in s.path.iter_mut().enumerate() {
        let base = 800 + i * 8;
        g.text(x, *y + 3.0, LABEL_WIDTH, &format!("Stop {}", i + 1), GOLD);
        let bx = x + LABEL_WIDTH;
        if g.button(bx, *y, 54.0, "Here") {
            stop.at = (me.north, me.west);
        }
        if g.button(bx + 58.0, *y, 40.0, "Go") {
            actions.push(Action::Teleport(stop.at.0, stop.at.1));
        }
        if g.button(bx + 102.0, *y, 40.0, "Up") && i > 0 {
            swap = Some((i - 1, i));
        }
        if g.button(bx + 146.0, *y, 50.0, "Down") && i + 1 < count {
            swap = Some((i, i + 1));
        }
        if g.button(bx + 200.0, *y, 64.0, "Remove") {
            remove = Some(i);
        }
        *y += ROW + 3.0;
        let mut place = [stop.at.0, stop.at.1];
        if numbers_row(g, y, x, w, "  Position", base, &["North", "West"], &mut place) {
            stop.at = (place[0], place[1]);
        }
        value_row(g, y, x, w, "  Waits (s)", base + 1, &mut stop.wait);
        stop.wait = stop.wait.max(0.0);
        option_value_row(g, y, x, w, "  Turns to (deg)", base + 2, &mut stop.facing);
        steps_form(g, y, x, w, &mut stop.steps, 5000 + i * 100);
        *y += 4.0;
    }
    if let Some((a, b)) = swap {
        s.path.swap(a, b);
    }
    if let Some(i) = remove {
        s.path.remove(i);
    }
    if count > 0 {
        hint(g, y, x, "\"Waits\": how long it stands at the stop; \"Turns to\": the heading while it does (empty: as it walks).");
    }
    if g.button(x + LABEL_WIDTH, *y, 220.0, "Add a stop where I stand") {
        s.path.push(Waypoint { at: (me.north, me.west), wait: 0.0, facing: None, steps: Vec::new() });
    }
    *y += ROW + 3.0;
    if placer.placing && placer.aim == Aim::Stop {
        if g.button(x + LABEL_WIDTH, *y, 220.0, "Stop adding (Esc)") {
            placer.placing = false;
        }
    } else if g.button(x + LABEL_WIDTH, *y, 220.0, "Click in the world to add stops") {
        placer.start(Aim::Stop, "", me.yaw);
    }
    *y += ROW + 3.0;
    if count > 0 {
        if g.danger("clear-patrol", x + LABEL_WIDTH, *y, 220.0, "Clear the patrol") {
            s.path.clear();
        }
        *y += ROW + 3.0;
    }
    *y += 8.0;
}

/// What a stop of a patrol makes the creature do (the original's waypoint script), one thing to a line, each some seconds after it arrives.
pub(super) fn steps_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, steps: &mut Vec<(f32, Step)>, base: usize) {
    const KINDS: [&str; 5] = ["Runs or walks on", "Strolls around", "Stays for good", "Plays an animation", "Looks like another kind"];
    let mut remove = None;
    for (j, (after, what)) in steps.iter_mut().enumerate().take(10) {
        let n = base + j * 10;
        let now = match what {
            Step::Run(_) => 0,
            Step::Roam(..) => 1,
            Step::Idle => 2,
            Step::Emote(_) => 3,
            Step::Morph(_) => 4,
        };
        g.text(x, *y + 3.0, LABEL_WIDTH, &format!("  Then, step {}", j + 1), DIM);
        let by = g.arrows(x + LABEL_WIDTH, *y, 220.0, KINDS[now]);
        if g.button(x + LABEL_WIDTH + 230.0, *y, 70.0, "Remove") {
            remove = Some(j);
        }
        *y += ROW + 3.0;
        if by != 0 {
            *what = match (now as i32 + by).rem_euclid(KINDS.len() as i32) {
                0 => Step::Run(true),
                1 => Step::Roam(5.0, 10.0),
                2 => Step::Idle,
                3 => Step::Emote(1),
                _ => Step::Morph(String::new()),
            };
        }
        value_row(g, y, x, w, "    After (s)", n, after);
        *after = after.max(0.0);
        match what {
            Step::Run(run) => {
                toggle_row(g, y, x, "    Moves", run, "Runs", "Walks");
            }
            Step::Roam(yards, seconds) => {
                value_row(g, y, x, w, "    Within (yd)", n + 1, yards);
                value_row(g, y, x, w, "    For (s)", n + 2, seconds);
                hint(g, y, x, "Then it goes on with the patrol.");
            }
            Step::Idle => hint(g, y, x, "No more patrol until it is back from a fight or has died and come again."),
            Step::Emote(animation) => {
                value_row(g, y, x, w, "    Animation", n + 1, animation);
                hint(g, y, x, "A number of the model's animations, held while it stands; 0 stops it.");
            }
            Step::Morph(kind) => {
                text_row_ref(g, y, x, w, "    Looks like", n + 1, kind, Ref::Creature, None);
                hint(g, y, x, "It stays what it is; only its look changes.");
            }
        }
    }
    if let Some(j) = remove {
        steps.remove(j);
    }
    if steps.len() < 10 && g.button(x + LABEL_WIDTH, *y, 150.0, "Add something to do") {
        steps.push((0.0, Step::Run(true)));
    }
    *y += ROW + 3.0;
}

/// A node as the gizmo and the ghost see it: a prop of the kind's model and size, standing where `ground` puts it, turned by the node's heading. None for
/// an unknown kind or a place without ground.
pub(super) fn node_prop(kinds: &[content::NodeKind], n: &content::Node, ground: Ground) -> Option<Prop> {
    let kind = kinds.iter().find(|k| k.id == n.kind)?;
    Some(Prop { model: kind.model.clone(), map: None, zone: None, pickup: None, sleep: false, physics: false, station: 0, at: (n.at.0, n.at.1, ground(-n.at.1, -n.at.0)?), rotation: [0.0, n.facing, 0.0], scale: kind.scale })
}

/// The kinds of nodes: what a vein or an herb is called, which profession gathers it at what rank, how it looks, what it gives and how often it can be gathered.
#[allow(clippy::too_many_arguments)]
fn node_kinds(g: &mut Gui, d: &mut Data, p: &mut Pick, placer: &mut Placer, body: f32, bottom: f32, rows: usize) {
    let lines: Vec<String> = d.cur.node_kinds.iter().map(|k| format!("{}  ({} {})", k.name, k.skill.name(), k.rank)).collect();
    let selected = p.selected;
    let (made, gone) = list_header(g, body, "delete-node-kind", "New kind", !d.cur.node_kinds.is_empty());
    if made {
        // Like the selected one (else like the first), under a new id.
        let mut kind = d.cur.node_kinds.get(selected).or(d.cur.node_kinds.first()).cloned().unwrap_or_else(|| content::NodeKind { id: String::new(), name: String::new(), skill: crate::skills::Skill::Mining, rank: 1, model: String::new(), scale: 1.0, loot: String::new(), uses: (1, 1) });
        kind.id = fresh(&d.cur.node_kinds.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(), "node");
        kind.name = "New node".into();
        d.cur.node_kinds.push(kind);
        p.selected = d.cur.node_kinds.len() - 1;
        p.reveal = true;
    }
    list(g, 8.0, body + 26.0, LIST_WIDTH, rows, &lines, p);
    let used = d.cur.node_kinds.get(selected).map_or(0, |k| d.cur.nodes.iter().filter(|n| n.kind == k.id).count());
    form(g, p, PANE_X, body, PANE_W, bottom, |g, y, _| {
        let (x, w) = (PANE_X, PANE_W);
        let Some(k) = d.cur.node_kinds.get_mut(selected) else {
            empty_pane(g, x, *y, w, "No kind is selected. Pick one in the list, or make one with \"New kind\".");
            return;
        };
        g.text(x, *y, w, "The selected kind", GOLD);
        *y += ROW + 4.0;
        g.text(x, *y + 3.0, LABEL_WIDTH, "Id", DIM);
        g.text(x + LABEL_WIDTH + 4.0, *y + 3.0, w - LABEL_WIDTH, &k.id, TEXT);
        *y += ROW + 3.0;
        text_row(g, y, x, w, "Name", 911, &mut k.name);
        let mut mining = k.skill == crate::skills::Skill::Mining;
        toggle_row(g, y, x, "Gathered by", &mut mining, "Mining", "Herbalism");
        k.skill = if mining { crate::skills::Skill::Mining } else { crate::skills::Skill::Herbalism };
        value_row(g, y, x, w, "Rank needed", 912, &mut k.rank);
        k.rank = k.rank.min(crate::skills::PROFESSION_MAX);
        hint(g, y, x, "The rank of the profession it asks for (0 to 300).");
        let tag = fid("pick", 913);
        if let Some(model) = g.picked(tag) {
            k.model = model;
        }
        g.text(x, *y + 3.0, LABEL_WIDTH, "Model", DIM);
        if !(placeable(&k.model) && k.model.to_ascii_lowercase().ends_with(".m2") && placer.mount.exists(&k.model)) {
            g.tint = Some(RED);
        }
        g.field("Model", 913, x + LABEL_WIDTH, *y, w - LABEL_WIDTH - BROWSE - 4.0, &mut k.model);
        if g.button(x + w - BROWSE, *y, BROWSE, "...") {
            let current = k.model.clone();
            placer.open_gallery_for(g, tag, &current);
        }
        *y += ROW + 3.0;
        value_row(g, y, x, w, "Size", 914, &mut k.scale);
        k.scale = k.scale.max(0.05);
        hint(g, y, x, "1 is the model's own size.");
        text_row_ref(g, y, x, w, "Loot table", 915, &mut k.loot, Ref::Loot, None);
        hint(g, y, x, "What one gathering gives.");
        let mut uses = [k.uses.0, k.uses.1];
        if numbers_row(g, y, x, w, "Gathered (times)", 916, &["At least", "At most"], &mut uses) {
            k.uses = (uses[0].max(1), uses[1].max(uses[0]).max(1));
        }
        hint(g, y, x, "How many times it can be gathered before it is gone; picked between the two.");
        hint(g, y, x, &format!("{used} nodes of this kind stand in the world."));
    });
    if gone {
        if used > 0 {
            g.say("Nodes of this kind stand in the world: take them away first.", RED);
        } else {
            d.cur.node_kinds.remove(selected);
            p.selected = selected.saturating_sub(1);
        }
    }
}

/// The veins and herbs standing in the world, which Mining and Herbalism gather. The form changes the one selected (its kind too); clicks in the world put the chosen kind down.
#[allow(clippy::too_many_arguments)]
fn nodes(g: &mut Gui, d: &mut Data, p: &mut Pick, placer: &mut Placer, me: Place, around: Place, actions: &mut Vec<Action>, body: f32, bottom: f32, rows: usize) {
    let kinds = d.cur.node_kinds.clone();
    let name = |kind: &str| kinds.iter().find(|k| k.id == kind).map_or(kind.to_string(), |k| k.name.clone());
    // Of the world's thousands only those near the editor's eye are listed (a line left empty is not shown).
    let lines: Vec<String> = d.cur.nodes.iter().map(|n| if near_here((n.at.0, n.at.1), around) { format!("{} {:.0}yd  ({:.0}, {:.0})", name(&n.kind), yards_from(me, n.at.0, n.at.1), n.at.0, n.at.1) } else { String::new() }).collect();
    if placer.node.is_empty() {
        placer.node = kinds.first().map(|k| k.id.clone()).unwrap_or_default();
    }
    let selected = p.selected;
    let (made, gone) = list_header(g, body, "delete-node", "New here", !d.cur.nodes.is_empty());
    if made {
        // Of the kind of the selected one, else of the one chosen for putting down.
        let kind = d.cur.nodes.get(selected).map_or_else(|| placer.node.clone(), |n| n.kind.clone());
        d.cur.nodes.push(content::Node { kind, map: Some(me.map.to_string()), at: (me.north, me.west, me.height), facing: me.yaw, respawn: None });
        p.selected = d.cur.nodes.len() - 1;
        p.reveal = true;
    }
    list(g, 8.0, body + 26.0, LIST_WIDTH, rows, &lines, p);
    form(g, p, PANE_X, body, PANE_W, bottom, |g, y, _| {
        let (x, w) = (PANE_X, PANE_W);
        if let Some(n) = d.cur.nodes.get_mut(selected) {
            g.text(x, *y, w, "The selected node", GOLD);
            *y += ROW + 4.0;
            text_row_ref(g, y, x, w, "Kind", 900, &mut n.kind, Ref::NodeKind, None);
            match kinds.iter().find(|k| k.id == n.kind) {
                Some(k) => hint(g, y, x, &format!("{}: needs {} {}; can be gathered {} to {} times.", k.name, k.skill.name(), k.rank, k.uses.0, k.uses.1)),
                None => hint(g, y, x, "No such kind: it is left out of the game."),
            }
            let mut at = [n.at.0, n.at.1, n.at.2];
            if numbers_row(g, y, x, w, "Position", 901, &["North", "West", "Height"], &mut at) {
                n.at = (at[0], at[1], at[2]);
            }
            value_row(g, y, x, w, "Facing (deg)", 904, &mut n.facing);
            option_value_row(g, y, x, w, "Back after (s)", 905, &mut n.respawn);
            hint(g, y, x, "Seconds from its last gathering until it is back; empty: five minutes.");
            option_row(g, y, x, w, "Map", 906, &mut n.map);
            hint(g, y, x, "The map it stands in (Azeroth, Kalimdor, a dungeon's name); empty: the map being played.");
            if here_and_go(g, y, x, (n.at.0, n.at.1), actions) {
                (n.at, n.facing, n.map) = ((me.north, me.west, me.height), me.yaw, Some(me.map.to_string()));
            }
            gizmo_row(g, y, x, placer, false);
        } else {
            empty_pane(g, x, *y, w, "No node is selected. Pick one in the list, or put one down below.");
            *y += ROW + 8.0;
        }
        *y += 12.0;
        g.text(x, *y, w, "Put new nodes down", GOLD);
        *y += ROW + 4.0;
        // What is chosen in the window starts the putting down at once, as for creatures.
        let chosen = matches!(&g.modal.picked, Some((t, _)) if *t == fid("pick", 902));
        let mut kind = placer.node.clone();
        text_row_ref(g, y, x, w, "Kind", 902, &mut kind, Ref::NodeKind, None);
        placer.node = kind.clone();
        if chosen {
            placer.start(Aim::Node, &kind, me.yaw);
        }
        put_down_button(g, y, x, placer, Aim::Node, &kind, me.yaw);
        hint(g, y, x, "\"New here\" over the list puts one where you stand.");
        *y += 8.0;
    });
    if gone {
        d.cur.nodes.remove(selected);
        p.selected = selected.saturating_sub(1);
    }
}

/// The props: models put into the world. The form changes the one selected; clicks in the world put the chosen model down.
#[allow(clippy::too_many_arguments)]
fn props(g: &mut Gui, d: &mut Data, p: &mut Pick, placer: &mut Placer, me: Place, actions: &mut Vec<Action>, body: f32, bottom: f32, rows: usize) {
    let lines: Vec<String> = d.cur.props.iter().map(|q| format!("{} {:.0}yd {}", file_stem(&q.model), yards_from(me, q.at.0, q.at.1), q.zone.as_deref().unwrap_or(""))).collect();
    let selected = p.selected;
    let (made, gone) = list_header(g, body, "delete-prop", "Copy to here", !d.cur.props.is_empty());
    if made {
        if let Some(mut copy) = d.cur.props.get(selected).cloned() {
            copy.at = (me.north, me.west, me.height);
            (copy.map, copy.zone) = (None, None);
            d.cur.props.push(copy);
            p.selected = d.cur.props.len() - 1;
            p.scroll = d.cur.props.len().saturating_sub(rows);
        } else {
            g.say("There is no prop to copy: put one down with the model button below.", RED);
        }
    }
    list(g, 8.0, body + 26.0, LIST_WIDTH, rows, &lines, p);
    form(g, p, PANE_X, body, PANE_W, bottom, |g, y, _| {
        let (x, w) = (PANE_X, PANE_W);
        if let Some(q) = d.cur.props.get_mut(selected) {
            g.text(x, *y, w, &q.zone.as_ref().map_or("The selected prop".to_string(), |z| format!("The selected prop ({z})")), GOLD);
            *y += ROW + 4.0;
            // The model: its file, which a gallery of pictures can choose.
            let tag = fid("pick", 40);
            if let Some(model) = g.picked(tag) {
                q.model = model;
            }
            g.text(x, *y + 3.0, LABEL_WIDTH, "Model", DIM);
            if !q.model.is_empty() && !(placeable(&q.model) && placer.mount.exists(&q.model)) {
                g.tint = Some(RED);
            }
            g.field("Model", 40, x + LABEL_WIDTH, *y, w - LABEL_WIDTH - BROWSE - 4.0, &mut q.model);
            if g.button(x + w - BROWSE, *y, BROWSE, "...") {
                let current = q.model.clone();
                placer.open_gallery_for(g, tag, &current);
            }
            *y += ROW + 3.0;
            let mut at = [q.at.0, q.at.1, q.at.2];
            if numbers_row(g, y, x, w, "Position", 41, &["North", "West", "Height"], &mut at) {
                q.at = (at[0], at[1], at[2]);
            }
            let mut turn = [q.rotation[1], q.rotation[0], q.rotation[2]];
            if numbers_row(g, y, x, w, "Rotation (deg)", 44, &["Turn", "Tilt", "Roll"], &mut turn) {
                q.rotation = [turn[1], turn[0], turn[2]];
            }
            value_row(g, y, x, w, "Scale", 47, &mut q.scale);
            hint(g, y, x, "1 is the model's own size.");
            if here_and_go(g, y, x, (q.at.0, q.at.1), actions) {
                q.at = (me.north, me.west, me.height);
            }
            g.text(x, *y, w, "What it does", GOLD);
            *y += ROW + 4.0;
            // What it gives when it is picked up (a quest item; only while a quest wants it): none when no item is named.
            let mut item = q.pickup.as_ref().map_or_else(String::new, |k| k.item.clone());
            text_row_ref(g, y, x, w, "Gives item", 49, &mut item, Ref::Item, None);
            hint(g, y, x, "A quest item that can be picked up from it, while a quest wants it; empty: nothing.");
            if item.trim().is_empty() {
                q.pickup = None;
            } else {
                let k = q.pickup.get_or_insert_with(|| content::Pickup { item: String::new(), respawn: content::Pickup::respawn_default(), vanish: true });
                k.item = item.trim().to_string();
                toggle_row(g, y, x, "Disappears", &mut k.vanish, "Yes, then back", "No, it stays");
                if k.vanish {
                    value_row(g, y, x, w, "Back after (s)", 50, &mut k.respawn);
                }
            }
            value_row(g, y, x, w, "Station", 51, &mut q.station);
            hint(g, y, x, "A place the trades work at: 1 anvil, 2 loom, 3 forge, 4 cooking fire, 663 alchemy lab; 0: none.");
            toggle_row(g, y, x, "Bed", &mut q.sleep, "Yes, sleep here", "No");
            toggle_row(g, y, x, "Physics", &mut q.physics, "Yes, loose", "No");
            hint(g, y, x, "Loose: it can be pushed (hold E) and falls over.");
            gizmo_row(g, y, x, placer, true);
        } else {
            empty_pane(g, x, *y, w, "No prop is selected. Pick one in the list, or put one down below.");
            *y += ROW + 8.0;
        }
        *y += 12.0;
        g.text(x, *y, w, "Put new props down", GOLD);
        *y += ROW + 4.0;
        g.text(x, *y + 3.0, LABEL_WIDTH, "Model", DIM);
        let ok = placeable(&placer.model) && placer.mount.exists(&placer.model);
        if !placer.model.is_empty() && !ok {
            g.tint = Some(RED);
        }
        g.field("Model", 48, x + LABEL_WIDTH, *y, w - LABEL_WIDTH - BROWSE - 4.0, &mut placer.model);
        if g.button(x + w - BROWSE, *y, BROWSE, "...") {
            let current = placer.model.clone();
            placer.open_gallery(g, &current);
        }
        *y += ROW + 6.0;
        if placer.placing {
            if (!g.blocked && g.input.keys.contains(&Key::Escape)) || g.button(x, *y, 260.0, "Stop putting down (Esc)") {
                placer.placing = false;
            }
            *y += ROW + 4.0;
            hint(g, y, x, "Every click in the world, beside this window, stands the model there.");
        } else if ok {
            if g.button(x, *y, 260.0, "Click in the world to put it down") {
                (placer.aim, placer.placing) = (Aim::Prop, true);
            }
            *y += ROW + 6.0;
        } else {
            hint(g, y, x, "Choose a model with the button \"...\" (pictures of all of them) to put it down.");
        }
        *y += 8.0;
    });
    if gone {
        d.cur.props.remove(selected);
        p.selected = selected.saturating_sub(1);
    }
}

/// The places to discover: the question marks of the compass until the player comes near.
#[allow(clippy::too_many_arguments)]
fn pois(g: &mut Gui, d: &mut Data, p: &mut Pick, me: Place, actions: &mut Vec<Action>, body: f32, bottom: f32, rows: usize) {
    let lines: Vec<String> = d.cur.pois.iter().map(|q| format!("{} {:.0}yd", q.name, yards_from(me, q.at.0, q.at.1))).collect();
    let ids: Vec<String> = d.cur.pois.iter().map(|q| q.id.clone()).collect();
    let selected = p.selected;
    let (made, gone) = list_header(g, body, "delete-poi", "New here", !d.cur.pois.is_empty());
    if made {
        d.cur.pois.push(content::Poi { id: fresh(&ids.iter().map(String::as_str).collect::<Vec<_>>(), "place"), name: "New place".into(), at: (me.north, me.west), radius: 30.0, icon: 4, height: None });
        p.selected = d.cur.pois.len() - 1;
        p.scroll = d.cur.pois.len().saturating_sub(rows);
    }
    list(g, 8.0, body + 26.0, LIST_WIDTH, rows, &lines, p);
    form(g, p, PANE_X, body, PANE_W, bottom, |g, y, _| {
        let (x, w) = (PANE_X, PANE_W);
        let Some(q) = d.cur.pois.get_mut(selected) else {
            empty_pane(g, x, *y, w, "No place is selected. Pick one in the list, or make one with \"New here\".");
            return;
        };
        g.text(x, *y, w, "The selected place", GOLD);
        *y += ROW + 4.0;
        if let Some(icon) = g.modal.icon_picked.take() {
            q.icon = icon;
        }
        text_row(g, y, x, w, "Id", 20, &mut q.id);
        text_row(g, y, x, w, "Name", 21, &mut q.name);
        hint(g, y, x, "The name shown when the player discovers it.");
        let mut at = [q.at.0, q.at.1];
        if numbers_row(g, y, x, w, "Position", 22, &["North", "West"], &mut at) {
            q.at = (at[0], at[1]);
        }
        value_row(g, y, x, w, "Radius (yd)", 24, &mut q.radius);
        hint(g, y, x, "It is discovered when the player comes this near.");
        option_value_row(g, y, x, w, "Floor height", 25, &mut q.height);
        hint(g, y, x, "The height of the feet on a floor of a building or a bridge; empty: the terrain.");
        if here_and_go(g, y, x, q.at, actions) {
            q.at = (me.north, me.west);
        }
        g.text(x, *y + 9.0, LABEL_WIDTH, "Icon", DIM);
        if let Some(texture) = g.poi_icons {
            g.fill(x + LABEL_WIDTH, *y, 34.0, 34.0, BOX);
            g.picture_part(texture, x + LABEL_WIDTH + 3.0, *y + 3.0, 28.0, 28.0, crate::compass::poi_uv(q.icon));
        }
        if g.button(x + LABEL_WIDTH + 44.0, *y + 8.0, 110.0, "Choose...") {
            g.modal.icons = Some(q.icon);
        }
        *y += 34.0 + 8.0;
    });
    if gone {
        d.cur.pois.remove(selected);
        p.selected = selected.saturating_sub(1);
    }
}
