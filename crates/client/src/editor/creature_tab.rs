use super::*;

/// The creatures tab: the kinds of creatures, the NPCs and the enemies.
pub(super) fn creatures(g: &mut Gui, d: &mut Data, p: &mut Pick, pv: &mut Preview, top: f32, bottom: f32) {
    let rows = ((bottom - top - 26.0) / ROW) as usize;
    let lines: Vec<String> = d.cur.creatures.iter().map(|c| format!("{}{}", c.id, if c.hostile { "  (enemy)" } else if c.neutral { "  (neutral)" } else { "" })).collect();
    let ids: Vec<String> = d.cur.creatures.iter().map(|c| c.id.clone()).collect();
    let (made, gone) = list_header(g, top, "delete-creature", "New copy", !ids.is_empty());
    if made && !ids.is_empty() {
        let mut copy = d.cur.creatures[p.selected.min(ids.len() - 1)].clone();
        copy.id = fresh(&ids.iter().map(String::as_str).collect::<Vec<_>>(), "creature");
        copy.name = "New creature".into();
        d.cur.creatures.push(copy);
        p.selected = d.cur.creatures.len() - 1;
        p.scroll = d.cur.creatures.len().saturating_sub(rows);
    }
    if gone && !ids.is_empty() {
        d.cur.creatures.remove(p.selected.min(ids.len() - 1));
        p.selected = p.selected.saturating_sub(1);
    }
    list(g, 8.0, top + 26.0, LIST_WIDTH, rows, &lines, p);
    let Some(c) = d.cur.creatures.get_mut(p.selected) else {
        empty_pane(g, PANE_X, top + 4.0, PANE_W, "No creature is selected. Pick one in the list, or make a copy of one with \"New copy\".");
        return;
    };
    let items = &d.cur.items;
    // What the creature teaches is taken out of the map for the form and put back (if it teaches anything) under the id it has then.
    let mut taught = d.cur.lessons.remove(&c.id).unwrap_or_default();
    let (others, mut lesson) = (&d.cur.lessons, p.lesson);
    form(g, p, PANE_X, top, PANE_W, bottom, |g, y, _| creature_form(g, y, PANE_X, PANE_W, c, pv, items, &mut taught, others, &mut lesson));
    p.lesson = lesson;
    if !taught.is_empty() {
        d.cur.lessons.insert(c.id.clone(), taught);
    }
}

