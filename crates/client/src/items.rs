//! Items (defined in `content/items.ron`), what the player wears and carries, and the stats that follow from it. The original keeps item data on
//! the server, so these few items are the game's own; the slot numbers, stat names and formulas are the original's.

use crate::content;

/// Equipment slot numbers of the original (`INVSLOT_*`); 0 is the ammunition slot.
pub const HEAD: usize = 1;
pub const SHOULDER: usize = 3;
pub const SHIRT: usize = 4;
pub const CHEST: usize = 5;
pub const LEGS: usize = 7;
pub const HANDS: usize = 10;
pub const BACK: usize = 15;
pub const TABARD: usize = 19;
pub const FEET: usize = 8;
pub const MAIN_HAND: usize = 16;
pub const OFF_HAND: usize = 17;
pub const RANGED: usize = 18;
pub const SLOTS: usize = 20;
/// The slots the backpack shows at a time, and how many of them make a row; it holds more, scrolled a row at a time.
pub const BACKPACK_SIZE: usize = 16;
pub const BACKPACK_COLUMNS: usize = 4;

/// What the character can carry before it slows down: a base and what each point of strength adds.
pub fn carry_capacity(strength: i32) -> f32 {
    40.0 + 2.0 * strength.max(0) as f32
}

/// The share of its running speed that a load takes: nothing up to the capacity; over it a fifth at once (so that it is felt), and the further
/// over, the more, up to three quarters.
pub fn burden_slowdown(weight: f32, capacity: f32) -> f32 {
    if weight <= capacity { 0.0 } else { (0.2 + weight / capacity - 1.0).min(0.75) }
}

/// The names of the equipment slot buttons of `PaperDollFrame.xml`, by slot number.
pub const SLOT_BUTTONS: [&str; SLOTS] = [
    "CharacterAmmoSlot",
    "CharacterHeadSlot",
    "CharacterNeckSlot",
    "CharacterShoulderSlot",
    "CharacterShirtSlot",
    "CharacterChestSlot",
    "CharacterWaistSlot",
    "CharacterLegsSlot",
    "CharacterFeetSlot",
    "CharacterWristSlot",
    "CharacterHandsSlot",
    "CharacterFinger0Slot",
    "CharacterFinger1Slot",
    "CharacterTrinket0Slot",
    "CharacterTrinket1Slot",
    "CharacterBackSlot",
    "CharacterMainHandSlot",
    "CharacterSecondaryHandSlot",
    "CharacterRangedSlot",
    "CharacterTabardSlot",
];

/// The picture an empty slot shows (`GetInventorySlotInfo`).
pub const SLOT_BACKGROUNDS: [&str; SLOTS] = [
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Ammo",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Head",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Neck",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Shoulder",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Shirt",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Chest",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Waist",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Legs",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Feet",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Wrists",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Hands",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Finger",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Finger",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Trinket",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Trinket",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Chest",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-MainHand",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-SecondaryHand",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Ranged",
    r"Interface\PaperDoll\UI-PaperDoll-Slot-Tabard",
];

/// Where an item can be worn: the original's inventory types (`InventoryType` of its item data).
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize, serde::Serialize)]
pub enum Wear {
    Head,
    Neck,
    Shoulder,
    Shirt,
    Chest,
    Waist,
    Legs,
    Feet,
    Wrist,
    Hands,
    Finger,
    Trinket,
    Back,
    /// A weapon held in one hand: the main hand (dual wielding is not modelled).
    OneHand,
    /// A weapon that goes in the main hand only.
    MainHand,
    /// A weapon of both hands: it takes the off hand too.
    TwoHand,
    Shield,
    /// A weapon for the off hand only.
    OffHand,
    /// Something held in the off hand (a torch, a book, an orb).
    Held,
    /// A bow, a gun, a crossbow or a wand; or a relic, idol, libram or totem.
    Ranged,
    Thrown,
    Ammo,
    Tabard,
    /// Not worn.
    None,
}

impl Wear {
    /// Whether the item goes in equipment slot `slot`.
    pub fn fits(self, slot: usize) -> bool {
        match self {
            Wear::Head => slot == HEAD,
            Wear::Neck => slot == 2,
            Wear::Shoulder => slot == 3,
            Wear::Shirt => slot == 4,
            Wear::Chest => slot == CHEST,
            Wear::Waist => slot == 6,
            Wear::Legs => slot == 7,
            Wear::Feet => slot == FEET,
            Wear::Wrist => slot == 9,
            Wear::Hands => slot == 10,
            Wear::Finger => slot == 11 || slot == 12,
            Wear::Trinket => slot == 13 || slot == 14,
            Wear::Back => slot == 15,
            Wear::OneHand | Wear::MainHand | Wear::TwoHand => slot == MAIN_HAND,
            Wear::Shield | Wear::OffHand | Wear::Held => slot == OFF_HAND,
            Wear::Ranged | Wear::Thrown => slot == RANGED,
            Wear::Ammo => slot == 0,
            Wear::Tabard => slot == 19,
            Wear::None => false,
        }
    }

