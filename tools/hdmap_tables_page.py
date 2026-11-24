"""Write docs/exports/hdmap_tables.md from SimReady's schema.sql and its model pages (docs/2-model).

python tools/hdmap_tables_page.py <simready folder> docs/exports/hdmap_tables.md <simready commit>

Each column gets the first sentence of its comment in schema.sql. The schema diagrams are the mermaid blocks of
SimReady's docs/2-model/schema.md.
"""
import re
import sys
from pathlib import Path

sim, out, commit = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
sql = (sim / "schema.sql").read_text()

GROUPS = {
    "Along the road": ["carriageway", "attribute", "event", "lane_section", "lane", "kerb_profile", "kerb_line",
                       "street_face", "structure", "structure_group", "elevation_profile"],
    "At the junction": ["node", "junction", "stop_line", "turn_rule", "movement", "island", "roundabout",
                        "signal_plan", "signal_phase"],
    "Project, sources and rules": ["project", "source", "model_override", "design_profile", "design_criterion",
                                   "rule_attribute", "rule_case", "rule_design"],
    "Checks": ["check_result", "loss_report"],
}
# one short line for each table (SimReady's model pages give some tables the line of another table)
ROW = {
    "carriageway": "one directed roadway from junction to junction: one chain of duckOSM",
    "attribute": "one value of a carriageway (lanes, speed, width), and how it was found",
    "event": "a station where the cross-section changes",
    "lane_section": "the part of a carriageway between two events",
    "lane": "one lane of a lane section: number, role, width and edges",
    "kerb_profile": "the width of the roadway at stations",
    "kerb_line": "an edge of the roadway as a line",
    "street_face": "the street space of one group at one level",
    "structure": "one span of a bridge or a tunnel",
    "structure_group": "the spans of one bridge or tunnel",
    "elevation_profile": "the height of the road along a carriageway",
    "node": "a junction point, from the nodes of duckOSM",
    "junction": "one node, or a cluster of nodes, with its area",
    "stop_line": "where a road ends and the junction starts",
    "turn_rule": "one turn from a road into a road: allowed or not, and why",
    "movement": "one way through a junction, from a lane to a lane",
    "island": "a traffic island in a junction",
    "roundabout": "a ring of one-way carriageways and its junctions",
    "signal_plan": "the signal controller of a junction",
    "signal_phase": "one phase of a signal plan and its movements",
    "project": "one row: the area, the CRS and the versions of the inputs",
    "source": "one data source: an OSM file, a DEM, a survey, a model",
    "model_override": "one value that a person changed by hand",
    "design_profile": "a named set of design values",
    "design_criterion": "one design value of a profile, with its source",
    "rule_attribute": "the rules that find the attributes",
    "rule_case": "the patterns of the junction and lane-change cases",
    "rule_design": "the list of the design rules",
    "check_result": "the result of one check on one row",
    "loss_report": "what a simulator format cannot hold",
}
EV = ("ev_status", "ev_method", "ev_confidence", "ev_source_ids", "ev_review")

tables = {}
for m in re.finditer(r"-- (\w+) \([\w-]+\.md\)\nCREATE TABLE sim\.\w+ \((.*?)\n\);", sql, re.S):
    cols = []
    for line in m.group(2).splitlines():
        c = re.match(r'\s+"(\w+)" ([A-Z]+(?:\[\])?(?:\([^)]*\))?)(?: /\* (\w+) \*/)?(.*)', line)
        if c:
            col, typ, geom, rest = c.groups()
            chk = re.search(r'CHECK \("\w+" IN \((.*?)\)\)', rest)
            values = [v.strip().strip("'") for v in chk.group(1).split(",")] if chk else []
            g = re.match(r"'((?:POINT|LINESTRING|POLYGON)(?: Z)?)\. ", sql.split(f'sim.{m.group(1)}."{col}" IS ', 1)[-1])
            geom = g.group(1) if g else geom
            cols.append((col, f"{typ} ({geom})" if geom else typ, "PRIMARY KEY" in rest, [v for v in values if v]))
    tables[m.group(1)] = cols
assert set(tables) == {t for g in GROUPS.values() for t in g}, set(tables) ^ {t for g in GROUPS.values() for t in g}
comments = {(t, c): v.replace("''", "'")
            for t, c, v in re.findall(r"COMMENT ON COLUMN sim\.(\w+)\.\"(\w+)\" IS '((?:[^']|'')*)';", sql)}


