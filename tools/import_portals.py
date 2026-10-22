"""Makes `content/classic_portals.ron`: the doors of the original game (1.12.1) that carry the character to another place or map (dungeon entrances and exits, tower portals).

A portal is an `AreaTrigger.dbc` row (the map it is on, the place, a ball or a box) joined with the CMaNGOS classic database's `areatrigger_teleport` (where it leads, the level it asks for).
Left out: triggers whose map or target has no WDT in the client (nothing to load), the doors that arrive over no floor (NO_FLOOR, none now). The battleground exits that only one faction may use carry that `side`.
Items and quests some of them ask for (attunements) and the raid group are not asked: nothing in the offline game gives them.

Usage: py -3 tools/import_portals.py [folder with db.sql, AreaTrigger.dbc and Map.dbc (default: tools/data)] [content folder]
(extract the tables with `wowtool extract DBFilesClient\\AreaTrigger.dbc` and the same for Map.dbc, and list the WDTs with `wowtool list .wdt > tools/data/wdts.txt`)
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
from dbc import Dbc
from import_items import q


# Doors whose destination has no floor in the client's files (checked by the test `every_portal_arrives_on_a_floor`): the graveyard of the Monastery
# arrives on a bare plane at height 0, Blackwing Lair above its lava. The Shadowfang "falling out" door is kept: it drops the character onto the ground.
NO_FLOOR = set()


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    maps_dbc = Dbc(os.path.join(folder, "Map.dbc"))
    directory = {maps_dbc.u32(i, 0): maps_dbc.string(i, 1) for i in range(len(maps_dbc))}
    with open(os.path.join(folder, "wdts.txt"), encoding="utf8") as f:
        have = {line.strip().split("\\")[2].lower() for line in f if line.strip()}
    triggers = Dbc(os.path.join(folder, "AreaTrigger.dbc"))
    shape = {triggers.u32(i, 0): i for i in range(len(triggers))}

    out, skipped = [], 0
    for r in sorted(classicdb.rows(os.path.join(folder, "db.sql"), "areatrigger_teleport"), key=lambda r: r["id"]):
        text = r["status_failed_text"] or ""
        row = shape.get(r["id"])
        side = next((s for s in ("Horde", "Alliance") if text.startswith("Only the %s " % s)), None)
        if row is None or r["id"] in NO_FLOOR:
            skipped += 1
            continue
        source, to = directory.get(triggers.u32(row, 1)), directory.get(r["target_map"])
        if not source or not to or source.lower() not in have or to.lower() not in have:
            skipped += 1
            continue
        f = lambda n: triggers.f32(row, n)
        out.append(
            "    (id: %d, name: %s, map: %s, at: (%r, %r, %r), radius: %r, size: (%r, %r, %r), turn: %r, level: %d, to_map: %s, to: (%r, %r, %r), facing: %r%s%s),\n"
            % (r["id"], q(r["name"]), q(source), f(2), f(3), f(4), f(5), f(6), f(7), f(8), f(9), r["required_level"], q(to),
               r["target_position_x"], r["target_position_y"], r["target_position_z"], r["target_orientation"], ", refusal: Some(%s)" % q(text) if text else "", ", side: Some(%s)" % q(side) if side else ""))

    header = ("// Every door of the original game (1.12.1) that carries the character elsewhere, made by tools/import_portals.py from AreaTrigger.dbc and the CMaNGOS classic database (areatrigger_teleport).\n"
              "// Do not edit by hand. `at` is (north, west, height) of the trigger on map `map`; it is a ball of `radius`, or (radius 0) a box `size` = (length, width, height) turned `turn` radians.\n"
              "// `to` is the same kind of place on `to_map`, `facing` the original's orientation (radians, counter-clockwise from north).\n")
    with open(os.path.join(target, "classic_portals.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header + "[\n" + "".join(out) + "]\n")
    print("wrote", len(out), "portals, left out", skipped)


if __name__ == "__main__":
    main()
