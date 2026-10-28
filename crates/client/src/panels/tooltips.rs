//! Tooltip lines for items and stats.

use super::*;

impl Panels {
    // ---- tooltips

    pub(super) fn item_lines(&self, stack: Stack) -> Vec<TipLine> {
        let mut lines = self.item_lines_plain(stack);
        // What was put on it with Enchanting, in green.
        if let Some(e) = content::get().enchants.get(&stack.enchant) {
            lines.push(TipLine::new(e.name.clone(), GREEN));
        }
        lines
    }

    pub(super) fn item_lines_plain(&self, stack: Stack) -> Vec<TipLine> {
        let item: &ItemDef = crate::items::get(stack.item);
        let q = QUALITY_COLORS[item.quality];
        let mut lines = vec![TipLine::new(item.name, [q[0], q[1], q[2], 1.0])];
        // How it binds, whether there can be only one, and a conjured item that is gone in time.
        let bind = match item.bind {
            crate::items::Bind::Pickup => Some("ITEM_BIND_ON_PICKUP"),
            crate::items::Bind::Equip => Some("ITEM_BIND_ON_EQUIP"),
            crate::items::Bind::Use => Some("ITEM_BIND_ON_USE"),
            crate::items::Bind::Quest => Some("ITEM_BIND_QUEST"),
            crate::items::Bind::Never => item.quest.then_some("ITEM_BIND_QUEST"),
        };
        if let Some(key) = bind {
            let text = self.tree.string(key);
            lines.push(TipLine::new(if text.is_empty() { "Quest Item".to_string() } else { text }, WHITE));
        }
        if item.unique {
            lines.push(TipLine::new(self.tree.string("ITEM_UNIQUE"), WHITE));
        }
        if item.flags & 2 != 0 {
            lines.push(TipLine::new(self.tree.string("ITEM_CONJURED"), WHITE));
        }
        if item.slots > 0 {
            lines.push(TipLine::new(self.tree.string("CONTAINER_SLOTS").replacen("%d", &item.slots.to_string(), 1).replacen("%s", if item.subclass == 0 { "Bag" } else { "Quiver" }, 1), WHITE));
        }
        if !item.kind.0.is_empty() {
            lines.push(TipLine::double(item.kind.0, item.kind.1, WHITE, WHITE));
        }
        if let Some(w) = item.weapon {
            lines.push(TipLine::double(format!("{:.0} - {:.0} Damage", w.min, w.max), format!("Speed {:.2}", w.speed), WHITE, WHITE));
            for &(low, high, school) in item.more_damage {
                lines.push(TipLine::new(format!("+ {:.0} - {:.0} {} Damage", low, high, SCHOOLS[(school as usize).min(6)]), WHITE));
            }
            let dps = (w.min + w.max + item.more_damage.iter().map(|d| d.0 + d.1).sum::<f32>()) / 2.0 / w.speed;
            lines.push(TipLine::new(format!("({dps:.1} damage per second)"), WHITE));
        }
        if item.armor > 0 {
            lines.push(TipLine::new(format!("{} Armor", item.armor), WHITE));
        }
        if item.block > 0 {
            lines.push(TipLine::new(format!("{} Block", item.block), WHITE));
        }
        for (name, bonus) in ["Strength", "Agility", "Stamina", "Intellect", "Spirit"].iter().zip(item.stats) {
            if bonus != 0 {
                lines.push(TipLine::new(format!("{}{} {name}", if bonus > 0 { "+" } else { "-" }, bonus.abs()), WHITE));
            }
        }
        for (name, bonus) in [("Health", item.health), ("Mana", item.mana)] {
            if bonus != 0 {
                lines.push(TipLine::new(format!("{}{} {name}", if bonus > 0 { "+" } else { "-" }, bonus.abs()), WHITE));
            }
        }
        for (school, bonus) in item.resist.iter().enumerate() {
            if *bonus != 0 {
                lines.push(TipLine::new(format!("{}{} {} Resistance", if *bonus > 0 { "+" } else { "-" }, bonus.abs(), SCHOOLS[school + 1]), WHITE));
            }
        }
        if item.weight > 0.0 {
            lines.push(TipLine::new(format!("Weight: {:.2}", item.weight), WHITE));
        }
        if item.durability > 0 {
            lines.push(TipLine::new(self.tree.string("DURABILITY_TEMPLATE").replacen("%d", &item.durability.to_string(), 2), WHITE));
        }
        // Who it is for, and what is asked of whoever uses it (red when this character falls short).
        if item.classes != 0 {
            let names: Vec<&str> = CLASSES.iter().filter(|(bit, _)| item.classes & bit != 0).map(|(_, n)| *n).collect();
            lines.push(TipLine::new(self.tree.string("ITEM_CLASSES_ALLOWED").replacen("%s", &names.join(", "), 1), WHITE));
        }
        if item.races != 0 {
            let names: Vec<&str> = RACES.iter().filter(|(bit, _)| item.races & bit != 0).map(|(_, n)| *n).collect();
            lines.push(TipLine::new(self.tree.string("ITEM_RACES_ALLOWED").replacen("%s", &names.join(", "), 1), if item.races & self.race.mask() != 0 { WHITE } else { RED }));
        }
        if item.level > 1 {
            lines.push(TipLine::new(self.tree.string("ITEM_MIN_LEVEL").replacen("%d", &item.level.to_string(), 1), if self.progress.level >= item.level { WHITE } else { RED }));
        }
        if item.skill.0 != 0 {
            lines.push(TipLine::new(self.tree.string("ITEM_REQ_SKILL").replacen("%s", &format!("{} ({})", skill_name(item.skill.0), item.skill.1), 1), RED));
        }
        if let Some(spell) = item.tome {
            let def = content::spell(spell);
            if def.level > 1 {
                lines.push(TipLine::new(format!("Requires {} ({})", def.class.name(), def.level), if self.skills.rank(def.class) >= def.level { WHITE } else { RED }));
            }
            lines.push(TipLine::new(format!("Use: Teaches you {}.", def.name), [0.12, 1.0, 0.0, 1.0]));
        }
        // What the original's spells do when it is used, worn or lands a blow, in the words of the spell.
        for s in item.spells {
            let trigger = match s.trigger {
                0 | 4 | 5 => "ITEM_SPELL_TRIGGER_ONUSE",
                1 => "ITEM_SPELL_TRIGGER_ONEQUIP",
                2 => "ITEM_SPELL_TRIGGER_ONPROC",
                _ => continue,
            };
            if !s.text.is_empty() {
                lines.push(TipLine::new(format!("{} {}", self.tree.string(trigger), s.text.replace("$z", "Northshire Valley")), [0.12, 1.0, 0.0, 1.0]).wrapped());
            }
        }
        if !item.text.is_empty() {
            lines.push(TipLine::new(format!("\"{}\"", item.text), [1.0, 0.82, 0.0, 1.0]).wrapped());
        }
        lines
    }