def short(text, key):
    """The first sentence of a column comment, without links, decisions and asides: about 100 characters at most."""
    if key or text.startswith("`hash("):
        return "the id: a hash of its content keys" if key else "a hash of its content keys"
    text = re.sub(r"\[([^]]*)\]\([^)]*\)", r"\1", text)
    text = re.sub(r",? ?\(?decision D\d+\)?", "", text)
    text = re.split(r"(?<=[a-z0-9)`])\. |; ", text, maxsplit=1)[0].rstrip(".")
    text = re.sub(r"^(POINT|LINESTRING|POLYGON)( Z)?\. ", "", text)
    text = re.sub(r" ?\([\w-]+\.md\)", "", text)
    if len(text) > 80:
        text = re.sub(r" \([^()]*\)", "", text)
    text = re.sub(r"\.? ?Written by .*$", "", text)
    if len(text) > 110:
        cut = max(text.rfind(", ", 0, 110), text.rfind(": ", 0, 110))
        cut = cut if cut > 40 else text.rfind(", ")
        text = text[:cut] if cut > 40 else text
    if text.count("(") > text.count(")"):
        text = text[:text.rfind("(")]
    text = text.rstrip(".:, ")
    text = text.replace("|", "\\|").strip()
    return text[0].lower() + text[1:] if text[:2] != text[:2].upper() else text


diagrams = re.findall(r"```mermaid\n.*?```", (sim / "docs/2-model/schema.md").read_text(), re.S)
# the first diagram is too wide for the page: its lines of the project and the design profile go into a third one
road = diagrams[0].splitlines()
is_project = [ln.strip().startswith(("project", "design_profile")) for ln in road]
project = "\n".join(["```mermaid", "erDiagram"] + [ln for ln, p in zip(road, is_project) if p] + ["```"])
# left to right: the seven children of carriageway go down the page, not across it
diagrams[0] = "\n".join(ln for ln, p in zip(road, is_project) if not p).replace("erDiagram", "erDiagram\n    direction LR", 1)
w = [f"""# HD map tables

Each table of the HD map database, and each of its columns. [SimReady](https://github.com/Khoshkhah/simready) builds
this database from a duckOSM database.

| Rule | |
|---|---|
| Schema | all tables are in the schema `sim` |
| Carriageway | one carriageway for each chain of `driving.chains`, with the same `chain_id` ([Edges and chains](../concepts/edges-and-chains.md)) |
| Ids | a 64-bit hash of the content keys, with the hash of duckOSM's [`edge_id`](../concepts/edge-ids.md). `node_id` is the OSM node id |
| Units | in the column name: `_m` metres, `_s` seconds, `_deg` degrees, `_kmh` km/h |
| Geometry | DuckDB `GEOMETRY` in the CRS of `project.crs`: metres, in the UTM zone of the area |
| Stations | metres along the reference line of a carriageway, from its start |
| Left and right | in the direction of travel. Lane 1 is the leftmost lane |

Made from SimReady's `schema.sql` (commit `{commit}`) by `tools/hdmap_tables_page.py`.

## The schema

Two trees meet at the lane: along the road, and at the junction. The project holds both.

### Schema: along the road

{diagrams[0]}

### Schema: at the junction

{diagrams[1]}

### Schema: the project

{project}

## The tables
"""]
for group, names in GROUPS.items():
    w.append(f"\n**{group}**\n\n| Table | One row is |\n|---|---|")
    w += [f"| [`{t}`](#{t}) | {ROW[t]} |" for t in names]
w.append("""
## Evidence

Most tables have five evidence columns: where each value of the row came from.

| Column | Type | Holds |
|---|---|---|
| `ev_status` | VARCHAR | `observed`, `derived`, `inferred`, `assumed`, `unknown` or `conflict` |
| `ev_method` | VARCHAR | the rule, the algorithm or the model that made the value |
| `ev_confidence` | DOUBLE | 0 to 1, for `inferred` and `conflict` only |
| `ev_source_ids` | VARCHAR[] | the sources, each as `"{source_id}:{object}"` |
| `ev_review` | VARCHAR | `none`, `needed`, `accepted` or `rejected` |""")
for group, names in GROUPS.items():
    w.append(f"\n## {group}")
    for t in names:
        w.append(f"\n### {t}\n\nOne row is {ROW[t]}.\n\n| Column | Type | Holds |\n|---|---|---|")
        for col, typ, key, values in tables[t]:
            if col in EV:
                continue
            d = short(comments.get((t, col), ""), key)
            if values:
                head = d.split("`")[0].split(":")[0].strip()
                d = (head + ": " if head else "") + ", ".join(f"`{v}`" for v in values)
            w.append(f"| `{col}` | {typ} | {d} |")
        if any(c[0] in EV for c in tables[t]):
            w.append("\nAnd the five [evidence](#evidence) columns.")
out.write_text("\n".join(w) + "\n")
print(len(tables), "tables")
