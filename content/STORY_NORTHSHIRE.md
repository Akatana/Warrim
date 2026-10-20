# The Dark Beacon — the story of Northshire Abbey

*An original story for the valley, written for this project. Everything here is in `content/*.ron`; this file is the plan it follows.*

## The place

Northshire is a small valley of meadows and old oaks, folded between hills, on the road from Stormwind to Goldshire. At its head
stands **Northshire Abbey**, a house of the Order of the Light. It is a poor order: a dozen brothers, a cellar of winter stores,
a vineyard, a small garrison of one (Deputy Willem) and a merchant who wanders up from Goldshire (Trader Aldric).

What the abbey is known for is its **Beacon**: a great lantern in the tower, with a lens ground by the abbey's founders, whose
steady light has guided travellers down from the hills for two hundred years. Shepherds set their flocks by it. Merchants
plan their journeys by it. When the Beacon burns, Northshire is safe; so the old saying goes.

## What has gone wrong

Three weeks ago the Beacon went dark.

- **The thief.** *Garrick Thorne* was the abbey's steward for eleven years: he kept the books, the keys and the accounts. He was
  also a gambler, and in debt to the **Redhand**, a gang of cutpurses that squats at the Stone Bridge and robs the Goldshire road.
  Thorne paid his debt with the Beacon's lens, which he unscrewed from the tower on a moonless night, and fled to the Redhand
  camp.
- **The buyer.** The Redhand did not want a lens; they wanted coin. They sold it on the same week to *Foreman Gribble*, the
  boss of the kobolds in **Echo Ridge Mine**, who had seen the glow of a candle in the abbey and has wanted "the big shiny
  candle" ever since. The kobolds (who love nothing as much as light to dig by) have been robbing the abbey of candles for
  months, and now have the greatest light in the valley deep in their tunnels.
- **The consequences.** Without the Beacon the hill wolves, driven down by the kobolds' blasting, come into the valley at night;
  boars root up the vineyard; rats have the cellars; and the Redhand rob travellers on the road in the dark. A valley that was
  safe for two hundred years is suddenly not.

Nobody knows yet that any of this is one story. A newcomer in the valley — the player — will put it together, one chore at a time.

## The valley, and where things are

Coordinates are WoW's (north, west); west grows to the right of the minus sign, so the abbey, at (-8897, -189), is north-east of
the start. Hostile creatures notice the player from 14 yards and all at once, so every group is spread out; nothing hostile stands
within sixty yards of the plaza, and the beasts that the first quests need are a short walk away, in different directions:

- **The plaza before the abbey** (-8949, -132), where the player starts: Deputy Willem and Trader Aldric by the road, Abbot Hollis
  at the abbey door, Brother Corwin by the stalls. Safe ground.
- **Behind the abbey** (-8850, -140): a clearing in the oaks by the cellar door. Brother Tolan, the cellar rats and a Novice chest
  the brothers lost the key to.
- **The Wolf Den** (-8932, -24): the western meadows under the hills, a hundred yards from the plaza. Three wolves and two black
  wolves, and an Adept chest in the den. (Wolf meat always drops; pelts always from the black wolves.)
- **The Abbey Vineyard** (-9085, -340): south-east, down by the stream, where the boars root among the vines.
- **The Stone Bridge** (-8970, -283): east, on the Goldshire road. The Redhand camp, with Garrick Thorne and an Adept hoard.
- **Echo Ridge Mine** (-8702, -124): north, a long walk up the road. Kobold miners in the yard before the mine, Foreman Gribble
  at the door and an Expert stash.
- **Northshire Gate** (-9057, -44): the road out of the valley to the south-west; only a place to discover.

Three places that come with the second half of the story (levels 4 to 6). The coordinates are first guesses on the map; set them
right in the editor (F1):

- **The Hollow Oak Grove** (-8800, -230): a dark stand of old oaks north of the abbey, thick with spiders. Level 3.
- **The Redhand Watchpost** (-8920, -390): a lookout the Redhand built on the Goldshire road, past the vineyard, after Thorne
  fell. Someone else is paying for it now. Levels 4 to 6, with an Adept chest.
- **The Deep Gallery** (-8690, -95): the back galleries of Echo Ridge Mine, behind Gribble's door. The kobolds are not digging for
  light here, they are digging for glass. Levels 5 to 6.