    /// The slot an item goes in by default.
    pub fn default_slot(self) -> Option<usize> {
        (0..SLOTS).find(|&s| self.fits(s))
    }

    /// The folder of `Item\ObjectComponents` that the model of what is held comes from: weapons and what is held with them, or shields.
    pub fn holds(self) -> Option<&'static str> {
        match self {
            Wear::OneHand | Wear::MainHand | Wear::TwoHand | Wear::OffHand | Wear::Held | Wear::Ranged | Wear::Thrown => Some("Weapon"),
            Wear::Shield => Some("Shield"),
            _ => None,
        }
    }

    /// The name of the place in the tooltip (`INVTYPE_*`).
    pub fn name(self) -> &'static str {
        match self {
            Wear::Head => "Head",
            Wear::Neck => "Neck",
            Wear::Shoulder => "Shoulder",
            Wear::Shirt => "Shirt",
            Wear::Chest => "Chest",
            Wear::Waist => "Waist",
            Wear::Legs => "Legs",
            Wear::Feet => "Feet",
            Wear::Wrist => "Wrist",
            Wear::Hands => "Hands",
            Wear::Finger => "Finger",
            Wear::Trinket => "Trinket",
            Wear::Back => "Back",
            Wear::OneHand => "One-Hand",
            Wear::MainHand => "Main Hand",
            Wear::TwoHand => "Two-Hand",
            Wear::Shield | Wear::OffHand => "Off Hand",
            Wear::Held => "Held In Off-hand",
            Wear::Ranged => "Ranged",
            Wear::Thrown => "Thrown",
            Wear::Ammo => "Ammo",
            Wear::Tabard => "Tabard",
            Wear::None => "",
        }
    }
}

/// When an item binds to the character (`bonding` of its item data).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Deserialize, serde::Serialize)]
pub enum Bind {
    #[default]
    Never,
    Pickup,
    Equip,
    Use,
    Quest,
}

/// What using an item (or wearing it) does to the character, as far as the game models it.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum UseEffect {
    /// Health and mana restored at once: the lowest and the highest.
    Heal(i32, i32),
    Mana(i32, i32),
    /// Health and mana restored over some seconds (food, drink, bandages): the total and the seconds.
    HealOver(i32, f32),
    ManaOver(i32, f32),
    /// A stat raised for some seconds (0 for as long as it is worn): 0 to 4 the strength, agility, stamina, intellect and spirit, 5 the armor, 6 the health.
    Boost(u8, i32, f32),
    /// The hearthstone: back to the place the character is bound to.
    Hearth,
    /// A steed to ride (a row of `CreatureDisplayInfo.dbc`) and the percent of the running speed it adds.
    Mount(u32, i32),
}

/// What an item does besides what it is: a spell of the original that it casts when used (0), worn (1), or on a blow (2),
/// with its description made from the original's `Spell.dbc`, and what the game makes of it.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ItemSpell {
    pub spell: u32,
    pub trigger: u8,
    /// Negative: the item is used up by one use.
    #[serde(default)]
    pub charges: i32,
    /// Milliseconds before it can be used again (-1 for the spell's own).
    #[serde(default)]
    pub cooldown: i32,
    #[serde(default)]
    pub effects: Vec<UseEffect>,
    #[serde(default)]
    pub text: String,
}

#[derive(Clone, Copy, Debug)]
pub struct ItemSpellDef {
    pub spell: u32,
    pub trigger: u8,
    pub charges: i32,
    pub cooldown: i32,
    pub effects: &'static [UseEffect],
    pub text: &'static str,
}

/// Item qualities and the colours of their names (`ITEM_QUALITYn_COLOR`): poor, common, uncommon, rare, epic, legendary, artifact.
pub const QUALITY_COLORS: [[f32; 3]; 7] = [[0.62, 0.62, 0.62], [1.0, 1.0, 1.0], [0.12, 1.0, 0.0], [0.0, 0.44, 0.87], [0.64, 0.21, 0.93], [1.0, 0.5, 0.0], [0.9, 0.8, 0.5]];

/// The textures an item paints on the character's skin (the names of `ItemDisplayInfo.dbc`): arm upper, arm lower,
/// hand, torso upper, torso lower, leg upper, leg lower, foot.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Look {
    pub parts: [Option<&'static str>; 8],
}

