"""Makes `content/classic_lifts.ron`: the objects of the original game (1.12.1) that move on a fixed path.

The elevators, their doors and the tram of Deeprun (gameobject type 11, TRANSPORT) come from the CMaNGOS classic database (gameobject) and the client table TransportAnimation: each
stands at the one place the database puts it and moves by the offsets of its path (time in milliseconds, offset north, west and up in yards, turned by the object's orientation:
the table gives them in the object's own frame), round and round, the path's last time being its period.

The ships and zeppelins (type 15, MO_TRANSPORT) sail the taxi path of their template (TaxiPathNode.dbc; the database's `transports` table gives the length of a round). A path is cut
into runs wherever it jumps (to another map, or a node flagged to teleport); each run is an entry with the `route` of its ship that is there only from its first key to its last,
so the ship leaves one map as it enters the next. `turns` is the heading (degrees) at each key.

Left out: what has no model in the game files, and the same spawn standing again in a map that is only a copy of the place (the Burning Steppes lifts in Blackwing Lair).

Needs, beside db.sql, GameObjectDisplayInfo.dbc, Map.dbc, TransportAnimation.dbc, TaxiPathNode.dbc (`wowtool extract DBFilesClient\\TransportAnimation.dbc`) and files.txt in the same
folder; see the head of import_props.py for the rest.

Usage: py -3 tools/import_lifts.py [folder with db.sql (default: tools/data)] [content folder]
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import dbc

TRANSPORT = 11
SHIP = 15
# Taxi nodes: flag 1 jumps to the next node, flag 2 stops for `delay` seconds; a gap this long (yards) between two nodes is a jump too.
JUMP, STOP, FAR = 1, 2, 500.0
# SoundEntries of a ship: the flap of its sails while it sails, the horn as it docks (its model names no sounds; the zeppelin's doodad does).
BOAT_SAILS, BOAT_HORN = 5454, 5495
# Yards between two keys of a sailed path.
STEP = 20.0


def spline(points, per=24):
    """A smooth line through the points (Catmull-Rom), as a dense list of points and the index in it of each given point."""
    p = [points[0]] + points + [points[-1]]
    line, at = [], []
    for i in range(1, len(p) - 2):
        at.append(len(line))
        for k in range(per):
            u = k / per
            line.append(tuple(0.5 * (2 * p[i][d] + (-p[i - 1][d] + p[i + 1][d]) * u + (2 * p[i - 1][d] - 5 * p[i][d] + 4 * p[i + 1][d] - p[i + 2][d]) * u * u
                                    + (-p[i - 1][d] + 3 * p[i][d] - 3 * p[i + 1][d] + p[i + 2][d]) * u ** 3) for d in range(3)))
    at.append(len(line))
    line.append(points[-1])
    return line, at


def runs_of(nodes):
    """The runs of a path: lists of nodes (map, x, y, z, flags, delay) with no jump inside, starting where a jump ended (a path with no jump is one run, closed on itself)."""
    n = len(nodes)
    jump = [nodes[i][0] != nodes[(i + 1) % n][0] or nodes[i][4] & JUMP or math.dist(nodes[i][1:4], nodes[(i + 1) % n][1:4]) > FAR for i in range(n)]
    if not any(jump):
        return [nodes + [nodes[0]]]
    first = (jump.index(True) + 1) % n
    order = nodes[first:] + nodes[:first]
    cut = jump[first:] + jump[:first]
    runs, run = [], []
    for node, j in zip(order, cut):
        run.append(node)
        if j:
            runs.append(run)
            run = []
    return runs


def sail(nodes, speed):
    """Every run of a path as (map, keys), the keys being (seconds, (north, west, up), heading in degrees): the ship goes `speed` yards a second and waits (the delay of a node is in seconds) at the stops."""
    out, clock = [], 0.0
    for run in runs_of(nodes):
        line, at = spline([n[1:4] for n in run])
        length = [0.0]
        for a, b in zip(line, line[1:]):
            length.append(length[-1] + math.dist(a, b))
        stops = {length[at[i]]: float(run[i][5]) for i in range(len(run)) if run[i][4] & STOP and run[i][5]}
        marks = sorted({0.0, length[-1], *stops, *[x * STEP for x in range(1, int(length[-1] / STEP) + 1) if length[-1] - x * STEP > STEP / 2]})
        keys, waited, j = [], 0.0, 0
        for s in marks:
            while j + 1 < len(line) - 1 and length[j + 1] <= s:
                j += 1
            f = min((s - length[j]) / (length[j + 1] - length[j]), 1.0) if length[j + 1] > length[j] else 0.0
            pos = tuple(line[j][d] + (line[j + 1][d] - line[j][d]) * f for d in range(3))
            # It heads where the line goes there; the transports are modelled with the bow to the back (a lift's heading adds 180).
            a, b = line[max(j - 1, 0)], line[min(j + 2, len(line) - 1)]
            heading = math.degrees(math.atan2(b[1] - a[1], b[0] - a[0])) % 360.0
            keys.append((clock + s / speed + waited, pos, heading))
            if s in stops:
                waited += stops[s]
                keys.append((clock + s / speed + waited, pos, heading))
        clock = keys[-1][0]
        out.append((run[0][0], keys))
    return out, clock


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    db = os.path.join(folder, "db.sql")
    files = {l.strip().lower(): l.strip() for l in open(os.path.join(folder, "files.txt"), encoding="utf8", errors="replace")}
    table = dbc.Dbc(os.path.join(folder, "GameObjectDisplayInfo.dbc"))
    model = {table.u32(i, 0): table.string(i, 1).replace("/", "\\") for i in range(len(table))}
    directory = dbc.Dbc(os.path.join(folder, "Map.dbc"))
    maps = {directory.u32(i, 0): directory.string(i, 1) for i in range(len(directory))}
    animation = dbc.Dbc(os.path.join(folder, "TransportAnimation.dbc"))
    paths = {}
    for i in range(len(animation)):
        paths.setdefault(animation.u32(i, 1), []).append((animation.u32(i, 2), animation.f32(i, 3), animation.f32(i, 4), animation.f32(i, 5)))
    templates = {t["entry"]: t for t in classicdb.rows(db, "gameobject_template") if t["type"] == TRANSPORT and t["entry"] in paths}

    def file_of(display):
        name = model.get(display, "")
        stem, _, ext = name.rpartition(".")
        path = name if ext.lower() == "wmo" else stem + ".m2"
        return files.get(path.lower()) if stem else None

    spawns = list(classicdb.rows(db, "gameobject"))
    # A lift of an instance that stands where the same one stands on a continent is a copy of the place, not a lift of its own.
    outside = {(s["id"], round(s["position_x"]), round(s["position_y"])) for s in spawns if s["map"] in (0, 1)}
    out, skipped = [], 0
    for s in spawns:
        t = templates.get(s["id"])
        if t is None or s["map"] not in maps or (s["map"] not in (0, 1) and (s["id"], round(s["position_x"]), round(s["position_y"])) in outside):
            continue
        path = file_of(t["displayId"])
        if path is None:
            skipped += 1
            continue
        heading = (math.degrees(s["orientation"]) + 180.0) % 360.0
        # The path is in the object's own frame: x along the way it faces (the tram runs along the tracks it was turned to).
        c, n = math.cos(s["orientation"]), math.sin(s["orientation"])
        way = [(ms, x * c - y * n, x * n + y * c, z) for ms, x, y, z in sorted(paths[s["id"]])]
        out.append((maps[s["map"]], path, s["position_x"], s["position_y"], s["position_z"], heading, t["size"] or 1.0, way, "", [], 0))

    # The ships and zeppelins.
    node_table = dbc.Dbc(os.path.join(folder, "TaxiPathNode.dbc"))
    nodes = {}
    for i in range(len(node_table)):
        nodes.setdefault(node_table.u32(i, 1), []).append((node_table.u32(i, 2), (node_table.u32(i, 3), node_table.f32(i, 4), node_table.f32(i, 5), node_table.f32(i, 6), node_table.u32(i, 7), node_table.u32(i, 8))))
    periods = {r["entry"]: (r["name"], r["period"]) for r in classicdb.rows(db, "transports")}
    for t in classicdb.rows(db, "gameobject_template"):
        if t["type"] != SHIP or t["entry"] not in periods or t["data0"] not in nodes:
            continue
        path = file_of(t["displayId"])
        if path is None:
            skipped += 1
            continue
        name, period = periods[t["entry"]]
        way = [n for _, n in sorted(nodes[t["data0"]])]
        waiting = sum(n[5] for n in way if n[4] & STOP)
        # Speed from the length of a round: what it is sailed in is the time not spent waiting.
        speed = (sail(way, 1.0)[1] - waiting) / max(period / 1000.0 - waiting, 1.0)
        runs, end = sail(way, speed)
        for map_id, run in runs:
            (_, (n0, w0, up0), _) = run[0]
            out.append((maps[map_id], path, n0, w0, up0, 0.0, t["size"] or 1.0,
                        [(round(t_ * 1000), p[0] - n0, p[1] - w0, p[2] - up0) for t_, p, _ in run], name, [round(h, 1) for _, _, h in run], round(period)))
        print("%-45s %6.1f yd/s (template says %s), round %.0f s of %.0f s, %d runs" % (name, speed, t["data1"], end, period / 1000.0, len(runs)))

    header = ("// The lifts, trams, ships and zeppelins of the original game (1.12.1), made by tools/import_lifts.py from the CMaNGOS classic database and the client's TransportAnimation and\n"
              "// TaxiPathNode tables. `path` is (milliseconds, north, west, up) offsets from `at`; the object goes through them in turn, round and round, the last time (or `period`) being the\n"
              "// length of a round. One with a `route` is there only from its first key to its last (`turns`: its heading at each key). Do not edit by hand.\n")
    with open(os.path.join(target, "classic_lifts.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header + "[\n")
        for name, path, x, y, z, h, scale, keys, route, turns, period in sorted(out, key=lambda o: o[:5]):
            way = ", ".join("(%d, %.3f, %.3f, %.3f)" % k for k in keys)
            extra = ""
            if route:
                extra = ', route: "%s", period: %d, turns: [%s]' % (route.replace('"', ""), period, ", ".join("%.1f" % t_ for t_ in turns))
                if "transport_ship" in path.lower():
                    extra += ", hum: %d, horn: %d" % (BOAT_SAILS, BOAT_HORN)
            f.write('    (model: r"%s", map: "%s", at: (%.2f, %.2f, %.2f), heading: %.1f, scale: %.2f, path: [%s]%s),\n' % (path, name, x, y, z, h, scale, way, extra))
        f.write("]\n")
    print("wrote", len(out), "lifts, left out", skipped, "without a model")


if __name__ == "__main__":
    main()
