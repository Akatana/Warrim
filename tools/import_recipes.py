"""Makes `content/classic_recipes.ron`: what the trades of the original game (1.12.1) make: Alchemy, Blacksmithing, Cooking, Enchanting, Engineering, First Aid,
Leatherworking, Tailoring and the smelting of Mining. A recipe is a spell of the CMaNGOS classic database (spell_template) that a row of `SkillLineAbility.dbc` puts in
the skill line of one of them: the item it makes (or the enchantment it puts on a worn item), the reagents and tools it asks for, the spell focus it has to be done at
(forge, anvil, loom, cooking fire), its cast time and the ranks of the skill at which it is orange, yellow, green and grey (what the skill rises by).

Left out: the spells of the lines that make nothing (the trade's own spell, its ranks), and recipes whose item or reagents are not in the game's item files.

Needs, beside db.sql, SkillLineAbility.dbc and SpellCastTimes.dbc in the same folder (`wowtool extract DBFilesClient\\SkillLineAbility.dbc ...`).

Usage: py -3 tools/import_recipes.py [folder with db.sql (default: tools/data)] [content folder]
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import dbc
import import_items
import import_loot

# The skill lines of the original (SkillLine.dbc) -> the game's skills.
LINES = {164: "Blacksmithing", 165: "Leatherworking", 171: "Alchemy", 185: "Cooking", 129: "FirstAid", 197: "Tailoring", 202: "Engineering", 333: "Enchanting", 186: "Mining"}


def q(text):
    return '"%s"' % text.replace("\\", "\\\\").replace('"', '\\"')


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    sql = os.path.join(folder, "db.sql")
    made = set()
    for name in ("items.ron", "classic_items.ron", "classic_pieces.ron"):
        made |= set(re.findall(r'\bid: "([^"]+)"', open(os.path.join(target, name), encoding="utf8").read()))
    cast = dbc.Dbc(os.path.join(folder, "SpellCastTimes.dbc"))
    times = {cast.u32(i, 0): max(0, cast.i32(i, 1)) for i in range(len(cast))}
    spells = {s["Id"]: s for s in classicdb.rows(sql, "spell_template")}
    abilities = dbc.Dbc(os.path.join(folder, "SkillLineAbility.dbc"))
    out, skipped, seen = [], {"no spell": 0, "makes nothing": 0, "unknown item": 0}, set()
    for i in range(len(abilities)):
        line, spell_id = abilities.u32(i, 1), abilities.u32(i, 2)
        if line not in LINES or spell_id in seen:
            continue
        s = spells.get(spell_id)
        if s is None:
            skipped["no spell"] += 1
            continue
        effects = [n for n in (1, 2, 3) if s["Effect%d" % n] in (24, 53)]
        if not effects or not s["SpellName"]:
            skipped["makes nothing"] += 1
            continue
        n = effects[0]
        reagents = [(s["Reagent%d" % k], s["ReagentCount%d" % k]) for k in range(1, 9) if s["Reagent%d" % k] > 0]
        tools = [s["Totem%d" % k] for k in (1, 2) if s["Totem%d" % k] > 0]
        ids = [import_loot.Loot.name(e) for e in [r[0] for r in reagents] + tools]
        item = ""
        if s["Effect%d" % n] == 24:
            item = import_loot.Loot.name(s["EffectItemType%d" % n])
            ids.append(item)
        if any(x not in made for x in ids):
            skipped["unknown item"] += 1
            continue
        seen.add(spell_id)
        min_rank, learn, high, low = abilities.u32(i, 7), abilities.u32(i, 9), abilities.u32(i, 10), abilities.u32(i, 11)
        # No grey rank given: nothing is ever grey.
        high = high or min_rank + 100
        low = low or min_rank
        low, high = min(low, high), max(low, high)
        green = (high + low) // 2
        fields = ["spell: %d" % spell_id, "name: %s" % q(s["SpellName"]), "skill: %s" % LINES[line], "ranks: (%d, %d, %d, %d)" % (min_rank, low, green, high)]
        if item:
            base, die = s["EffectBasePoints%d" % n] + 1, max(0, s["EffectDieSides%d" % n] - 1)
            fields.append("item: %s" % q(item))
            if base > 1 or die:
                fields.append("count: (%d, %d)" % (base, base + die))
        else:
            fields.append("enchant: %d" % s["EffectMiscValue%d" % n])
            fields.append("worn: (%d, %d, %d)" % (s["EquippedItemClass"], s["EquippedItemSubClassMask"], s["EquippedItemInventoryTypeMask"]))
        if reagents:
            fields.append("reagents: [%s]" % ", ".join("(%s, %d)" % (q(import_loot.Loot.name(r[0])), r[1]) for r in reagents))
        if tools:
            fields.append("tools: [%s]" % ", ".join(q(import_loot.Loot.name(t)) for t in tools))
        if s["RequiresSpellFocus"]:
            fields.append("focus: %d" % s["RequiresSpellFocus"])
        if times.get(s["CastingTimeIndex"]):
            fields.append("cast: %d" % times[s["CastingTimeIndex"]])
        if learn == 1:
            fields.append("auto: true")
        out.append((LINES[line], min_rank, s["SpellName"], "(" + ", ".join(fields) + ")"))
    out.sort(key=lambda r: r[:3])
    header = ("// What the trades of the original game (1.12.1) make, made by tools/import_recipes.py from the CMaNGOS classic database (spell_template) and SkillLineAbility.dbc:\n"
              "// `ranks` are the skill at which a recipe is orange, yellow, green and grey; `item` is what it makes, `enchant` the enchantment (SpellItemEnchantment.dbc) it puts\n"
              "// on a worn item of the kind `worn` (class, subclass mask, inventory type mask), `focus` the SpellFocusObject.dbc row it is done at. Do not edit by hand.\n")
    with open(os.path.join(target, "classic_recipes.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header + "[\n" + "".join("    %s,\n" % r[3] for r in out) + "]\n")
    print("wrote", len(out), "recipes; left out", skipped)


if __name__ == "__main__":
    main()