impl Look {
    pub const NONE: Look = Look { parts: [None; 8] };
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Weapon {
    pub min: f32,
    pub max: f32,
    pub speed: f32,
}

/// What an item looks like besides the textures it paints and the model it holds (the names of its `ItemDisplayInfo.dbc` row): the mesh parts
/// of the body it puts on, the models that hang on the head or the shoulders, and the cape's texture.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Visual {
    /// The three geoset groups of the row: a sleeve, a robe's skirt, and a third.
    pub geo: [u8; 3],
    /// The models (a helmet; the left and the right shoulder) and their textures, without folder or ending.
    pub models: [Option<&'static str>; 2],
    pub textures: [Option<&'static str>; 2],
    /// The rows of `HelmetGeosetVisData.dbc` for the male and the female: what a helmet hides.
    pub helm: [u16; 2],
    /// The row of `ItemVisuals.dbc` of the glows, flames and drips on a weapon (0 for none).
    pub enchant: u16,
    /// The row of `SpellVisual.dbc` of a ranged weapon's shot: the draw and release of the bow, the sounds, what the missile and its impact look like.
    pub shot: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct ItemDef {
    /// What other files and saves call it (`content/items.ron`).
    pub id: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub quality: usize,
    pub wear: Wear,
    /// The type line of the tooltip, e.g. "One-Hand" and "Sword".
    pub kind: (&'static str, &'static str),
    pub armor: i32,
    pub block: i32,
    /// Strength, agility, stamina, intellect, spirit.
    pub stats: [i32; 5],
    pub weapon: Option<Weapon>,
    /// What it looks like on the character: textures painted on the skin, and for what is held, the model in the
    /// hand (model file, skin texture). Both come from the item's `ItemDisplayInfo.dbc` row (`content/items.ron`).
    /// EVERY ITEM THAT CAN BE WORN MUST HAVE ONE OF THEM, so that it shows on the character (a test checks it); a
    /// new kind of equipment needs its drawing added to `Player::set_equipment`.
    pub look: Look,
    pub held: Option<(&'static str, &'static str)>,
    pub stack: u32,
    /// What a merchant asks for it and pays for it, copper; 0 pays nothing (it cannot be sold).
    pub buy: u32,
    pub sell: u32,
    /// How it sounds picked up and put down (the `PickUp...` / `PutDown...` sounds of the original).
    pub handling: &'static str,
    /// The spell that reading it teaches (an index into the content's spells): a tome.
    pub tome: Option<usize>,
    /// A quest item (see `RawItem::quest`).
    pub quest: bool,
    /// What it is in the original's tables: the class (weapon, armor, consumable, ...) and the subclass of it.
    pub class: u8,
    pub subclass: u8,
    /// The level the character must have reached to use it, and the classes and races (bit masks, 0 for all) it is for.
    pub level: u32,
    pub classes: u32,
    pub races: u32,
    pub bind: Bind,
    /// Only one of it can be had.
    pub unique: bool,
    pub durability: u32,
    /// What it adds to health and mana, and its resistances (holy, fire, nature, frost, shadow, arcane).
    pub health: i32,
    pub mana: i32,
    pub resist: [i32; 6],
    /// More damage of a weapon: (lowest, highest, school), the school as the original numbers them (1 holy ... 6 arcane).
    pub more_damage: &'static [(f32, f32, u8)],
    /// How many a merchant sells at a time, and the lore the tooltip tells in yellow.
    pub buy_count: u32,
    pub text: &'static str,
    pub spells: &'static [ItemSpellDef],
    /// The skill (the original's number) and rank that are needed to use it; 0 for none.
    pub skill: (u32, u32),
    /// The bag: how many slots it adds.
    pub slots: u32,
    /// The original's flags: conjured, lootable, and the like.
    pub flags: u32,
    /// What one of it weighs (see `Inventory::weight`).
    pub weight: f32,
    pub visual: Visual,
}

impl ItemDef {
    /// An item with nothing but a place to be worn (for showing a look of the tables: its textures, model and mesh parts are filled in by the caller).
    pub fn plain(wear: Wear) -> ItemDef {
        ItemDef {
            id: "", name: "", icon: "", quality: 1, wear, kind: ("", ""), armor: 0, block: 0, stats: [0; 5], weapon: None, look: Look::NONE, held: None, stack: 1, buy: 0, sell: 0, handling: "", tome: None,
            quest: false, class: 0, subclass: 0, level: 0, classes: 0, races: 0, bind: Bind::Never, unique: false, durability: 0, health: 0, mana: 0, resist: [0; 6], more_damage: &[], buy_count: 1,
            text: "", spells: &[], skill: (0, 0), slots: 0, flags: 0, weight: 0.0, visual: Visual::default(),
        }
    }
}

/// Every item of the game, in the order of `content/items.ron`; a stack refers to an item by its index here.
pub fn all() -> &'static [ItemDef] {
    &content::get().items
}

pub fn get(index: usize) -> &'static ItemDef {
    &all()[index]
}

/// The index of the item with this id.
pub fn find(id: &str) -> usize {
    all().iter().position(|i| i.id == id).unwrap_or_else(|| panic!("no item {id}"))
}

/// Why an item was not put on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EquipError {
    /// The character has not reached the level (that of the item) yet.
    Level(u32),
    /// A weapon of both hands is in the main hand.
    TwoHanded,
    /// The off hand's item had nowhere to go.
    PackFull,
    DoesNotGoThere,
}

/// Why a purchase was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    TooPoor,
    PackFull,
}

/// Some of an item on a stack.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stack {
    pub item: usize,
    pub count: u32,
}

