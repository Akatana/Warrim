"""Makes `content/classic_props.ron`: the objects standing in the world of the original game (1.12.1), from the CMaNGOS classic database (gameobject): signs, banners,
mailboxes, benches and chairs, forges, anvils, doors, lamps and the rest of what is put down for looks.

Left out: chests (with the herbs, ores and crates, all of type CHEST), what moves (boats and zeppelins), traps and other things without a place of their own, everything that
a game event puts out (Hallow's End, Winter Veil, Darkmoon Faire, the Scourge invasion... `game_event_gameobject`), and what has no model in the game files.
The turn about the vertical is taken (its tilt is not); the size of the template is the scale.

Each one is tagged with its zone (the top area of the map file's chunk it stands in, read with `target/release/wowtool.exe`: build it first, and run this from the folder of the game).

Needs, beside db.sql, the client tables GameObjectDisplayInfo and AreaTable and files.txt (`wowtool list "" > files.txt`) in the same folder; see the head of import_items.py for the rest.

Usage: py -3 tools/import_props.py [folder with db.sql (default: tools/data)] [content folder]
"""
import math
import os
import struct
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import dbc

MAPS = {0: "Azeroth", 1: "Kalimdor"}
# The size of a map tile and of its chunks (yards), and the number of tiles to the middle of a map.
TILE, CHUNK, HALF = 533.33333, 533.33333 / 16, 32
# Door, button, quest giver, generic, chair, spell focus (forge, anvil, fire), text (a book, a plaque), goober, mailbox, meeting stone, fishing hole.
KEPT = {0, 1, 2, 5, 7, 8, 9, 10, 19, 23, 25}


class Zones:
    """The zone (AreaTable name of the top area) at a place of a map, from the area of the map file's chunk that holds it."""

    def __init__(self, folder):
        table = dbc.Dbc(os.path.join(folder, "AreaTable.dbc"))
        self.areas = {table.u32(i, 0): (table.u32(i, 2), table.string(i, 11)) for i in range(len(table))}
        self.exe = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "wowtool.exe")
        self.tiles = {}

    def chunks(self, map_name, x, y):
        """The area ids of the 256 chunks of a tile (None when the map has no such file)."""
        if (map_name, x, y) not in self.tiles:
            with tempfile.TemporaryDirectory() as tmp:
                out = os.path.join(tmp, "tile.adt")
                done = subprocess.run([self.exe, "extract", r"World\Maps\%s\%s_%d_%d.adt" % (map_name, map_name, x, y), out], capture_output=True)
                data = open(out, "rb").read() if done.returncode == 0 and os.path.exists(out) else None
            areas, i = [0] * 256, 0
            while data is not None and i + 8 <= len(data):
                size = struct.unpack_from("<I", data, i + 4)[0]
                if data[i:i + 4] == b"KNCM":
                    index_x, index_y = struct.unpack_from("<II", data, i + 8 + 4)
                    areas[index_y * 16 + index_x] = struct.unpack_from("<I", data, i + 8 + 0x34)[0]
                i += 8 + size
            self.tiles[(map_name, x, y)] = areas if data is not None else None
        return self.tiles[(map_name, x, y)]

    def at(self, map_name, north, west):
        fx, fz = -west + HALF * TILE, -north + HALF * TILE
        tx, ty = int(fx // TILE), int(fz // TILE)
        areas = self.chunks(map_name, tx, ty)
        if areas is None:
            return ""
        area = areas[min(int((fz - ty * TILE) / CHUNK), 15) * 16 + min(int((fx - tx * TILE) / CHUNK), 15)]
        while area in self.areas and self.areas[area][0]:
            area = self.areas[area][0]
        return self.areas[area][1] if area in self.areas else ""


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    db = os.path.join(folder, "db.sql")
    files = {l.strip().lower(): l.strip() for l in open(os.path.join(folder, "files.txt"), encoding="utf8", errors="replace")}
    table = dbc.Dbc(os.path.join(folder, "GameObjectDisplayInfo.dbc"))
    model = {table.u32(i, 0): table.string(i, 1).replace("/", "\\") for i in range(len(table))}
    templates = {t["entry"]: t for t in classicdb.rows(db, "gameobject_template") if t["type"] in KEPT}
    events = {r["guid"] for r in classicdb.rows(db, "game_event_gameobject") if r["event"] > 0}

    def file_of(display):
        """The .m2 or .wmo of a display id (the table names the old .mdx/.mdl), as the game's file list spells it; None if the game has no such file."""
        name = model.get(display, "")
        stem, _, ext = name.rpartition(".")
        path = name if ext.lower() == "wmo" else stem + ".m2"
        return files.get(path.lower()) if stem else None

    zones = Zones(folder)
    out = []
    seen, skipped = set(), {"event": 0, "no model": 0}
    for s in classicdb.rows(db, "gameobject"):
        t = templates.get(s["id"])
        if s["map"] not in MAPS or t is None:
            continue
        if s["guid"] in events:
            skipped["event"] += 1
            continue
        path = file_of(t["displayId"])
        if path is None:
            skipped["no model"] += 1
            continue
        heading = (math.degrees(s["orientation"]) + 180.0) % 360.0
        key = (s["map"], path.lower(), round(s["position_x"], 1), round(s["position_y"], 1), round(s["position_z"], 1), round(heading))
        if key in seen:
            continue
        seen.add(key)
        out.append((MAPS[s["map"]], path, s["position_x"], s["position_y"], s["position_z"], heading, t["size"] or 1.0, zones.at(MAPS[s["map"]], s["position_x"], s["position_y"])))

    header = ("// The objects of the original game (1.12.1) that stand in the world for looks, made by tools/import_props.py from the CMaNGOS classic database (gameobject).\n"
              "// They are props like those of `props.ron` (and are edited in the editor like them), each with the map it stands in. Do not edit by hand: what the editor changes of them\n"
              "// is kept in `props.ron` and `editor.ron`, and running the tool again leaves that as it is.\n")
    with open(os.path.join(target, "classic_props.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header + "[\n")
        for name, path, x, y, z, h, scale, zone in sorted(out):
            f.write('    (model: r"%s", map: Some("%s"),%s at: (%.2f, %.2f, %.2f), rotation: (0.0, %.1f, 0.0), scale: %.2f),\n' % (path, name, ' zone: Some("%s"),' % zone if zone else "", x, y, z, h, scale))
        f.write("]\n")
    print("wrote", len(out), "objects, left out", skipped)


if __name__ == "__main__":
    main()
