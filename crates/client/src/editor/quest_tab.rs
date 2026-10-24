use super::*;

/// A line of switches for the races a quest is for (none marked: all of them).
fn races_row(g: &mut Gui, y: &mut f32, x: f32, mask: &mut u32) {
    g.text(x, *y + 3.0, LABEL_WIDTH, "Races", DIM);
    for (i, race) in Race::ALL.iter().enumerate() {
        let on = *mask & race.mask() != 0;
        let short = ["Hu", "Or", "Dw", "Ni", "Un", "Ta", "Gn", "Tr"][i].to_string();
        if g.button(x + LABEL_WIDTH + i as f32 * 42.0, *y, 38.0, &if on { format!("[{short}]") } else { short }) {
            *mask ^= race.mask();
        }
    }
    *y += ROW + 3.0;
    hint(g, y, x, "Hu Or Dw Ni Un Ta Gn Tr; none marked: every race may take it.");
}

/// The quests tab.
pub(super) fn quests(g: &mut Gui, d: &mut Data, p: &mut Pick, me: Place, top: f32, bottom: f32) {
    let rows = ((bottom - top - 26.0) / ROW) as usize;
    let lines: Vec<String> = d.cur.quests.iter().map(|q| q.title.clone()).collect();
    let ids: Vec<String> = d.cur.quests.iter().map(|q| q.id.clone()).collect();
    let (made, gone) = list_header(g, top, "delete-quest", "New quest", !ids.is_empty());
    if made {
        let id = fresh(&ids.iter().map(String::as_str).collect::<Vec<_>>(), "quest");
        let giver = d.cur.creatures.iter().find(|c| c.gossip.is_some() && !c.hostile).map_or(String::new(), |c| c.id.clone());
        let text = format!(r#"(id: "{id}", title: "New quest", giver: "{giver}", turn_in: "{giver}", details: "What is asked.", objective: "What to do.", progress: "Not yet.", completion: "Well done.")"#);
        if let Ok(q) = ron::from_str::<RawQuest>(&text) {
            d.cur.quests.push(q);
            p.selected = d.cur.quests.len() - 1;
            p.scroll = d.cur.quests.len().saturating_sub(rows);
        }
    }
    if gone && !ids.is_empty() {
        d.cur.quests.remove(p.selected.min(ids.len() - 1));
        p.selected = p.selected.saturating_sub(1);
    }
    list(g, 8.0, top + 26.0, LIST_WIDTH, rows, &lines, p);
    let Some(q) = d.cur.quests.get_mut(p.selected) else {
        empty_pane(g, PANE_X, top + 4.0, PANE_W, "No quest is selected. Pick one in the list, or make one with \"New quest\".");
        return;
    };
    form(g, p, PANE_X, top, PANE_W, bottom, |g, y, _| {
        let (x, w) = (PANE_X, PANE_W);
        text_row(g, y, x, w, "Id", 200, &mut q.id);
        hint(g, y, x, "What other quests and the saved characters call it by; keep it once it is in use.");
        text_row(g, y, x, w, "Title", 201, &mut q.title);
        text_row_ref(g, y, x, w, "Given by", 202, &mut q.giver, Ref::Creature, None);
        text_row_ref(g, y, x, w, "Turned in to", 203, &mut q.turn_in, Ref::Creature, None);
        option_ref_row(g, y, x, w, "Needs quest", 204, &mut q.requires, Ref::Quest);
        hint(g, y, x, "Another quest that has to be done first; empty: none.");
        value_row(g, y, x, w, "Level", 213, &mut q.level);
        hint(g, y, x, "0 for none; the experience falls as the character outgrows the quest.");
        value_row(g, y, x, w, "Lowest level", 215, &mut q.min_level);
        races_row(g, y, x, &mut q.races);
        option_ref_row(g, y, x, w, "Begun by item", 216, &mut q.begins_with, Ref::Item);
        hint(g, y, x, "Using the item (a letter, a note) opens the quest, and nobody offers it; empty: the one who gives it offers it.");
        *y += 4.0;
        g.text(x, *y, w, "What is said", GOLD);
        *y += ROW + 4.0;
        text_area_row(g, y, x, w, "Offer text", 205, 4, &mut q.details);
        text_area_row(g, y, x, w, "Objective", 206, 2, &mut q.objective);
        hint(g, y, x, "The short line of the quest log.");
        text_area_row(g, y, x, w, "While waiting", 207, 2, &mut q.progress);
        hint(g, y, x, "Said when the quest is brought back before it is done.");
        text_area_row(g, y, x, w, "On completion", 208, 4, &mut q.completion);
        *y += 4.0;
        g.text(x, *y, w, "Reward", GOLD);
        *y += ROW + 4.0;
        money_row(g, y, x, w, "Coins", 209, &mut q.reward.copper);
        value_row(g, y, x, w, "Experience", 210, &mut q.reward.xp);
        stacks_row(g, y, x, w, "Gives", 211, &mut q.reward.items);
        hint(g, y, x, "Items the character always gets: item:count, separated by commas.");
        stacks_row(g, y, x, w, "Choose one of", 212, &mut q.reward.choice);
        hint(g, y, x, "Items of which the character takes one.");
        stacks_row(g, y, x, w, "Given at start", 214, &mut q.start_items);
        hint(g, y, x, "Handed over when the quest is taken (a letter to deliver); taken back if it is given up.");
        *y += 4.0;
        g.text(x, *y, w, "Goals", GOLD);
        *y += ROW + 2.0;
        if !q.goals.is_empty() {
            g.text(x, *y, 110.0, "What to do", DIM);
            g.text(x + 116.0, *y, 120.0, "Creature or item", DIM);
            g.text(x + w - 124.0, *y, 40.0, "How many", DIM);
            *y += ROW;
        }
        let mut remove = None;
        for (i, goal) in q.goals.iter_mut().enumerate() {
            let (kill, target, count) = match goal {
                RawGoal::Kill(t, c) => (true, t.clone(), *c),
                RawGoal::Collect(t, c) => (false, t.clone(), *c),
                RawGoal::Reach(map, at, radius) => {
                    // A walk to a place: the map, north, west and how close, yards.
                    g.text(x, *y + 3.0, 56.0, "Walk to", DIM);
                    g.field("walkmap", 340 + i, x + 60.0, *y, 130.0, map);
                    let mut numbers = [at.0, at.1, *radius].map(|v| format!("{v}"));
                    for (k, (label, width)) in [("walkn", 80.0), ("walkw", 80.0), ("walkr", 50.0)].into_iter().enumerate() {
                        let at_x = x + 196.0 + [0.0, 86.0, 172.0][k];
                        if g.field(label, 360 + i, at_x, *y, width, &mut numbers[k]) {
                            if let Ok(v) = numbers[k].trim().parse::<f32>() {
                                match k {
                                    0 => at.0 = v,
                                    1 => at.1 = v,
                                    _ => *radius = v.max(1.0),
                                }
                            }
                        }
                    }
                    if g.button(x + w - 78.0, *y, 70.0, "Remove") {
                        remove = Some(i);
                    }
                    *y += ROW + 3.0;
                    continue;
                }
            };
            let mut kind = if kill { "Kill".to_string() } else { "Collect".to_string() };
            let by = g.arrows(x, *y, 110.0, &kind);
            let (mut target, mut count) = (target, count.to_string());
            let (tag, target_kind) = (fid("goalpick", i), if kill { Ref::Creature } else { Ref::Item });
            if !target.trim().is_empty() && !g.ids.has(target_kind, target.trim()) {
                g.tint = Some(RED);
            }
            g.field("goal", 300 + i, x + 116.0, *y, w - 116.0 - 130.0 - 32.0, &mut target);
            if let Some(chosen) = g.picked(tag) {
                target = chosen;
            }
            g.browse_ids(tag, x + w - 158.0, *y, target_kind, target.trim());
            g.field("count", 320 + i, x + w - 124.0, *y, 40.0, &mut count);
            if g.button(x + w - 78.0, *y, 70.0, "Remove") {
                remove = Some(i);
            }
            if by != 0 {
                kind = step(&["Kill".into(), "Collect".into()], &kind, by).to_string();
            }
            let count = count.trim().parse().unwrap_or(1);
            *goal = if kind == "Kill" { RawGoal::Kill(target, count) } else { RawGoal::Collect(target, count) };
            *y += ROW + 3.0;
        }
        if let Some(i) = remove {
            q.goals.remove(i);
        }
        if g.button(x, *y, 120.0, "Add a goal") {
            q.goals.push(RawGoal::Kill(String::new(), 1));
        }
        if g.button(x + 126.0, *y, 190.0, "Add a walk to here") {
            q.goals.push(RawGoal::Reach(me.map.to_string(), (me.north, me.west), 10.0));
        }
        *y += ROW + 3.0;
        hint(g, y, x, "A name in red is not a creature or item that exists.");
    });
}

/// The loot tables: what creatures and chests drop.
pub(super) fn loot(g: &mut Gui, d: &mut Data, p: &mut Pick, top: f32, bottom: f32) {
    let rows = ((bottom - top - 26.0) / ROW) as usize;
    let keys: Vec<String> = d.cur.loot.keys().cloned().collect();
    let (made, gone) = list_header(g, top, "delete-loot", "New table", !keys.is_empty());
    if made {
        let id = fresh(&keys.iter().map(String::as_str).collect::<Vec<_>>(), "loot");
        d.cur.loot.insert(id.clone(), content::RawTable { copper: (1, 5), items: Vec::new(), also: Vec::new() });
        p.selected = d.cur.loot.keys().position(|k| *k == id).unwrap_or(0);
        p.scroll = p.selected.saturating_sub(rows / 2);
    }
    if gone {
        if let Some(key) = keys.get(p.selected) {
            d.cur.loot.remove(key);
            p.selected = p.selected.saturating_sub(1);
        }
    }
    let keys: Vec<String> = d.cur.loot.keys().cloned().collect();
    list(g, 8.0, top + 26.0, LIST_WIDTH, rows, &keys, p);
    let Some(key) = keys.get(p.selected).cloned() else {
        empty_pane(g, PANE_X, top + 4.0, PANE_W, "No loot table is selected. Pick one in the list, or make one with \"New table\".");
        return;
    };
    let mut rename = None;
    if let Some(table) = d.cur.loot.get_mut(&key) {
        form(g, p, PANE_X, top, PANE_W, bottom, |g, y, _| {
            let (x, w) = (PANE_X, PANE_W);
            let mut id = key.clone();
            if text_row(g, y, x, w, "Id", 500, &mut id) && !id.trim().is_empty() {
                rename = Some(id.trim().to_string());
            }
            hint(g, y, x, "Creatures and chests name their table by this; they have to be changed too if it is.");
            let mut copper = [table.copper.0, table.copper.1];
            if numbers_row(g, y, x, w, "Copper", 501, &["From", "To"], &mut copper) {
                table.copper = (copper[0], copper[1]);
            }
            hint(g, y, x, "The coins that drop, a random amount between the two.");
            list_row(g, y, x, w, "Also rolls", 502, &mut table.also, Some(Ref::Loot));
            hint(g, y, x, "Other tables rolled as well: their copper is added and their items are among these (one step deep).");
            *y += 4.0;
            g.text(x, *y, 170.0, "Item", GOLD);
            g.text(x + 176.0, *y, 60.0, "Count", GOLD);
            g.text(x + 242.0, *y, 60.0, "Chance %", GOLD);
            *y += ROW + 2.0;
            let mut remove = None;
            for (i, drop) in table.items.iter_mut().enumerate() {
                let (mut id, mut count, mut chance) = (drop.0.clone(), if drop.1.0 == drop.1.1 { drop.1.0.to_string() } else { format!("{}-{}", drop.1.0, drop.1.1) }, drop.2.to_string());
                let tag = fid("lootpick", i);
                if !id.trim().is_empty() && !g.ids.has(Ref::Item, id.trim()) {
                    g.tint = Some(RED);
                }
                if g.field("lootitem", 510 + i, x, *y, 140.0, &mut id) {
                    drop.0 = id.clone();
                }
                if let Some(chosen) = g.picked(tag) {
                    drop.0 = chosen.clone();
                    id = chosen;
                }
                g.browse_ids(tag, x + 144.0, *y, Ref::Item, id.trim());
                if g.field("lootcount", 600 + i, x + 176.0, *y, 60.0, &mut count) {
                    let (low, high) = count.split_once('-').unwrap_or((&count, &count));
                    match (low.trim().parse(), high.trim().parse()) {
                        (Ok(l), Ok(h)) => drop.1 = (l, h),
                        _ => g.say("Count: write 2 or 1-3", RED),
                    }
                }
                if g.field("lootchance", 700 + i, x + 242.0, *y, 44.0, &mut chance) {
                    match chance.trim().parse() {
                        Ok(c) => drop.2 = c,
                        Err(_) => g.say("Chance: a number from 0 to 100", RED),
                    }
                }
                if g.button(x + 292.0, *y, 70.0, "Remove") {
                    remove = Some(i);
                }
                *y += ROW + 3.0;
            }
            if let Some(i) = remove {
                table.items.remove(i);
            }
            if g.button(x, *y, 120.0, "Add a drop") {
                table.items.push((String::new(), (1, 1), 50.0));
            }
            *y += ROW + 3.0;
            hint(g, y, x, "Count: 2, or 1-3 for a random number. Chance: 0 to 100. A name in red is not an item that exists.");
        });
    }
    // A new id: the table moves to it.
    if let Some(id) = rename.filter(|id| *id != key && !d.cur.loot.contains_key(id)) {
        if let Some(table) = d.cur.loot.remove(&key) {
            d.cur.loot.insert(id.clone(), table);
            p.selected = d.cur.loot.keys().position(|k| *k == id).unwrap_or(0);
        }
    }
}