pub struct Inventory {
    pub equipment: [Option<Stack>; SLOTS],
    /// The backpack has no end: `tidy` keeps a row of free slots after the last item (and at least `BACKPACK_SIZE` slots).
    pub bag: Vec<Option<Stack>>,
    /// What the mouse is carrying.
    pub cursor: Option<Stack>,
    /// Copper.
    pub money: u32,
}

impl Inventory {
    /// The starting kit of `content/start.ron`.
    pub fn new() -> Inventory {
        let start = &content::get().start;
        let mut inv = Inventory { equipment: [None; SLOTS], bag: vec![None; BACKPACK_SIZE], cursor: None, money: start.money };
        for &(slot, stack) in &start.equipment {
            inv.equipment[slot] = Some(stack);
        }
        for (slot, &stack) in start.bag.iter().enumerate() {
            inv.bag[slot] = Some(stack);
        }
        inv.tidy();
        inv
    }

    /// Size the backpack to what it holds: whole rows, a free one after the last item.
    pub fn tidy(&mut self) {
        let used = self.bag.iter().rposition(Option::is_some).map_or(0, |last| last + 1);
        let rows = used.div_ceil(BACKPACK_COLUMNS) + 1;
        self.bag.resize((rows * BACKPACK_COLUMNS).max(BACKPACK_SIZE), None);
    }

    /// The index of the first free backpack slot (a new one if there is none).
    fn free_slot(&mut self) -> usize {
        self.bag.iter().position(Option::is_none).unwrap_or_else(|| {
            self.bag.push(None);
            self.bag.len() - 1
        })
    }

    /// What everything the character has weighs: the pack, what is worn and what the mouse carries.
    pub fn weight(&self) -> f32 {
        self.bag.iter().chain(&self.equipment).flatten().chain(&self.cursor).map(|s| get(s.item).weight * s.count as f32).sum()
    }

    /// Whether `item` may be put in equipment slot `slot` by a character of this `level`.
    pub fn check_equip(&self, item: usize, slot: usize, level: u32) -> Result<(), EquipError> {
        let def = get(item);
        if !def.wear.fits(slot) {
            return Err(EquipError::DoesNotGoThere);
        }
        if def.level > level {
            return Err(EquipError::Level(def.level));
        }
        // The off hand takes nothing while a weapon of both hands is wielded.
        if slot == OFF_HAND && self.equipment[MAIN_HAND].is_some_and(|m| get(m.item).wear == Wear::TwoHand) {
            return Err(EquipError::TwoHanded);
        }
        Ok(())
    }

    /// A weapon of both hands was put on: what the off hand held goes into the pack. False, and nothing changed, if there is no room.
    fn clear_off_hand_for(&mut self, item: usize) -> bool {
        if get(item).wear != Wear::TwoHand {
            return true;
        }
        match self.equipment[OFF_HAND].take() {
            Some(off) if !self.add(off) => {
                self.equipment[OFF_HAND] = Some(off);
                false
            }
            _ => true,
        }
    }

    /// Click an equipment slot with the left button: pick up what is worn, or put the carried item on (swapping).
    pub fn click_equipment(&mut self, slot: usize, level: u32) -> Result<(), EquipError> {
        let Some(c) = self.cursor else {
            self.cursor = self.equipment[slot].take();
            return Ok(());
        };
        self.check_equip(c.item, slot, level)?;
        if !self.clear_off_hand_for(c.item) {
            return Err(EquipError::PackFull);
        }
        self.cursor = std::mem::replace(&mut self.equipment[slot], Some(c));
        Ok(())
    }

    /// Click a backpack slot with the left button: pick up, put down, swap, or add to a stack.
    pub fn click_bag(&mut self, slot: usize) {
        match (self.cursor, self.bag[slot]) {
            (None, _) => self.cursor = self.bag[slot].take(),
            (Some(c), Some(b)) if c.item == b.item && get(c.item).stack > 1 => {
                let room = get(b.item).stack - b.count;
                let moved = c.count.min(room);
                self.bag[slot] = Some(Stack { count: b.count + moved, ..b });
                self.cursor = (c.count > moved).then_some(Stack { count: c.count - moved, ..c });
            }
            (Some(c), _) => self.cursor = std::mem::replace(&mut self.bag[slot], Some(c)),
        }
        self.tidy();
    }

    /// Right-click an equipment slot: take the item off into the first free backpack slot.
    pub fn unequip(&mut self, slot: usize) -> bool {
        let Some(s) = self.equipment[slot].take() else { return false };
        let free = self.free_slot();
        self.bag[free] = Some(s);
        self.tidy();
        true
    }

    /// Right-click a backpack item: wear it if it can be worn (in a free place of those it fits, else the first), the old item taking its place.
    pub fn equip_from_bag(&mut self, slot: usize, level: u32) -> Result<(), EquipError> {
        let Some(s) = self.bag[slot] else { return Err(EquipError::DoesNotGoThere) };
        let places: Vec<usize> = (0..SLOTS).filter(|&p| get(s.item).wear.fits(p)).collect();
        let Some(&target) = places.iter().find(|&&p| self.equipment[p].is_none()).or(places.first()) else { return Err(EquipError::DoesNotGoThere) };
        self.check_equip(s.item, target, level)?;
        let old = self.equipment[target].replace(s);
        self.bag[slot] = old;
        if !self.clear_off_hand_for(s.item) {
            (self.equipment[target], self.bag[slot]) = (old, Some(s));
            return Err(EquipError::PackFull);
        }
        self.tidy();
        Ok(())
    }

