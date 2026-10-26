"""Makes `content/classic_trainers.ron`: what the trade trainers of the original game (1.12.1) teach, from the CMaNGOS classic database (creature_template with
TrainerType 2, npc_trainer and npc_trainer_template). Each lesson is a recipe of `classic_recipes.ron` (the trainer's own spell is a "learn" spell that
teaches it: its effect 36 names the recipe) with its price in copper, the skill rank it asks for and the level.

The steps of the trades themselves (apprentice to artisan: the spells with effect 44) are lessons too (`step`). Left out: what is no recipe of the game (the abilities of the class trainers), and trainers the game has no creature for.

Usage: py -3 tools/import_trainers.py [folder with db.sql (default: tools/data)] [content folder]
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb

# The skill lines of the original -> the game's skills (import_recipes.py has the same).
LINES = {164: "Blacksmithing", 165: "Leatherworking", 171: "Alchemy", 185: "Cooking", 129: "FirstAid", 197: "Tailoring", 202: "Engineering", 333: "Enchanting", 186: "Mining",
         182: "Herbalism", 356: "Fishing", 393: "Skinning"}
# The gathering trades can be learned from the first level (the original's tables say so too for the apprentice step of these three).
FROM_THE_FIRST = {"Mining", "Herbalism", "Skinning"}


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    sql = os.path.join(folder, "db.sql")
    recipes = {int(x) for x in re.findall(r"\(spell: (\d+),", open(os.path.join(target, "classic_recipes.ron"), encoding="utf8").read())}
    made = set(re.findall(r'\bid: "(c\d+)"', open(os.path.join(target, "classic_creatures.ron"), encoding="utf8").read()))
    spells = {s["Id"]: s for s in classicdb.rows(sql, "spell_template")}

    def step_of(spell):
        """(the trade, the step) that a spell of a trainer teaches: its effect 44 (a step of a skill: apprentice 1 to artisan 4)."""
        s = spells.get(spell)
        for n in (1, 2, 3):
            if s and s["Effect%d" % n] == 44 and s["EffectMiscValue%d" % n] in LINES:
                return LINES[s["EffectMiscValue%d" % n]], s["EffectBasePoints%d" % n] + 1
        return None

    def resolve(spell):
        if spell in recipes:
            return [spell]
        s = spells.get(spell)
        return [s["EffectTriggerSpell%d" % n] for n in (1, 2, 3) if s and s["Effect%d" % n] == 36 and s["EffectTriggerSpell%d" % n] in recipes]

    own, template = {}, {}
    for r in classicdb.rows(sql, "npc_trainer"):
        own.setdefault(r["entry"], []).append(r)
    for r in classicdb.rows(sql, "npc_trainer_template"):
        template.setdefault(r["entry"], []).append(r)
    out = []
    for c in sorted(classicdb.rows(sql, "creature_template"), key=lambda c: c["Entry"]):
        if c["TrainerType"] != 2 or "c%d" % c["Entry"] not in made:
            continue
        lessons, seen = [], set()
        for r in own.get(c["Entry"], []) + template.get(c["TrainerTemplateId"], []):
            step = step_of(r["spell"])
            for spell in [r["spell"]] if step else resolve(r["spell"]):
                if spell in seen:
                    continue
                seen.add(spell)
                fields = ["spell: %d" % spell, "cost: %d" % r["spellcost"]]
                if r["reqskill"] in LINES and r["reqskillvalue"]:
                    fields.append("skill: Some((%s, %d))" % (LINES[r["reqskill"]], r["reqskillvalue"]))
                if r["reqlevel"] > 1 and not (step and step[0] in FROM_THE_FIRST and step[1] == 1):
                    fields.append("level: %d" % r["reqlevel"])
                if step:
                    fields.append("step: Some((%s, %d))" % step)
                lessons.append("(" + ", ".join(fields) + ")")
        if lessons:
            out.append('    "c%d": [%s],  // %s' % (c["Entry"], ", ".join(lessons), c["Name"]))
    header = ("// What the trade trainers of the original game teach (the recipes of `classic_recipes.ron`), made by tools/import_trainers.py from the CMaNGOS classic database:\n"
              "// for each creature kind the recipe's spell, the price in copper, the skill and rank it asks for and the level. Do not edit by hand.\n")
    with open(os.path.join(target, "classic_trainers.ron"), "w", encoding="utf8", newline="\n") as f:
        f.write(header + "{\n" + "\n".join(out) + "\n}\n")
    print("wrote", len(out), "trainers")


if __name__ == "__main__":
    main()