/// The picture of the creature, and beside it what changes how it looks. A beast: the model, from all those of the game, and its
/// skins, from those the game has for the model. A person: his skin, face, hair and beard, and what he wears.
pub(super) fn look_panel(g: &mut Gui, y: &mut f32, x: f32, w: f32, c: &mut RawCreature, pv: &mut Preview, items: &[RawItem]) {
    const PICTURE: (f32, f32) = (180.0, 216.0);
    pv.show(&c.body);
    g.fill(x, *y, PICTURE.0, PICTURE.1, BOX);
    g.picture(pv.texture(), x, *y, PICTURE.0, PICTURE.1, [1.0; 4]);
    if let Some(e) = &pv.error {
        g.text(x + 4.0, *y + 4.0, PICTURE.0 - 8.0, e, RED);
    }
    let (rx, rw) = (x + PICTURE.0 + 10.0, w - PICTURE.0 - 10.0);
    let mut ry = *y;
    const TAG: f32 = 64.0;
    match &mut c.body {
        RawBody::Beast { model, skins, .. } => {
            pv.load_catalogue();
            let (model_tag, skin_tag) = (fid("model", 0), fid("skin", 0));
            match &pv.catalogue {
                Some(Ok(catalogue)) => {
                    // The model: any of those of the game, chosen in a window with a search and a picture.
                    if let Some(chosen) = g.picked(model_tag) {
                        *skins = catalogue.skins_of(&chosen).first().cloned().unwrap_or_default();
                        *model = chosen;
                    }
                    g.text(rx, ry + 3.0, TAG, "Model", DIM);
                    g.text(rx + TAG, ry + 3.0, rw - TAG - BROWSE - 4.0, file_stem(model), TEXT);
                    if g.button(rx + rw - BROWSE, ry, BROWSE, "...") {
                        let entries = catalogue.models.iter().map(|m| Entry { value: m.clone(), label: model_label(m) }).collect();
                        g.open_chooser(model_tag, &format!("Choose a model ({} in the game)", catalogue.models.len()), Shows::Model, entries, model);
                    }
                    ry += ROW + 3.0;
                    // The skin: the sets the game has for the model, one at a time with arrows or all in a window.
                    let sets = catalogue.skins_of(model);
                    if let Some(n) = g.picked(skin_tag).and_then(|v| v.parse::<usize>().ok()) {
                        *skins = sets.get(n).cloned().unwrap_or_default();
                    }
                    let at = sets.iter().position(|s| preview::same_skins(s, skins));
                    g.text(rx, ry + 3.0, TAG, "Skin", DIM);
                    let name = match (at, sets.len()) {
                        (_, 0) => "(the model has none)".to_string(),
                        (Some(i), n) => format!("{} {}/{}", file_stem(&sets[i][0].1), i + 1, n),
                        (None, n) => format!("(not one of {n})"),
                    };
                    let by = g.arrows(rx + TAG, ry, rw - TAG - BROWSE - 4.0, &name);
                    if g.button(rx + rw - BROWSE, ry, BROWSE, "...") && !sets.is_empty() {
                        let entries = sets.iter().enumerate().map(|(i, s)| Entry { value: i.to_string(), label: s.iter().map(|(_, f)| file_stem(f)).collect::<Vec<_>>().join(" + ") }).collect();
                        g.open_chooser(skin_tag, "Choose the skins", Shows::Skin(model.clone()), entries, &at.map_or(String::new(), |i| i.to_string()));
                    }
                    ry += ROW + 3.0;
                    if by != 0 && !sets.is_empty() {
                        *skins = sets[(at.map_or(if by > 0 { -1 } else { 0 }, |i| i as i32) + by).rem_euclid(sets.len() as i32) as usize].clone();
                    }
                }
                Some(Err(e)) => g.text(rx, ry + 3.0, rw, e, RED),
                None => {}
            }
        }
        RawBody::Humanoid { outfit, look } => {
            // The race and sex; what the tables have for a race and sex limits the rest, so a change takes the rest back into them.
            let all: Vec<Race> = Race::ALL.iter().copied().chain([Race::Goblin]).collect();
            let races: Vec<String> = all.iter().map(|r| r.name().to_string()).collect();
            let mut race = look.race.name().to_string();
            g.text(rx, ry + 3.0, TAG + 20.0, "Race", DIM);
            let by = g.arrows(rx + TAG + 20.0, ry, rw - TAG - 20.0, &race);
            if by != 0 {
                race = step(&races, &race, by).to_string();
                look.race = all[races.iter().position(|r| *r == race).unwrap_or(0)];
            }
            ry += ROW + 3.0;
            g.text(rx, ry + 3.0, TAG + 20.0, "Sex", DIM);
            if g.arrows(rx + TAG + 20.0, ry, rw - TAG - 20.0, look.sex.name()) != 0 {
                look.sex = if look.sex == Sex::Male { Sex::Female } else { Sex::Male };
            }
            ry += ROW + 3.0;
            let max = pv.ranges(look.race, look.sex, look.skin, look.hair_style);
            (look.skin, look.face, look.hair_style, look.hair_color, look.beard) = (look.skin.min(max.skin), look.face.min(max.face), look.hair_style.min(max.hair_style), look.hair_color.min(max.hair_color), look.beard.min(max.beard));
            let mut set = |g: &mut Gui, label: &str, value: &mut u32, max: u32| {
                g.text(rx, ry + 3.0, TAG + 20.0, label, DIM);
                let by = g.arrows(rx + TAG + 20.0, ry, rw - TAG - 20.0, &value.to_string());
                *value = (*value as i32 + by).rem_euclid(max as i32 + 1) as u32;
                ry += ROW + 3.0;
            };
            set(g, "Skin", &mut look.skin, max.skin);
            set(g, "Face", &mut look.face, max.face);
            set(g, "Hair style", &mut look.hair_style, max.hair_style);
            set(g, "Hair colour", &mut look.hair_color, max.hair_color);
            set(g, "Beard", &mut look.beard, max.beard);
            ry += 4.0;
            // What he wears: chosen from every item that can be worn, in a window with a search; it takes the place of what he wears in the same place.
            let tag = fid("outfit", 0);
            if let Some(id) = g.picked(tag) {
                if let Some(new) = items.iter().find(|i| i.id == id) {
                    outfit.retain(|o| items.iter().find(|i| i.id == *o).is_none_or(|i| i.wear != new.wear));
                    outfit.push(id);
                }
            }
            g.text(rx, ry + 3.0, TAG, "Item", DIM);
            if g.button(rx + TAG, ry, rw - TAG, "Put on...") {
                let entries = items.iter().filter(|i| i.wear != Wear::None).map(|i| Entry { value: i.id.clone(), label: format!("{}  ({}, level {})", i.name, i.wear.name(), i.level) }).collect();
                g.open_chooser(tag, "Choose what he puts on", Shows::Nothing, entries, "");
            }
            ry += ROW + 3.0;
            if g.button(rx + TAG, ry, rw - TAG, "Take all off") {
                outfit.clear();
            }
            ry += ROW + 3.0;
        }
    }
    g.text(rx, ry + 3.0, TAG, "Turn", DIM);
    let by = g.arrows(rx + TAG, ry, rw - TAG, "round");
    pv.turn += by as f32 * 30f32.to_radians();
    *y += PICTURE.1 + 6.0;
}