    /// Put a stack in the backpack: onto stacks of its item first, then into free slots. The backpack grows, so this is
    /// always true (the weight, not the room, is what limits what can be carried).
    pub fn add(&mut self, mut stack: Stack) -> bool {
        let max = get(stack.item).stack;
        for b in self.bag.iter_mut().flatten().filter(|b| b.item == stack.item && b.count < max) {
            let moved = (max - b.count).min(stack.count);
            b.count += moved;
            stack.count -= moved;
        }
        while stack.count > 0 {
            let (free, moved) = (self.free_slot(), stack.count.min(max));
            self.bag[free] = Some(Stack { count: moved, ..stack });
            stack.count -= moved;
        }
        self.tidy();
        true
    }

    /// How many of `item` are in the backpack.
    pub fn count(&self, item: usize) -> u32 {
        self.bag.iter().flatten().filter(|s| s.item == item).map(|s| s.count).sum()
    }

    /// Take `count` of `item` out of the backpack (the last stacks first). False, and nothing changed, if there are fewer.
    pub fn remove(&mut self, item: usize, mut count: u32) -> bool {
        if self.count(item) < count {
            return false;
        }
        for slot in self.bag.iter_mut().rev() {
            if let Some(s) = slot.filter(|s| s.item == item) {
                let taken = s.count.min(count);
                count -= taken;
                *slot = (s.count > taken).then_some(Stack { count: s.count - taken, ..s });
            }
        }
        self.tidy();
        true
    }

    /// Buy one of `item` from a merchant at its price: refused (nothing changes) when the purse or the pack is too small.
    pub fn buy(&mut self, item: usize) -> Result<(), Refusal> {
        self.pay(Stack { item, count: get(item).buy_count }, get(item).buy)
    }

    /// Take `stack` for `price` copper (a purchase, or a buyback).
    pub fn pay(&mut self, stack: Stack, price: u32) -> Result<(), Refusal> {
        if self.money < price {
            return Err(Refusal::TooPoor);
        }
        if !self.add(stack) {
            return Err(Refusal::PackFull);
        }
        self.money -= price;
        Ok(())
    }

    /// Sell the stack in backpack slot `slot` to a merchant: what was sold and what it fetched, or `None` when the slot is
    /// empty or the item is worth nothing to a merchant.
    pub fn sell(&mut self, slot: usize) -> Option<(Stack, u32)> {
        let stack = self.bag[slot].filter(|s| get(s.item).sell > 0)?;
        let price = get(stack.item).sell * stack.count;
        self.bag[slot] = None;
        self.money += price;
        self.tidy();
        Some((stack, price))
    }

    /// Put whatever the mouse carries back into the first free backpack slot (when a panel is closed).
    pub fn put_cursor_away(&mut self) {
        if let Some(c) = self.cursor.take() {
            let free = self.free_slot();
            self.bag[free] = Some(c);
            self.tidy();
        }
    }
}


/// What the character is, from what is worn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub level: u32,
    /// Strength, agility, stamina, intellect, spirit.
    pub attributes: [i32; 5],
    pub armor: i32,
    pub attack_power: i32,
    /// Main hand weapon damage range, speed, and what attack power adds to it.
    pub damage: (f32, f32),
    pub speed: f32,
    pub max_health: f32,
    /// Weapon skill: 5 per level.
    pub attack: i32,
    /// What the worn items add to the mana, and to the resistances (holy, fire, nature, frost, shadow, arcane).
    pub mana: i32,
    pub resist: [i32; 6],
}

/// Level 1 human paladin (`levels::attribute_bonus` adds the growth): strength, agility, stamina, intellect, spirit.
const BASE_ATTRIBUTES: [i32; 5] = [22, 20, 22, 20, 21];
/// Attack power per point of damage per second (`ATTACK_POWER_MAGIC_NUMBER`).
pub const ATTACK_POWER_PER_DPS: f32 = 14.0;

