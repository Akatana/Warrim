use super::*;

/// What a spell line of an item does, one effect to a line: its kind chosen with the arrows, and its numbers under names. `base` keeps the
/// fields of this list apart from those of the others.
pub(super) fn effects_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, effects: &mut Vec<UseEffect>, base: usize) {
    use UseEffect as E;
    const KINDS: [&str; 8] = ["Heals", "Restores mana", "Heals over time", "Restores mana over time", "Raises a stat", "Returns to the hearth", "Gives a mount", "Gives a bonus while worn"];
    let stats: Vec<String> = ["Strength", "Agility", "Stamina", "Intellect", "Spirit", "Armor", "Health"].iter().map(|s| s.to_string()).collect();
    let mut remove = None;
    for (j, e) in effects.iter_mut().enumerate().take(10) {
        let n = base + j * 10;
        let now = match e {
            E::Heal(..) => 0,
            E::Mana(..) => 1,
            E::HealOver(..) => 2,
            E::ManaOver(..) => 3,
            E::Boost(..) => 4,
            E::Hearth => 5,
            E::Mount(..) => 6,
            E::Equip(..) => 7,
        };
        g.text(x, *y + 3.0, LABEL_WIDTH, &format!("  Effect {}", j + 1), DIM);
        let by = g.arrows(x + LABEL_WIDTH, *y, 220.0, KINDS[now]);
        if g.button(x + LABEL_WIDTH + 230.0, *y, 70.0, "Remove") {
            remove = Some(j);
        }
        *y += ROW + 3.0;
        if by != 0 {
            *e = match (now as i32 + by).rem_euclid(KINDS.len() as i32) {
                0 => E::Heal(20, 30),
                1 => E::Mana(20, 30),
                2 => E::HealOver(100, 10.0),
                3 => E::ManaOver(100, 10.0),
                4 => E::Boost(2, 10, 60.0),
                5 => E::Hearth,
                6 => E::Mount(0, 60),
                _ => E::Equip(0, 0, 1),
            };
        }
        match e {
            E::Heal(low, high) | E::Mana(low, high) => {
                value_row(g, y, x, w, "    At least", n, low);
                value_row(g, y, x, w, "    At most", n + 1, high);
            }
            E::HealOver(total, seconds) | E::ManaOver(total, seconds) => {
                value_row(g, y, x, w, "    In all", n, total);
                value_row(g, y, x, w, "    Over (s)", n + 1, seconds);
            }
            E::Boost(stat, amount, seconds) => {
                let mut name = stats[(*stat as usize).min(stats.len() - 1)].clone();
                if choice_row(g, y, x, w, "    Stat", &stats, &mut name) {
                    *stat = stats.iter().position(|s| *s == name).unwrap_or(0) as u8;
                }
                value_row(g, y, x, w, "    By", n + 1, amount);
                value_row(g, y, x, w, "    For (s)", n + 2, seconds);
                hint(g, y, x, "0 seconds: for as long as it is worn.");
            }
            E::Hearth => {}
            E::Mount(display, percent) => {
                value_row(g, y, x, w, "    Mount look", n, display);
                hint(g, y, x, "A number of the game's creature looks (a row of CreatureDisplayInfo.dbc).");
                value_row(g, y, x, w, "    Speed added %", n + 1, percent);
            }
            E::Equip(aura, misc, amount) => {
                value_row(g, y, x, w, "    Aura number", n, aura);
                value_row(g, y, x, w, "    Misc", n + 1, misc);
                value_row(g, y, x, w, "    Amount", n + 2, amount);
                hint(g, y, x, "The aura of the original's spell (what a worn item gives); misc: for a spell bonus, its schools.");
            }
        }
    }
    if let Some(j) = remove {
        effects.remove(j);
    }
    if effects.len() < 10 && g.button(x + LABEL_WIDTH, *y, 150.0, "Add an effect") {
        effects.push(E::Heal(20, 30));
    }
    *y += ROW + 3.0;
}

