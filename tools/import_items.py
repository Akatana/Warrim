"""Makes `content/classic_items.ron`: every item of the original game (1.12.1), from the CMaNGOS classic database.

Where the data comes from (kept in tools/data, which is not shipped; this writes the rest):
  curl -L -o db.sql.gz https://raw.githubusercontent.com/cmangos/classic-db/master/Full_DB/ClassicDB_1_12_1_z2815.sql.gz ; gunzip db.sql.gz
  wowtool extract DBFilesClient\\ItemDisplayInfo.dbc  (and Spell.dbc, SpellDuration.dbc, SpellRadius.dbc) into the same folder

Usage: py -3 tools/import_items.py [folder with db.sql and the .dbc files (default: tools/data)] [content folder]
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import dbc
import spelltext

# Items the game already has under its own ids: the original's entry -> our id (their hand made versions are removed from items.ron).
ALIASES = {6948: "hearthstone", 2589: "linen_cloth", 750: "tough_wolf_meat"}

# The inventory types of the original -> Wear.
WEAR = {1: "Head", 2: "Neck", 3: "Shoulder", 4: "Shirt", 5: "Chest", 6: "Waist", 7: "Legs", 8: "Feet", 9: "Wrist", 10: "Hands", 11: "Finger", 12: "Trinket",
        13: "OneHand", 14: "Shield", 15: "Ranged", 16: "Back", 17: "TwoHand", 19: "Tabard", 20: "Chest", 21: "MainHand", 22: "OffHand",
        23: "Held", 24: "Ammo", 25: "Thrown", 26: "Ranged", 28: "Ranged"}
BIND = {1: "Pickup", 2: "Equip", 3: "Use", 4: "Quest"}
# The sounds of picking an item up and putting it down, by the sound group of its look (`ItemGroupSounds.dbc`).
HANDLING = {7: "Cloth_Leather", 8: "MetalSmall", 9: "MetalLarge", 10: "SmallChain", 11: "LargeChain", 12: "WoodSmall", 13: "WoodLarge", 14: "Ring",
            15: "Meat", 16: "FoodGeneric", 17: "Water_Liquid", 18: "Bag", 19: "Book", 20: "Parchment_Paper", 21: "Wand", 22: "Rocks_Ore", 23: "Herb",
            24: "Gems"}
EXCLUDED = re.compile(r"\(test\)|\[ph\]|\btest\b|^monster - |^deprecated|^unused |^zz|\(old\)|^qa |\(nyi\)|\(dnd\)|\(deprecated\)|^\[", re.I)
KEPT_DESPITE = re.compile(r"high test", re.I)


def q(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n") + '"'


def f(x):
    return repr(float(x))


def entry_text(i, display_rows, spells):
    """One item as RON; what is the default is left out."""
    entry = i["entry"]
    out = ['id: %s' % q(ALIASES.get(entry, "i%d" % entry)), 'name: %s' % q(i["name"]), 'display: %d' % i["displayid"]]
    if i["Quality"] != 1:
        out.append("quality: %d" % i["Quality"])
    out.append("classic: true")
    inv = i["InventoryType"]
    wear = WEAR.get(inv)
    # Bags, quivers and the like are carried, not worn.
    if wear:
        out.append("wear: %s" % wear)
    if i["class"]:
        out.append("class: %d" % i["class"])
    if i["subclass"]:
        out.append("sub: %d" % i["subclass"])
    if i["RequiredLevel"]:
        out.append("level: %d" % i["RequiredLevel"])
    if i["AllowableClass"] > 0 and i["AllowableClass"] & 1535 != 1535:
        out.append("classes: %d" % i["AllowableClass"])
    if i["AllowableRace"] > 0 and i["AllowableRace"] & 255 != 255:
        out.append("races: %d" % i["AllowableRace"])
    if i["bonding"] in BIND:
        out.append("bind: %s" % BIND[i["bonding"]])
    if i["class"] == 12:
        out.append("quest: true")
    if i["maxcount"] == 1:
        out.append("unique: true")
    if i["MaxDurability"]:
        out.append("durability: %d" % i["MaxDurability"])
    if i["ItemLevel"]:
        out.append("ilvl: %d" % i["ItemLevel"])
    if i["DisenchantID"]:
        out.append("disenchant: %d" % i["DisenchantID"])
    if i["armor"]:
        out.append("armor: %d" % i["armor"])
    if i["block"]:
        out.append("block: %d" % i["block"])
    # Stats: strength, agility, stamina, intellect, spirit; and the health and mana an item adds.
    stats = {4: 0, 3: 0, 7: 0, 5: 0, 6: 0}
    health = mana = 0
    for n in range(1, 11):
        t, v = i["stat_type%d" % n], i["stat_value%d" % n]
        if t == 0:
            mana += v
        elif t == 1:
            health += v
        elif t in stats:
            stats[t] += v
    if any(stats.values()):
        out.append("stats: (%d, %d, %d, %d, %d)" % (stats[4], stats[3], stats[7], stats[5], stats[6]))
    if health:
        out.append("health: %d" % health)
    if mana:
        out.append("mana: %d" % mana)
    res = [i["holy_res"], i["fire_res"], i["nature_res"], i["frost_res"], i["shadow_res"], i["arcane_res"]]
    if any(res):
        out.append("resist: (%s)" % ", ".join(str(r) for r in res))
    if i["class"] == 2 and i["dmg_max1"] > 0 and i["delay"] > 0:
        out.append("weapon: Some((min: %s, max: %s, speed: %s))" % (f(i["dmg_min1"]), f(i["dmg_max1"]), f(i["delay"] / 1000.0)))
        more = [(i["dmg_min%d" % n], i["dmg_max%d" % n], i["dmg_type%d" % n]) for n in range(2, 6) if i["dmg_max%d" % n] > 0]
        if more:
            out.append("more: [%s]" % ", ".join("(%s, %s, %d)" % (f(a), f(b), t) for a, b, t in more))
    if i["stackable"] > 1:
        out.append("stack: %d" % i["stackable"])
    if i["BuyCount"] > 1:
        out.append("buy_count: %d" % i["BuyCount"])
    if i["BuyPrice"]:
        out.append("buy: %d" % i["BuyPrice"])
    # A quest item is not sold (the original's tables give some of them a price that nobody can get).
    if i["SellPrice"] and i["class"] != 12:
        out.append("sell: %d" % i["SellPrice"])
    if i["description"]:
        out.append("text: %s" % q(i["description"]))
    # What reading it shows (`classic_pages.ron`), and the material of the page.
    if i["PageText"]:
        out.append("page: %d" % i["PageText"])
        if i["PageMaterial"]:
            out.append("material: %d" % i["PageMaterial"])
    if i["RequiredSkill"]:
        out.append("skill: (%d, %d)" % (i["RequiredSkill"], i["RequiredSkillRank"]))
    if i["ContainerSlots"]:
        out.append("slots: %d" % i["ContainerSlots"])
    if i["Flags"]:
        out.append("flags: %d" % i["Flags"])
    row = display_rows.get(i["displayid"])
    if row is not None and row[0] in HANDLING:
        out.append("handling: %s" % q(HANDLING[row[0]]))
    sp = []
    for n in range(1, 6):
        sid = i["spellid_%d" % n]
        if not sid:
            continue
        text = spells.describe(sid) if spells else ""
        cd = i["spellcooldown_%d" % n]
        extra = ""
        if i["spellcharges_%d" % n]:
            extra += ", charges: %d" % i["spellcharges_%d" % n]
        if cd > 0:
            extra += ", cooldown: %d" % cd
        # Spells that go off on a hit: how often a minute (instead of the spell's own chance).
        if i["spellppmRate_%d" % n] > 0:
            extra += ", ppm: %s" % f(i["spellppmRate_%d" % n])
        effects = spells.effects(sid) if spells and i["spelltrigger_%d" % n] in (0, 1, 5) else []
        if effects:
            extra += ", effects: [%s]" % ", ".join(effects)
        sp.append("(spell: %d, trigger: %d%s, text: %s)" % (sid, i["spelltrigger_%d" % n], extra, q(text)))
    if sp:
        out.append("spells: [%s]" % ", ".join(sp))
    return "(" + ", ".join(out) + ")"


def main():
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "content")
    sql = os.path.join(folder, "db.sql")
    display = dbc.Dbc(os.path.join(folder, "ItemDisplayInfo.dbc"))
    display_rows = {display.u32(r, 0): (display.u32(r, 11),) for r in range(len(display))}
    spells = spelltext.Spells(folder)
    items = [i for i in classicdb.rows(sql, "item_template") if not EXCLUDED.search(i["name"]) or KEPT_DESPITE.search(i["name"])]
    items.sort(key=lambda i: i["entry"])
    lines = [entry_text(i, display_rows, spells) for i in items]
    header = ("// Every item of the original game (1.12.1), made by tools/import_items.py from the CMaNGOS classic database: what the original's tables say\n"
              "// of its name, class, stats, damage, prices, binding, level, spells and lore. Do not edit by hand (the editor writes its changes to editor.ron);\n"
              "// `display` is a row of ItemDisplayInfo.dbc. Ids are `i<entry>`, or the id the game gave the item before this file (see ALIASES in the tool).\n")
    with open(os.path.join(target, "classic_items.ron"), "w", encoding="utf8", newline="\n") as out:
        out.write(header + "[\n" + "".join("    %s,\n" % l for l in lines) + "]\n")
    print("wrote", len(lines), "items")


if __name__ == "__main__":
    main()
