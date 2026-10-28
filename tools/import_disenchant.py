"""Makes `content/classic_disenchant.ron`: what breaking an item gives in the original game (`disenchant_loot_template` of the CMaNGOS classic database).
Each item of `classic_items.ron` names its table by `disenchant` (its `DisenchantID`). A row is (item, group, chance, low, high): the rows of group 0 are
rolled one by one; of a group above 0 exactly one row (or none) comes out, the ones with a chance first and, when none of them is hit, one of the rows
without a chance (chance 0) at random (CMaNGOS `LootGroup::Process`).

Usage: py -3 tools/import_disenchant.py [folder with db.sql (default: tools/data)] [content folder]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import import_loot


def main():
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "content")
    tables = {}
    for r in classicdb.rows(os.path.join(folder, "db.sql"), "disenchant_loot_template"):
        # A reference to another table (a negative count) does not occur in the original's disenchant tables.
        assert r["mincountOrRef"] > 0, r
        tables.setdefault(r["entry"], []).append(r)
    lines = []
    for entry in sorted(tables):
        rows = ", ".join('(item: "%s", group: %d, chance: %s, low: %d, high: %d)' % (import_loot.Loot.name(r["item"]), r["groupid"], repr(float(r["ChanceOrQuestChance"])), r["mincountOrRef"], r["maxcount"]) for r in tables[entry])
        lines.append("    %d: [%s]," % (entry, rows))
    header = ("// What breaking an item gives in the original game (1.12.1), made by tools/import_disenchant.py from the CMaNGOS classic database (disenchant_loot_template).\n"
              "// The key is the `disenchant` of an item in classic_items.ron. Do not edit by hand.\n")
    with open(os.path.join(target, "classic_disenchant.ron"), "w", encoding="utf8", newline="\n") as out:
        out.write(header + "{\n" + "\n".join(lines) + "\n}\n")
    print("wrote", len(lines), "tables")


if __name__ == "__main__":
    main()
