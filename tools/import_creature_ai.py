"""Makes `content/classic_creature_ai.ron`: how the creatures of `classic_creatures.ron` fight, from the CMaNGOS classic database.

What is taken, for every creature the player can fight (hostile or neutral): its mana, the seconds between its swings, the damage of its ranged weapon (what
Shoot and Throw do), who it calls to help and its faction, and its EventAI script (`creature_ai_scripts`: the events that make it cast, shoot, heal, buff, summon,
charge, flee, call for help and keep its distance) with the old-style spell lists of `creature_spell_list` turned into the same events. Events and actions the game
does not play (speech, emotes, quests, waypoints, instance data) are left out. The game plays the rest with the rules of CMaNGOS (`brain.rs`).

Needs db.sql in the data folder and the files `classic_creatures.ron` and `classic_spells.ron` of the content folder.

Usage: py -3 tools/import_creature_ai.py [folder with db.sql (default: tools/data)] [content folder]
"""
import os
import re
import sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb

# Events the game plays: timer in combat, hp, mana, aggro, death, range, spawned, target hp, target casting, friendly hp, friendly missing buff, target mana,
# aura, target aura, missing aura, target missing aura, generic timer, select attacking target, facing target.
EVENTS = {0, 2, 3, 4, 6, 9, 11, 12, 13, 14, 16, 18, 23, 24, 27, 28, 29, 32, 33}
# Actions: cast, summon, auto attack, combat movement, set phase, inc phase, evade, flee for assist, remove aura, ranged movement, random phases, die,
# call for help, force despawn, ranged mode.
ACTIONS = {11, 20, 21, 22, 23, 24, 25, 28, 29, 30, 31, 37, 39, 41, 57}
SUMMONS = {12, 32}
# What makes a creature worth a script: a cast, a summon, or a change of how it fights (not only its phases).
DOES = {11, 12, 32, 20, 21, 24, 25, 29, 37, 39, 57}
NO_CALL_ASSIST, NO_AGGRO_ON_SIGHT = 0x800, 0x2
ASSIST_RADIUS = 10.0
# Spell list TargetId -> the EventAI target code (0 self, 1 victim, 12 friend).
LIST_TARGET = {0: 1, 1: 1, 2: 0, 3: 12, 4: 12, 5: 12, 6: 12, 7: 1}
FRIENDS_RADIUS = 30


def f(x):
    return ("%.2f" % x).rstrip("0").rstrip(".")


def ron_event(on, p, flags, chance, off, acts):
    while p and p[-1] == 0:
        p = p[:-1]
    out = ["on: %d" % on]
    if p:
        out.append("p: [%s]" % ", ".join(str(v) for v in p))
    if flags:
        out.append("flags: %d" % flags)
    if chance != 100:
        out.append("chance: %d" % chance)
    if off:
        out.append("off: %d" % off)
    out.append("acts: [%s]" % ", ".join("(%d, %d, %d, %d)" % a for a in acts))
    return "(" + ", ".join(out) + ")"


