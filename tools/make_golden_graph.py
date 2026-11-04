"""Write the golden file of the edge graph test from duckOSM databases.

    python tools/make_golden_graph.py tests/data/monaco_graph_summary.tsv NAME=DB [NAME=DB ...]

NAME is the variant: `sample` (tests/data/monaco_sample.osm.pbf, no boundary, so no component filter) or
`cut` (the same file with the Monaco boundary: duckOSM cuts it to tests/data/monaco.osm.pbf and runs the
component filter). Build each database with duckOSM, everything of the network step on, `build_graph` and
`extract_restrictions` on, and `bench/python_steps.py <yaml> --no-lane-profile` (docs/edge_graph.md).

For each mode and table (edge_graph, turn_permission, turn_restrictions, turn_path_restrictions) the file
has the number of rows and a checksum: the sum (mod 2**64) of the 64-bit FNV-1a hash of the text of each
row. The text is the columns joined by a tab: an integer in decimal, a FLOAT as the hex of its 32 bits,
a boolean as true or false, NULL as \\N, a list as [a,b]. rust/duckosm-core/tests/graph.rs makes the same.
"""
import struct
import sys

import duckdb

TABLES = ["edge_graph", "turn_permission", "turn_restrictions", "turn_path_restrictions"]


def text(v, is_float):
    if v is None:
        return "\\N"
    if isinstance(v, bool):
        return "true" if v else "false"
    if is_float:
        return format(struct.unpack(">I", struct.pack(">f", v))[0], "08x")
    if isinstance(v, list):
        return "[" + ",".join(str(x) for x in v) + "]"
    return str(v)


def fnv(s):
    h = 0xCBF29CE484222325
    for b in s.encode():
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


out = open(sys.argv[1], "w")
out.write("variant\tmode\ttable\trows\tchecksum\n")
for arg in sys.argv[2:]:
    name, db = arg.split("=", 1)
    con = duckdb.connect(db, read_only=True)
    for mode in ("driving", "walking", "cycling"):
        have = {r[0] for r in con.execute(
            "SELECT table_name FROM information_schema.tables WHERE table_schema = ?", [mode]).fetchall()}
        for t in TABLES:
            if t not in have:
                continue
            floats = [r[1] == "FLOAT" for r in con.execute(f"DESCRIBE {mode}.{t}").fetchall()]
            rows = con.execute(f"SELECT * FROM {mode}.{t}").fetchall()
            s = 0
            for r in rows:
                s = (s + fnv("\t".join(text(v, f) for v, f in zip(r, floats)))) & 0xFFFFFFFFFFFFFFFF
            out.write(f"{name}\t{mode}\t{t}\t{len(rows)}\t{s:x}\n")