/// The items tab.
pub(super) fn items(g: &mut Gui, d: &mut Data, p: &mut Pick, pv: &mut Preview, top: f32, bottom: f32) {
    let rows = ((bottom - top - 26.0) / ROW) as usize;
    let lines: Vec<String> = d.cur.items.iter().map(|i| i.name.clone()).collect();
    let ids: Vec<String> = d.cur.items.iter().map(|i| i.id.clone()).collect();
    let (made, gone) = list_header(g, top, "delete-item", "New copy", !ids.is_empty());
    if made && !ids.is_empty() {
        let mut copy = d.cur.items[p.selected.min(ids.len() - 1)].clone();
        copy.id = fresh(&ids.iter().map(String::as_str).collect::<Vec<_>>(), "item");
        copy.name = "New item".into();
        d.cur.items.push(copy);
        p.selected = d.cur.items.len() - 1;
        p.scroll = d.cur.items.len().saturating_sub(rows);
    }
    if gone && !ids.is_empty() {
        d.cur.items.remove(p.selected.min(ids.len() - 1));
        p.selected = p.selected.saturating_sub(1);
    }
    list(g, 8.0, top + 26.0, LIST_WIDTH, rows, &lines, p);
    let Some(it) = d.cur.items.get_mut(p.selected) else {
        empty_pane(g, PANE_X, top + 4.0, PANE_W, "No item is selected. Pick one in the list, or make a copy of one with \"New copy\".");
        return;
    };
    form(g, p, PANE_X, top, PANE_W, bottom, |g, y, _| item_form(g, y, PANE_X, PANE_W, it, pv));
}

/// Everywhere an item can be worn.
pub(super) const WEARS: [Wear; 24] = [
    Wear::None,
    Wear::Head,
    Wear::Neck,
    Wear::Shoulder,
    Wear::Shirt,
    Wear::Chest,
    Wear::Waist,
    Wear::Legs,
    Wear::Feet,
    Wear::Wrist,
    Wear::Hands,
    Wear::Finger,
    Wear::Trinket,
    Wear::Back,
    Wear::OneHand,
    Wear::MainHand,
    Wear::TwoHand,
    Wear::Shield,
    Wear::OffHand,
    Wear::Held,
    Wear::Ranged,
    Wear::Thrown,
    Wear::Ammo,
    Wear::Tabard,
];

/// The picture of what an item looks like worn or held, and beside it its look (a row of `ItemDisplayInfo.dbc`, chosen in a window
/// with a search and the picture of each), its quality and where it is worn.
pub(super) fn item_panel(g: &mut Gui, y: &mut f32, x: f32, w: f32, it: &mut RawItem, pv: &mut Preview) {
    const PICTURE: (f32, f32) = (180.0, 216.0);
    const TAG: f32 = 64.0;
    pv.show_display(it.display, it.wear);
    g.fill(x, *y, PICTURE.0, PICTURE.1, BOX);
    g.picture(pv.texture(), x, *y, PICTURE.0, PICTURE.1, [1.0; 4]);
    if let Some(e) = &pv.error {
        g.text(x + 4.0, *y + 4.0, PICTURE.0 - 8.0, e, RED);
    }
    let (rx, rw) = (x + PICTURE.0 + 10.0, w - PICTURE.0 - 10.0);
    let mut ry = *y;
    let tag = fid("display", 0);
    if let Some(v) = g.picked(tag).and_then(|v| v.parse().ok()) {
        it.display = v;
    }
    g.text(rx, ry + 3.0, TAG, "Look", DIM);
    let mut text = it.display.to_string();
    if g.field("display", 402, rx + TAG, ry, rw - TAG - BROWSE - 4.0, &mut text) {
        match text.trim().parse() {
            Ok(v) => it.display = v,
            Err(_) => g.say(format!("Look: \"{text}\" is not a number"), RED),
        }
    }
    if g.button(rx + rw - BROWSE, ry, BROWSE, "...") {
        let entries: Vec<Entry> = pv.displays(it.wear).into_iter().map(|(id, label)| Entry { value: id.to_string(), label }).collect();
        let title = format!("Choose a look: {} in the game that fit \"{:?}\"", entries.len(), it.wear);
        g.open_chooser(tag, &title, Shows::Display(it.wear), entries, &it.display.to_string());
    }
    ry += ROW + 3.0;
    g.text(rx, ry, rw, &pv.note, DIM);
    ry += ROW + 4.0;
    let qualities: Vec<String> = ["0 Poor", "1 Common", "2 Uncommon", "3 Rare", "4 Epic", "5 Legendary", "6 Artifact"].iter().map(|s| s.to_string()).collect();
    let mut quality = qualities[it.quality.min(6)].clone();
    if choice_row(g, &mut ry, rx, rw, "Quality", &qualities, &mut quality) {
        it.quality = qualities.iter().position(|q| *q == quality).unwrap_or(1);
    }
    let wears: Vec<String> = WEARS.iter().map(|w| format!("{w:?}")).collect();
    let mut wear = format!("{:?}", it.wear);
    if choice_row(g, &mut ry, rx, rw, "Worn as", &wears, &mut wear) {
        it.wear = WEARS.into_iter().find(|v| format!("{v:?}") == wear).unwrap_or(Wear::None);
    }
    g.text(rx, ry + 3.0, TAG, "Turn view", DIM);
    let by = g.arrows(rx + TAG, ry, rw - TAG, "round");
    pv.turn += by as f32 * 30f32.to_radians();
    *y += PICTURE.1 + 6.0;
}

