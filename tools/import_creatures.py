"""Makes `content/classic_creatures.ron` and `content/classic_creature_loot.ron`: every creature of the original game (1.12.1) from the CMaNGOS classic
database, WITHOUT where they stand (no spawns), and the drop tables they use.

What is taken: name, title, level, health and melee damage of the tables, whether the creature attacks a human player (its FactionTemplate), its look
(the first of its four displays the game can draw: an animal or monster is a model with its skins; a person is the race, sex and looks of
CreatureDisplayInfoExtra dressed in the items of its equipment and of the extra's pieces), what it drops and, for a friendly merchant, what it sells.
Not taken: gossip texts, quests, trainers, abilities, spawns and waypoints.

Needs, beside db.sql, the client tables CreatureDisplayInfo, CreatureDisplayInfoExtra, CreatureModelData, FactionTemplate and ItemDisplayInfo, and files.txt
(`wowtool list "" > files.txt`: the game's file names) in the same folder; see the head of import_items.py for the rest.

Usage: py -3 tools/import_creatures.py [folder with db.sql and the tables (default: tools/data)] [content folder]
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import dbc
import import_items
import import_loot
from import_items import q

RACES = {1: "Human", 2: "Orc", 3: "Dwarf", 4: "NightElf", 5: "Undead", 6: "Tauren", 7: "Gnome", 8: "Troll", 9: "Goblin"}
# Creatures that are not creatures: markers, triggers, test dummies, unused entries.
NOT_REAL = re.compile(r"\bdnd\b|^npc |invisible|waypoint|trigger|\(visual\)|\bunused\b|\bdummy\b|\bplaceholder\b|\bbunny\b|^spirit of |^\w+ visual$", re.I)
# The pieces of a person's outfit in CreatureDisplayInfoExtra (fields 8-17): where each is worn.
PIECES = {8: "Head", 9: "Shoulder", 10: "Shirt", 11: "Chest", 12: "Waist", 13: "Legs", 14: "Feet", 15: "Wrist", 16: "Hands", 17: "Tabard"}
RACE_CODES = [r + s for r in ("Hu", "Or", "Dw", "Ni", "Sc", "Ta", "Gn", "Tr") for s in "MF"]
# The skin textures of a creature model are slots 11, 12, 13.
FIRST_SKIN = 11
PLAYER_FACTION, PLAYER_GROUP = 1, 3
# The same for an orc, who is on the Horde's side: its faction template and the group of the player and the Horde.
HORDE_FACTION, HORDE_GROUP = 2, 5
# The least chance (percent) of a drop that is kept. The tables of the original reach the thousand pieces of random armor and weapons through references,
# each at 0.01-0.2%; they would make the file 14 MB. shortcut: those long tails are cut, lower it for the full tables.
LEAST_CHANCE = 0.2


class Tables:
    def __init__(self, folder):
        self.files = {l.strip().lower() for l in open(os.path.join(folder, "files.txt"), encoding="utf8", errors="replace")}
        d = lambda name: dbc.Dbc(os.path.join(folder, name + ".dbc"))
        self.display, self.model, self.extra, self.factions = d("CreatureDisplayInfo"), d("CreatureModelData"), d("CreatureDisplayInfoExtra"), d("FactionTemplate")
        self.display_row = {self.display.u32(r, 0): r for r in range(len(self.display))}
        self.model_row = {self.model.u32(r, 0): r for r in range(len(self.model))}
        self.extra_row = {self.extra.u32(r, 0): r for r in range(len(self.extra))}
        self.items = d("ItemDisplayInfo")
        self.item_row = {self.items.u32(r, 0): r for r in range(len(self.items))}
        self.faction_row = {self.factions.u32(r, 0): r for r in range(len(self.factions))}
        # CharSections: 1 race, 2 sex, 3 kind (0 skin, 1 face, 2 facial hair, 3 hair, 4 underwear), 4 variation, 5 colour, 6 texture.
        sections = d("CharSections")
        self.sections = {(sections.u32(r, 1), sections.u32(r, 2), sections.u32(r, 3), sections.u32(r, 4), sections.u32(r, 5)) for r in range(len(sections)) if sections.string(r, 6)}
        hair = d("CharHairGeosets")
        self.hairs = {(hair.u32(r, 1), hair.u32(r, 2), hair.u32(r, 3)) for r in range(len(hair)) if hair.u32(r, 4)}

    def drawn(self, slot, display):
        """Whether the game can show a piece of this display worn in the slot (the conditions of the test `every_wearable_item_has_a_look_on_the_character`)."""
        r = self.item_row.get(display)
        if r is None:
            return False
        i = self.items
        stem = lambda n: i.string(r, n).rsplit(".", 1)[0] if i.string(r, n) else ""
        exists = lambda path: path.lower() in self.files
        if slot == "Head":
            return bool(stem(1) and i.string(r, 3)) and all(exists(r"Item\ObjectComponents\Head\%s_%s.m2" % (stem(1), code)) for code in RACE_CODES)
        if slot == "Shoulder":
            return any(stem(n) and exists(r"Item\ObjectComponents\Shoulder\%s.m2" % stem(n)) for n in (1, 2))
        return any(i.string(r, n) for n in range(14, 22))

    def looks(self, race, sex, skin, face, style, color, beard):
        """The look the game can make that is nearest to the one the extra names (what it has no texture or mesh for is left out)."""
        has = lambda *key: (race, sex) + key in self.sections
        # The goblins have skins and no faces, hair or beards: the rest is the picture painted for the person.
        if not any(k[0] == race and k[2] == 1 for k in self.sections):
            return (skin if has(0, 0, skin) else 0), 0, 0, 0, 0
        faces = lambda skin: [f for f in [face] + list(range(12)) if has(1, f, skin)]
        skin = skin if has(0, 0, skin) and faces(skin) else 0
        face = faces(skin)[0]
        if style and not (has(3, style, color) and (race, sex, style) in self.hairs):
            style = 0
        if beard and not has(2, beard, color):
            beard = 0
        return min(skin, 9), min(face, 11), min(style, 11), min(color, 9), min(beard, 8)

    def hostile(self, template, player=(PLAYER_FACTION, PLAYER_GROUP)):
        """Whether a creature of this FactionTemplate attacks a human player (as MaNGOS decides it: the lists first, then the groups); `player` is another one's."""
        r = self.faction_row.get(template)
        if r is None:
            return False
        f = self.factions
        enemies = [f.u32(r, n) for n in range(6, 10)]
        friends = [f.u32(r, n) for n in range(10, 14)]
        if player[0] in enemies:
            return True
        if player[0] in friends:
            return False
        if f.u32(r, 5) & player[1]:
            return True
        return False

    def neutral(self, template, player=(PLAYER_FACTION, PLAYER_GROUP)):
        """Whether a creature of this FactionTemplate neither attacks a human player nor is his friend: the yellow ones (it fights back when hit)."""
        r = self.faction_row.get(template)
        if r is None or self.hostile(template, player):
            return False
        f = self.factions
        return not (player[0] in [f.u32(r, n) for n in range(10, 14)] or f.u32(r, 4) & player[1])

    def body(self, display, scale, piece, held):
        """The RON of the look of display row `display`, or None when the game cannot draw it."""
        r = self.display_row.get(display)
        if r is None:
            return None
        d = self.display
        m = self.model_row.get(d.u32(r, 1))
        if m is None:
            return None
        path = self.model.string(m, 2)
        stem = path.rsplit(".", 1)[0]
        model = stem + ".m2"
        if re.search(r"invisible|dummy", model, re.I):
            return None
        extra = d.u32(r, 3)
        # The size is the creature's own scale, or the display's when it has none (the server does that: CMaNGOS makes the object scale the display's when the template has 0), times the model's.
        size = (scale or d.f32(r, 4)) * self.model.f32(m, 4)
        if model.lower().startswith("character\\"):
            e = self.extra_row.get(extra)
            if e is None or self.extra.u32(e, 1) not in RACES:
                return None
            x = self.extra
            # The original paints most people's face and clothes into one picture (the extra's bake name); it replaces the skin when the game has it.
            bake = x.string(e, 18)
            baked = bake and (r"textures\bakednpctextures" + "\\" + bake).lower() in self.files
            look = "(race: %s, sex: %s, skin: %d, face: %d, hair_style: %d, hair_color: %d, beard: %d" % (
                (RACES[x.u32(e, 1)], "Female" if x.u32(e, 2) else "Male") + self.looks(x.u32(e, 1), x.u32(e, 2), *[x.u32(e, n) for n in range(3, 8)]))
            look += (", baked: %d)" % x.u32(e, 0)) if baked else ")"
            outfit = held + [piece(PIECES[n], x.u32(e, n)) for n in range(8, 18) if x.u32(e, n)]
            return "Humanoid(outfit: [%s], look: %s)" % (", ".join(q(o) for o in outfit if o), look)
        if not model.lower().startswith("creature\\") or model.lower() not in self.files:
            return None
        folder = model.rsplit("\\", 1)[0]
        skins = []
        for n in range(3):
            name = d.string(r, 6 + n)
            if name:
                skins.append((FIRST_SKIN + n, "%s\\%s.blp" % (folder, name)))
        if any(f.lower() not in self.files for _, f in skins):
            return None
        return "Beast(model: %s, skins: [%s], scale: %s)" % (q(model), ", ".join("(%d, %s)" % (slot, q(f)) for slot, f in skins), import_items.f(round(size, 3)))


