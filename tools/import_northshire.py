"""Makes the original Northshire of the game (1.12.1) from the CMaNGOS classic database: `content/classic_spawns.ron` (where its creatures stand), `classic_quests.ron`
(its quests), `classic_gossip.ron` (what its people say) and `classic_pickups.ron` (the objects that quests ask the player to pick up).

The valley is every map chunk of the areas Northshire Valley, Northshire Abbey, Northshire River, Echo Ridge Mine and Northshire Vineyards (the area of the chunk is read
with `target/release/wowtool.exe`: build it first). The creatures of `classic_creatures.ron` that stand there are placed as the database places them (heading, and the
yards they stroll when the spawn walks at random). A quest is taken when one of the valley's people gives it, somebody who stands in the world takes it back (the two
people beyond the valley whom its chains end with are placed as well), the chain before it is taken, and it is not for another class than the paladin: Its experience is the server's own rule (CMaNGOS `Quest::XPValue`): `RewMoneyMaxLevel` / 0.6, less as the character outgrows
the quest (done in the game). The start item of a quest is given when it is taken. Left out: the valley's class trainers' teaching, quests of other classes, the voucher of "Welcome!" (it has no giver) and the chain of Brother
Paxton (it starts in Stormwind).

A patrol's places (creature_movement) keep their wait, the heading the creature turns to while it waits (the database's 100 means none) and the waypoint script of the place
(dbscripts_on_creature_movement): run on and off, a stroll at random for a while, standing still for good, a state emote (as the animation of Emotes.dbc) and a morph into
another model, which is a creature kind `m<display>` of `classic_morphs.ron`. The script commands that do not act on the creature alone (an AI event, a one-shot emote)
are left out.

Needs, beside db.sql, the client tables AreaTable, GameObjectDisplayInfo and Emotes and files.txt in the same folder (and those of import_creatures.py); classic_creatures.ron and the item files must be made.

Usage: py -3 tools/import_northshire.py [folder with db.sql (default: tools/data)] [content folder]
"""
import math
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import dbc
import import_creatures
import import_props
from import_items import q
from import_loot import Loot

AREAS = {9, 23, 24, 34, 59}
# The box round the valley that is searched (north, west) and the continent.
BOX = (-9500, -8300, -700, 400)
# The two people beyond the valley whom a quest of it ends with: Marshal Dughan and Innkeeper Farley of Goldshire.
BEYOND = {240: "Marshal Dughan", 295: "Innkeeper Farley"}
# Never placed: the spirit healer of the graveyards.
NEVER = re.compile(r"spirit healer", re.I)
PALADIN, HUMAN = 2, 1
PICK_RESPAWN = 120.0


def words(text):
    """The text of the database in the form the game says it: line breaks, and the male one of `$gman:woman;`."""
    text = re.sub(r"\$[bB]", "\n", text or "")
    text = re.sub(r"\$[gG]([^:;]*):[^;]*;", r"\1", text)
    return text.replace("\r", "").strip()