impl Stats {
    /// What the character is at this level in this equipment, and with these boosts (the stats, armor and health that elixirs and the like give for a while).
    pub fn of(equipment: &[Option<Stack>; SLOTS], level: u32, boosts: &[i32; 7]) -> Stats {
        let mut attributes = BASE_ATTRIBUTES;
        for (a, bonus) in attributes.iter_mut().zip(crate::levels::attribute_bonus(level)) {
            *a += bonus;
        }
        for (a, boost) in attributes.iter_mut().zip(boosts) {
            *a += boost;
        }
        let (mut armor, mut health, mut mana, mut resist) = (boosts[5], boosts[6], 0, [0; 6]);
        for stack in equipment.iter().flatten() {
            let item = &get(stack.item);
            armor += item.armor;
            (health, mana) = (health + item.health, mana + item.mana);
            resist.iter_mut().zip(item.resist).for_each(|(r, i)| *r += i);
            // What a worn item does with its spells, as far as the game models it: more of a stat, armor or health.
            for effect in item.spells.iter().filter(|s| s.trigger == 1).flat_map(|s| s.effects) {
                match *effect {
                    UseEffect::Boost(stat @ 0..=4, amount, _) => attributes[stat as usize] += amount,
                    UseEffect::Boost(5, amount, _) => armor += amount,
                    UseEffect::Boost(6, amount, _) => health += amount,
                    _ => {}
                }
            }
            for (a, s) in attributes.iter_mut().zip(item.stats) {
                *a += s;
            }
        }
        // Agility adds 2 armor per point; strength adds 2 attack power per point, less 20 for a paladin.
        armor += attributes[1] * 2;
        let attack_power = (attributes[0] * 2 - 20).max(0);
        let (min, max, speed) = equipment[MAIN_HAND].and_then(|s| get(s.item).weapon).map_or((1.0, 2.0, 2.0), |w| (w.min, w.max, w.speed));
        // The first 20 points of stamina give 1 health each, the rest 10 each, on top of the level's base.
        let sta = attributes[2];
        let max_health = (crate::levels::base_health(level) as i32 + sta.min(20) + (sta - 20).max(0) * 10 + health) as f32;
        let bonus = attack_power as f32 / ATTACK_POWER_PER_DPS * speed;
        Stats { level, attributes, armor, attack_power, damage: (min + bonus, max + bonus), speed, max_health, attack: 5 * level as i32, mana, resist }
    }

