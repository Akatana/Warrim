"""Makes `content/classic_node_kinds.ron`, `classic_nodes.ron` and `classic_node_loot.ron`: the mineral veins and herbs of the original game (1.12.1) that the Mining
and Herbalism skills gather, from the CMaNGOS classic database (gameobject_template of type CHEST whose lock is a mining or herbalism lock, their spawns and their loot).

A kind is one object template: its model, size, the skill and rank its lock asks for (Lock.dbc), how many times it can be gathered (`data4`..`data5`) and its loot table
`n<loot id>` (flattened as for the chests, see import_loot.py). A placed node is a spawn with the map it stands in. Left out: spawns that a game event puts out, those
of a pool beyond as many as it lets stand, templates with no loot, no model or only in later patches (entry above 180000), and the quest objects that use the lock but are no plants or ores.

Usage: py -3 tools/import_nodes.py [folder with db.sql (default: tools/data)] [content folder]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import dbc
import import_loot

# The lock types of Lock.dbc that mean a skill, and the skill each is.
LOCKS = {2: "Herbalism", 3: "Mining"}
LAST_ENTRY = 180000
# Quest objects that open with an herb lock but are no plants.
NOT_PLANTS = {"Beached Sea Creature", "Skeletal Sea Turtle", "Plant Bundle", "Dragonspine", "Olemba Root", "Drycap Mushroom", "Burstcap Mushroom"}


def q(text):
    return '"%s"' % text.replace("\\", "\\\\").replace('"', '\\"')


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    sql = os.path.join(folder, "db.sql")
    files = {l.strip().lower(): l.strip() for l in open(os.path.join(folder, "files.txt"), encoding="utf8", errors="replace")}
    table = dbc.Dbc(os.path.join(folder, "GameObjectDisplayInfo.dbc"))
    model = {table.u32(i, 0): table.string(i, 1).replace("/", "\\") for i in range(len(table))}
    lock = dbc.Dbc(os.path.join(folder, "Lock.dbc"))
    locks = {}
    for i in range(len(lock)):
        for k in range(8):
            if lock.u32(i, 1 + k) == 2 and lock.u32(i, 9 + k) in LOCKS:
                locks[lock.u32(i, 0)] = (LOCKS[lock.u32(i, 9 + k)], lock.u32(i, 17 + k))
    maps_dbc = dbc.Dbc(os.path.join(folder, "Map.dbc"))
    with open(os.path.join(folder, "wdts.txt"), encoding="utf8") as f:
        have = {line.strip().split("\\")[2].lower() for line in f if line.strip()}
    maps = {maps_dbc.u32(i, 0): maps_dbc.string(i, 1) for i in range(len(maps_dbc))}
    maps = {i: name for i, name in maps.items() if name.lower() in have}

    def file_of(display):
        name = model.get(display, "")
        stem, _, ext = name.rpartition(".")
        return files.get((name if ext.lower() == "wmo" else stem + ".m2").lower()) if stem else None

    g_loot = {}
    for r in classicdb.rows(sql, "gameobject_loot_template"):
        g_loot.setdefault(r["entry"], []).append(r)
    loot = import_loot.Loot(sql)
    kinds, tables = {}, {}
    for t in classicdb.rows(sql, "gameobject_template"):
        if t["type"] != 3 or t["data0"] not in locks or t["entry"] >= LAST_ENTRY or t["name"] in NOT_PLANTS or not t["data1"] or not file_of(t["displayId"]):
            continue
        skill, rank = locks[t["data0"]]
        table_id = "n%d" % t["data1"]
        tables.setdefault(table_id, (loot.table(g_loot.get(t["data1"], []), (0, 0)), t["name"]))
        low, high = max(1, t["data4"]), max(1, t["data5"], t["data4"])
        kinds[t["entry"]] = ('(id: "n%d", name: %s, skill: %s, rank: %d, model: r"%s", scale: %s, loot: "%s", uses: (%d, %d))'
                             % (t["entry"], q(t["name"]), skill, rank, file_of(t["displayId"]), repr(float(t["size"] or 1.0)), table_id, low, high))

    events = {r["guid"] for r in classicdb.rows(sql, "game_event_gameobject") if r["event"] > 0}
    limit = {r["entry"]: r["max_limit"] or 1 for r in classicdb.rows(sql, "pool_template")}
    pools = {}
    for r in classicdb.rows(sql, "pool_gameobject"):
        pools.setdefault(r["pool_entry"], []).append((-r["chance"], r["guid"]))
    pooled_out = set()
    for entry, members in pools.items():
        pooled_out.update(g for _, g in sorted(members)[limit.get(entry, 1):])
    nodes, seen = [], set()
    for s in classicdb.rows(sql, "gameobject"):
        if s["id"] not in kinds or s["map"] not in maps or s["guid"] in events or s["guid"] in pooled_out:
            continue
        key = (s["map"], s["id"], round(s["position_x"], 1), round(s["position_y"], 1))
        if key in seen:
            continue
        seen.add(key)
        # The original's heading is counter-clockwise; the game's turn is clockwise, as the props' (see import_props.py).
        heading = (s["orientation"] * 57.29578 + 180.0) % 360.0
        nodes.append((maps[s["map"]], s["id"], s["position_x"], s["position_y"], s["position_z"], heading, (s["spawntimesecsmin"] + s["spawntimesecsmax"]) / 2.0))
    used = {n[1] for n in nodes}
    nodes.sort()

    with open(os.path.join(target, "classic_node_kinds.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write("// The mineral veins and herbs of the original game (1.12.1) that Mining and Herbalism gather, made by tools/import_nodes.py (gameobject_template, Lock.dbc).\n"
                "// `rank` is the skill the lock asks for, `uses` how many times a node can be gathered before it is gone. Do not edit by hand.\n[\n")
        f.write("".join("    %s,\n" % kinds[e] for e in sorted(used)))
        f.write("]\n")
    with open(os.path.join(target, "classic_nodes.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write("// Where the original's veins and herbs stand, made by tools/import_nodes.py (gameobject). The editor keeps only what it changes of them in `editor.ron`.\n[\n")
        for name, kind, x, y, z, h, respawn in nodes:
            f.write('    (kind: "n%d", map: Some("%s"), at: (%.2f, %.2f, %.2f), facing: %.0f, respawn: Some(%.0f)),\n' % (kind, name, x, y, z, h, respawn))
        f.write("]\n")
    wanted = {kinds[e].split('loot: "')[1].split('"')[0] for e in used}
    with open(os.path.join(target, "classic_node_loot.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write("// What the original's veins and herbs give (gameobject_loot_template), made by tools/import_nodes.py with the references and groups flattened. Do not edit by hand.\n{\n")
        f.write("".join('    "%s": %s,  // %s\n' % (k, v[0], v[1]) for k, v in sorted(tables.items()) if k in wanted))
        f.write("}\n")
    print("wrote", len(used), "kinds,", len(nodes), "nodes in", len({n[0] for n in nodes}), "maps")


if __name__ == "__main__":
    main()
