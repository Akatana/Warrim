use super::*;
use crate::content::Lesson;
use crate::skills::Skill;

/// What a lesson is called in the list: the recipe, or the step of the trade.
fn name(l: &Lesson) -> String {
    match (l.step, crate::recipes::get(l.spell)) {
        (None, Some((_, r))) => r.name.clone(),
        _ => content::get().classic_index.get(&l.spell).map_or_else(|| format!("spell {}", l.spell), |&i| content::get().classic_spells[i].name.clone()),
    }
}

/// The spell that teaches a step of a trade, as the original's teachers have it (for the name and the picture).
fn step_spell(skill: Skill, step: u32) -> Option<u32> {
    content::get().lessons.values().flatten().find(|l| l.step == Some((skill, step))).map(|l| l.spell)
}

fn price(copper: u32) -> String {
    format!("{}g {}s {}c", copper / 10_000, copper / 100 % 100, copper % 100)
}

fn trades() -> Vec<String> {
    crate::skills::ALL.iter().filter(|s| s.is_profession()).map(|s| s.name().to_string()).collect()
}

fn trade_named(name: &str) -> Option<Skill> {
    crate::skills::ALL.into_iter().find(|s| s.is_profession() && s.name() == name)
}

/// What a teacher teaches: the lessons of the trainer window, each a recipe or a step of a trade (apprentice to artisan) with its price and what it asks for. The
/// first lesson also gives the creature the menu line that opens them. `others` are the lessons of the other teachers, to copy from.
pub(super) fn lessons_form(g: &mut Gui, y: &mut f32, x: f32, w: f32, c: &mut RawCreature, lessons: &mut Vec<Lesson>, others: &std::collections::BTreeMap<String, Vec<Lesson>>, selected: &mut usize) {
    hint(g, y, x, "The recipes and the steps of the trades that the creature teaches for a price (the trainer window). Give it a menu line \"Opens the lessons\" (added with the first lesson).");
    *selected = (*selected).min(lessons.len().saturating_sub(1));
    for (i, l) in lessons.iter().enumerate() {
        let line = format!("{}{}   {}", if i == *selected { "> " } else { "  " }, name(l), price(l.cost));
        if g.button(x + LABEL_WIDTH, *y, w - LABEL_WIDTH, &line) {
            *selected = i;
        }
        *y += ROW + 2.0;
    }
    *y += 4.0;
    let mut add = None;
    if g.button(x + LABEL_WIDTH, *y, 140.0, "Add a recipe") {
        add = crate::content::get().recipes.first().map(|r| Lesson { spell: r.spell, cost: 100, skill: None, level: 0, step: None });
    }
    if g.button(x + LABEL_WIDTH + 146.0, *y, 190.0, "Add a step of a trade") {
        add = step_spell(Skill::Mining, 1).map(|spell| Lesson { spell, cost: 10, skill: None, level: 0, step: Some((Skill::Mining, 1)) });
    }
    *y += ROW + 3.0;
    // Another teacher's lessons, all at once.
    let mut from = String::new();
    if text_row_ref(g, y, x, w, "Copy the lessons of", 7090, &mut from, Ref::Creature, None) && others.contains_key(from.trim()) {
        *lessons = others[from.trim()].clone();
        add = None;
    }
    if let Some(l) = add {
        lessons.push(l);
        *selected = lessons.len() - 1;
    }
    if !lessons.is_empty() {
        let gossip = c.gossip.get_or_insert_with(|| content::Gossip { greeting: "Greetings, $N.".into(), options: Vec::new() });
        if !gossip.options.iter().any(|o| o.action == CreatureAction::Teach) {
            gossip.options.push(GossipOption { text: "Train me.".into(), action: CreatureAction::Teach });
        }
    }
    let Some(l) = lessons.get_mut(*selected) else { return };
    g.text(x, *y + 6.0, w, &format!("The selected lesson: {}", name(l)), GOLD);
    *y += ROW + 8.0;
    match l.step {
        None => {
            let mut spell = l.spell.to_string();
            if text_row_ref(g, y, x, w, "Recipe", 7001, &mut spell, Ref::Recipe, None) {
                if let Some(s) = spell.trim().parse::<u32>().ok().filter(|s| crate::recipes::get(*s).is_some()) {
                    l.spell = s;
                }
            }
        }
        Some((skill, step)) => {
            let names = trades();
            let mut trade = skill.name().to_string();
            let mut number = step;
            let changed = choice_row(g, y, x, w, "Trade", &names, &mut trade) | value_row(g, y, x, w, "Step (1 to 4)", 7002, &mut number);
            if changed {
                if let Some(skill) = trade_named(&trade) {
                    let step = number.clamp(1, 4);
                    if let Some(spell) = step_spell(skill, step) {
                        (l.spell, l.step) = (spell, Some((skill, step)));
                    }
                }
            }
            hint(g, y, x, "1 the apprentice (to rank 75), 2 the journeyman, 3 the expert, 4 the artisan.");
        }
    }
    money_row(g, y, x, w, "Price", 7004, &mut l.cost);
    value_row(g, y, x, w, "Min. level", 7005, &mut l.level);
    let mut options = vec!["None".to_string()];
    options.extend(trades());
    let mut needs = l.skill.map_or("None".to_string(), |(s, _)| s.name().to_string());
    if choice_row(g, y, x, w, "Needs skill", &options, &mut needs) {
        l.skill = trade_named(&needs).map(|s| (s, l.skill.map_or(1, |k| k.1)));
    }
    if let Some((_, rank)) = &mut l.skill {
        value_row(g, y, x, w, "  of rank", 7006, rank);
    }
    if g.danger("delete-lesson", x + LABEL_WIDTH, *y, 160.0, "Remove the lesson") {
        lessons.remove(*selected);
    }
    *y += ROW + 6.0;
}
