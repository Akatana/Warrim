"""Makes `content/classic_loot.ron`: what the original's creatures and chests drop (the loot tables of the CMaNGOS classic database), for the
ones the game has an equivalent of; `content/loot.ron` rolls them with `also: [...]`. The tables are flattened: a reference to another table, and
the items of a group that excludes the others, become one chance of the item each (what each item's chance is in the original, though the items
of a group are no longer exclusive of one another).

Usage: py -3 tools/import_loot.py [folder with db.sql (default: tools/data)] [content folder]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import import_items

# The tables of creatures (by their entry in creature_template) and chests (by gameobject entry) of the original that the game's ones stand for.
CREATURES = {"wolf": 299, "black_wolf": 69, "boar": 113, "kobold": 80, "cutpurse": 94, "garrick": 103, "gribble": 40}
CHESTS = {"chest_poor": 2845, "chest_wolfden": 2844, "chest_hoard": 2846, "chest_stash": 2850}
# Items of the original that the game has under another id.
SAME = {4865: "ruined_pelt"}


class Loot:
    """The loot tables of the dump: `table(rows, gold)` is the RON of one drop table."""

    def __init__(self, sql):
        self.items = {i["entry"]: i for i in classicdb.rows(sql, "item_template") if not import_items.EXCLUDED.search(i["name"]) or import_items.KEPT_DESPITE.search(i["name"])}
        self.references = {}
        for r in classicdb.rows(sql, "reference_loot_template"):
            self.references.setdefault(r["entry"], []).append(r)

    def flatten(self, rows, factor, out, depth=0):
        """Add the chance of each item of the rows (times `factor`) to `out`: {entry: [chance, lowest count, highest count]}."""
        groups = {}
        for r in rows:
            if r["groupid"]:
                groups.setdefault(r["groupid"], []).append(r)
        for r in rows:
            ref = -r["mincountOrRef"] if r["mincountOrRef"] < 0 else 0
            chance = abs(r["ChanceOrQuestChance"])
            if r["groupid"]:
                group = groups[r["groupid"]]
                explicit = sum(abs(g["ChanceOrQuestChance"]) for g in group)
                equal = [g for g in group if g["ChanceOrQuestChance"] == 0]
                if chance == 0 and equal:
                    chance = max(0.0, 100.0 - explicit) / len(equal)
            if ref:
                if depth < 4 and ref in self.references and chance > 0:
                    self.flatten(self.references[ref], factor * chance / 100.0 * max(1, r["maxcount"]), out, depth + 1)
                continue
            if r["item"] not in self.items:
                continue
            low, high = max(1, r["mincountOrRef"]), max(1, r["maxcount"])
            slot = out.setdefault(r["item"], [0.0, low, high])
            slot[0] = min(100.0, slot[0] + chance * factor)
            slot[1], slot[2] = min(slot[1], low), max(slot[2], high)

    @staticmethod
    def name(entry):
        """The id the game gives the item of this entry."""
        return SAME.get(entry) or import_items.ALIASES.get(entry, "i%d" % entry)

    def table(self, rows, gold, least=0.01):
        out = {}
        self.flatten(rows, 1.0, out)
        drops = []
        for entry, (chance, low, high) in sorted(out.items(), key=lambda kv: -kv[1][0]):
            if chance < least:
                continue
            # A drop never asks for more of an item than a stack holds.
            stack = max(1, self.items[entry]["stackable"])
            low, high = min(low, stack), min(max(low, high), stack)
            drops.append('("%s", (%d, %d), %s)' % (self.name(entry), low, high, repr(round(chance, 3))))
        return "(copper: (%d, %d), items: [%s])" % (gold[0], max(gold), ", ".join(drops))


def main():
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "content")
    sql = os.path.join(folder, "db.sql")
    loot = Loot(sql)
    items = loot.items
    creatures = {c["Entry"]: c for c in classicdb.rows(sql, "creature_template")}
    objects = {g["entry"]: g for g in classicdb.rows(sql, "gameobject_template")}
    c_loot, g_loot = {}, {}
    for r in classicdb.rows(sql, "creature_loot_template"):
        c_loot.setdefault(r["entry"], []).append(r)
    for r in classicdb.rows(sql, "gameobject_loot_template"):
        g_loot.setdefault(r["entry"], []).append(r)

    lines = []
    for name, entry in CREATURES.items():
        c = creatures[entry]
        lines.append('    "c_%s": %s,  // %s (%d)' % (name, loot.table(c_loot.get(c["LootId"], []), (c["MinLootGold"], c["MaxLootGold"])), c["Name"], entry))
    for name, entry in CHESTS.items():
        g = objects[entry]
        lines.append('    "c_%s": %s,  // %s (%d)' % (name, loot.table(g_loot.get(g["data1"], []), (g["mingold"], g["maxgold"])), g["name"], entry))
    header = ("// What the original's creatures and chests drop, made by tools/import_loot.py from the CMaNGOS classic database: each table is the original's, with its\n"
              "// references and groups flattened into one chance for each item. `loot.ron` rolls these with `also`. Do not edit by hand.\n")
    with open(os.path.join(target, "classic_loot.ron"), "w", encoding="utf8", newline="\n") as out:
        out.write(header + "{\n" + "\n".join(lines) + "\n}\n")
    print("wrote", len(lines), "tables")
    # What the merchants of the original sell, for the game's merchant.
    vendors = {}
    for r in classicdb.rows(sql, "npc_vendor"):
        vendors.setdefault(r["entry"], []).append(r["item"])
    for name, entry in (("Godric Rothgar", 1213), ("Janos Hammerknuckle", 78), ("Brother Danil", 152)):
        print(name, [Loot.name(i) for i in vendors.get(entry, []) if i in items])


if __name__ == "__main__":
    main()