    /// The tooltip of a stat of the character sheet: the original's (the stat and the class's words on it), and under it what the game works
    /// out of it (the original shows none of these numbers; its 1.12 sheet has no lines for them).
    pub(super) fn stat_lines(&self, name: &str) -> Option<Vec<TipLine>> {
        let s = self.stats;
        let level = self.progress.level;
        let head = |text: String| TipLine::new(text, WHITE);
        let note = |text: String| TipLine::new(text, GOLD);
        let more = |text: String| TipLine::new(text, WHITE);
        if let Some(i) = name.strip_prefix("CharacterStatFrame").and_then(|n| n.chars().next()?.to_digit(10)).filter(|i| (1..=5).contains(i)) {
            let i = i as usize - 1;
            let stat = ["STRENGTH", "AGILITY", "STAMINA", "INTELLECT", "SPIRIT"][i];
            let words = self.string(&format!("PALADIN_{stat}_TOOLTIP"));
            let words = if words.starts_with("PALADIN_") { self.string(&format!("DEFAULT_{stat}_TOOLTIP")) } else { words };
            let mut lines = vec![head(format!("{} {}", self.string(&format!("SPELL_STAT{i}_NAME")), s.attributes[i])), note(words)];
            match i {
                0 => {
                    lines.push(more(format!("Melee attack power: {}", s.attack_power)));
                    if s.block > 0.0 {
                        lines.push(more(format!("Block value: {:.0}", s.block_value)));
                    }
                }
                1 => lines.extend([
                    more(format!("Critical strike chance: {:.2}%", s.crit)),
                    more(format!("Dodge chance: {:.2}%", s.dodge)),
                    more(format!("Ranged attack power: {}", s.ranged_attack_power)),
                ]),
                2 => lines.push(more(format!("Maximum health: {:.0}", s.max_health))),
                3 => {
                    let (low, high) = s.spell_crit[1..].iter().fold((f32::MAX, 0.0f32), |(l, h), c| (l.min(*c), h.max(*c)));
                    let crit = if high - low < 0.005 { format!("{low:.2}%") } else { format!("{low:.2}% to {high:.2}% by school") };
                    lines.extend([more(format!("Spell critical strike chance: {crit}")), more(format!("Maximum mana: {:.0}", s.max_mana))]);
                    if s.healing > 0 || s.spell_damage.iter().any(|d| *d > 0) {
                        lines.push(more(format!("Healing done: +{}; holy damage done: +{}", s.healing, s.spell_damage[1])));
                    }
                    if s.spell_hit != 0.0 {
                        lines.push(more(format!("Chance to hit with spells: +{:.0}%", s.spell_hit)));
                    }
                }
                _ => lines.extend([
                    more(format!("Health regeneration: {:.1} per second out of combat", s.health_regen)),
                    more(format!("Mana regeneration: {:.1} per second ({:.1} just after a spell)", s.mana_regen, s.mana_regen_casting)),
                ]),
            }
            return Some(lines);
        }
        Some(match name {
            "CharacterArmorFrame" => {
                let reduction = 100.0 * s.mitigation();
                let words = self.string("ARMOR_TOOLTIP").replacen("%d", &level.to_string(), 1).replacen("%.1f%%", &format!("{reduction:.1}%"), 1);
                vec![
                    head(format!("{} {}", self.string("ARMOR"), s.armor)),
                    note(words),
                    more(format!("Dodge chance: {:.2}%", s.dodge)),
                    more(format!("Parry chance: {:.2}%", s.parry)),
                    more(format!("Block chance: {:.2}%", s.block)),
                ]
            }
            "CharacterAttackFrame" => vec![
                head(self.string("ATTACK_TOOLTIP")),
                note(self.string("ATTACK_TOOLTIP_SUBTEXT")),
                more(format!("Critical strike chance: {:.2}%", s.crit)),
                more(format!("Chance to miss a creature of your level: {:.1}%", crate::levels::melee_miss(level, level, s.hit))),
            ],
            "CharacterAttackPowerFrame" => vec![
                head(format!("{} {}", self.string("MELEE_ATTACK_POWER"), s.attack_power)),
                note(self.string("MELEE_ATTACK_POWER_TOOLTIP").replacen("%.1f", &format!("{:.1}", s.attack_power as f32 / crate::items::ATTACK_POWER_PER_DPS), 1)),
            ],
            "CharacterRangedAttackPowerFrame" if s.ranged.is_some() => vec![
                head(format!("{} {}", self.string("RANGED_ATTACK_POWER"), s.ranged_attack_power)),
                note(self.string("RANGED_ATTACK_POWER_TOOLTIP").replacen("%.1f", &format!("{:.1}", s.ranged_attack_power as f32 / crate::items::ATTACK_POWER_PER_DPS), 1)),
            ],
            _ => return None,
        })
    }