Creatures take five minutes to return (the captains fifteen), so every thing a quest asks for is *guaranteed*: quest items always
drop while the quest is active and the item is still wanted (they drop only then, like in the original), and every quest has at least as many of the creatures it needs as it asks for (five rats for five, six boars for four pieces of meat, six cutpurses for five, six kobolds for six candles; and in the second half seven spiders for five silks, eight lookouts for six,
ten tunnelers for eight).

## Cast

| Who | Where | Role |
|---|---|---|
| **Deputy Willem** (existing) | by the abbey road | The valley's only soldier. Gruff, honest, out of his depth. Gives the wolf and bandit quests. |
| **Trader Aldric** (existing) | beside him | Sells wares and complains about the wolves, who ruined his trade in pelts. |
| **Abbot Hollis** | the abbey's front | Old, kind and ashamed: the lens was stolen on his watch. He gives the Beacon quests and the last reward. |
| **Brother Tolan** | behind the abbey | Keeps the cellar and the vineyard. Furious with rats and boars. |
| **Brother Corwin** | by the stalls on the plaza | The brothers' healer. Wants meat for broth: the brothers have eaten bread alone for weeks. |
| **Garrick Thorne** | the Redhand camp | The traitor steward. Cowardly but cornered. |
| **Foreman Gribble** | the mouth of Echo Ridge Mine | The kobold who bought the lens. Not evil, only greedy, and now has no idea what to do with a lamp like that. |
| **Clerk Pell Dunmore** | the Redhand Watchpost | A Stormwind clerk in a clean coat, who has come to collect for the people who now own the Redhand's debts. Polite, frightened, and a bad liar. |
| **Skorn the Geomancer** | the Deep Gallery | A kobold who has learned that glass can be dug, and who is paid in iron tools to dig more of it. |
| **Old Tusk** and **Old Greymane** | the vineyard, the Wolf Den | The valley's two old beasts: a boar and a wolf, each the terror of its corner. Not evil, only large. |


