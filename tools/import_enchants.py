"""Makes `content/classic_enchants.ron`: what the enchantments that the recipes of Enchanting put on worn items give (`SpellItemEnchantment.dbc`), as the effects the game
models for worn items (`UseEffect`): a stat, armor, a resistance, weapon damage, and what the spell that an enchantment names does (its auras, read as
`spelltext.py` reads those of items). What the game cannot model (the blows that cast a spell: Fiery Weapon, Crusader) is kept with its name and no effect.

Usage: py -3 tools/import_enchants.py [folder with the .dbc files (default: tools/data)] [content folder]
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import dbc
import spelltext


def q(text):
    return '"%s"' % text.replace("\\", "\\\\").replace('"', '\\"')


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    wanted = {int(x) for x in re.findall(r"enchant: (\d+)", open(os.path.join(target, "classic_recipes.ron"), encoding="utf8").read())}
    # And those that items put on the weapon in the hand for a while (lures, sharpening stones).
    wanted |= {int(x) for x in re.findall(r"Temp\((\d+),", open(os.path.join(target, "classic_items.ron"), encoding="utf8").read())}
    # And those that spells of the classes put on it (a shaman's weapons, a rogue's poisons): the effect 54 of a spell names the enchantment.
    wanted |= {int(x) for x in re.findall(r"\(kind: 54,[^)]*?misc: (\d+)\)", open(os.path.join(target, "classic_spells.ron"), encoding="utf8").read())}
    table = dbc.Dbc(os.path.join(folder, "SpellItemEnchantment.dbc"))
    spells = spelltext.Spells(folder)
    ppm = {r["entry"]: r["ppmRate"] for r in classicdb.rows(os.path.join(folder, "db.sql"), "spell_proc_item_enchant")}
    out = []
    for i in range(len(table)):
        enchant = table.u32(i, 0)
        if enchant not in wanted:
            continue
        effects, procs, worn = [], [], []
        for n in range(3):
            kind, amount, arg = table.u32(i, 1 + n), table.i32(i, 4 + n), table.u32(i, 10 + n)
            if kind == 2 and amount:
                # Weapon damage: the game's own number for it (see `Gear::add`).
                effects.append("Equip(1000, 0, %d)" % amount)
            elif kind == 4 and amount:
                # Resistance: school 0 is armor.
                effects.append("Boost(5, %d, 0.0)" % amount if arg == 0 else "Equip(22, %d, %d)" % (1 << arg, amount))
            elif kind == 5 and amount and arg < 5:
                effects.append("Boost(%d, %d, 0.0)" % (arg, amount))
            elif kind == 1 and arg:
                # A spell that goes off on a blow: by blows a minute if the original's table says, else by the percent of the enchantment, else by the weapon's own.
                procs.append("(%d, %s, %s)" % (arg, repr(float(ppm.get(arg, 0))), repr(float(amount))))
            elif kind == 3 and arg:
                effects.extend(spells.effects(arg))
                # A worn spell that goes off when the wearer is struck (the absorptions of shields) is kept by its number.
                row = spells.rows.get(arg)
                if row is not None and any(spells.spell.u32(row, spelltext.AURA + m) == 42 for m in range(3)):
                    worn.append(str(arg))
        out.append('    %d: (name: %s, effects: [%s]%s%s%s),' % (enchant, q(table.string(i, 13)), ", ".join(effects), ", procs: [%s]" % ", ".join(procs) if procs else "", ", spells: [%s]" % ", ".join(worn) if worn else "", ", visual: %d" % table.u32(i, 22) if table.u32(i, 22) else ""))
    header = ("// What the enchantments of the recipes of Enchanting give to a worn item, made by tools/import_enchants.py from SpellItemEnchantment.dbc and Spell.dbc: the name\n"
              "// and the effects the game models (as the worn spells of items have). Do not edit by hand.\n")
    with open(os.path.join(target, "classic_enchants.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header + "{\n" + "\n".join(out) + "\n}\n")
    print("wrote", len(out), "enchantments,", sum(1 for line in out if "effects: []" in line and "procs" not in line and "spells" not in line), "without an effect the game models")


if __name__ == "__main__":
    main()