/// The kinds of items of the original's tables (the class number and its name).
const CLASSES: [(u8, &str); 11] = [(0, "Consumable"), (1, "Container"), (2, "Weapon"), (4, "Armor"), (6, "Projectile"), (7, "Trade goods"), (9, "Recipe"), (11, "Quiver"), (12, "Quest"), (13, "Key"), (15, "Miscellaneous")];

/// The classes and races of the original that a bit mask of an item names.
const CLASS_BITS: [(&str, u32); 9] = [("Warrior", 1), ("Paladin", 2), ("Hunter", 4), ("Rogue", 8), ("Priest", 16), ("Shaman", 64), ("Mage", 128), ("Warlock", 256), ("Druid", 1024)];
const RACE_BITS: [(&str, u32); 8] = [("Human", 1), ("Orc", 2), ("Dwarf", 4), ("Night Elf", 8), ("Undead", 16), ("Tauren", 32), ("Gnome", 64), ("Troll", 128)];

/// When a spell line of an item goes off (`ItemSpell::trigger`).
const TRIGGERS: [&str; 3] = ["When used", "While worn", "On a blow"];

/// The name of a spell of the original by its number, as the spell list has it.
fn spell_name(number: u32) -> Option<String> {
    let c = content::get();
    c.classic_index.get(&number).map(|&i| &c.classic_spells[i]).map(|s| if s.rank.is_empty() { s.name.clone() } else { format!("{} ({})", s.name, s.rank) })
}

/// The spell lines of an item: what it casts when used, worn or on a blow, in the words of the original's spell.
fn spells_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, it: &mut RawItem) {
    let triggers: Vec<String> = TRIGGERS.iter().map(|s| s.to_string()).collect();
    let mut remove = None;
    for (i, s) in it.spells.iter_mut().enumerate() {
        g.text(x, *y, w, &format!("Spell line {}", i + 1), GOLD);
        *y += ROW + 2.0;
        let mut when = triggers[(s.trigger as usize).min(2)].clone();
        if choice_row(g, y, x, w, "  Goes off", &triggers, &mut when) {
            s.trigger = triggers.iter().position(|t| *t == when).unwrap_or(0) as u8;
        }
        value_row(g, y, x, w, "  Spell number", 430 + i, &mut s.spell);
        hint(g, y, x, &spell_name(s.spell).map_or("Not a spell of the original's list.".to_string(), |name| format!("= {name}")));
        text_area_row(g, y, x, w, "  Tooltip text", 440 + i, 2, &mut s.text);
        value_row(g, y, x, w, "  Charges", 1100 + i * 4, &mut s.charges);
        hint(g, y, x, "Negative: one use uses the item up.");
        value_row(g, y, x, w, "  Cooldown (ms)", 1101 + i * 4, &mut s.cooldown);
        hint(g, y, x, "-1: the spell's own.");
        value_row(g, y, x, w, "  Procs a minute", 1102 + i * 4, &mut s.ppm);
        hint(g, y, x, "For a spell on a blow; 0: the spell's own chance.");
        effects_form(g, y, x, w, &mut s.effects, 3000 + i * 100);
        if g.danger(&format!("remove-spell-{i}"), x + LABEL_WIDTH, *y, 150.0, "Remove this spell line") {
            remove = Some(i);
        }
        *y += ROW + 10.0;
    }
    if let Some(i) = remove {
        it.spells.remove(i);
    }
    if g.button(x, *y, 150.0, "Add a spell line") {
        it.spells.push(crate::items::ItemSpell { spell: 0, trigger: 0, charges: 0, cooldown: 0, ppm: 0.0, effects: Vec::new(), text: String::new() });
    }
    *y += ROW + 3.0;
}