/// A model as the list of models shows it: the folder and the name, or the name alone when the folder is called the same.
pub(super) fn model_label(path: &str) -> String {
    let short = path.trim_start_matches("Creature\\").trim_end_matches(".m2");
    match short.split_once('\\') {
        Some((folder, name)) if folder.eq_ignore_ascii_case(name) => name.to_string(),
        _ => short.to_string(),
    }
}

/// How a creature fights beyond the melee attack: its mana, the time between its swings, its shot, whom it calls, and the events of its script (each written as the
/// content file has it, with a sentence over it that says what it means).
pub(super) fn fighting_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, c: &mut RawCreature) {
    let b = &mut c.brain;
    value_row(g, y, x, w, "Mana", 200, &mut b.mana);
    value_row(g, y, x, w, "Swing time (s)", 201, &mut b.swing);
    hint(g, y, x, "Seconds between its blows; 0: the usual 2.");
    value_row(g, y, x, w, "Ranged damage", 202, &mut b.shot);
    hint(g, y, x, "What its shot or throw does; 0: it has none.");
    value_row(g, y, x, w, "Faction", 204, &mut b.faction);
    hint(g, y, x, "Creatures with the same number help each other; 0: none.");
    value_row(g, y, x, w, "Calls for help (yd)", 203, &mut b.assist);
    hint(g, y, x, "When it starts a fight, others of its faction within this many yards join in.");
    switch_row(g, y, x, "Fights back only", &mut b.calm);
    hint(g, y, x, "Never attacks first, even as an enemy.");
    let mut remove = None;
    for (i, e) in b.events.iter_mut().enumerate() {
        g.text(x + LABEL_WIDTH, *y, w - LABEL_WIDTH, &crate::brain::describe(e), DIM);
        *y += ROW;
        let mut text = ron::to_string(&*e).unwrap_or_default();
        if text_row(g, y, x, w, &format!("Script event {}", i + 1), 300 + i, &mut text) {
            match ron::from_str::<content::BrainEvent>(&text) {
                Ok(parsed) => *e = parsed,
                Err(err) => g.say(format!("Script event {}: {err}", i + 1), RED),
            }
        }
        if g.button(x + LABEL_WIDTH, *y, 70.0, "Remove") {
            remove = Some(i);
        }
        *y += ROW + 3.0;
    }
    if let Some(i) = remove {
        b.events.remove(i);
    }
    if g.button(x + LABEL_WIDTH, *y, 160.0, "Add a script event") {
        // Casts a fireball (spell 133) at the player every few seconds of a fight.
        b.events.push(content::BrainEvent { on: crate::brain::event::TIMER, p: vec![3000, 6000, 8000, 12000], flags: 0x401, acts: vec![(crate::brain::act::CAST, 133, 1, 0)], ..Default::default() });
    }
    *y += ROW + 3.0;
    hint(g, y, x, "A new event casts a fireball every few seconds of a fight. Events are written as in content/classic_creature_ai.ron.");
}