def ids_of(path):
    return set(re.findall(r'\bid: "([^"]+)"', open(path, encoding="utf8").read()))


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    db = os.path.join(folder, "db.sql")
    zones = import_props.Zones(folder)
    T, H, HALF = import_props.TILE, import_props.CHUNK, import_props.HALF

    def area(north, west):
        fx, fz = -west + HALF * T, -north + HALF * T
        tx, ty = int(fx // T), int(fz // T)
        areas = zones.chunks("Azeroth", tx, ty)
        return areas[min(int((fz - ty * T) / H), 15) * 16 + min(int((fx - tx * T) / H), 15)] if areas else 0

    in_valley = lambda x, y: BOX[0] < x < BOX[1] and BOX[2] < y < BOX[3] and area(x, y) in AREAS
    made = ids_of(os.path.join(target, "classic_creatures.ron"))
    items = ids_of(os.path.join(target, "items.ron")) | ids_of(os.path.join(target, "classic_items.ron")) | ids_of(os.path.join(target, "classic_pieces.ron"))
    templates = {c["Entry"]: c for c in classicdb.rows(db, "creature_template")}

    # Where they stand.
    spawns, guids = [], {}
    for c in classicdb.rows(db, "creature"):
        t = templates.get(c["id"])
        if c["map"] != 0 or t is None or "c%d" % c["id"] not in made or NEVER.search(t["Name"]):
            continue
        if not (in_valley(c["position_x"], c["position_y"]) or (c["id"] in BEYOND and BEYOND[c["id"]] == t["Name"])):
            continue
        spawns.append(c)
        guids.setdefault(c["id"], []).append(c["guid"])
    # Only one of the two people beyond the valley (the first spawn in the table), not every Marshal Dughan of the dump.
    seen = set()
    spawns = [c for c in spawns if c["id"] not in BEYOND or (c["id"] not in seen and not seen.add(c["id"]))]
    here_ids = {c["id"] for c in spawns}
    valley_ids = {c["id"] for c in spawns if c["id"] not in BEYOND}

    paths = {}
    for r in classicdb.rows(db, "creature_movement"):
        paths.setdefault(r["Id"], []).append(r)
    scripts = {}
    for r in classicdb.rows(db, "dbscripts_on_creature_movement"):
        scripts.setdefault(r["id"], []).append(r)
    emotes = dbc.Dbc(os.path.join(folder, "Emotes.dbc"))
    # A state emote (the table's type is above 0) is the animation it plays; 0 stops it. One-shots are not played.
    state_emote = {emotes.u32(i, 0): emotes.u32(i, 2) for i in range(len(emotes)) if emotes.u32(i, 4) > 0}
    tables = import_creatures.Tables(folder)
    morphs = {}

    def steps_of(script, c):
        out = []
        for r in sorted(scripts.get(script, []), key=lambda r: r["delay"]):
            at, command = r["delay"] / 1000.0, r["command"]
            if command == 25:
                out.append((at, "Run(%s)" % ("true" if r["datalong"] else "false")))
            elif command == 20 and r["datalong"] == 0:
                out.append((at, "Idle"))
            elif command == 20 and r["datalong"] == 1:
                out.append((at, "Roam(%s, %s)" % (float(r["datalong2"]), float((r["datalong3"] or 0) / 1000.0))))
            elif command == 1 and (r["datalong"] == 0 or r["datalong"] in state_emote):
                out.append((at, "Emote(%d)" % state_emote.get(r["datalong"], 0)))
            elif command == 23 and r["data_flags"] & 8:
                # Into the look of a model: a kind of its own for the display (the creature's own display is itself).
                display, t = r["datalong"], templates[c["id"]]
                if display and display not in morphs:
                    body = tables.body(display, t["Scale"], lambda slot, d: None, [])
                    morphs[display] = (t["Name"], body)
                if display and morphs[display][1]:
                    out.append((at, "Morph(%s)" % q("m%d" % display)))
        return [(d, st) for d, st in out]
    lines = []
    for c in sorted(spawns, key=lambda c: (c["id"], c["guid"])):
        out = ['creature: "c%d"' % c["id"], "at: (%s, %s)" % (round(c["position_x"], 2), round(c["position_y"], 2)),
               "facing: %s" % round((-math.degrees(c["orientation"])) % 360.0, 1)]
        wander = c["spawndist"] if c["MovementType"] == 1 else 0
        if wander != 5:
            out.append("wander: %s" % float(wander))
        out.append("height: Some(%s)" % round(c["position_z"], 1))
        respawn = (c["spawntimesecsmin"] + c["spawntimesecsmax"]) / 2.0
        if respawn != 300:
            out.append("respawn: Some(%s)" % float(respawn))
        walk = sorted(paths.get(c["guid"], []), key=lambda r: r["Point"]) if c["MovementType"] == 2 else []
        if walk:
            places = []
            for r in walk:
                place = ["at: (%s, %s)" % (round(r["PositionX"], 2), round(r["PositionY"], 2))]
                if r["WaitTime"]:
                    place.append("wait: %s" % round(r["WaitTime"] / 1000.0, 2))
                    if r["Orientation"] != 100:
                        place.append("facing: Some(%s)" % round((-math.degrees(r["Orientation"])) % 360.0, 1))
                steps = steps_of(r["ScriptId"], c) if r["ScriptId"] else []
                if steps:
                    place.append("steps: [%s]" % ", ".join("(%s, %s)" % (round(d, 2), st) for d, st in steps))
                places.append("(%s)" % ", ".join(place))
            out.append("path: [%s]" % ", ".join(places))
        lines.append("    (%s)," % ", ".join(out))
    header = ("// Where the original's Northshire people and animals stand (Northshire Valley, the Abbey, the Vineyards and Echo Ridge Mine), made by tools/import_northshire.py from the\n"
              "// CMaNGOS classic database (creature): heading in degrees clockwise from north, `wander` the yards a spawn that walks at random strolls, `height` where it stands (the floor of a building),\n"
              "// `respawn` the seconds it takes to come back and `path` the places a patrol walks to (north, west, seconds it stands there). Do not edit by hand.\n")
    with open(os.path.join(target, "classic_spawns.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header + "[\n" + "\n".join(lines) + "\n]\n")
    print("wrote", len(lines), "spawns of", len(here_ids), "kinds")
    # The looks the scripts change creatures into: kinds of their own that nobody stands as.
    kinds = ['    (id: %s, name: %s, body: %s),' % (q("m%d" % d), q(name), body) for d, (name, body) in sorted(morphs.items()) if body]
    with open(os.path.join(target, "classic_morphs.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write("// The looks that the waypoint scripts of Northshire's people turn them into (a peasant carrying logs), made by tools/import_northshire.py: kinds that nobody stands as.\n"
                "// Ids are `m<display>`. Do not edit by hand.\n[\n" + "\n".join(kinds) + "\n]\n")
    print("wrote", len(kinds), "looks")

    # The quests.
    start, end = {}, {}
    for r in classicdb.rows(db, "creature_questrelation"):
        start.setdefault(r["quest"], []).append(r["id"])
    for r in classicdb.rows(db, "creature_involvedrelation"):
        end.setdefault(r["quest"], []).append(r["id"])
    quests = {x["entry"]: x for x in classicdb.rows(db, "quest_template")}
    # Loot rows of the creatures that stand here and of the objects: which items can be had.
    loot_of = {}
    for r in classicdb.rows(db, "creature_loot_template"):
        loot_of.setdefault(r["entry"], set()).add(r["item"])
    chosen = {}
    for e, x in quests.items():
        giver = [i for i in start.get(e, []) if i in valley_ids]
        taker = [i for i in end.get(e, []) if i in here_ids]
        if not giver or not taker:
            continue
        if (x["RequiredClasses"] and not x["RequiredClasses"] & (1 << (PALADIN - 1))) or (x["RequiredRaces"] and not x["RequiredRaces"] & HUMAN):
            continue
        if any(x["ReqSpellCast%d" % n] for n in range(1, 5)) or any(x["ReqCreatureOrGOId%d" % n] < 0 for n in range(1, 5)):
            print("left out (spell or object goal):", e, x["Title"])
            continue
        chosen[e] = (giver[0], taker[0])
    # A quest needs its chain.
    while True:
        gone = [e for e in chosen if quests[e]["PrevQuestId"] > 0 and quests[e]["PrevQuestId"] not in chosen]
        if not gone:
            break
        for e in gone:
            print("left out (its chain is not here):", e, quests[e]["Title"])
            del chosen[e]

    def depth(e):
        return 0 if quests[e]["PrevQuestId"] <= 0 else 1 + depth(quests[e]["PrevQuestId"])

    # Objects that hold a quest item: pickups.
    object_loot = {}
    for r in classicdb.rows(db, "gameobject_loot_template"):
        object_loot.setdefault(r["item"], set()).add(r["entry"])
    objects = {g["entry"]: g for g in classicdb.rows(db, "gameobject_template") if g["type"] == 3}
    pick_items = {}
    out, texts = [], {}
    for e in sorted(chosen, key=lambda e: (depth(e), e)):
        x = quests[e]
        goals, skipped = [], []
        for n in range(1, 5):
            c, count = x["ReqCreatureOrGOId%d" % n], x["ReqCreatureOrGOCount%d" % n]
            if c > 0:
                if c not in valley_ids or "c%d" % c not in made:
                    skipped.append("kill %d" % c)
                goals.append('Kill("c%d", %d)' % (c, max(1, count)))
            item, number = x["ReqItemId%d" % n], x["ReqItemCount%d" % n]
            if item:
                name = Loot.name(item)
                if name not in items:
                    skipped.append("item %d" % item)
                got = item == x["SrcItemId"] or any(item in loot_of.get(templates[i]["LootId"], ()) for i in valley_ids)
                if not got and item in object_loot:
                    pick_items[item] = x["Title"]
                elif not got:
                    skipped.append("nobody drops %d" % item)
                goals.append('Collect("%s", %d)' % (name, max(1, number)))
        if skipped:
            print("left out (%s):" % ", ".join(skipped), e, x["Title"])
            continue
        level = x["QuestLevel"] if 0 < x["QuestLevel"] <= 60 else 0
        # CMaNGOS Quest::XPValue for quest levels 1 to 60 (the quests here are all of them), before the fall for a character who outgrew it.
        xp = int(math.ceil(x["RewMoneyMaxLevel"] / 0.6)) if level else 0
        reward = []
        if x["RewOrReqMoney"] > 0:
            reward.append("copper: %d" % x["RewOrReqMoney"])
        reward.append("xp: %d" % xp)
        choice = [(Loot.name(x["RewChoiceItemId%d" % n]), max(1, x["RewChoiceItemCount%d" % n])) for n in range(1, 7) if x["RewChoiceItemId%d" % n]]
        given = [(Loot.name(x["RewItemId%d" % n]), max(1, x["RewItemCount%d" % n])) for n in range(1, 5) if x["RewItemId%d" % n]]
        choice = [c for c in choice if c[0] in items]
        given = [c for c in given if c[0] in items]
        if len(choice) == 1:
            given, choice = choice + given, []
        if given:
            reward.append("items: [%s]" % ", ".join("(%s, %d)" % (q(i), n) for i, n in given))
        if choice:
            reward.append("choice: [%s]" % ", ".join("(%s, %d)" % (q(i), n) for i, n in choice))
        objective = words(x["Objectives"]) or x["Title"]
        fields = ["id: %s" % q("q%d" % e), "title: %s" % q(x["Title"]), "giver: %s" % q("c%d" % chosen[e][0]), "turn_in: %s" % q("c%d" % chosen[e][1]),
                  "details: %s" % q(words(x["Details"])), "objective: %s" % q(objective),
                  "progress: %s" % q(words(x["RequestItemsText"])), "completion: %s" % q(words(x["OfferRewardText"]) or "Well done, $N.")]
        if goals:
            fields.append("goals: [%s]" % ", ".join(goals))
        if x["SrcItemId"] and Loot.name(x["SrcItemId"]) in items:
            fields.append("start_items: [(%s, %d)]" % (q(Loot.name(x["SrcItemId"])), max(1, x["SrcItemCount"])))
        if level:
            fields.append("level: %d" % level)
        if x["PrevQuestId"] > 0:
            fields.append("requires: Some(%s)" % q("q%d" % x["PrevQuestId"]))
        fields.append("reward: (%s)" % ", ".join(reward))
        out.append("    (" + ", ".join(fields) + "),")
        texts[e] = x["Title"]
    # A chain whose earlier quest was left out is left out with it.
    kept = {int(re.match(r'\s+\(id: "q(\d+)"', l).group(1)) for l in out}
    out = [l for l in out if (m := re.search(r'requires: Some\("q(\d+)"\)', l)) is None or int(m.group(1)) in kept]
    header = ("// The quests of the original's Northshire (and the chains that lead out of it), made by tools/import_northshire.py from the CMaNGOS classic database (quest_template).\n"
              "// Ids are `q<entry>`, givers and takers are creatures `c<entry>`. The experience is the database's `RewMoneyMaxLevel` / 0.6 (CMaNGOS). Do not edit by hand.\n")
    with open(os.path.join(target, "classic_quests.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header + "[\n" + "\n".join(out) + "\n]\n")
    print("wrote", len(out), "quests:", ", ".join(texts[e] for e in sorted(texts) if e in kept))

    # What the people say: the text of the creature's gossip, else of its menu, else a greeting.
    npc_text = {r["ID"]: r for r in classicdb.rows(db, "npc_text")}
    by_guid = {r["npc_guid"]: r["textid"] for r in classicdb.rows(db, "npc_gossip")}
    menus = {}
    for r in classicdb.rows(db, "gossip_menu"):
        menus.setdefault(r["entry"], r["text_id"])
    said = {}
    for i in sorted(here_ids):
        t = templates[i]
        talks = t["NpcFlags"] != 0 or any(i in start.get(e, []) or i in end.get(e, []) for e in chosen)
        if not talks:
            continue
        text_id = next((by_guid[g] for g in guids[i] if g in by_guid), None) or menus.get(t["GossipMenuId"])
        row = npc_text.get(text_id)
        text = words((row["text0_0"] or row["text0_1"]) if row else "") or "Greetings, $N."
        said[i] = text
    with open(os.path.join(target, "classic_gossip.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write("// What the original's Northshire people say when talked to, made by tools/import_northshire.py (npc_text). Do not edit by hand.\n{\n"
                + "".join('    "c%d": (greeting: %s),\n' % (i, q(said[i])) for i in sorted(said)) + "}\n")
    print("wrote", len(said), "greetings")

    # The pickups.
    files = {l.strip().lower(): l.strip() for l in open(os.path.join(folder, "files.txt"), encoding="utf8", errors="replace")}
    table = dbc.Dbc(os.path.join(folder, "GameObjectDisplayInfo.dbc"))
    model = {table.u32(r, 0): table.string(r, 1).replace("/", "\\") for r in range(len(table))}
    props = []
    for item, title in sorted(pick_items.items()):
        holders = {e for e, t in objects.items() if t["data1"] in object_loot[item]}
        for s in classicdb.rows(db, "gameobject"):
            if s["id"] not in holders or s["map"] != 0 or not in_valley(s["position_x"], s["position_y"]):
                continue
            t = objects[s["id"]]
            stem = model.get(t["displayId"], "").rpartition(".")[0]
            path = files.get((stem + ".m2").lower())
            if not path:
                print("no model for", t["name"])
                continue
            props.append('    (model: r"%s", map: Some("Azeroth"), zone: Some("%s"), pickup: Some((item: %s, respawn: %s)), at: (%.2f, %.2f, %.2f), rotation: (0.0, %.1f, 0.0), scale: %.2f),\n' % (
                path, zones.areas[area(s["position_x"], s["position_y"])][1], q(Loot.name(item)), repr(float(s["spawntimesecsmin"] or PICK_RESPAWN)),
                s["position_x"], s["position_y"], s["position_z"], (math.degrees(s["orientation"]) + 180.0) % 360.0, t["size"] or 1.0))
    with open(os.path.join(target, "classic_pickups.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write("// Objects of the original's Northshire that hold a quest item, made by tools/import_northshire.py (gameobject, gameobject_loot_template): props with a pickup. Do not edit by hand.\n[\n" + "".join(props) + "]\n")
    print("wrote", len(props), "pickups")


if __name__ == "__main__":
    main()
