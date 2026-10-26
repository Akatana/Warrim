"""Makes the original population of the whole world (1.12.1) from the CMaNGOS classic database: `content/classic_spawns.ron` (where every creature of every map stands
and walks), `classic_quests.ron` (the quests), `classic_gossip.ron` (what the people say), `classic_pickups.ron` (the objects that quests ask the player to pick up or
use), `classic_goal_items.ron` (what using such an object gives), `classic_objects.ron` and `classic_morphs.ron` (the looks that waypoint scripts turn creatures into).

Every map the client has a WDT for is done (the two continents and the dungeons). The creatures of `classic_creatures.ron` that stand there are placed as the database places them
(heading, the yards they stroll when the spawn walks at random, the seconds to come back, the patrol of the spawn or of its kind). Left out: what only a game event puts out
(Hallow's End...), the spirit healers, and of a pool of spawns (rare ones and their placeholders) all but as many as the pool lets stand at once.

A quest is kept when a placed creature gives it and a placed creature takes it back, it is for no class (the game has none) nor for a profession, what it asks for can be had
(kills of placed creatures, items that a placed creature drops, a placed object holds or somebody sells, the use of a placed object, the walk to a place) and the chain before it is
kept. The quests that an object gives or takes back (a notice board, a statue) are given by a creature of its own kind, `g<entry>` (`classic_objects.ron`), and those
that an item begins carry `begins_with`. A quest of one race, or of a level to reach, carries that (`races` as the original's mask, `min_level`). Its experience is the server's own rule (CMaNGOS `Quest::XPValue`):
`RewMoneyMaxLevel` / 0.6, less as the character outgrows the quest (done in the game). The start item of a quest is given when it is taken. Left out:
escorts and other quests a script finishes, and what the quests' scripts do.

A patrol's places keep their wait, the heading the creature turns to while it waits (the database's 100 means none) and the waypoint script of the place
(dbscripts_on_creature_movement): run on and off, a stroll at random for a while, standing still for good, a state emote (as the animation of Emotes.dbc) and a morph into
another model, which is a creature kind `m<display>` of `classic_morphs.ron`. The script commands that do not act on the creature alone are left out.

Needs, beside db.sql, the client tables AreaTable, AreaTrigger, GameObjectDisplayInfo, Emotes and Map and files.txt and wdts.txt in the same folder (and those of import_creatures.py);
classic_creatures.ron, classic_creature_loot.ron and the item files must be made. `target/release/wowtool.exe` must be built (the zone of a pickup is read with it).

Usage: py -3 tools/import_world.py [folder with db.sql (default: tools/data)] [content folder]
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

# Never placed: the spirit healer of the graveyards.
NEVER = re.compile(r"spirit healer", re.I)
PICK_RESPAWN = 120.0
# The icon of what using an object gives (the item of "Torn Ledger Page").
GOAL_DISPLAY = 7629
# How many places hold an item at most for a quest to ask for it (a quest for herbs would have the whole world covered with them).
MOST_PLACES = 150


def words(text):
    """The text of the database in the form the game says it: line breaks, and the male one of `$gman:woman;`."""
    text = re.sub(r"\$[bB]", "\n", text or "")
    text = re.sub(r"\$[gG]([^:;]*):[^;]*;", r"\1", text)
    return text.replace("\r", "").strip()


def ids_of(path):
    return set(re.findall(r'\bid: "([^"]+)"', open(path, encoding="utf8").read()))


def drops_of(target):
    """The items (ids) that the creatures of the game drop or sell: {creature id: set of item ids}, from the files made by import_creatures.py."""
    tables = {}
    for line in open(os.path.join(target, "classic_creature_loot.ron"), encoding="utf8"):
        m = re.match(r'\s+"([^"]+)": .*', line)
        if m:
            tables[m.group(1)] = set(re.findall(r'\("(i\d+|[a-z_]+)", \(\d+, \d+\)', line))
    out = {}
    for line in open(os.path.join(target, "classic_creatures.ron"), encoding="utf8"):
        m = re.match(r'\s+\(id: "(c\d+)"', line)
        if not m:
            continue
        loot = re.search(r'loot: Some\("([^"]+)"\)', line)
        ware = re.search(r'vendor: \[([^\]]*)\]', line)
        out[m.group(1)] = (tables.get(loot.group(1), set()) if loot else set()) | (set(re.findall(r'"([^"]+)"', ware.group(1))) if ware else set())
    return out


def friends(target):
    """The creatures (entries) that a player of the Alliance can talk to, and those a player of the Horde can (not the ones that attack or fight back), and
    those that either can fight."""
    alliance, horde, foes = set(), set(), set()
    for line in open(os.path.join(target, "classic_creatures.ron"), encoding="utf8"):
        m = re.match(r'\s+\(id: "c(\d+)"', line)
        if not m:
            continue
        entry = int(m.group(1))
        mine = not re.search(r"(?:hostile|neutral): true", line)
        theirs = re.search(r"horde: Some\(\((true|false), (true|false)\)\)", line)
        if mine:
            alliance.add(entry)
        else:
            foes.add(entry)
        if (theirs is None and mine) or (theirs is not None and theirs.groups() == ("false", "false")):
            horde.add(entry)
        if theirs is not None and "true" in theirs.groups():
            foes.add(entry)
    return alliance, horde, foes


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    db = os.path.join(folder, "db.sql")
    zones = import_props.Zones(folder)
    maps_dbc = dbc.Dbc(os.path.join(folder, "Map.dbc"))
    with open(os.path.join(folder, "wdts.txt"), encoding="utf8") as f:
        have = {line.strip().split("\\")[2].lower() for line in f if line.strip()}
    maps = {maps_dbc.u32(i, 0): maps_dbc.string(i, 1) for i in range(len(maps_dbc))}
    maps = {i: name for i, name in maps.items() if name.lower() in have}
    made = ids_of(os.path.join(target, "classic_creatures.ron"))
    items = ids_of(os.path.join(target, "items.ron")) | ids_of(os.path.join(target, "classic_items.ron")) | ids_of(os.path.join(target, "classic_pieces.ron"))
    templates = {c["Entry"]: c for c in classicdb.rows(db, "creature_template")}

    # What a game event puts out is not there all year; of a pool only as many stand as it lets.
    events = {r["guid"] for r in classicdb.rows(db, "game_event_creature") if r["event"] > 0}
    limit = {r["entry"]: r["max_limit"] or 1 for r in classicdb.rows(db, "pool_template")}
    pools = {}
    for r in classicdb.rows(db, "pool_creature"):
        pools.setdefault(r["pool_entry"], []).append((-r["chance"], r["guid"]))
    pooled_out = set()
    for entry, members in pools.items():
        pooled_out.update(g for _, g in sorted(members)[limit.get(entry, 1):])

    # A spawn that has no creature of its own (id 0) stands for one of several: of its own list, else of its group's. Each is one of them, by turns.
    options, group_of, group_entries = {}, {}, {}
    for r in classicdb.rows(db, "creature_spawn_entry"):
        options.setdefault(r["guid"], []).append(r["entry"])
    groups = {r["Id"]: r for r in classicdb.rows(db, "spawn_group")}
    members = {}
    for r in classicdb.rows(db, "spawn_group_spawn"):
        if groups.get(r["Id"], {}).get("Type") == 0:
            members.setdefault(r["Id"], []).append(r["Guid"])
    for r in classicdb.rows(db, "spawn_group_entry"):
        group_entries.setdefault(r["Id"], []).append(r["Entry"])
    for gid, guid_list in members.items():
        for k, guid in enumerate(sorted(guid_list)):
            group_of[guid] = (gid, k)
        # A group that lets only so many stand at once.
        if groups[gid]["MaxCount"]:
            pooled_out.update(sorted(guid_list)[groups[gid]["MaxCount"]:])

    def kind_of(c):
        if c["id"]:
            return c["id"]
        if options.get(c["guid"]):
            return options[c["guid"]][c["guid"] % len(options[c["guid"]])]
        gid, k = group_of.get(c["guid"], (0, 0))
        return group_entries[gid][k % len(group_entries[gid])] if group_entries.get(gid) else 0

    # Where they stand.
    spawns, guids = [], {}
    for c in classicdb.rows(db, "creature"):
        c["id"] = kind_of(c)
        t = templates.get(c["id"])
        if c["map"] not in maps or t is None or "c%d" % c["id"] not in made or NEVER.search(t["Name"]) or c["guid"] in events or c["guid"] in pooled_out:
            continue
        spawns.append(c)
        guids.setdefault(c["id"], []).append(c["guid"])
    placed = {c["id"] for c in spawns}

    paths, by_kind = {}, {}
    for r in classicdb.rows(db, "creature_movement"):
        paths.setdefault(r["Id"], []).append(r)
    for r in classicdb.rows(db, "creature_movement_template"):
        if r["PathId"] == 0:
            by_kind.setdefault(r["Entry"], []).append(r)
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
        return out

    lines = []
    for c in sorted(spawns, key=lambda c: (maps[c["map"]], c["id"], c["guid"])):
        out = ['creature: "c%d"' % c["id"], 'map: Some("%s")' % maps[c["map"]], "at: (%s, %s)" % (round(c["position_x"], 2), round(c["position_y"], 2)),
               "facing: %s" % round((-math.degrees(c["orientation"])) % 360.0, 1)]
        wander = c["spawndist"] if c["MovementType"] == 1 else 0
        if wander != 5:
            out.append("wander: %s" % float(wander))
        out.append("height: Some(%s)" % round(c["position_z"], 1))
        respawn = (c["spawntimesecsmin"] + c["spawntimesecsmax"]) / 2.0
        if respawn != 300:
            out.append("respawn: Some(%s)" % float(respawn))
        walk = sorted(paths.get(c["guid"]) or by_kind.get(c["id"], []), key=lambda r: r["Point"]) if c["MovementType"] == 2 else []
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
    header = ("// Where the original's creatures stand, in every map of the game (made by tools/import_world.py from the CMaNGOS classic database, creature): the map, heading in degrees\n"
              "// clockwise from north, `wander` the yards a spawn that walks at random strolls, `height` where it stands (the floor of a building), `respawn` the seconds it takes to come back\n"
              "// and `path` the places a patrol walks to (north, west, seconds it stands there). Do not edit by hand.\n")
    spawn_header, creature_spawns = header, len(lines)
    # The looks the scripts change creatures into: kinds of their own that nobody stands as.
    kinds = ['    (id: %s, name: %s, body: %s),' % (q("m%d" % d), q(name), body) for d, (name, body) in sorted(morphs.items()) if body]
    with open(os.path.join(target, "classic_morphs.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write("// The looks that the waypoint scripts of the original's people turn them into (a peasant carrying logs), made by tools/import_world.py: kinds that nobody stands as.\n"
                "// Ids are `m<display>`. Do not edit by hand.\n[\n" + "\n".join(kinds) + "\n]\n")
    print("wrote", len(kinds), "looks")

    # The quests.
    start, end = {}, {}
    for r in classicdb.rows(db, "creature_questrelation"):
        start.setdefault(r["quest"], []).append(r["id"])
    for r in classicdb.rows(db, "creature_involvedrelation"):
        end.setdefault(r["quest"], []).append(r["id"])
    quests = {x["entry"]: x for x in classicdb.rows(db, "quest_template")}
    explore = {}
    triggers = dbc.Dbc(os.path.join(folder, "AreaTrigger.dbc"))
    trigger_row = {triggers.u32(i, 0): i for i in range(len(triggers))}
    for r in classicdb.rows(db, "areatrigger_involvedrelation"):
        explore.setdefault(r["quest"], []).append(r["id"])
    drops = drops_of(target)
    dropped = set()
    for cid in placed:
        dropped |= drops.get("c%d" % cid, set())
    # Objects: which hold an item (chests with a loot id) and which are used for a quest.
    object_loot = {}
    for r in classicdb.rows(db, "gameobject_loot_template"):
        object_loot.setdefault(r["item"], set()).add(r["entry"])
    objects = {g["entry"]: g for g in classicdb.rows(db, "gameobject_template")}
    object_events = {r["guid"] for r in classicdb.rows(db, "game_event_gameobject") if r["event"] > 0}
    places = {}
    object_options = {}
    for r in classicdb.rows(db, "gameobject_spawn_entry"):
        object_options.setdefault(r["guid"], []).append(r["entry"])
    for s in classicdb.rows(db, "gameobject"):
        if not s["id"] and object_options.get(s["guid"]):
            s["id"] = object_options[s["guid"]][s["guid"] % len(object_options[s["guid"]])]
        if s["map"] in maps and s["id"] in objects and s["guid"] not in object_events:
            places.setdefault(s["id"], []).append(s)
    files = {l.strip().lower(): l.strip() for l in open(os.path.join(folder, "files.txt"), encoding="utf8", errors="replace")}
    table = dbc.Dbc(os.path.join(folder, "GameObjectDisplayInfo.dbc"))
    model = {table.u32(r, 0): table.string(r, 1).replace("/", "\\") for r in range(len(table))}

    def file_of(entry):
        """The .m2 of an object that can be drawn as a pickup, else None."""
        stem = model.get(objects[entry]["displayId"], "").rpartition(".")[0]
        return files.get((stem + ".m2").lower()) if stem else None

    def holders(item):
        """The objects (entry) that hold an item and have a place and a model."""
        found = {e for e, t in objects.items() if t["type"] == 3 and t["data1"] in object_loot.get(item, ()) and places.get(e) and file_of(e)}
        return found if sum(len(places[e]) for e in found) <= MOST_PLACES else set()

    chosen, goal_objects, pick_items, texts = {}, {}, {}, {}
    alliance, horde, foes = friends(target)
    # An object that gives or takes back quests (type 2) is made a creature of its own, `g<entry>`, that nobody can see but all can talk to: it has its name and the marks.
    object_start, object_end = {}, {}
    for r in classicdb.rows(db, "gameobject_questrelation"):
        object_start.setdefault(r["quest"], []).append(r["id"])
    for r in classicdb.rows(db, "gameobject_involvedrelation"):
        object_end.setdefault(r["quest"], []).append(r["id"])
    talking = {o for v in list(object_start.values()) + list(object_end.values()) for o in v if o in objects and objects[o]["type"] == 2 and places.get(o)}
    side_ids = [{"c%d" % i for i in side & placed} | {"g%d" % o for o in talking} for side in (alliance, horde)]
    # A quest that an item begins (a letter, a note): the item is used to take it, and it is not offered by anybody.
    item_start = {i["startquest"]: i["entry"] for i in classicdb.rows(db, "item_template") if i["startquest"] and Loot.name(i["entry"]) in items}
    begins = {}
    # The quests that give a warlock's demon (the imp, the voidwalker, the succubus, the felhunter) and teach a hunter to tame beasts, and the chains before them, are kept though
    # they are for a class: the game has none, and the spells are tomes (`import_class_spells.py`) that these quests give.
    pet_tomes = {688: "tome_summon_imp", 697: "tome_summon_voidwalker", 712: "tome_summon_succubus", 691: "tome_summon_felhunter", 23356: "tome_tame_beast"}
    pet_quests, pending = set(), [e for e, x in quests.items() if x["RewSpell"] in pet_tomes]
    while pending:
        e = pending.pop()
        if e not in quests or e in pet_quests:
            continue
        pet_quests.add(e)
        pending.append(quests[e]["PrevQuestId"])
    spell_names = {s["Id"]: s["SpellName"] for s in classicdb.rows(db, "spell_template")}
    item_spell = {i["entry"]: i["spellid_1"] for i in classicdb.rows(db, "item_template")}
    for e, x in quests.items():
        if (x["RequiredClasses"] and e not in pet_quests) or x["RequiredSkill"]:
            continue
        # Somebody who gives it and somebody who takes it back, whom one side can talk to both.
        starts = ["c%d" % i for i in start.get(e, [])] + ["g%d" % o for o in object_start.get(e, [])]
        ends = ["c%d" % i for i in end.get(e, [])] + ["g%d" % o for o in object_end.get(e, [])]
        pairs = [(g, t) for side in side_ids for g in starts if g in side for t in ends if t in side]
        if not pairs and e in item_start:
            pairs = [(t, t) for side in side_ids for t in ends if t in side]
            if pairs:
                begins[e] = item_start[e]
        if pairs:
            chosen[e] = pairs[0]
    # A quest needs its chain.
    while True:
        gone = [e for e in chosen if quests[e]["PrevQuestId"] > 0 and quests[e]["PrevQuestId"] not in chosen]
        if not gone:
            break
        for e in gone:
            del chosen[e]

    def depth(e):
        return 0 if quests[e]["PrevQuestId"] <= 0 else 1 + depth(quests[e]["PrevQuestId"])

    def rewards_of(entries):
        """The items the given quests reward."""
        return {quests[e]["RewItemId%d" % n] for e in entries for n in range(1, 5)} | {quests[e]["RewChoiceItemId%d" % n] for e in entries for n in range(1, 7)}

    # What a quest asks for may be what another quest gives, if that quest is kept: the rounds go on until the quests kept do not change.
    rewarded = rewards_of(chosen)
    for _round in range(6):
        goal_objects, pick_items, texts = {}, {}, {}
        out, left_out = [], {}
        for e in sorted(chosen, key=lambda e: (depth(e), e)):
            x = quests[e]
            goals, skipped = [], []
            summons = tames = None
            # A taming rod: used on the beast it names (`Tame <name>`), which then counts as killed for the quest.
            rod = x["SrcItemId"] if e in pet_quests and x["RequiredClasses"] == 4 and spell_names.get(item_spell.get(x["SrcItemId"]), "").startswith("Tame ") else 0
            if rod:
                wild = spell_names[item_spell[rod]][len("Tame "):]
                beast = min((c for c, t in templates.items() if t["Name"] == wild and c in placed and c in foes), default=None)
                if beast is None:
                    skipped.append("beast " + wild)
                else:
                    goals.append('Kill("c%d", 1)' % beast)
                    tames = Loot.name(rod)
            for n in range(1, 5):
                c, count = x["ReqCreatureOrGOId%d" % n], x["ReqCreatureOrGOCount%d" % n]
                if c > 0:
                    # The demon of a binding is summoned by the quest's item and fought (a creature of its own that stands nowhere).
                    called = e in pet_quests and "c%d" % c in made and c not in placed and x["SrcItemId"]
                    if called:
                        summons = (Loot.name(x["SrcItemId"]), "c%d" % c)
                    elif c not in placed or c not in foes:
                        skipped.append("kill %d" % c)
                    goals.append('Kill("c%d", %d)' % (c, max(1, count)))
                elif c < 0:
                    # The use of an object: it gives a quest item of its own, which the quest then asks for.
                    obj = -c
                    if obj not in places or obj not in objects or not file_of(obj) or len(places[obj]) > MOST_PLACES:
                        skipped.append("object %d" % obj)
                    else:
                        name = words(x["ObjectiveText%d" % n]) or objects[obj]["name"]
                        goal_objects[obj] = name
                        goals.append('Collect("o%d", %d)' % (obj, max(1, count)))
                item, number = x["ReqItemId%d" % n], x["ReqItemCount%d" % n]
                if item and item == rod:
                    continue
                if item:
                    name = Loot.name(item)
                    if name not in items:
                        skipped.append("item %d" % item)
                    got = item == x["SrcItemId"] or name in dropped or item in rewarded
                    if not got and holders(item):
                        pick_items[item] = holders(item)
                    elif not got:
                        skipped.append("nobody has %d" % item)
                    goals.append('Collect("%s", %d)' % (name, max(1, number)))
            if e in begins:
                item = begins[e]
                if not (Loot.name(item) in dropped or item in rewarded):
                    if holders(item):
                        pick_items[item] = holders(item)
                    else:
                        skipped.append("nobody has %d" % item)
            if any(x["ReqSpellCast%d" % n] for n in range(1, 5)) and not goals:
                skipped.append("spell")
            # A walk to a place: the area trigger that completes the quest.
            reach = []
            for trigger in explore.get(e, []):
                row = trigger_row.get(trigger)
                if row is not None and triggers.u32(row, 1) in maps:
                    size = max(triggers.f32(row, 6), triggers.f32(row, 7)) / 2.0
                    reach.append('Reach(%s, (%s, %s), %s)' % (q(maps[triggers.u32(row, 1)]), round(triggers.f32(row, 2), 2), round(triggers.f32(row, 3), 2), round(max(triggers.f32(row, 5), size, 5.0), 1)))
            goals += reach[:1]
            if x["SpecialFlags"] & 2 and not goals:
                skipped.append("a script finishes it")
            if skipped:
                left_out[e] = (x["Title"], ", ".join(skipped))
                continue
            level = x["QuestLevel"] if 0 < x["QuestLevel"] <= 60 else 0
            # CMaNGOS Quest::XPValue for quest levels 1 to 60, before the fall for a character who outgrew it.
            xp = int(math.ceil(x["RewMoneyMaxLevel"] / 0.6)) if level else 0
            reward = []
            if x["RewOrReqMoney"] > 0:
                reward.append("copper: %d" % x["RewOrReqMoney"])
            reward.append("xp: %d" % xp)
            choice = [(Loot.name(x["RewChoiceItemId%d" % n]), max(1, x["RewChoiceItemCount%d" % n])) for n in range(1, 7) if x["RewChoiceItemId%d" % n]]
            given = [(Loot.name(x["RewItemId%d" % n]), max(1, x["RewItemCount%d" % n])) for n in range(1, 5) if x["RewItemId%d" % n]]
            choice = [c for c in choice if c[0] in items]
            given = [c for c in given if c[0] in items]
            # A spell that the quest teaches is the tome of it.
            if x["RewSpell"] in pet_tomes:
                given.append((pet_tomes[x["RewSpell"]], 1))
            if len(choice) == 1:
                given, choice = choice + given, []
            # The window has ten places.
            given, choice = given[:10], choice[:max(0, 10 - len(given))]
            if len(choice) == 1:
                given, choice = given + choice, []
            if given:
                reward.append("items: [%s]" % ", ".join("(%s, %d)" % (q(i), n) for i, n in given))
            if choice:
                reward.append("choice: [%s]" % ", ".join("(%s, %d)" % (q(i), n) for i, n in choice))
            objective = words(x["Objectives"]) or x["Title"]
            fields = ["id: %s" % q("q%d" % e), "title: %s" % q(x["Title"]), "giver: %s" % q(chosen[e][0]), "turn_in: %s" % q(chosen[e][1]),
                      "details: %s" % q(words(x["Details"])), "objective: %s" % q(objective),
                      "progress: %s" % q(words(x["RequestItemsText"])), "completion: %s" % q(words(x["OfferRewardText"]) or "Well done, $N.")]
            if goals:
                fields.append("goals: [%s]" % ", ".join(goals))
            if x["SrcItemId"] and Loot.name(x["SrcItemId"]) in items:
                fields.append("start_items: [(%s, %d)]" % (q(Loot.name(x["SrcItemId"])), max(1, x["SrcItemCount"])))
            if level:
                fields.append("level: %d" % level)
            if x["MinLevel"] > 1:
                fields.append("min_level: %d" % x["MinLevel"])
            if x["RequiredRaces"]:
                fields.append("races: %d" % x["RequiredRaces"])
            if x["PrevQuestId"] > 0:
                fields.append("requires: Some(%s)" % q("q%d" % x["PrevQuestId"]))
            if e in begins:
                fields.append("begins_with: Some(%s)" % q(Loot.name(begins[e])))
            if summons:
                fields.append("summons: Some((%s, %s))" % (q(summons[0]), q(summons[1])))
            if tames:
                fields.append("tames: Some(%s)" % q(tames))
            fields.append("reward: (%s)" % ", ".join(reward))
            out.append("    (" + ", ".join(fields) + "),")
            texts[e] = x["Title"]
        # A chain whose earlier quest was left out is left out with it (again and again, to the end of the chain).
        while True:
            kept = {int(re.match(r'\s+\(id: "q(\d+)"', l).group(1)) for l in out}
            after = [l for l in out if (m := re.search(r'requires: Some\("q(\d+)"\)', l)) is None or int(m.group(1)) in kept]
            if len(after) == len(out):
                break
            out = after
        now = rewards_of({int(re.match(r'\s+\(id: "q(\d+)"', l).group(1)) for l in out})
        if now == rewarded:
            break
        rewarded = now
    # The objects of the kept quests: a kind of creature of each (a rabbit too small to see, with the name of the object) and where it stands.
    used = sorted({int(i) for l in out for i in re.findall(r'(?:giver|turn_in): "g(\d+)"', l)})
    rabbit = next(re.search(r'body: (Beast\(model: "[^"]+", skins: \[[^\]]*\]), scale: [0-9.]+\)', l).group(1) for l in open(os.path.join(target, "classic_creatures.ron"), encoding="utf8") if '(id: "c721", name: "Rabbit"' in l)
    kinds = ['    (id: "g%d", name: %s, body: %s, scale: 0.01), gossip: Some((greeting: %s))),' % (o, q(objects[o]["name"]), rabbit, q(objects[o]["name"])) for o in used]
    with open(os.path.join(target, "classic_objects.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write("// The objects of the original that give or take back quests (a notice board, a statue, a corpse), made by tools/import_world.py: each is a creature kind `g<entry>` that\n"
                "// is too small to see, has the object's name and can be talked to. Do not edit by hand.\n[\n" + "\n".join(kinds) + "\n]\n")
    for o in used:
        for spot in places[o]:
            lines.append('    (creature: "g%d", map: Some("%s"), at: (%s, %s), facing: %s, wander: 0.0, height: Some(%s)),' % (o, maps[spot["map"]], round(spot["position_x"], 2), round(spot["position_y"], 2), round((-math.degrees(spot["orientation"])) % 360.0, 1), round(spot["position_z"], 1)))
    with open(os.path.join(target, "classic_spawns.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(spawn_header + "[\n" + "\n".join(lines) + "\n]\n")
    print("wrote", len(lines), "spawns of", len(placed), "kinds in", len({c["map"] for c in spawns}), "maps;", len(lines) - creature_spawns, "of them of", len(used), "objects")
    header = ("// The quests of the original (and the chains that lead through them), made by tools/import_world.py from the CMaNGOS classic database (quest_template).\n"
              "// Ids are `q<entry>`, givers and takers are creatures `c<entry>`. The experience is the database's `RewMoneyMaxLevel` / 0.6 (CMaNGOS). Do not edit by hand.\n")
    with open(os.path.join(target, "classic_quests.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header + "[\n" + "\n".join(out) + "\n]\n")
    print("wrote", len(out), "quests of", len(quests), ";", len(chosen), "had people to give and take them; left out:", len(left_out))
    reasons = {}
    for _, why in left_out.values():
        reasons[why.split(" ")[0]] = reasons.get(why.split(" ")[0], 0) + 1
    print("  why:", reasons)
    for e, (title, why) in sorted(left_out.items())[:400]:
        if os.environ.get("WHY") and os.environ["WHY"] in why:
            print("   ", e, title, "|", why)

    # What the people say: the text of the creature's gossip, else of its menu, else a greeting. Who talks: whoever has an NPC flag or a quest to give or take.
    npc_text = {r["ID"]: r for r in classicdb.rows(db, "npc_text")}
    by_guid = {r["npc_guid"]: r["textid"] for r in classicdb.rows(db, "npc_gossip")}
    menus = {}
    for r in classicdb.rows(db, "gossip_menu"):
        menus.setdefault(r["entry"], r["text_id"])
    kept_ids = {int(i) for l in out for i in re.findall(r'(?:giver|turn_in): "c(\d+)"', l)}
    said = {}
    for i in sorted(placed):
        t = templates[i]
        if not (t["NpcFlags"] or i in kept_ids):
            continue
        text_id = next((by_guid[g] for g in guids[i] if g in by_guid), None) or menus.get(t["GossipMenuId"])
        row = npc_text.get(text_id)
        said[i] = words((row["text0_0"] or row["text0_1"]) if row else "") or "Greetings, $N."
    with open(os.path.join(target, "classic_gossip.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write("// What the original's people say when talked to, made by tools/import_world.py (npc_text). Do not edit by hand.\n{\n"
                + "".join('    "c%d": (greeting: %s),\n' % (i, q(said[i])) for i in sorted(said)) + "}\n")
    print("wrote", len(said), "greetings")

    # The pickups: the objects that hold an item a kept quest asks for, and the objects it asks to be used.
    kept_text = "\n".join(out)
    wanted_items = {Loot.name(i): i for i in pick_items if '"%s"' % Loot.name(i) in kept_text}
    wanted_objects = {o: n for o, n in goal_objects.items() if '"o%d"' % o in kept_text}
    props, seen = [], set()

    def pickup(spot, give, entry):
        stem = file_of(entry)
        name = maps[spot["map"]]
        zone = zones.areas[zones_area(name, spot)][1] if name in ("Azeroth", "Kalimdor") and zones_area(name, spot) in zones.areas else ""
        props.append('    (model: r"%s", map: Some("%s"),%s pickup: Some((item: %s, respawn: %s)), at: (%.2f, %.2f, %.2f), rotation: (0.0, %.1f, 0.0), scale: %.2f),\n' % (
            stem, name, ' zone: Some("%s"),' % zone if zone else "", q(give), repr(float(min(900.0, max(30.0, spot["spawntimesecsmin"] or PICK_RESPAWN)))),
            spot["position_x"], spot["position_y"], spot["position_z"], (math.degrees(spot["orientation"]) + 180.0) % 360.0, objects[entry]["size"] or 1.0))

    def zones_area(name, spot):
        fx, fz = -spot["position_y"] + import_props.HALF * import_props.TILE, -spot["position_x"] + import_props.HALF * import_props.TILE
        tx, ty = int(fx // import_props.TILE), int(fz // import_props.TILE)
        areas = zones.chunks(name, tx, ty)
        if not areas:
            return 0
        area = areas[min(int((fz - ty * import_props.TILE) / import_props.CHUNK), 15) * 16 + min(int((fx - tx * import_props.TILE) / import_props.CHUNK), 15)]
        while area in zones.areas and zones.areas[area][0]:
            area = zones.areas[area][0]
        return area

    for name, item in sorted(wanted_items.items()):
        for entry in sorted(pick_items[item]):
            for spot in places[entry]:
                pickup(spot, name, entry)
    for obj in sorted(wanted_objects):
        for spot in places[obj]:
            pickup(spot, "o%d" % obj, obj)
    with open(os.path.join(target, "classic_pickups.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write("// Objects of the original that hold a quest item or that a quest asks to use, made by tools/import_world.py (gameobject, gameobject_loot_template): props with a pickup. Do not edit by hand.\n[\n" + "".join(props) + "]\n")
    print("wrote", len(props), "pickups")
    goal_items = ['    (id: %s, name: %s, display: %d, quest: true, stack: 20),\n' % (q("o%d" % o), q(n), GOAL_DISPLAY) for o, n in sorted(wanted_objects.items())]
    with open(os.path.join(target, "classic_goal_items.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write("// What using an object that a quest asks to be used gives (a quest item of its own, `o<object entry>`), made by tools/import_world.py. Do not edit by hand.\n[\n" + "".join(goal_items) + "]\n")
    print("wrote", len(goal_items), "goal items")


if __name__ == "__main__":
    main()
