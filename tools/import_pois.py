"""Makes `content/classic_pois.ron`: the places the original game (1.12.1) points to on the map, from the CMaNGOS classic database (points_of_interest).

Name, place and height are taken (the height of the nearest creature or object of the database, which stands on the floor there: a building, a bridge); the table has no continent, so the places of Kalimdor are in the list too, at their Kalimdor coordinates.
Entries that point to a person (a name that is a creature of the database) or to a service (trainers, battlemasters, mailboxes) are left out: only places are kept.

Usage: py -3 tools/import_pois.py [folder with db.sql (default: tools/data)] [content folder]
"""
import math
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
from import_items import q

# How close (yards) the player must come to discover a place, and its picture (4 a building), as the places made in the editor have.
RADIUS, ICON = 30.0, 4
# How far (yards) the spawn that gives a place its height may be.
NEAR = 25.0
# People that no creature is named after (two of them, or with a title).
PEOPLE = {"Bixi and Buliwyf", "Sayoc & Hanashi", "Champion Cyssa Dawnrose"}
SERVICE = re.compile(r"Trainer|^Battlemasters|Mailbox$", re.I)


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    db = os.path.join(folder, "db.sql")
    people = {c["Name"].lower() for c in classicdb.rows(db, "creature_template")} | {n.lower() for n in PEOPLE}
    rows = sorted((p for p in classicdb.rows(db, "points_of_interest") if p["icon_name"].lower() not in people and not SERVICE.search(p["icon_name"])), key=lambda p: p["entry"])
    spawns = [(r["position_x"], r["position_y"], r["position_z"]) for t in ("creature", "gameobject") for r in classicdb.rows(db, t) if r["map"] in (0, 1)]

    def height(p):
        d, z = min((math.hypot(p["x"] - x, p["y"] - y), z) for x, y, z in spawns)
        return ", height: Some(%.1f)" % z if d <= NEAR else ""

    header = ("// Every place the original game (1.12.1) points to on the map, made by tools/import_pois.py from the CMaNGOS classic database (points_of_interest).\n"
              "// Do not edit by hand. Ids are `poi<entry>`; `at` is (north, west). The table has no continent: the places of Kalimdor are here at their Kalimdor coordinates.\n")
    with open(os.path.join(target, "classic_pois.ron"), "w", encoding="utf8", newline="\n") as out:
        out.write(header + "[\n")
        for p in rows:
            out.write("    (id: \"poi%d\", name: %s, at: (%s, %s), radius: %s, icon: %d%s),\n" % (p["entry"], q(p["icon_name"]), p["x"], p["y"], RADIUS, ICON, height(p)))
        out.write("]\n")
    print("wrote", len(rows), "places")


if __name__ == "__main__":
    main()
