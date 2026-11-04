"""Write the summary golden file of the three-mode test from duckOSM databases.

    python tools/make_golden_summary.py tests/data/monaco_modes_summary.tsv NAME=DB [NAME=DB ...]

NAME is the variant: merge_gj, nomerge_gj, merge_nogj or nomerge_nogj (merge_segments and
global_junctions on or off), merge_gj_cf (the cut file of Monaco with a boundary, so the component filter runs) merge_gj_sw (the sidewalk rule on), merge_gj_cp (the dangling-path connector on) and merge_gj_cfcp (connector and component filter, on the cut file) and merge_gj_sp (speeds and costs on; the _cf and _cfcp databases have them too). Build each database with tests/data/golden_monaco_modes.yaml (see the
comments there) and `bench/python_steps.py <yaml> --no-lane-profile`. For each mode the file has the
number of edges, private edges, nodes and edge_id_map rows, and one checksum for the edges and one for
the private edges. The checksum is the XOR over the rows of the edge_id hash of
(edge_id, source * 1000003 ^ target, (lanes + 1) << 32 | length_m bits | dismount << 48), and, when the database has speeds, a second term with maxspeed_kmh and cost_s, and, when it has walk_type or cycle_type, a third term with the type (not for connectors).
"""
import struct
import sys

import duckdb

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from edge_id_reference import duck_hash

M = (1 << 64) - 1


def f32bits(v):
    return struct.unpack(">I", struct.pack(">f", v))[0]


def checksum(rows):
    x = 0
    for edge_id, source, target, lanes, length_m, dismount, kmh, cost, ftype, osm_id in rows:
        b = ((source * 1000003) ^ target) & M
        c = ((((lanes if lanes is not None else -1) + 1) & 0xFFFF) << 32) | struct.unpack(">I", struct.pack(">f", length_m))[0]
        c |= (1 << 48) if dismount else 0
        x ^= duck_hash(edge_id & M, b, c)
        if kmh is not None:      # only when the speed step ran: a second term, so the other checksums stay the same
            x ^= duck_hash((edge_id ^ 0x5555) & M, f32bits(kmh), f32bits(cost) if cost is not None else 0)
        if ftype is not None and osm_id > 0:    # not the connectors: their class follows a highway that duckOSM does not fix
            x ^= duck_hash((edge_id ^ 0xAAAA) & M, 0, ftype)
    return x


out = open(sys.argv[1], "w")
out.write("variant\tmode\tedges\tprivate_edges\tnodes\tedge_id_map\tedges_checksum\tprivate_checksum\n")
for arg in sys.argv[2:]:
    name, db = arg.split("=", 1)
    con = duckdb.connect(db, read_only=True)
    for mode in ("driving", "walking", "cycling"):
        cols = {r[0] for r in con.execute(f"DESCRIBE {mode}.edges").fetchall()}
        dis = "dismount" if "dismount" in cols else "FALSE"
        kmh = "maxspeed_kmh" if "maxspeed_kmh" in cols else "NULL::FLOAT"
        cost = "cost_s" if "cost_s" in cols else "NULL::FLOAT"
        ft = "walk_type" if "walk_type" in cols else "cycle_type" if "cycle_type" in cols else "NULL::VARCHAR"
        q = lambda t: con.execute(f"SELECT edge_id, source, target, lanes, length_m, {dis}, {kmh}, {cost}, {ft}, osm_id FROM {mode}.{t}").fetchall()
        n_map = con.execute(f"SELECT count(*) FROM {mode}.edge_id_map").fetchone()[0] if name.startswith("merge") else 0
        e, p = q("edges"), q("private_edges")
        n = con.execute(f"SELECT count(*) FROM {mode}.nodes").fetchone()[0]
        out.write(f"{name}\t{mode}\t{len(e)}\t{len(p)}\t{n}\t{n_map}\t{checksum(e):x}\t{checksum(p):x}\n")