def main():
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "data")
    content = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "content")
    sql = os.path.join(folder, "db.sql")
    fighters = {}
    for l in open(os.path.join(content, "classic_creatures.ron"), encoding="utf8"):
        m = re.search(r'\(id: "c(\d+)"', l)
        if m and ("hostile: true" in l or "neutral: true" in l):
            fighters[int(m[1])] = True
    everyone = {int(m[1]) for m in re.finditer(r'\(id: "c(\d+)"', open(os.path.join(content, "classic_creatures.ron"), encoding="utf8").read())}
    spells = {}
    for l in open(os.path.join(content, "classic_spells.ron"), encoding="utf8"):
        m = re.search(r'\(id: "s(\d+)", spell: \d+, name: "[^"]*"', l)
        if m:
            r = re.search(r"[ (]range: ([\d.]+)", l)
            spells[int(m[1])] = float(r[1]) if r else 0.0

    creatures = {c["Entry"]: c for c in classicdb.rows(sql, "creature_template") if c["Entry"] in fighters}
    scripts = defaultdict(list)
    for r in classicdb.rows(sql, "creature_ai_scripts"):
        if r["creature_id"] in creatures:
            scripts[r["creature_id"]].append(r)
    lists = defaultdict(list)
    for r in classicdb.rows(sql, "creature_spell_list"):
        lists[r["Id"]].append(r)
    targeting = {r["Id"]: r for r in classicdb.rows(sql, "creature_spell_targeting")}

    lines, counts = [], defaultdict(int)
    for entry in sorted(creatures):
        c = creatures[entry]
        events = []
        for r in sorted(scripts[entry], key=lambda r: r["id"]):
            if r["event_type"] not in EVENTS:
                continue
            acts = []
            for n in (1, 2, 3):
                kind, a = r["action%d_type" % n], [r["action%d_param%d" % (n, k)] for k in (1, 2, 3)]
                if kind == 11 and a[0] in spells:
                    acts.append((11, a[0], a[1], a[2]))
                elif kind in SUMMONS and a[0] in everyone:
                    acts.append((12, a[0], a[1], a[2] if kind == 12 else 0))
                elif kind in ACTIONS and kind != 11:
                    acts.append((kind, *a))
            if acts:
                events.append(ron_event(r["event_type"], [r["event_param%d" % k] for k in range(1, 7)], r["event_flags"], r["event_chance"], r["event_inverse_phase_mask"], acts))
                counts["event"] += 1
        # The old spell lists: a timer for each spell (the support ones wait for a hurt friend), and a caster stands back at the range of its main spell.
        main_range = 0.0
        for row in sorted(lists.get(c["SpellList"], []), key=lambda r: r["Position"]) if c["SpellList"] else []:
            if row["SpellId"] not in spells:
                continue
            flags = 0x401
            cast = (11, row["SpellId"], LIST_TARGET.get(row["TargetId"], 1), 0x100 if row["Flags"] & 2 else 0)
            if 200 <= row["TargetId"] < 300 and row["TargetId"] in targeting:
                events.append(ron_event(14, [targeting[row["TargetId"]]["Param1"], FRIENDS_RADIUS, row["RepeatMin"], row["RepeatMax"], 1], flags, 100, 0, [(11, row["SpellId"], 12, 0)]))
            else:
                if row["Flags"] & 2:
                    main_range = max(main_range, spells[row["SpellId"]])
                events.append(ron_event(0, [row["InitialMin"], row["InitialMax"], row["RepeatMin"], row["RepeatMax"]], flags, 100, 0, [cast]))
            counts["list"] += 1
        if main_range > 0:
            events.append(ron_event(4, [], 0, 100, 0, [(57, 1, int(max(8, min(30, main_range * 0.8))), 0)]))
        useful = any(re.search(r"\((?:%s), " % "|".join(str(k) for k in DOES), e[e.index("acts:"):]) for e in events)
        out = []
        mana = (c["MinLevelMana"] + c["MaxLevelMana"]) / 2.0
        if mana > 0:
            out.append("mana: %s" % f(mana))
        if c["MeleeBaseAttackTime"] not in (0, 2000):
            out.append("swing: %s" % f(c["MeleeBaseAttackTime"] / 1000.0))
        shot = (c["MinRangedDmg"] + c["MaxRangedDmg"]) / 2.0 * (c["DamageMultiplier"] or 1)
        if useful and shot > 0:
            out.append("shot: %s" % f(round(shot, 1)))
        radius = 0.0 if c["ExtraFlags"] & NO_CALL_ASSIST else (float(c["CallForHelp"]) if c["CallForHelp"] > 0 else ASSIST_RADIUS)
        if radius != ASSIST_RADIUS:
            out.append("assist: %s" % f(radius))
        if c["ExtraFlags"] & NO_AGGRO_ON_SIGHT:
            out.append("calm: true")
        out.append("faction: %d" % c["Faction"])
        if useful:
            out.append("events: [%s]" % ", ".join(events))
            counts["scripted"] += 1
        elif mana == 0 and len(out) == 1 and radius == ASSIST_RADIUS:
            # Nothing to say but its faction: kept, a pack still pulls together.
            pass
        lines.append('    "c%d": (%s),' % (entry, ", ".join(out)))
    header = ("// How the creatures of `classic_creatures.ron` fight, from the CMaNGOS classic database (EventAI scripts and spell lists, mana, swing time, who they call\n"
              "// to help), made by tools/import_creature_ai.py. An event is `on` (the EventAI event type) with its parameters `p`, the EventAI `flags`, the percent `chance`, the\n"
              "// phases it is `off` in, and `acts` (EventAI action type and three parameters). `brain.rs` plays them. Do not edit by hand.\n")
    with open(os.path.join(content, "classic_creature_ai.ron"), "w", encoding="utf8", newline="\n") as out:
        out.write(header + "{\n" + "\n".join(lines) + "\n}\n")
    print("wrote", len(lines), "creatures,", counts["scripted"], "with a script,", counts["event"], "events,", counts["list"], "spell list entries")


if __name__ == "__main__":
    main()
