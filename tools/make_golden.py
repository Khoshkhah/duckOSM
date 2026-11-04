"""Write the golden file of the Rust tests from a duckOSM database (driving edges, one row each).

    python tools/make_golden.py monaco_golden.duckdb tests/data/monaco_driving_edges.tsv

Build the database with tests/data/golden_monaco.yaml (duckOSM 0.6.0, DuckDB 1.5.3):
    cd <empty folder> && duckosm build --config <path>/golden_monaco.yaml
The columns are edge_id, source, target, osm_id, is_reverse, lanes, length_m (the 32-bit float as its 8 hex digits).
"""
import struct
import sys

import duckdb

con = duckdb.connect(sys.argv[1], read_only=True)
rows = con.execute("SELECT edge_id, source, target, osm_id, is_reverse, lanes, length_m, edge_ref, highway "
                   "FROM driving.edges ORDER BY edge_id").fetchall()
with open(sys.argv[2], "w") as f:
    f.write("edge_id\tsource\ttarget\tosm_id\tis_reverse\tlanes\tlength_m_hex\tedge_ref\thighway\n")
    for e, s, t, o, r, l, m, ref, hw in rows:
        f.write(f"{e}\t{s}\t{t}\t{o}\t{int(r)}\t{'' if l is None else l}\t{struct.pack('>f', m).hex()}\t{ref}\t{hw}\n")
print(len(rows), "edges")