Who is in the creature tables: see [Creatures](#creatures) at the end of this file, with their ids.

## The chain of quests

Quests marked ☆ are the main line; the others are the valley's own troubles, which can be taken in any order.

1. **Wolves in the Valley** (Willem) — the first lesson of the sword. *(existing)*
2. **A Word with the Trader** (Willem → Aldric) *(existing)*
3. **Pelts for Aldric** (Aldric) *(existing)*
4. **Rats in the Cellar** (Brother Tolan) — five rats, to clear the cellar steps. Introduces the abbey.
5. **Broth for the Brothers** (Brother Corwin) — four pieces of boar meat, from the boars that have been ruining the vineyard.
   The reward is a choice of warm things for the road: a tunic or boots.
6. ☆ **The Dark Beacon** (Abbot Hollis → Deputy Willem) — Hollis tells the newcomer about the Beacon; someone must ask the
   deputy who has been on the road at night. (Needs: Rats in the Cellar.)
7. ☆ **The Redhand at the Bridge** (Willem) — Willem knows the Redhand and cannot leave his post: five cutpurses must go.
8. ☆ **Garrick Thorne** (Willem) — the ringleader of the camp is the abbey's missing steward. Kill him and bring back what he
   carries: a torn page of the ledger which tells who bought the lens.
9. ☆ **Candles in the Dark** (Abbot Hollis) — the ledger names the kobolds of Echo Ridge. They have been stealing the abbey's
   candles too; get six of them back so that the brothers can hold the evening office, and learn what the kobolds are doing.
10. ☆ **The Lens Below** (Abbot Hollis) — kill Foreman Gribble and bring back the Beacon Lens. The Abbot relights the Beacon;
    the reward is a choice of the abbey's gifts: a mace, a buckler or the Beaconkeeper's Tunic.

### The second half: the valley's own troubles, and the first sight of the Ledger (levels 4 to 6)

The first ten quests end at level 4. These add the rest of the way to level 6 and tie the valley to the world story in
[STORY_WORLD.md](STORY_WORLD.md). Three are side quests that can be taken as soon as the quest they follow is done (their place
in the chain is only the earliest); the other four are a chain after the Beacon is relit.

11. **Silk for the Healer** (Brother Corwin) — side; follows *Broth for the Brothers*. The brothers need bandages and Corwin has no
    cloth. Five spider silks from the oak spiders of the Hollow Oak Grove. Reward: bandages.
12. **The Tyrant of the Vines** (Brother Tolan) — side; follows *Rats in the Cellar*. Old Tusk, a boar the size of a cart, has been
    rooting up the best vines. Kill him.
13. **The Alpha of the Den** (Deputy Willem) — side; follows *Pelts for Aldric*. The pack in the Wolf Den has a leader, Old
    Greymane, who led the wolves into the valley. Kill him and bring Willem a fang as proof.
14. ☆ **New Hands at the Bridge** (Willem) — follows *The Lens Below*. The Beacon burns again and Thorne is dead, but the Redhand have
    a new watchpost on the road, better built than the old camp, with better boots. Someone is paying. Kill six lookouts.
15. ☆ **The Scale and the Flame** (Willem → Abbot Hollis) — follows 14. At the head of the watchpost stands Clerk Pell Dunmore, a
    man who does not belong there, with two enforcers. Kill him and bring the Abbot the brass seal he carries: a balance scale
    holding a flame. The Abbot knows it at once and closes his eyes: it is the seal on the Torn Ledger Page. He opens the
    **Compact's book**, tells the player of the Lamp Truce and the twelve lamps (see [STORY_WORLD.md](STORY_WORLD.md)), and gives
    the **Lamplighter's Lantern**: a key item that is the player's proof of the Truce from here on. Reward: the Lantern, and the
    most experience of any quest in the valley.
16. ☆ **Glass in the Dark** (Hollis) — follows 15. The Abbot guesses what the kobolds' blasting hides: the vein of starglass that
    the founders once cut the lens from. Someone is digging it again. Kill eight tunnelers in the Deep Gallery and bring five
    starglass shards.
17. ☆ **Skorn the Geomancer** (Hollis) — follows 16. The tunnelers answer to Skorn, who sends the blanks up the mine to a cart on
    the road. Kill him and bring the **Ledger Writ** he carries: an order for glass, to be delivered to a warehouse in Stormwind.
    Hollis reads it twice. *"This was never about one lens."* The valley is safe; the road to Elwynn Forest, and the next lamp,
    begins here. Reward: a choice of a weapon or a piece of armor for level 6.

## What the player finds on the way

- Places to discover (compass): **Wolf Den**, **Northshire Abbey**, **Stone Bridge**, **Northshire Gate**, **Echo Ridge Mine**, **Abbey Vineyard**,
  and in the second half **Hollow Oak Grove**, **Redhand Watchpost** and **The Deep Gallery** (ids `hollow_oak_grove`, `redhand_watchpost`, `deep_gallery`).
- Five chests: behind the abbey (Novice), in the Wolf Den (Adept), at the Redhand camp (Adept), in the mine (Expert) and in the
  Watchpost (Adept, id `watchpost_cache`, loot table `chest_watchpost`).
- Gear: padded tunic and pilgrim boots (broth quest), Thorne's dagger (ledger quest), the abbey's gifts (the finale), bandages
  (silk quest), a level 6 weapon or armor of the player's choice (the last quest). Anything new that is worn must also be drawn on
  the character; take the pieces from `classic_pieces.ron`, which are.
- Quest items (all `quest: true`, and dropped only while the quest wants them): `spider_silk`, `greymane_fang`, `brass_seal`,
  `starglass_shard`, `ledger_writ`, and the key item `lamplighter_lantern` (given, not dropped, and not worn, so nothing to draw).

## Levels: 1 to 6

Measured against the game's own experience table (`levels.rs`) and kill formula, with the quests done in the order below and
nothing killed twice. The first ten quests alone end at level 4 with 756 of 2100 experience, short of the 1 to 6 this valley is meant
to cover; the second half closes it.

| After | Level |
|---|---|
| Wolves, A Word, Pelts | 2 |
| Rats, Broth, The Dark Beacon | 3 |
| Silk, Tyrant, Alpha (side quests) | 3 (1238 of 1400) |
| The Redhand at the Bridge | 4 |
| Garrick Thorne, Candles | 4 |
| The Lens Below | 4 (1848 of 2100) |
| New Hands at the Bridge | 5 |
| The Scale and the Flame | 5 |
| Glass in the Dark | 5 (2470 of 2800) |
| Skorn the Geomancer | **6** (298 of 3600) |

The margin is thin on purpose: level 6 is reached with the last quest, and the kills on the way (a rat that was not needed, the
second black wolf) are a small cushion. Rewards and creature levels are chosen to give this; if either changes, run the sum again.

## Creatures

The enemies of the valley, with their ids (as in `creatures.ron`; the people are listed after). `hp` and `bite` are the
numbers of the file; level 1 is the default and is left out of the file. Creatures that stand in the world are in
`spawns.ron` and are matched by their place in the spawn list, so a creature added in the middle of it moves the saved
creatures after it (see `save.rs`): add new creatures to the end.

**In the game now:**

| id | Name | Lv | hp | bite | Where | Count | Respawn | Drops (`loot.ron`) |
|---|---|---|---|---|---|---|---|---|
| `wolf` | Wolf | 1 | 50 | 8 | Wolf Den | 3 | 5 min | meat always (`c_wolf`), a tome by chance |
| `black_wolf` | Black Wolf | 3 | 95 | 12 | Wolf Den | 2 | 5 min | pelts always (`c_black_wolf`) |
| `cellar_rat` | Cellar Rat | 1 | 22 | 3 | behind the abbey | 5 | 5 min | 1 to 4 copper (`rat`) |
| `valley_boar` | Valley Boar | 2 | 70 | 8 | Abbey Vineyard | 6 | 5 min | boar meat always (`boar`) |
| `redhand_cutpurse` | Redhand Cutpurse | 3 | 80 | 9 | Stone Bridge | 6 | 5 min | lockpicks (`cutpurse`) |
| `garrick_thorne` | Garrick Thorne, Redhand Captain | 5 | 150 | 12 | Stone Bridge | 1 | 15 min | the torn page always (`garrick`) |
| `kobold_miner` | Kobold Miner | 3 | 58 | 7 | Echo Ridge Mine yard | 6 | 5 min | abbey candles always (`kobold`) |
| `foreman_gribble` | Foreman Gribble, Kobold Foreman | 5 | 170 | 13 | mine door | 1 | 15 min | the Beacon Lens always (`gribble`) |

**To add (second half):**

| id | Name | Lv | hp | bite | Where | Count | Respawn | Drops (new `loot.ron` tables) | Quest |
|---|---|---|---|---|---|---|---|---|---|
| `oak_spider` | Oak Spider | 3 | 60 | 6 | Hollow Oak Grove | 7 | 5 min | `spider_silk` always (`spider`) | 11 |
| `old_tusk` | Old Tusk | 4 | 140 | 11 | Abbey Vineyard, among the boars | 1 | 15 min | boar meat (`old_tusk`) | 12 |
| `old_greymane` | Old Greymane | 4 | 160 | 12 | Wolf Den, behind the pack | 1 | 15 min | `greymane_fang` always (`greymane`) | 13 |
| `redhand_lookout` | Redhand Lookout | 4 | 105 | 10 | Watchpost | 8 | 5 min | lockpicks, a few coppers (`lookout`) | 14 |
| `redhand_enforcer` | Redhand Enforcer | 5 | 130 | 12 | Watchpost, at the clerk | 2 | 5 min | lockpicks (`enforcer`) | 15 |
| `clerk_pell` | Clerk Pell Dunmore, Ledger Clerk | 6 | 190 | 13 | Watchpost, in the tent | 1 | 15 min | `brass_seal` always (`pell`) | 15 |
| `kobold_tunneler` | Kobold Tunneler | 5 | 120 | 10 | Deep Gallery | 10 | 5 min | `starglass_shard` always, a candle by chance (`tunneler`) | 16 |
| `skorn_geomancer` | Skorn the Geomancer, Kobold Digger | 6 | 230 | 14 | Deep Gallery, at the end | 1 | 15 min | `ledger_writ` always (`skorn`) | 17 |

Looks: `oak_spider` takes the model of the original's Forest Spider (`c30`); the old wolf and boar use the wolf and boar models, larger
(scale 1.3 to 1.4) with darker skins; the Redhand use the cutpurse's look (the clerk, the blue jerkin of Thorne without the weapons);
the tunneler and Skorn use the kobold model, Skorn at scale 1.3 with the albino skin like Gribble, but with a different outfit.
Quest items are `quest: true` and drop only while a quest wants them, like the abbey candles and the lens.

**The people** (ids in `creatures.ron`): `deputy_willem`, `trader_aldric`, `instructor_hale`, `abbot_hollis`, `brother_tolan`, `brother_corwin`.

## Tone

Small and warm, with humour: the kobolds are greedy, not cruel; the villain is a coward; nobody is a chosen one. The only thing
at stake is whether the lamp on the hill is lit.
