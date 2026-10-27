"""Makes `content/classic_pages.ron`: the pages of the books, letters and plaques of the original game (1.12.1), from the CMaNGOS classic database (page_text).

A page is its text (as the original has it: `$B` breaks a line, `$N` is the name of the reader, a few have HTML) and the page after it. What leads to a page is
`page:` in `classic_items.ron` (`tools/import_items.py`) and in `classic_props.ron` (`tools/import_props.py`).

Usage: py -3 tools/import_pages.py [folder with db.sql (default: tools/data)] [content folder]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import classicdb


def q(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\r", "").replace("\n", "\\n") + '"'


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    folder = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, "data")
    target = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "content")
    pages = sorted(classicdb.rows(os.path.join(folder, "db.sql"), "page_text"), key=lambda r: r["entry"])
    known = {p["entry"] for p in pages}
    header = ("// The pages of the books, letters and plaques of the original game (1.12.1), made by tools/import_pages.py from the CMaNGOS classic database (page_text).\n"
              "// Do not edit by hand. The key is the entry of the original; `next` is the page after it.\n")
    with open(os.path.join(target, "classic_pages.ron"), "w", encoding="utf8", newline="\n") as out:
        out.write(header + "{\n")
        for p in pages:
            # A page that leads to one the database does not have ends there.
            nxt = p["next_page"] if p["next_page"] in known else 0
            out.write("    %d: (text: %s%s),\n" % (p["entry"], q(p["text"]), ", next: %d" % nxt if nxt else ""))
        out.write("}\n")
    print("wrote", len(pages), "pages")


if __name__ == "__main__":
    main()
