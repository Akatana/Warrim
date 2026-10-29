"""The description of a spell as the original client writes it (`Spell.dbc` text with its $s1, $d, $o1 ... tokens filled in), for the tooltips of items."""
import os
import re

import classicdb
import dbc

# Spell.dbc fields (1.12): 30 duration, 25 proc chance, 26 charges, 39 stack, 61-63 effect, 64-66 die sides, 67-69 base dice, 76-78 base points,
# 88-90 radius, 91-93 aura, 94-96 amplitude (ms), 100-102 chain targets, 120 name, 138 description, 147 aura description.
DURATION, PROC, CHARGES, STACK = 30, 25, 26, 39
EFFECT, DIE, DICE, BASE, RADIUS, AURA, AMPLITUDE, CHAIN = 61, 64, 67, 76, 88, 91, 94, 100
NAME, DESCRIPTION, AURA_TEXT = 120, 138, 147


# What a worn item's spell may do, by aura (`Equip(aura, misc, amount)` in the items): 13 damage of the schools of `misc`, 135 healing, 99 and 124 attack
# power (melee, ranged), 102 131 and 180 the same and the damage of spells against the types of creatures of `misc`, 52 and 71 crit (melee and ranged, spells), 54 and 55 hit (weapons, spells), 49 dodge, 47 parry, 51 block, 158 block value,
# 85 mana and 161 health every 5 seconds, 138 and 141 melee and ranged attack speed in percent, 31 the running speed in percent, 30 skill (misc: the skill line),
# 154 stealth, 130 the speed of mounts in percent, 10 threat in percent, 59 the damage of blows against the types of creatures of `misc`.
EQUIPPED = {13, 135, 99, 124, 52, 71, 54, 55, 49, 47, 51, 158, 85, 161, 138, 141, 31, 102, 131, 180, 30, 154, 130, 10, 59}


UNTRUSTED = object()


def seconds_text(ms):
    s = ms / 1000.0
    if s < 60:
        return ("%g sec" % s)
    if s < 3600:
        m = s / 60.0
        return ("%g min" % m) if m == int(m) else ("%g sec" % s)
    h = s / 3600.0
    return "%g %s" % (h, "hour" if h == 1 else "hours")