/// What a creature says when talked to, and the lines it offers.
fn talk_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, c: &mut RawCreature) {
    let mut talks = c.gossip.is_some();
    if switch_row(g, y, x, "Talks", &mut talks) {
        c.gossip = talks.then(|| content::Gossip { greeting: "Hello, $N.".into(), options: Vec::new() });
    }
    let Some(gossip) = &mut c.gossip else { return };
    text_area_row(g, y, x, w, "Greeting", 120, 3, &mut gossip.greeting);
    hint(g, y, x, "$N stands for the player's name.");
    let kinds: Vec<String> = ["Says something", "Opens the shop", "Teaches a skill", "Opens the lessons"].iter().map(|s| s.to_string()).collect();
    let mut remove = None;
    for (i, o) in gossip.options.iter_mut().enumerate() {
        text_row(g, y, x, w, &format!("Menu line {}", i + 1), 130 + i, &mut o.text);
        let now = match o.action {
            CreatureAction::Say(_) => &kinds[0],
            CreatureAction::Vendor => &kinds[1],
            CreatureAction::Train { .. } => &kinds[2],
            CreatureAction::Teach => &kinds[3],
        };
        g.text(x, *y + 3.0, LABEL_WIDTH, "  Choosing it", DIM);
        let by = g.arrows(x + LABEL_WIDTH, *y, 200.0, now);
        if by != 0 {
            o.action = match kinds.iter().position(|k| k == step(&kinds, now, by)) {
                Some(0) => CreatureAction::Say("...".into()),
                Some(1) => CreatureAction::Vendor,
                Some(3) => CreatureAction::Teach,
                _ => CreatureAction::Train { skill: crate::skills::Skill::Riding, rank: 75, needs: 0, level: 40, cost: 900_000 },
            };
        }
        if g.button(x + LABEL_WIDTH + 210.0, *y, 70.0, "Remove") {
            remove = Some(i);
        }
        *y += ROW + 3.0;
        match &mut o.action {
            CreatureAction::Say(text) => {
                text_area_row(g, y, x, w, "  Answer", 160 + i, 3, text);
            }
            CreatureAction::Vendor => hint(g, y, x, "Shows what the creature sells (\"Sells\" above)."),
            CreatureAction::Teach => hint(g, y, x, "Shows the recipes the creature teaches, for a price (classic_trainers.ron)."),
            CreatureAction::Train { skill, rank, needs, level, cost } => {
                let names: Vec<String> = crate::skills::ALL.iter().map(|s| s.name().to_string()).collect();
                let mut taught = skill.name().to_string();
                if choice_row(g, y, x, w, "  Teaches", &names, &mut taught) {
                    *skill = crate::skills::ALL.into_iter().find(|s| s.name() == taught).unwrap_or(*skill);
                }
                value_row(g, y, x, w, "  Up to rank", 1000 + i * 4, rank);
                value_row(g, y, x, w, "  Needs rank", 1001 + i * 4, needs);
                value_row(g, y, x, w, "  Min. level", 1002 + i * 4, level);
                money_row(g, y, x, w, "  Cost", 1003 + i * 4, cost);
                hint(g, y, x, &format!("For a character who has rank {needs} of it and is level {level} or more."));
            }
        }
    }
    if let Some(i) = remove {
        gossip.options.remove(i);
    }
    if g.button(x + LABEL_WIDTH, *y, 140.0, "Add a menu line") {
        gossip.options.push(GossipOption { text: "Tell me more.".into(), action: CreatureAction::Say("...".into()) });
    }
    *y += ROW + 3.0;
}

