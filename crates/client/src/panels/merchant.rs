//! Buying from and selling to a merchant.

use super::*;

impl Panels {
    // ---- the merchant

    /// Open the merchant window for creature `creature` of kind `def` (with the backpack, as the original does).
    pub fn open_merchant(&mut self, creature: usize, def: &'static CreatureDef) {
        self.gossip = None;
        self.tree.show(GOSSIP, false);
        self.merchant = Some((creature, def));
        self.merchant_page = 0;
        self.tree.show(MERCHANT, true);
        self.tree.show(BACKPACK, true);
        self.events.push(Sfx::Named("igBackPackOpen"));
        self.fill_merchant();
    }

    pub fn close_merchant(&mut self) {
        if self.merchant.take().is_some() {
            self.tree.show(BACKPACK, false);
            self.events.push(Sfx::Named("igBackPackClose"));
        }
        self.tree.show(MERCHANT, false);
        self.after_close();
    }

    /// `MerchantFrame_UpdateMerchantInfo`: the wares with their prices, and the last thing sold in the buyback slot.
    pub(super) fn fill_merchant(&mut self) {
        let Some((_, def)) = self.merchant else { return };
        self.tree.set_text("MerchantNameText", &def.name);
        // More than ten wares make pages, with the buttons and the page number of the original.
        let pages = def.vendor.len().div_ceil(MERCHANT_ITEMS).max(1);
        self.merchant_page = self.merchant_page.min(pages - 1);
        for button in ["MerchantPrevPageButton", "MerchantNextPageButton"] {
            self.tree.show(button, pages > 1);
        }
        let page_text = self.string("PAGE_NUMBER").replacen("%d", &(self.merchant_page + 1).to_string(), 1);
        self.tree.set_text("MerchantPageText", &page_text);
        self.tree.set_region_hidden("MerchantPageText", pages <= 1);
        for i in 1..=MERCHANT_ITEMS {
            let (frame, button) = (format!("MerchantItem{i}"), format!("MerchantItem{i}ItemButton"));
            match def.vendor.get(self.merchant_page * MERCHANT_ITEMS + i - 1) {
                Some(&item) => {
                    let it = crate::items::get(item);
                    self.tree.set_text(&format!("{frame}Name"), it.name);
                    self.tree.set_texture(&format!("{button}IconTexture"), Some(it.icon));
                    // What a merchant sells at a time (arrows by the hundred) is the number on the button.
                    self.tree.set_text(&format!("{button}Count"), &it.buy_count.to_string());
                    self.tree.set_region_hidden(&format!("{button}Count"), it.buy_count <= 1);
                    self.tree.set_money(&format!("{frame}MoneyFrame"), it.buy, true, false);
                    self.tree.show(&format!("{frame}MoneyFrame"), true);
                    self.tree.show(&button, true);
                }
                None => {
                    self.tree.set_text(&format!("{frame}Name"), "");
                    self.tree.show(&format!("{frame}MoneyFrame"), false);
                    self.tree.show(&button, false);
                }
            }
        }
        let back = "MerchantBuyBackItem";
        match self.buyback {
            Some((stack, price)) => {
                let it = crate::items::get(stack.item);
                self.tree.set_text(&format!("{back}Name"), it.name);
                self.tree.set_texture(&format!("{back}ItemButtonIconTexture"), Some(it.icon));
                self.tree.set_text(&format!("{back}ItemButtonCount"), &stack.count.to_string());
                self.tree.set_region_hidden(&format!("{back}ItemButtonCount"), stack.count <= 1);
                self.tree.set_money(&format!("{back}MoneyFrame"), price, true, false);
                self.tree.show(&format!("{back}MoneyFrame"), true);
            }
            None => {
                self.tree.set_text(&format!("{back}Name"), "");
                self.tree.set_texture(&format!("{back}ItemButtonIconTexture"), None);
                self.tree.set_region_hidden(&format!("{back}ItemButtonCount"), true);
                self.tree.show(&format!("{back}MoneyFrame"), false);
            }
        }
    }

    /// The error for a purchase the purse or the pack cannot take.
    pub(super) fn refuse(&mut self, why: crate::items::Refusal) {
        let key = match why {
            crate::items::Refusal::TooPoor => "ERR_NOT_ENOUGH_MONEY",
            crate::items::Refusal::PackFull => "ERR_INV_FULL",
        };
        self.error(&self.string(key));
    }

    /// Turn to a page of the wares (for previews).
    pub fn merchant_page_to(&mut self, page: usize) {
        self.merchant_page = page;
        self.fill_merchant();
    }

    /// Click on ware `n` (counted from 1): buy one.
    pub(super) fn buy_ware(&mut self, n: usize) {
        let Some(&item) = self.merchant.and_then(|(_, def)| def.vendor.get(self.merchant_page * MERCHANT_ITEMS + n - 1)) else { return };
        match self.inv.buy(item) {
            Ok(()) => {
                self.events.push(Sfx::Named("LOOTWINDOWCOINSOUND"));
                self.events.push(Sfx::PickUp(crate::items::get(item).handling));
            }
            Err(why) => self.refuse(why),
        }
    }

    /// Click on the buyback slot: get the last thing sold back for what it fetched.
    pub(super) fn buy_back(&mut self) {
        let Some((stack, price)) = self.buyback else { return };
        match self.inv.pay(stack, price) {
            Ok(()) => {
                self.buyback = None;
                self.events.push(Sfx::Named("LOOTWINDOWCOINSOUND"));
                self.events.push(Sfx::PickUp(crate::items::get(stack.item).handling));
            }
            Err(why) => self.refuse(why),
        }
    }

    /// Right-click on a backpack item with a merchant open: sell it (the whole stack).
    pub(super) fn sell_item(&mut self, slot: usize) {
        match self.inv.sell(slot) {
            Some((stack, price)) => {
                self.buyback = Some((stack, price));
                self.events.push(Sfx::PutDown(crate::items::get(stack.item).handling));
                self.events.push(Sfx::Named("LOOTWINDOWCOINSOUND"));
            }
            None if self.inv.bag[slot].is_some() => self.error(&self.string("ERR_VENDOR_NOT_INTERESTED")),
            None => {}
        }
    }

    // ---- gossip

    /// What a creature or quest says, with the player filled in: `$N` the name, `$R` the race, `$C` the class (`$n`, `$r`, `$c` as the original writes them, race and class in lower case).
    pub(super) fn speak(&self, text: &str) -> String {
        speak_to(&self.name, self.race, text)
    }
}
