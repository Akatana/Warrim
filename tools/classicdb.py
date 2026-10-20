"""Reads tables out of the CMaNGOS classic database dump (ClassicDB_1_12_1_*.sql): the rows of `INSERT INTO `table` VALUES (..),(..);`.

Used by the importers in this folder; the dump itself is not part of the repository (see import_items.py for where it comes from).
"""
import re


def parse_rows(text):
    """Every `(a,'b',3.5,NULL)` row of the text after VALUES, as lists of Python values (numbers, strings, None)."""
    n, i = len(text), 0
    while i < n:
        while i < n and text[i] != "(":
            i += 1
        if i >= n:
            return
        i += 1
        row = []
        while True:
            c = text[i]
            if c == "'":
                j, buf = i + 1, []
                while True:
                    d = text[j]
                    if d == "\\":
                        nxt = text[j + 1]
                        buf.append({"n": "\n", "r": "\r", "t": "\t", "0": "\0"}.get(nxt, nxt))
                        j += 2
                    elif d == "'":
                        if text[j + 1] == "'":
                            buf.append("'")
                            j += 2
                        else:
                            break
                    else:
                        buf.append(d)
                        j += 1
                row.append("".join(buf))
                i = j + 1
            elif c == ",":
                i += 1
            elif c == ")":
                i += 1
                break
            else:
                j = i
                while text[j] not in ",)":
                    j += 1
                tok = text[i:j].strip()
                if tok.upper() == "NULL":
                    row.append(None)
                else:
                    try:
                        row.append(int(tok))
                    except ValueError:
                        row.append(float(tok))
                i = j
        yield row


def columns(path, table):
    """The column names of a table, from its CREATE TABLE."""
    cols, inside = [], False
    with open(path, encoding="utf8", errors="replace") as f:
        for line in f:
            if line.startswith("CREATE TABLE `%s`" % table):
                inside = True
            elif inside:
                m = re.match(r"\s+`([^`]+)`", line)
                if m:
                    cols.append(m.group(1))
                else:
                    break
    return cols


def rows(path, table):
    """Every row of a table as a dict by column name."""
    cols = columns(path, table)
    prefix = "INSERT INTO `%s` VALUES " % table
    with open(path, encoding="utf8", errors="replace") as f:
        for line in f:
            if line.startswith(prefix):
                for r in parse_rows(line[len(prefix):]):
                    assert len(r) == len(cols), (table, len(r), len(cols))
                    yield dict(zip(cols, r))
