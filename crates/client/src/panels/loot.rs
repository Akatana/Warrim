//! The loot window.

use super::*;

impl Panels {
    // ---- loot

    /// Open the loot window for corpse `corpse` holding `loot`. The character sheet closes.
    pub fn open_loot(&mut self, corpse: usize, loot: Loot) {
        self.tree.show(CHARACTER, false);
        self.tree.show(SPELLBOOK, false);
        self.tree.show(TALENTS, false);
        self.loot = Some((corpse, loot));
        self.tree.show(LOOT, true);
        self.fill_loot();
    }

    /// Close the loot window; what is left on the corpse is handed back (see `take_closed_loot`).
    pub fn close_loot(&mut self) {
        if let Some(open) = self.loot.take() {
            self.closed_loot = Some(open);
        }
        self.tree.show(LOOT, false);
        self.after_close();
    }

    /// The corpse a loot window closed on, and what is still on it.
    pub fn take_closed_loot(&mut self) -> Option<(usize, Loot)> {
        self.closed_loot.take()
    }

    /// The slots of the loot window in order: the money (`None`), then the items.
    pub(super) fn loot_slots(loot: &Loot) -> Vec<Option<usize>> {
        (loot.copper > 0).then_some(None).into_iter().chain((0..loot.items.len()).map(Some)).collect()
    }

    /// `LootFrame_Update`: the buttons for what is on the corpse, each with its picture, name and count.
    pub(super) fn fill_loot(&mut self) {
        let Some((_, loot)) = &self.loot else { return };
        let slots = Self::loot_slots(loot);
        for i in 1..=LOOT_BUTTONS {
            let button = format!("LootButton{i}");
            let Some(&slot) = slots.get(i - 1) else {
                self.tree.show(&button, false);
                continue;
            };
            let (icon, name, count, color) = match slot {
                None => (coin_icon(loot.copper), self.coin_text(loot.copper), 0, [1.0; 4]),
                Some(k) => {
                    let (stack, item) = (loot.items[k], crate::items::get(loot.items[k].item));
                    let q = QUALITY_COLORS[item.quality];
                    (item.icon, item.name.to_string(), stack.count, [q[0], q[1], q[2], 1.0])
                }
            };
            self.tree.set_texture(&format!("{button}IconTexture"), Some(icon));
            self.tree.set_text(&format!("{button}Text"), &name);
            self.tree.set_text_color(&format!("{button}Text"), color);
            self.tree.set_text(&format!("{button}Count"), &count.to_string());
            self.tree.set_region_hidden(&format!("{button}Count"), count <= 1);
            self.tree.show(&button, true);
        }
    }

    /// "5 Copper", or one denomination to a line: "1 Silver\n20 Copper".
    pub(super) fn coin_text(&self, copper: u32) -> String {
        [(copper / 10000, "GOLD"), (copper / 100 % 100, "SILVER"), (copper % 100, "COPPER")]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, key)| format!("{n} {}", self.string(key)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Click a loot button: the money goes into the purse, an item into the backpack (if it fits). An emptied
    /// corpse closes the window.
    pub(super) fn take_loot(&mut self, button: usize) {
        let Some((_, loot)) = &mut self.loot else { return };
        let Some(&slot) = Self::loot_slots(loot).get(button - 1) else { return };
        match slot {
            None => {
                self.inv.money += std::mem::take(&mut loot.copper);
                self.events.push(Sfx::Named("LOOTWINDOWCOINSOUND"));
            }
            Some(k) => {
                let stack = loot.items[k];
                if self.inv.add(stack) {
                    loot.items.remove(k);
                    self.events.push(Sfx::PickUp(crate::items::get(stack.item).handling));
                } else {
                    self.error(&self.string("INVENTORY_FULL"));
                    return;
                }
            }
        }
        if loot.is_empty() {
            self.tree.show(trade::TRADE, false);
            self.close_loot();
        } else {
            self.fill_loot();
        }
    }

    /// Take everything from the open loot window; what does not fit in the pack stays on the corpse.
    pub fn loot_all(&mut self) {
        let left = |p: &Self| p.loot.as_ref().map(|(_, l)| Self::loot_slots(l).len());
        let mut skip = 0;
        while let Some(n) = left(self).filter(|&n| skip < n) {
            self.take_loot(skip + 1);
            if left(self) == Some(n) {
                skip += 1;
            }
        }
    }

    /// An item the mouse carried goes back into the pack when no panel is left to put it in.
    pub(super) fn after_close(&mut self) {
        if !self.any_open() {
            self.inv.put_cursor_away();
            self.tree.hover = None;
            self.tree.pressed = None;
        }
        self.refresh();
    }
}