class Spells:
    def __init__(self, folder):
        self.spell = dbc.Dbc(os.path.join(folder, "Spell.dbc"))
        self.rows = {self.spell.u32(r, 0): r for r in range(len(self.spell))}
        d = dbc.Dbc(os.path.join(folder, "SpellDuration.dbc"))
        self.duration = {d.u32(r, 0): d.i32(r, 1) for r in range(len(d))}
        d = dbc.Dbc(os.path.join(folder, "SpellRadius.dbc"))
        self.radius = {d.u32(r, 0): d.f32(r, 1) for r in range(len(d))}
        # A mount spell's aura names the creature (its entry in creature_template) whose first display (CreatureDisplayInfo.dbc) is the steed.
        sql = os.path.join(folder, "db.sql")
        self.steeds = {c["Entry"]: c["ModelId1"] for c in classicdb.rows(sql, "creature_template") if c["ModelId1"]} if os.path.exists(sql) else {}
        # The values that only the database names ($e, $q, $b, $f, $i, $v).
        self.extra = {r["Id"]: r for r in classicdb.rows(sql, "spell_template")} if os.path.exists(sql) else {}

    def values(self, row, n):
        """(lowest, highest) of the n-th effect (1 to 3): the base points and the dice."""
        s = self.spell
        base, die = s.i32(row, BASE + n - 1), s.i32(row, DIE + n - 1)
        low = base + 1
        high = base + die if die > 1 else low
        return abs(low), abs(high)

    def total(self, row, n):
        s = self.spell
        low, high = self.values(row, n)
        amp = s.u32(row, AMPLITUDE + n - 1)
        dur = self.duration.get(s.u32(row, DURATION), 0)
        # The regeneration of food and drink (auras 84 and 85) is an amount for every 5 seconds.
        if s.u32(row, 61 + n - 1) == 6 and s.u32(row, AURA + n - 1) in (84, 85):
            ticks = max(dur / 5000.0, 1.0)
        else:
            ticks = (dur // amp) if amp else 1
        return round(low * max(ticks, 1)), round(high * max(ticks, 1))

    def effects(self, spell_id):
        """What using the spell does to the character, as RON `UseEffect`s: heal and mana at once or over time, and a stat raised for a while."""
        row = self.rows.get(spell_id)
        if row is None:
            return []
        s = self.spell
        dur = self.duration.get(s.u32(row, DURATION), 0) / 1000.0
        out = []
        for n in range(1, 4):
            effect, aura, misc = s.u32(row, 61 + n - 1), s.u32(row, AURA + n - 1), s.i32(row, 106 + n - 1)
            low, high = self.values(row, n)
            total = self.total(row, n)[0]
            if effect == 10:
                out.append("Heal(%d, %d)" % (low, high))
            elif effect == 30 and misc == 0:
                out.append("Mana(%d, %d)" % (low, high))
            elif effect == 6 and aura in (84, 8) and dur > 0:
                out.append("HealOver(%d, %s)" % (total, repr(float(dur))))
            elif effect == 6 and aura == 85 and dur > 0:
                out.append("ManaOver(%d, %s)" % (total, repr(float(dur))))
            elif effect == 6 and aura == 29 and misc in (0, 1, 2, 3, 4, -1):
                for stat in ([misc] if misc >= 0 else [0, 1, 2, 3, 4]):
                    out.append("Boost(%d, %d, %s)" % (stat, s.i32(row, BASE + n - 1) + 1, repr(float(max(dur, 0)))))
            elif effect == 6 and aura == 22 and misc == 1:
                out.append("Boost(5, %d, %s)" % (s.i32(row, BASE + n - 1) + 1, repr(float(max(dur, 0)))))
            elif effect == 6 and aura == 35 and misc == 0:
                out.append("Boost(7, %d, %s)" % (s.i32(row, BASE + n - 1) + 1, repr(float(max(dur, 0)))))
            elif effect == 6 and aura == 34:
                out.append("Boost(6, %d, %s)" % (s.i32(row, BASE + n - 1) + 1, repr(float(max(dur, 0)))))
            elif effect == 6 and aura == 22 and dur <= 0 and misc not in (0, 1):
                out.append("Equip(22, %d, %d)" % (misc, s.i32(row, BASE + n - 1) + 1))
            elif effect == 6 and aura == 8 and dur <= 0 and s.u32(row, AMPLITUDE + n - 1):
                # Health every so often while it is worn: stored as health every 5 seconds (aura 8, misc 0).
                out.append("Equip(161, 0, %d)" % round((s.i32(row, BASE + n - 1) + 1) * 5000.0 / s.u32(row, AMPLITUDE + n - 1)))
            elif effect == 6 and aura in EQUIPPED and dur <= 0:
                out.append("Equip(%d, %d, %d)" % (aura, misc, s.i32(row, BASE + n - 1) + 1))
            elif effect == 54 and misc:
                # Something put on the weapon in the hand for a while (a lure, a sharpening stone, a poison): the enchantment (`SpellItemEnchantment.dbc`) and the seconds, as the
                # item's text says them (the spell itself has no duration).
                m = re.search(r"(\d+)\s*(sec|min|hour)", self.describe(spell_id))
                secs = int(m.group(1)) * {"sec": 1, "min": 60, "hour": 3600}[m.group(2)] if m else 1800
                out.append("Temp(%d, %s)" % (misc, repr(float(secs))))
            elif effect == 36 and s.u32(row, 109 + n - 1):
                # A recipe, pattern, plan or formula: it teaches the spell that makes something (the trade skills).
                out.append("Teach(%d)" % s.u32(row, 109 + n - 1))
            elif effect == 6 and aura == 78 and misc in self.steeds:
                # Aura 32 (mounted speed) of the same spell is the percent of the running speed that the steed adds.
                speed = max([s.i32(row, BASE + m) + 1 for m in range(3) if s.u32(row, 61 + m) == 6 and s.u32(row, AURA + m) == 32] or [0])
                out.append("Mount(%d, %d)" % (self.steeds[misc], speed))
        if spell_id == 8690:
            out.append("Hearth")
        return out

    def describe(self, spell_id, mark=None):
        row = self.rows.get(spell_id)
        if row is None:
            return ""
        s = self.spell
        text = s.string(row, DESCRIPTION) or s.string(row, AURA_TEXT)
        return self.fill(row, text, mark).strip()

    def fill(self, row, text, mark=None):
        """The text with its tokens said. `mark(kind, n, named)`, if given, may say a token itself (a string) or give `UNTRUSTED` (the number is written, and the NUL character
        follows it, for the caller to find); `named` is whether the token names another spell."""
        s = self.spell
        last = [None]

        def number(low, high):
            last[0] = high
            return str(low) if low == high else "%d to %d" % (low, high)

        def value(r, kind, n, divide=1, multiply=1):
            """A number of spell row `r` (the text's own, or a spell it names: `$7922d`, `$26364s1`)."""
            extra = self.extra.get(s.u32(r, 0), {})
            if kind in "sSmM":
                low, high = self.values(r, n)
                if kind == "m":
                    high = low
                if kind == "M":
                    low = high
            elif kind == "o":
                low, high = self.total(r, n)
            elif kind == "t":
                low = high = s.u32(r, AMPLITUDE + n - 1) / 1000.0
            elif kind == "a":
                low = high = self.radius.get(s.u32(r, RADIUS + n - 1), 0)
            elif kind == "x":
                low = high = s.u32(r, CHAIN + n - 1)
            elif kind == "e":
                low = high = extra.get("EffectMultipleValue%d" % n, 0)
            elif kind == "q":
                low = high = extra.get("EffectMiscValue%d" % n, 0)
            elif kind == "b":
                low = high = extra.get("EffectPointsPerComboPoint%d" % n, 0)
            elif kind in "fF":
                low = high = extra.get("DmgMultiplier%d" % n, 1)
            elif kind == "i":
                low = high = extra.get("MaxAffectedTargets", 0)
            elif kind == "v":
                low = high = extra.get("MaxTargetLevel", 0)
            elif kind == "h":
                low = high = s.u32(r, PROC)
            elif kind == "n":
                low = high = s.u32(r, CHARGES)
            elif kind == "u":
                low = high = s.u32(r, STACK)
            else:
                return ""
            low, high = abs(low) * multiply / divide, abs(high) * multiply / divide
            low, high = (int(low) if low == int(low) else round(low, 1)), (int(high) if high == int(high) else round(high, 1))
            return number(low, high)

        def token(m):
            sign, factor, ref, kind, n = m.groups()
            r = self.rows.get(int(ref)) if ref else row
            if r is None:
                return ""
            if mark and not (kind == "b" and not ref and not n):
                said = mark(kind, int(n or 1), bool(ref))
                if said is UNTRUSTED:
                    return plain(sign, factor, ref, kind, n, r) + "\0"
                if said is not None:
                    last[0] = None
                    return said
            return plain(sign, factor, ref, kind, n, r)

        def plain(sign, factor, ref, kind, n, r):
            if kind in "dD":
                dur = self.duration.get(s.u32(r, DURATION), 0)
                return seconds_text(dur) if dur > 0 or kind == "D" else "until cancelled"
            if kind == "b" and not ref and not n:
                return "\n"
            factor = int(factor) if factor else 1
            return value(r, kind, int(n or 1), divide=factor if sign == "/" else 1, multiply=factor if sign == "*" else 1)

        # Conditionals: take the branch for "no" (nothing is known), or the only branch.
        for _ in range(4):
            text = re.sub(r"\$\?[a-zA-Z]?\d*\[[^\]]*\]\[([^\]]*)\]", r"\1", text)
            text = re.sub(r"\$\?[a-zA-Z]?\d*\[([^\]]*)\]", "", text)
        # (The gender words come first: their letters would read as numbers.)
        text = re.sub(r"\$g([^:;]*):([^;]*);", r"\1", text)
        text = re.sub(r"\$(?:([/*])(\d+);)?(\d{3,6})?([sSmMotaxeqvifFbhdDnu])(\d?)", token, text)
        # Plurals ("$lsecond:seconds;") follow the last number written.
        text = re.sub(r"\$l([^:;]*):([^;]*);", lambda m: m.group(1) if last[0] == 1 else m.group(2), text)
        text = text.replace("$B", "\n")
        # $z is the place the character is bound to, which the game fills in; any other token that is not known is dropped.
        text = re.sub(r"\$(?!z)\w", "", text)
        return text