    /// The tooltip of a button of the target frame's auras: the spell's name, the kind of curse it is (in the colour of its border), and the time it has left.
    fn target_aura_lines(&self, name: &str) -> Option<Vec<TipLine>> {
        let (harmful, n) = match (name.strip_prefix("TargetFrameDebuff"), name.strip_prefix("TargetFrameBuff")) {
            (Some(n), _) => (true, n),
            (_, Some(n)) => (false, n),
            _ => return None,
        };
        let list = if harmful { &self.target_auras.0 } else { &self.target_auras.1 };
        let aura = list.get(n.parse::<usize>().ok()?.checked_sub(1)?)?;
        let mut lines = vec![TipLine::new(aura.name.clone(), WHITE)];
        if let Some((kind, color)) = [None, Some(("Magic", [0.2, 0.6, 1.0, 1.0])), Some(("Curse", [0.6, 0.0, 1.0, 1.0])), Some(("Disease", [0.6, 0.4, 0.0, 1.0])), Some(("Poison", [0.0, 0.6, 0.0, 1.0]))].get(aura.dispel as usize).copied().flatten() {
            lines.push(TipLine::new(kind, color));
        }
        let left = aura.left.ceil() as u32;
        lines.push(TipLine::new(if left >= 60 { format!("{} min remaining", left.div_ceil(60)) } else { format!("{left} sec remaining") }, WHITE));
        Some(lines)
    }