pub(super) fn item_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, it: &mut RawItem, pv: &mut Preview) {
    text_row(g, y, x, w, "Id", 400, &mut it.id);
    hint(g, y, x, "What loot, shops, quests and saved characters call it by; keep it once it is in use.");
    text_row(g, y, x, w, "Name", 401, &mut it.name);
    text_area_row(g, y, x, w, "Lore", 425, 2, &mut it.text);
    hint(g, y, x, "The line of story text in the tooltip; may be left empty.");
    item_panel(g, y, x, w, it, pv);
    text_row(g, y, x, w, "Tooltip left", 403, &mut it.kind.0);
    text_row(g, y, x, w, "Tooltip right", 404, &mut it.kind.1);
    hint(g, y, x, "The two words under the name in the tooltip, such as One-Hand and Sword, or Chest and Leather.");
    *y += 4.0;

    if section(g, y, x, w, "item-combat", "Armor, weapon and bonuses", true) {
        value_row(g, y, x, w, "Armor", 405, &mut it.armor);
        value_row(g, y, x, w, "Block", 406, &mut it.block);
        let mut stats = [it.stats.0, it.stats.1, it.stats.2, it.stats.3, it.stats.4];
        if numbers_row(g, y, x, w, "Adds", 407, &["Str", "Agi", "Sta", "Int", "Spi"], &mut stats) {
            it.stats = (stats[0], stats[1], stats[2], stats[3], stats[4]);
        }
        hint(g, y, x, "Strength, agility, stamina, intellect and spirit it gives while worn.");
        let mut vitals = [it.health, it.mana];
        if numbers_row(g, y, x, w, "Adds health, mana", 423, &["Health", "Mana"], &mut vitals) {
            (it.health, it.mana) = (vitals[0], vitals[1]);
        }
        numbers_row(g, y, x, w, "Resists", 424, &["Holy", "Fire", "Nature", "Frost", "Shadow", "Arcane"], &mut it.resist);
        let mut weapon = it.weapon.is_some();
        if switch_row(g, y, x, "Is a weapon", &mut weapon) {
            it.weapon = weapon.then_some(crate::items::Weapon { min: 5.0, max: 8.0, speed: 2.0 });
        }
        if let Some(wp) = &mut it.weapon {
            let mut v = [wp.min, wp.max, wp.speed];
            if numbers_row(g, y, x, w, "  Blow", 408, &["Least", "Most", "Every (s)"], &mut v) {
                (wp.min, wp.max, wp.speed) = (v[0], v[1], v[2]);
            }
            // More damage of a weapon, of other schools than the blow itself.
            let mut remove = None;
            for (i, m) in it.more.iter_mut().enumerate() {
                let mut v = [m.0, m.1, m.2 as f32];
                if numbers_row(g, y, x, w, &format!("  More damage {}", i + 1), 460 + i * 3, &["Least", "Most", "School"], &mut v) {
                    *m = (v[0], v[1], v[2].clamp(0.0, 255.0) as u8);
                }
                if g.button(x + LABEL_WIDTH, *y, 90.0, "Remove") {
                    remove = Some(i);
                }
                *y += ROW + 3.0;
            }
            if let Some(i) = remove {
                it.more.remove(i);
            }
            if g.button(x + LABEL_WIDTH, *y, 150.0, "Add more damage") {
                it.more.push((1.0, 2.0, 2));
            }
            *y += ROW + 3.0;
            hint(g, y, x, "Damage of another school besides the blow. School: 1 holy, 2 fire, 3 nature, 4 frost, 5 shadow, 6 arcane.");
        }
    }

    if section(g, y, x, w, "item-value", "Price and stacking", true) {
        money_row(g, y, x, w, "Buy price", 412, &mut it.buy);
        money_row(g, y, x, w, "Sell price", 413, &mut it.sell);
        value_row(g, y, x, w, "Stack size", 409, &mut it.stack);
        hint(g, y, x, "How many fit on one place of the pack.");
        value_row(g, y, x, w, "Sold in packs of", 428, &mut it.buy_count);
        option_value_row(g, y, x, w, "Weight", 450, &mut it.weight);
        hint(g, y, x, "What one of it weighs in the pack; empty: the usual for its kind.");
        value_row(g, y, x, w, "Bag slots", 451, &mut it.slots);
        hint(g, y, x, "For a bag or a quiver: how many places it adds to the pack; 0 for none.");
    }

    if section(g, y, x, w, "item-use", "What it does", !it.spells.is_empty() || it.tome.is_some()) {
        option_ref_row(g, y, x, w, "Teaches spell", 411, &mut it.tome, Ref::Spell);
        hint(g, y, x, "Reading it teaches this spell; empty: nothing.");
        spells_form(g, y, x, w, it);
        let mut conjured = it.flags & 2 != 0;
        if switch_row(g, y, x, "Conjured", &mut conjured) {
            it.flags = if conjured { it.flags | 2 } else { it.flags & !2 };
        }
        hint(g, y, x, "Adds the line \"Conjured Item\" to the tooltip.");
        switch_row(g, y, x, "Quest item", &mut it.quest);
        hint(g, y, x, "Drops only for a quest that wants it, and is not sold.");
    }

    if section(g, y, x, w, "item-needs", "Who can use it", false) {
        value_row(g, y, x, w, "Level needed", 421, &mut it.level);
        let binds: Vec<String> = ["Never", "Pickup", "Equip", "Use", "Quest"].iter().map(|s| s.to_string()).collect();
        let mut bind = format!("{:?}", it.bind);
        if choice_row(g, y, x, w, "Binds", &binds, &mut bind) {
            it.bind = [crate::items::Bind::Never, crate::items::Bind::Pickup, crate::items::Bind::Equip, crate::items::Bind::Use, crate::items::Bind::Quest].into_iter().find(|b| format!("{b:?}") == bind).unwrap_or_default();
        }
        hint(g, y, x, "Which line of binding the tooltip shows: when picked up, when equipped, when used, or quest item.");
        switch_row(g, y, x, "Unique", &mut it.unique);
        hint(g, y, x, "Adds the line \"Unique\" to the tooltip.");
        value_row(g, y, x, w, "Durability", 422, &mut it.durability);
        let mut skill = [it.skill.0, it.skill.1];
        if numbers_row(g, y, x, w, "Needs skill", 426, &["Skill", "Rank"], &mut skill) {
            it.skill = (skill[0], skill[1]);
        }
        hint(g, y, x, "A skill number of the original and the rank of it; 0 for none.");
        mask_row(g, y, x, w, "Classes", &CLASS_BITS, &mut it.classes);
        mask_row(g, y, x, w, "Races", &RACE_BITS, &mut it.races);
    }

    if section(g, y, x, w, "item-original", "As in the original (advanced)", false) {
        let classes: Vec<String> = CLASSES.iter().map(|(_, name)| name.to_string()).collect();
        let mut class = CLASSES.iter().find(|(n, _)| *n == it.class).map_or_else(|| format!("Class {}", it.class), |(_, name)| name.to_string());
        if choice_row(g, y, x, w, "Kind of item", &classes, &mut class) {
            if let Some((n, _)) = CLASSES.iter().find(|(_, name)| *name == class) {
                (it.class, it.sub) = (*n, 0);
            }
        }
        let mut sub = it.sub as u32;
        if value_row(g, y, x, w, "Subclass", 429, &mut sub) {
            it.sub = sub.min(255) as u8;
        }
        let named = content::subclass_name(it.class, it.sub);
        hint(g, y, x, &if named.is_empty() { "The kind within the class (7 is a sword for a weapon, 1 cloth for armor ...).".to_string() } else { format!("= {named}") });
        text_row(g, y, x, w, "Sound set", 410, &mut it.handling);
        hint(g, y, x, "The sounds of picking it up and putting it down, such as MetalLarge, WoodSmall, Book, Gems or Bag.");
        value_row(g, y, x, w, "Flags", 452, &mut it.flags);
        hint(g, y, x, "The original's item flags as one number; the game uses 2 (conjured, set above) and keeps the rest as they are.");
    }
    *y += 6.0;
}