    /// The share of physical damage armor stops (`armor / (armor + 400 + 85 * level)`).
    pub fn mitigation(&self) -> f32 {
        let armor = self.armor as f32;
        armor / (armor + 400.0 + 85.0 * self.level as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything that can be worn must show on the character where the original shows it: textures on the skin, a model in the hand or on the back,
    /// a helmet or shoulder pads on the body, a cape; and the files of it must be in the game.
    #[test]
    fn every_wearable_item_has_a_look_on_the_character() {
        let Some(mount) = mpq::find_wow_dir().and_then(|d| mpq::Mount::open_wow_dir(d).ok()) else { return eprintln!("skipped: no game data") };
        let mut missing: Vec<String> = Vec::new();
        for item in all().iter().filter(|i| i.wear != Wear::None) {
            // Jewelry, relics, ammunition, wands and what is thrown are not drawn by the original either.
            let unseen = matches!(item.wear, Wear::Neck | Wear::Finger | Wear::Trinket | Wear::Ammo | Wear::Thrown) || (item.wear == Wear::Ranged && (item.class != 2 || item.subclass == 19));
            // The original's tables give these no look at all: the shoulders that are not drawn, the invisible cape, a test item.
            if unseen || ["i3579", "i11930", "i4574"].contains(&item.id) {
                continue;
            }
            let v = item.visual;
            let ok = match item.wear {
                Wear::Head => v.models[0].is_some() && v.textures[0].is_some() && crate::character::Race::ALL.iter().all(|&race| {
                    [crate::character::Sex::Male, crate::character::Sex::Female].iter().all(|&sex| mount.exists(&format!(r"Item\ObjectComponents\Head\{}_{}.m2", v.models[0].unwrap_or(""), crate::worn::race_code_of(race, sex))))
                }),
                // A pad of one shoulder only is the original's own data (Herod's Shoulder).
                Wear::Shoulder => v.models.iter().any(|m| m.is_some_and(|m| mount.exists(&format!(r"Item\ObjectComponents\Shoulder\{m}.m2")))),
                Wear::Back => v.textures[0].is_some_and(|t| mount.exists(&format!(r"Item\ObjectComponents\Cape\{t}.blp"))),
                wear if wear.holds().is_some() => item.held.is_some_and(|(model, skin)| mount.exists(model) && mount.exists(skin)),
                _ => item.look != Look::NONE,
            };
            if !ok {
                missing.push(format!("{} ({:?}, {})", item.name, item.wear, item.id));
            }
        }
        assert!(missing.is_empty(), "{} items can be worn but are not shown on the character: {:?}", missing.len(), &missing[..missing.len().min(12)]);
    }

    /// The original's items are in the game with what its tables say.
    #[test]
    fn the_originals_items_are_there_with_what_its_tables_say() {
        assert!(all().len() > 16_000, "{} items", all().len());
        let sword = get(find("i25"));
        assert_eq!((sword.name, sword.wear, sword.level, sword.weapon.map(|w| (w.min, w.max, w.speed)), sword.sell), ("Worn Shortsword", Wear::MainHand, 1, Some((1.0, 3.0, 1.9)), 7));
        let potion = get(find("i118"));
        assert_eq!((potion.name, potion.spells[0].trigger, potion.spells[0].effects), ("Minor Healing Potion", 0, &[UseEffect::Heal(70, 90)][..]));
        assert_eq!(potion.spells[0].text, "Restores 70 to 90 health.");
        // A helm, a cloak and a robe say where they are worn; a quest item is not sold; a weapon of both hands is that.
        let wears: Vec<Wear> = ["i7922", "i1190", "i1297", "i922"].iter().map(|id| get(find(id)).wear).collect();
        assert_eq!(wears, [Wear::Head, Wear::Back, Wear::Chest, Wear::TwoHand]);
        assert!(get(find("tough_wolf_meat")).quest && get(find("tough_wolf_meat")).sell == 0);
    }

    #[test]
    fn the_starting_kit_gives_the_stats_the_formulas_say() {
        let inv = Inventory::new();
        let s = Stats::of(&inv.equipment, 1, &[0; 7]);
        // Shield armor 25 plus 2 per point of agility (20).
        assert_eq!((s.armor, s.attack_power, s.attributes), (65, 24, [22, 20, 22, 20, 21]));
        // 10 - 14 plus 24 / 14 x 2.6 = 4.46 from attack power.
        assert!((s.damage.0 - 14.457).abs() < 0.01 && (s.damage.1 - 18.457).abs() < 0.01, "{:?}", s.damage);
        // 60 + 20 + 2 x 10.
        assert_eq!(s.max_health, 100.0);
        assert!((s.mitigation() - 65.0 / 550.0).abs() < 1e-4);
        // Without the sword: fists.
        let mut bare = inv.equipment;
        bare[MAIN_HAND] = None;
        let fists = Stats::of(&bare, 1, &[0; 7]).damage;
        assert!((fists.0 - (1.0 + 24.0 / 14.0 * 2.0)).abs() < 1e-3 && (fists.1 - fists.0 - 1.0).abs() < 1e-3, "{fists:?}");
    }

    #[test]
    fn a_higher_level_has_more_of_everything_that_grows() {
        let kit = Inventory::new().equipment;
        let (one, ten) = (Stats::of(&kit, 1, &[0; 7]), Stats::of(&kit, 10, &[0; 7]));
        assert!(ten.attributes.iter().zip(one.attributes).all(|(a, b)| *a > b));
        // Strength and agility feed attack power and armor; stamina and the level's base feed health; skill is 5 a level.
        assert!(ten.attack_power > one.attack_power && ten.armor > one.armor && ten.max_health > one.max_health);
        assert_eq!((one.attack, ten.attack), (5, 50));
        // Armor stops less of the damage of creatures of a higher level (`85 x level` in the divisor).
        let mut same_armor = one;
        same_armor.level = 10;
        assert!(same_armor.mitigation() < one.mitigation());
    }

    #[test]
    fn items_are_added_to_stacks_then_free_slots_and_a_full_pack_grows() {
        let mut inv = Inventory::new();
        // 3 linen in slot 3: 10 more join it.
        assert!(inv.add(Stack { item: find("linen_cloth"), count: 10 }));
        assert_eq!(inv.bag[3], Some(Stack { item: find("linen_cloth"), count: 13 }));
        // 20 more fill that stack (to 20) and the rest starts a new one in the first free slot (6, after the five items of the starting kit and the lockpicks).
        assert!(inv.add(Stack { item: find("linen_cloth"), count: 20 }));
        assert_eq!((inv.bag[3], inv.bag[6]), (Some(Stack { item: find("linen_cloth"), count: 20 }), Some(Stack { item: find("linen_cloth"), count: 13 })));
        // Fill every free slot with non-stacking items: the next one makes a new row instead of being refused.
        while let Some(free) = inv.bag.iter().position(Option::is_none) {
            inv.bag[free] = Some(Stack { item: find("hearthstone"), count: 1 });
        }
        let before = inv.bag.clone();
        assert!(inv.add(Stack { item: find("hatchet"), count: 1 }));
        assert_eq!((inv.bag.len(), inv.bag[before.len()].map(|s| s.item)), (before.len() + 2 * BACKPACK_COLUMNS, Some(find("hatchet"))));
        // A stack with room still takes more.
        assert!(inv.add(Stack { item: find("linen_cloth"), count: 7 }) && inv.bag[6].map(|s| s.count) == Some(20));
        assert_eq!(inv.bag[..6], before[..6]);
        // Taking the items out shrinks the pack back to the size it started with.
        for slot in 0..before.len() + 1 {
            inv.bag[slot] = None;
        }
        inv.tidy();
        assert_eq!(inv.bag.len(), BACKPACK_SIZE);
    }

    #[test]
    fn what_is_carried_weighs_and_slows_only_beyond_the_capacity() {
        let mut inv = Inventory::new();
        let (before, linen) = (inv.weight(), find("linen_cloth"));
        assert!(before > 0.0, "the starting kit weighs something");
        inv.add(Stack { item: linen, count: 20 });
        assert!((inv.weight() - before - 20.0 * get(linen).weight).abs() < 1e-4);
        let capacity = carry_capacity(22);
        assert_eq!(burden_slowdown(capacity, capacity), 0.0);
        assert!((burden_slowdown(capacity * 1.03, capacity) - 0.23).abs() < 1e-4, "a little over is felt at once");
        assert_eq!(burden_slowdown(capacity * 2.0, capacity), 0.75);
        assert_eq!(burden_slowdown(capacity * 100.0, capacity), 0.75);
        assert!(get(find("longsword")).weight > get(linen).weight, "a sword is heavier than a cloth");
    }

    #[test]
    fn items_are_counted_and_taken_from_the_last_stacks_first() {
        let mut inv = Inventory::new();
        let linen = find("linen_cloth");
        inv.bag[8] = Some(Stack { item: linen, count: 20 });
        assert_eq!(inv.count(linen), 23);
        assert!(!inv.remove(linen, 24) && inv.count(linen) == 23, "too few: nothing changes");
        assert!(inv.remove(linen, 21));
        assert_eq!((inv.bag[8], inv.bag[3]), (None, Some(Stack { item: linen, count: 2 })));
        assert!(inv.remove(linen, 2) && inv.bag[3].is_none());
    }

    #[test]
    fn buying_and_selling_move_money_and_items_and_refuse_what_does_not_fit() {
        let mut inv = Inventory::new();
        let boots = find("scuffed_boots");
        assert_eq!(inv.buy(boots), Err(Refusal::TooPoor));
        inv.money = 70;
        assert_eq!(inv.buy(boots), Ok(()));
        assert_eq!(inv.money, 70 - get(boots).buy);
        assert!(inv.bag.iter().flatten().filter(|s| s.item == boots).count() == 2);
        // A full pack makes room (the purse is what limits buying).
        while let Some(free) = inv.bag.iter().position(Option::is_none) {
            inv.bag[free] = Some(Stack { item: find("hearthstone"), count: 1 });
        }
        inv.money = 1000;
        assert_eq!(inv.buy(find("hatchet")), Ok(()));
        inv.money = 1000;
        // Selling pays the whole stack; the hearthstone is worth nothing to a merchant.
        let linen = inv.bag.iter().position(|s| s.is_some_and(|s| s.item == find("linen_cloth"))).unwrap();
        assert_eq!(inv.sell(linen), Some((Stack { item: find("linen_cloth"), count: 3 }, 39)));
        assert_eq!((inv.bag[linen], inv.money), (None, 1039));
        let stone = inv.bag.iter().position(|s| s.is_some_and(|s| s.item == find("hearthstone"))).unwrap();
        assert_eq!(inv.sell(stone), None);
        assert_eq!(inv.sell(linen), None);
        // Buying it back costs what it fetched.
        assert_eq!(inv.pay(Stack { item: find("linen_cloth"), count: 3 }, 39), Ok(()));
        assert_eq!(inv.money, 1000);
    }

    #[test]
    fn items_move_between_the_mouse_the_slots_and_the_pack() {
        let mut inv = Inventory::new();
        // Pick up the sword from the main hand, put it in an empty pack slot, pick up the vest, wear it.
        inv.click_equipment(MAIN_HAND, 60).unwrap();
        assert!(inv.equipment[MAIN_HAND].is_none() && inv.cursor.map(|c| c.item) == Some(find("longsword")));
        inv.click_bag(10);
        assert_eq!((inv.cursor, inv.bag[10].map(|s| s.item)), (None, Some(find("longsword"))));
        inv.click_bag(1);
        // A vest does not go in the main hand; it goes on the chest.
        assert!(inv.click_equipment(MAIN_HAND, 60).is_err());
        assert!(inv.equipment[MAIN_HAND].is_none());
        inv.click_equipment(CHEST, 60).unwrap();
        assert_eq!(inv.equipment[CHEST].map(|s| s.item), Some(find("worn_leather_vest")));
        // Right-click wears from the pack, swapping with what is worn.
        assert!(inv.equip_from_bag(10, 60).is_ok());
        assert_eq!((inv.equipment[MAIN_HAND].map(|s| s.item), inv.bag[10]), (Some(find("longsword")), None));
        assert!(inv.unequip(CHEST));
        assert!(inv.equipment[CHEST].is_none() && inv.bag.iter().flatten().any(|s| s.item == find("worn_leather_vest")));
        // Linen cloth stacks up to its limit.
        inv.click_bag(3);
        assert_eq!(inv.cursor, Some(Stack { item: find("linen_cloth"), count: 3 }));
        inv.click_bag(11);
        inv.click_bag(11);
        inv.click_bag(11);
        assert_eq!(inv.bag[11], Some(Stack { item: find("linen_cloth"), count: 3 }));
        inv.cursor = Some(Stack { item: find("linen_cloth"), count: 19 });
        inv.click_bag(11);
        assert_eq!((inv.bag[11], inv.cursor), (Some(Stack { item: find("linen_cloth"), count: 20 }), Some(Stack { item: find("linen_cloth"), count: 2 })));
        inv.put_cursor_away();
        assert!(inv.cursor.is_none());
    }
}
