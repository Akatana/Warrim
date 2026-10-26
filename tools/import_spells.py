"""Makes `content/classic_spells.ron`: every spell of the original game (1.12.1), from the CMaNGOS classic database (`spell_template`).

Same sources as import_items.py; also needs SpellCastTimes.dbc, SpellIcon.dbc and SpellRange.dbc extracted into the data folder.

Usage: py -3 tools/import_spells.py [folder with db.sql and the .dbc files (default: tools/data)] [content folder]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb
import dbc
import spelltext
from import_items import f, q


def table(folder, name, value):
    d = dbc.Dbc(os.path.join(folder, name + ".dbc"))
    return {d.u32(r, 0): value(d, r) for r in range(len(d))}


def effect_text(s, n, radius):
    """One of the three effects of a spell as RON; what is the default is left out."""
    kind = s["Effect%d" % n]
    if not kind:
        return None
    base, sides = s["EffectBasePoints%d" % n], s["EffectDieSides%d" % n]
    low = base + 1
    high = base + sides if sides > 1 else low
    out = ["kind: %d" % kind]
    if s["EffectApplyAuraName%d" % n]:
        out.append("aura: %d" % s["EffectApplyAuraName%d" % n])
    if low or high:
        out.append("low: %d" % low)
        out.append("high: %d" % high)
    if s["EffectRealPointsPerLevel%d" % n]:
        out.append("per_level: %s" % f(s["EffectRealPointsPerLevel%d" % n]))
    if s["EffectRadiusIndex%d" % n]:
        out.append("radius: %s" % f(radius.get(s["EffectRadiusIndex%d" % n], 0)))
    if s["EffectAmplitude%d" % n]:
        out.append("period: %d" % s["EffectAmplitude%d" % n])
    if s["EffectMiscValue%d" % n]:
        out.append("misc: %d" % s["EffectMiscValue%d" % n])
    if s["EffectTriggerSpell%d" % n]:
        out.append("trigger: %d" % s["EffectTriggerSpell%d" % n])
    if s["EffectItemType%d" % n]:
        out.append("item: %d" % s["EffectItemType%d" % n])
    if s["EffectChainTarget%d" % n]:
        out.append("chain: %d" % s["EffectChainTarget%d" % n])
    if s["EffectImplicitTargetA%d" % n] or s["EffectImplicitTargetB%d" % n]:
        out.append("targets: (%d, %d)" % (s["EffectImplicitTargetA%d" % n], s["EffectImplicitTargetB%d" % n]))
    return "(" + ", ".join(out) + ")"


def entry_text(s, spells, cast, icons, ranges, duration, radius):
    out = ['id: "s%d"' % s["Id"], "spell: %d" % s["Id"]]
    if s["SpellName"]:
        out.append("name: %s" % q(s["SpellName"]))
    if s["Rank1"]:
        out.append("rank: %s" % q(s["Rank1"]))
    icon = icons.get(s["SpellIconID"], "")
    if icon:
        out.append("icon: %s" % q(icon.rsplit("\\", 1)[-1]))
    for key, name in (("School", "school"), ("Mechanic", "mechanic"), ("Dispel", "dispel"), ("SpellLevel", "level"), ("BaseLevel", "base_level"), ("MaxLevel", "max_level"),
                      ("PowerType", "power"), ("ManaCost", "cost"), ("ManaCostPercentage", "cost_percent"), ("RecoveryTime", "cooldown"), ("Attributes", "attributes"),
                      ("SpellFamilyName", "family"), ("SpellFamilyFlags", "family_flags"), ("StackAmount", "stack")):
        if s[key]:
            out.append("%s: %d" % (name, s[key]))
    if cast.get(s["CastingTimeIndex"], 0) > 0:
        out.append("cast: %d" % cast[s["CastingTimeIndex"]])
    if ranges.get(s["RangeIndex"]):
        out.append("range: %s" % f(ranges[s["RangeIndex"]]))
    if duration.get(s["DurationIndex"], 0) > 0:
        out.append("duration: %d" % duration[s["DurationIndex"]])
    if s["ProcFlags"]:
        out.append("proc: (%d, %d, %d)" % (s["ProcFlags"], s["ProcChance"], s["ProcCharges"]))
    if s["EquippedItemClass"] >= 0:
        out.append("equipped: Some((%d, %d))" % (s["EquippedItemClass"], s["EquippedItemSubClassMask"]))
    reagents = [(s["Reagent%d" % n], s["ReagentCount%d" % n]) for n in range(1, 9) if s["Reagent%d" % n] > 0]
    if reagents:
        out.append("reagents: [%s]" % ", ".join("(%d, %d)" % r for r in reagents))
    effects = [e for e in (effect_text(s, n, radius) for n in (1, 2, 3)) if e]
    if effects:
        out.append("effects: [%s]" % ", ".join(effects))
    text = spells.describe(s["Id"])
    if text:
        out.append("text: %s" % q(text))
    return "(" + ", ".join(out) + ")"


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    spells = spelltext.Spells(folder)
    cast = table(folder, "SpellCastTimes", lambda d, r: d.i32(r, 1))
    icons = table(folder, "SpellIcon", lambda d, r: d.string(r, 1))
    ranges = table(folder, "SpellRange", lambda d, r: d.f32(r, 2))
    rows = sorted(classicdb.rows(os.path.join(folder, "db.sql"), "spell_template"), key=lambda s: s["Id"])
    lines = [entry_text(s, spells, cast, icons, ranges, spells.duration, spells.radius) for s in rows]
    header = ("// Every spell of the original game (1.12.1), made by tools/import_spells.py from the CMaNGOS classic database (spell_template): what the original's\n"
              "// tables say of its cost, timing, school, effects and text. Do not edit by hand. Ids are `s<Id>`; `spell` is the row of Spell.dbc (pictures and sounds).\n"
              "// `cast`, `cooldown`, `duration` and `period` are milliseconds, `range` and `radius` yards; an effect's `low`..`high` are its points (dice included).\n")
    with open(os.path.join(target, "classic_spells.ron"), "w", encoding="utf8", newline="\n") as out:
        out.write(header + "[\n" + "".join("    %s,\n" % l for l in lines) + "]\n")
    print("wrote", len(lines), "spells")


if __name__ == "__main__":
    main()