    /// The tooltip for the frame under the mouse, if it has one.
    pub(super) fn tooltip(&self) -> Option<Vec<TipLine>> {
        let hover = self.tree.hover?;
        let name = self.tree.name(hover);
        if let Some(action) = self.book_entry_of(name) {
            return Some(self.action_lines(action));
        }
        if let Some(tab) = name.strip_prefix("SpellBookSkillLineTab").and_then(|n| n.parse::<usize>().ok()).and_then(|i| self.book_tab_name(i - 1)) {
            return Some(vec![TipLine::new(tab, WHITE)]);
        }
        if let Some(lines) = self.talent_lines(name).or_else(|| self.trade_lines(name)).or_else(|| self.trainer_lines(name)) {
            return Some(lines);
        }
        if let Some(lines) = self.target_aura_lines(name) {
            return Some(lines);
        }
        if let Some(slot) = SLOT_BUTTONS.iter().position(|b| *b == name) {
            return Some(match self.inv.equipment[slot] {
                Some(stack) => self.item_lines(stack),
                // An empty slot names itself: `HEADSLOT` is "Head".
                None => vec![TipLine::new(self.string(&name["Character".len()..].to_uppercase()), WHITE)],
            });
        }
        if let Some(i) = name.strip_prefix("LootButton").and_then(|n| n.parse::<usize>().ok()) {
            let (_, loot) = self.loot.as_ref()?;
            let k = (*Self::loot_slots(loot).get(i - 1)?)?;
            return Some(self.item_lines(loot.items[k]));
        }
        if let Some(i) = name.strip_prefix("MerchantItem").and_then(|n| n.strip_suffix("ItemButton")).and_then(|n| n.parse::<usize>().ok()) {
            let &item = self.merchant?.1.vendor.get(self.merchant_page * MERCHANT_ITEMS + i - 1)?;
            return Some(self.item_lines(Stack { item, count: 1, enchant: 0 }));
        }
        if let Some(i) = name.strip_prefix("QuestLogItem").and_then(|n| n.parse::<usize>().ok()) {
            let quest = self.quests.active.get(self.log_selected)?.quest;
            return Some(self.item_lines(*content::quest(quest).rewards().nth(i - 1)?));
        }
        if let Some((_, _, quest, _)) = self.questing {
            let q = content::quest(quest);
            for prefix in ["QuestDetailItem", "QuestRewardItem", "QuestProgressItem"] {
                if let Some(i) = name.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok()) {
                    let stack = if prefix == "QuestProgressItem" {
                        q.goals.iter().filter_map(|g| if let Goal::Collect { item, count } = *g { Some(Stack { item, count, enchant: 0 }) } else { None }).nth(i - 1)?
                    } else {
                        *q.rewards().nth(i - 1)?
                    };
                    return Some(self.item_lines(stack));
                }
            }
        }
        if name == "MerchantBuyBackItemItemButton" {
            return self.buyback.map(|(stack, _)| self.item_lines(stack));
        }
        if let Some(lines) = self.stat_lines(name) {
            return Some(lines);
        }
        let j = name.strip_prefix(&format!("{BACKPACK}Item"))?.parse::<usize>().ok()?;
        let stack = self.inv.bag[self.bag_index(j)]?;
        let mut lines = self.item_lines(stack);
        // With a merchant open, the tooltip also says what he would pay.
        if self.merchant.is_some() {
            let price = crate::items::get(stack.item).sell * stack.count;
            lines.push(match price {
                0 => TipLine::new("No sell price", WHITE),
                p => TipLine::new(format!("Sell Price: {}", coins(p)), WHITE),
            });
        }
        Some(lines)
    }
}