#[allow(clippy::too_many_arguments)]
pub(super) fn creature_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, c: &mut RawCreature, pv: &mut Preview, items: &[RawItem], taught: &mut Vec<content::Lesson>, others: &std::collections::BTreeMap<String, Vec<content::Lesson>>, lesson: &mut usize) {
    look_panel(g, y, x, w, c, pv, items);
    text_row(g, y, x, w, "Id", 100, &mut c.id);
    hint(g, y, x, "What spawns, quests and loot call it by; renaming it leaves them pointing at nothing until they are changed too.");
    text_row(g, y, x, w, "Name", 101, &mut c.name);
    option_row(g, y, x, w, "Title", 102, &mut c.title);
    hint(g, y, x, "The line under the name, such as Stormwind Guard; may be left empty.");
    value_row(g, y, x, w, "Level", 103, &mut c.level);
    let types: Vec<String> = ["None", "Beast", "Dragonkin", "Demon", "Elemental", "Giant", "Undead", "Humanoid", "Critter", "Mechanical"].iter().map(|s| s.to_string()).collect();
    let mut kind = types[(c.kind as usize).min(types.len() - 1)].clone();
    if choice_row(g, y, x, w, "Type", &types, &mut kind) {
        c.kind = types.iter().position(|t| *t == kind).unwrap_or(0) as u32;
    }
    hint(g, y, x, "What the bonuses of gear \"versus\" name.");
    let attitudes: Vec<String> = ["Friendly", "Neutral", "Enemy"].iter().map(|s| s.to_string()).collect();
    let mut attitude = attitudes[if c.hostile { 2 } else if c.neutral { 1 } else { 0 }].clone();
    if choice_row(g, y, x, w, "Towards the player", &attitudes, &mut attitude) {
        (c.hostile, c.neutral) = (attitude == "Enemy", attitude == "Neutral");
    }
    hint(g, y, x, ["Can be talked to and cannot be fought.", "Leaves the player alone (yellow name) until hit, then fights back.", "Attacks the player when he comes near (red name)."][if c.hostile { 2 } else if c.neutral { 1 } else { 0 }]);
    // How a player of the Horde finds it, when that is not how the Alliance does.
    let mut differs = c.horde.is_some();
    if switch_row(g, y, x, "Horde differs", &mut differs) {
        c.horde = differs.then_some((c.hostile, c.neutral));
    }
    if let Some((hostile, neutral)) = &mut c.horde {
        switch_row(g, y, x, "  Enemy to Horde", hostile);
        switch_row(g, y, x, "  Neutral to Horde", neutral);
    }
    hint(g, y, x, "The attitude above is what the Alliance finds; the Horde's own view is kept only when it differs.");
    value_row(g, y, x, w, "Health", 104, &mut c.health);
    value_row(g, y, x, w, "Damage per hit", 105, &mut c.melee_damage);
    value_row(g, y, x, w, "Back after death (s)", 106, &mut c.respawn);
    if let RawBody::Beast { scale, .. } = &mut c.body {
        value_row(g, y, x, w, "Size", 111, scale);
        hint(g, y, x, "1 is the size of the model; 2 is twice as big.");
    }
    *y += 4.0;
    if section(g, y, x, w, "creature-goods", "Drops and shop", true) {
        option_ref_row(g, y, x, w, "Loot table", 107, &mut c.loot, Ref::Loot);
        hint(g, y, x, "What it drops when it dies; empty: nothing.");
        list_row(g, y, x, w, "Sells", 108, &mut c.vendor, Some(Ref::Item));
        hint(g, y, x, "Item ids separated by commas, or choose with the button; it sells them once it has a shop line below.");
        if let RawBody::Humanoid { outfit, .. } = &mut c.body {
            list_row(g, y, x, w, "Wears", 113, outfit, Some(Ref::Item));
            hint(g, y, x, "What it has on; \"Put on...\" next to the picture is easier.");
        }
    }
    if section(g, y, x, w, "creature-talk", "Conversation", c.gossip.is_some()) {
        talk_form(g, y, x, w, c);
    }
    if section(g, y, x, w, "creature-lessons", "Teaches a trade", !taught.is_empty()) {
        super::lessons::lessons_form(g, y, x, w, c, taught, others, lesson);
    }
    if section(g, y, x, w, "creature-fight", "Fighting (advanced)", !c.brain.events.is_empty()) {
        fighting_form(g, y, x, w, c);
    }
    if section(g, y, x, w, "creature-model", "Model details (advanced)", false) {
        match &mut c.body {
            RawBody::Beast { model, skins, .. } => {
                text_row(g, y, x, w, "Model file", 110, model);
                let mut text = skins.iter().map(|(slot, file)| format!("{slot}={file}")).collect::<Vec<_>>().join("; ");
                if text_row(g, y, x, w, "Skins", 112, &mut text) {
                    let parsed: Option<Vec<(u32, String)>> = text.split(';').map(str::trim).filter(|t| !t.is_empty()).map(|t| Some((t.split_once('=')?.0.trim().parse().ok()?, t.split_once('=')?.1.trim().to_string()))).collect();
                    match parsed {
                        Some(p) => *skins = p,
                        None => g.say("Skins: write slot=file; slot=file", RED),
                    }
                }
                hint(g, y, x, "The picture's \"Model\" and \"Skin\" choose these without typing.");
            }
            RawBody::Humanoid { look, .. } => {
                let mut v = [look.skin, look.face, look.hair_style, look.hair_color, look.beard];
                if numbers_row(g, y, x, w, "Look", 114, &["Skin", "Face", "Hair", "Colour", "Beard"], &mut v) {
                    (look.skin, look.face, look.hair_style, look.hair_color, look.beard) = (v[0], v[1], v[2], v[3], v[4]);
                }
                hint(g, y, x, "The arrows next to the picture keep these within what the race has.");
            }
        }
    }
    *y += 6.0;
}
