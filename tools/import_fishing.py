"""Makes `content/classic_fishing.ron` and `content/classic_fishing_loot.ron`: what the original game's waters give to a fishing pole (the CMaNGOS classic database:
fishing_loot_template by area, skill_fishing_base_level). `classic_fishing.ron` gives each area of `AreaTable.dbc` (id) the table (`f<area>`) of the area or of the nearest
area above it that has one, and the skill the water asks for (the area's own, else that of the one above it; a negative one asks for nothing);
the tables are flattened as the others of `import_loot.py`.

Usage: py -3 tools/import_fishing.py [folder with db.sql (default: tools/data)] [content folder]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import dbc
import import_loot


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    sql = os.path.join(folder, "db.sql")
    table = dbc.Dbc(os.path.join(folder, "AreaTable.dbc"))
    parent = {table.u32(i, 0): table.u32(i, 2) for i in range(len(table))}
    loot = import_loot.Loot(sql)
    rows = {}
    for r in classicdb.rows(sql, "fishing_loot_template"):
        rows.setdefault(r["entry"], []).append(r)
    skill = {r["entry"]: r["skill"] for r in classicdb.rows(sql, "skill_fishing_base_level")}
    tables = {area: loot.table(r, (0, 0)) for area, r in rows.items()}

    def above(area, have):
        seen = 0
        while area and seen < 8:
            if area in have:
                return area
            area, seen = parent.get(area, 0), seen + 1
        return None

    out = []
    for area in sorted(parent):
        t, s = above(area, tables), above(area, skill)
        if t is not None and "items: []" not in tables[t]:
            out.append("    %d: (table: \"f%d\", skill: %d)," % (area, t, skill[s] if s is not None else 0))
    used = sorted({int(line.split('"f')[1].split('"')[0]) for line in out})
    header = "// Fishing in the original game (1.12.1), made by tools/import_fishing.py from the CMaNGOS classic database (%s). Do not edit by hand.\n"
    with open(os.path.join(target, "classic_fishing_loot.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header % "fishing_loot_template, flattened" + "{\n" + "".join('    "f%d": %s,\n' % (a, tables[a]) for a in used) + "}\n")
    with open(os.path.join(target, "classic_fishing.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header % "for each area the table of its water and the skill it asks for (`skill_fishing_base_level`)" + "{\n" + "\n".join(out) + "\n}\n")
    print("wrote", len(out), "areas with", len(used), "tables")


if __name__ == "__main__":
    main()