def main():
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "content")
    sql = os.path.join(folder, "db.sql")
    tables = Tables(folder)
    loot = import_loot.Loot(sql)
    pieces = {}

    def piece(slot, display):
        """The id of an item that is a piece of a person's outfit (a row of ItemDisplayInfo.dbc worn in a slot), made for it: most of what the original's
        people wear is no item of the original's. None when the game cannot show it."""
        if not tables.drawn(slot, display):
            return None
        name = "p%s%d" % (slot, display)
        pieces[name] = '(id: %s, name: %s, display: %d, classic: true, wear: %s)' % (q(name), q("%s piece %d" % (slot, display)), display, slot)
        return name

    # The weapons people hold are the original's "Monster - ..." items, which the item import leaves out: each is made as a piece, like the armor.
    raw = {i["entry"]: i for i in classicdb.rows(sql, "item_template")}

    def hand(entry):
        """The id of the item held: the item itself where the game has it, else a piece made for it."""
        if entry in loot.items:
            return loot.name(entry)
        i = raw[entry]
        name = "w%d" % entry
        pieces[name] = '(id: %s, name: %s, display: %d, classic: true, wear: %s, class: %d, sub: %d)' % (
            q(name), q(i["name"]), i["displayid"], import_items.WEAR[i["InventoryType"]], i["class"], i["subclass"])
        return name

    c_loot, equip, wares = {}, {}, {}
    for r in classicdb.rows(sql, "creature_loot_template"):
        c_loot.setdefault(r["entry"], []).append(r)
    for r in classicdb.rows(sql, "creature_equip_template"):
        equip[r["entry"]] = [r["equipentry%d" % n] for n in (1, 2, 3)]
    for table in ("npc_vendor", "npc_vendor_template"):
        for r in classicdb.rows(sql, table):
            if r["item"] in loot.items and loot.items[r["item"]]["BuyPrice"] > 0:
                wares.setdefault((table, r["entry"]), []).append(r["item"])
    tables_made = {}
    lines, counts = [], {"beast": 0, "person": 0, "vendor": 0}
    for c in sorted(classicdb.rows(sql, "creature_template"), key=lambda c: c["Entry"]):
        if import_items.EXCLUDED.search(c["Name"]) or NOT_REAL.search(c["Name"]) or NOT_REAL.search(c["SubName"] or "") or c["UnitFlags"] & 2:
            continue
        # What a person holds comes from the equipment template, the rest of the outfit from the pieces of his display.
        held = [hand(e) for e in equip.get(c["EquipmentTemplateId"], []) if e in raw and import_items.WEAR.get(raw[e]["InventoryType"])]
        body = None
        for n in range(1, 5):
            body = c["ModelId%d" % n] and tables.body(c["ModelId%d" % n], c["Scale"], piece, held)
            if body:
                break
        if not body:
            continue
        counts["person" if body.startswith("Humanoid") else "beast"] += 1
        hostile = tables.hostile(c["Faction"])
        neutral = tables.neutral(c["Faction"])
        out = ["id: %s" % q("c%d" % c["Entry"]), "name: %s" % q(c["Name"])]
        if c["SubName"]:
            out.append("title: Some(%s)" % q(c["SubName"]))
        level = max(1, (c["MinLevel"] + c["MaxLevel"]) // 2)
        if level != 1:
            out.append("level: %d" % level)
        # The creature type (1 beast, 2 dragonkin, 3 demon, 4 elemental, 5 giant, 6 undead, 7 humanoid, 8 critter, 9 mechanical): what the gear's bonuses "versus" name.
        if c["CreatureType"] in range(1, 10):
            out.append("kind: %d" % c["CreatureType"])
        out.append("body: %s" % body)
        if hostile:
            out.append("hostile: true")
        elif neutral:
            out.append("neutral: true")
        # How the Horde's side sees it, where that is another thing.
        horde = (tables.hostile(c["Faction"], (HORDE_FACTION, HORDE_GROUP)), tables.neutral(c["Faction"], (HORDE_FACTION, HORDE_GROUP)))
        if horde != (hostile, neutral):
            out.append("horde: Some((%s, %s))" % (str(horde[0]).lower(), str(horde[1]).lower()))
        out.append("health: %s" % import_items.f(max(1, (c["MinLevelHealth"] + c["MaxLevelHealth"]) / 2.0)))
        # A swing is the weapon's damage (times the multiplier) and what its attack power adds (a fourteenth of it, a second long); the game rolls it within the
        # weapon's range, so what is kept is the middle of it and how far the range reaches, as a share.
        mult = c["DamageMultiplier"] or 1
        damage = (c["MinMeleeDmg"] + c["MaxMeleeDmg"]) / 2.0 * mult + c["MeleeAttackPower"] / 14.0 * (c["MeleeBaseAttackTime"] or 2000) / 1000.0
        if damage > 0:
            out.append("melee_damage: %s" % import_items.f(round(damage, 1)))
            spread = (c["MaxMeleeDmg"] - c["MinMeleeDmg"]) / 2.0 * mult / damage
            if spread >= 0.005:
                out.append("melee_spread: %s" % import_items.f(round(min(spread, 1.0), 2)))
        rows = c_loot.get(c["LootId"], []) if c["LootId"] else []
        gold = (c["MinLootGold"], c["MaxLootGold"])
        if rows or any(gold):
            # The table of a loot id, and of its gold when that differs.
            name = "c%d" % c["LootId"] + ("g%d_%d" % gold if any(gold) else "")
            if name not in tables_made:
                tables_made[name] = loot.table(rows, gold, LEAST_CHANCE)
            if tables_made[name] != "(copper: (0, 0), items: [])":
                out.append("loot: Some(%s)" % q(name))
        stock = []
        if not hostile and not neutral:
            for key in (("npc_vendor", c["Entry"]), ("npc_vendor_template", c["VendorTemplateId"])):
                stock += [e for e in wares.get(key, []) if e not in stock]
        if stock:
            out.append('gossip: Some((greeting: "Greetings, $N. Have a look at my wares.", options: [(text: "I would like to browse your goods.", action: Vendor)]))')
            out.append("vendor: [%s]" % ", ".join(q(loot.name(e)) for e in stock))
            counts["vendor"] += 1
        lines.append("(" + ", ".join(out) + ")")
    names = sorted(tables_made)
    with open(os.path.join(target, "classic_pieces.ron"), "w", encoding="utf8", newline="\n") as out:
        out.write("// What the original's people wear that is no item of its tables: a piece for each look (a row of ItemDisplayInfo.dbc) worn in a slot, which the people of\n"
                  "// classic_creatures.ron are dressed in. Made by tools/import_creatures.py. Do not edit by hand.\n[\n" + "".join("    %s,\n" % pieces[n] for n in sorted(pieces)) + "]\n")
    header = ("// Every creature of the original game (1.12.1) that the game can draw, made by tools/import_creatures.py from the CMaNGOS classic database: its name,\n"
              "// level, health, damage, whether it attacks a human, its look, its drops (`classic_creature_loot.ron`) and what a friendly merchant sells. Where they\n"
              "// stand is not in here. Ids are `c<entry>`. Do not edit by hand (the editor writes its changes to editor.ron).\n")
    with open(os.path.join(target, "classic_creatures.ron"), "w", encoding="utf8", newline="\n") as out:
        out.write(header + "[\n" + "".join("    %s,\n" % l for l in lines) + "]\n")
    header = ("// What the creatures of `classic_creatures.ron` drop: the original's loot tables (one for each loot id and gold, named `c<id>`, or `c<id>g<least>_<most>` with gold; the long tails of rare random items are cut), flattened as in classic_loot.ron.\n"
              "// Made by tools/import_creatures.py. Do not edit by hand.\n")
    with open(os.path.join(target, "classic_creature_loot.ron"), "w", encoding="utf8", newline="\n") as out:
        out.write(header + "{\n" + "".join('    "%s": %s,\n' % (n, tables_made[n]) for n in names if tables_made[n] != "(copper: (0, 0), items: [])") + "}\n")
    print("wrote", len(lines), "creatures:", counts, len(pieces), "pieces,", len(names), "tables")


if __name__ == "__main__":
    main()
