"""Makes `content/classic_skins.ron` and `content/classic_skin_loot.ron`: what the original game's beasts give to a skinning knife (the CMaNGOS classic database:
creature_template.SkinningLootId and skinning_loot_template), for the creature kinds the game has. `classic_skins.ron` says which table each kind gives (`k<id>`), the
tables are flattened as the others of `import_loot.py`.

Usage: py -3 tools/import_skins.py [folder with db.sql (default: tools/data)] [content folder]
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import import_loot


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    sql = os.path.join(folder, "db.sql")
    made = set(re.findall(r'\bid: "(c\d+)"', open(os.path.join(target, "classic_creatures.ron"), encoding="utf8").read()))
    loot = import_loot.Loot(sql)
    rows = {}
    for r in classicdb.rows(sql, "skinning_loot_template"):
        rows.setdefault(r["entry"], []).append(r)
    kinds, tables = {}, {}
    for c in sorted(classicdb.rows(sql, "creature_template"), key=lambda c: c["Entry"]):
        table = c["SkinningLootId"]
        if not table or "c%d" % c["Entry"] not in made or table not in rows:
            continue
        text = loot.table(rows[table], (0, 0))
        if "items: []" in text:
            continue
        tables.setdefault(table, text)
        kinds["c%d" % c["Entry"]] = "k%d" % table
    header = ("// What the original's beasts give to a skinning knife, made by tools/import_skins.py from the CMaNGOS classic database (skinning_loot_template), %s. Do not edit by hand.\n")
    with open(os.path.join(target, "classic_skin_loot.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header % "with the references and groups flattened" + "{\n" + "".join('    "k%d": %s,\n' % (k, v) for k, v in sorted(tables.items())) + "}\n")
    with open(os.path.join(target, "classic_skins.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header % "for each creature kind the table (`classic_skin_loot.ron`) it gives" + "{\n" + "".join('    "%s": "%s",\n' % kv for kv in kinds.items()) + "}\n")
    print("wrote", len(kinds), "kinds with", len(tables), "tables")


if __name__ == "__main__":
    main()
