"""Write docs/exports/hdmap_tables.md from SimReady's schema.sql and the first line of each entity page.

python tools/hdmap_tables_page.py <simready folder> docs/exports/hdmap_tables.md <simready commit>
"""
import re, sys, pathlib
sim = pathlib.Path(sys.argv[1]); out = pathlib.Path(sys.argv[2])
sql = (sim / "schema.sql").read_text()
tables = {}
for m in re.finditer(r"-- (\w+) \(([\w-]+)\.md\)\nCREATE TABLE sim\.\w+ \((.*?)\n\);", sql, re.S):
    name, page, body = m.groups()
    cols = []
    for line in body.splitlines():
        c = re.match(r'\s+"(\w+)" ([A-Z]+(?:\[\])?(?:\([^)]*\))?)(?: /\* (\w+) \*/)?(.*)', line)
        if not c: continue
        col, typ, geom, rest = c.groups()
        chk = re.search(r"CHECK \(\"\w+\" IN \((.*?)\)\)", rest)
        cols.append((col, typ + (f" ({geom})" if geom else ""), "PRIMARY KEY" in rest,
                     [v.strip().strip("'") for v in chk.group(1).split(",")] if chk else None))
    tables[name] = (page, cols)
comments = {(t, c): v.replace("''", "'") for t, c, v in re.findall(r"COMMENT ON COLUMN sim\.(\w+)\.\"(\w+)\" IS '((?:[^']|'')*)';", sql)}
pages = {p: t for t, (p, _) in tables.items()}
pages["width-profile"] = "lane"
def links(text):
    text = re.sub(r",? ?\(?decision D\d+\)?", "", text)
    def f(m):
        label, target = m.groups()
        stem = target.rsplit("/", 1)[-1].removesuffix(".md")
        if "/" not in target and stem in pages:
            return f"[{label}](#{pages[stem]})"
        return label
    return re.sub(r"\[([^]]*)\]\(([^)]+)\)", f, text)
OWN = {
    "design_profile": "A named set of design values. A profile can fall back to a parent profile.",
    "design_criterion": "One design value of a design profile, with the clause of its source. Every number that a step needs is a row here.",
    "rule_attribute": "The rules that resolve the attributes of a carriageway.",
    "rule_case": "The case signatures that the case matcher compares with each junction and each lane change.",
    "rule_design": "The register of the design rules.",
    "signal_plan": "The controller of a signalled junction.",
    "signal_phase": "One phase of a signal plan, and the movements that it serves.",
    "model_override": "One value of the model that a person changed by hand (the source `editor`).",
}
def first_line(page, table=None):
    if table in OWN: return OWN[table]
    lines = (sim / "docs/2-model" / f"{page}.md").read_text().splitlines()
    return next(l for l in lines[1:] if l.strip() and not l.startswith(("#", "!!!", "<")))
EV = ("ev_status", "ev_method", "ev_confidence", "ev_source_ids", "ev_review")
commit = sys.argv[3]
w = [f"""# HD map tables

The tables of the HD map database (level 4). [SimReady](https://github.com/Khoshkhah/simready) builds this database
from a duckOSM database: one carriageway for each chain of `driving.chains`, with the same `chain_id`
([Edges and chains](../concepts/edges-and-chains.md)). All tables are in the schema `sim`.

This page is made from SimReady's `schema.sql` (commit `{commit}`) by `tools/hdmap_tables_page.py`. The units are in the column names (`_m`, `_s`,
`_deg`, `_kmh`). The geometry is in the CRS of `project.crs` (metres, the UTM zone of the area). Each id is a 64-bit
hash of its content keys, with the hash function of duckOSM's [`edge_id`](../concepts/edge-ids.md).

## The tables

| Table | Holds |
|---|---|"""]
for t, (page, _) in sorted(tables.items()):
    w.append(f"| [`{t}`](#{t}) | {links(first_line(page, t))} |")
w.append("""
## Evidence

Most tables have five evidence columns. They say where each value of the row came from.

| Column | Type | Holds |
|---|---|---|""")
ev_types = {}
for t, (_, cols) in tables.items():
    for c, typ, _, _ in cols:
        if c in EV: ev_types[c] = typ
for c in EV:
    w.append(f"| `{c}` | {ev_types[c]} | {links(next(v for (tt, cc), v in comments.items() if cc == c))} |")
for t, (page, cols) in sorted(tables.items()):
    w.append(f"\n## {t}\n\n{links(first_line(page, t))}\n")
    has_ev = any(c in EV for c, *_ in cols)
    w.append("| Column | Type | Holds |\n|---|---|---|")
    for c, typ, pk, values in cols:
        if c in EV: continue
        d = links(comments.get((t, c), "")).replace("|", "\\|").replace("\n", " ")
        d = re.sub(r"\.? ?See (the list )?below\.?$", "", d.strip())
        d = re.sub(r', "[^"]+" below', "", d)
        d = re.sub(r': one of the values in "Source and status" below$', "", d)
        if values and not all(f"`{v}`" in d for v in values if v): d += " Values: " + ", ".join(f"`{v}`" for v in values if v) + "."
        w.append(f"| `{c}`{' (key)' if pk else ''} | {typ} | {d.strip()} |")
    if has_ev: w.append("\nAnd the five [evidence](#evidence) columns.")
out.write_text("\n".join(w) + "\n")
print(len(tables), "tables")
