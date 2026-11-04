"""Compare the tables of group 1, the lane profile (docs/tables.md) and the bus routes (docs/bus.md) of duckosm-rust
with a duckOSM database, row by row.

    python tools/compare_tables.py golden.duckdb ours.duckdb [modes]

`ways` compares its `tags` as sorted key-value lists (the order of the tags is not part of the data),
`main.boundary` and the boundary in `main.visualization_metadata` compare their shapes with ST_Equals, and
`built_at` and `duckosm_version` are not compared. Prints one line for each table and exits with 1 when a table
differs. Needs the duckdb Python package with the spatial extension.
"""
import sys

import duckdb

REF, OUT = sys.argv[1], sys.argv[2]
MODES = sys.argv[3].split(",") if len(sys.argv) > 3 else ["driving", "walking", "cycling"]
con = duckdb.connect()
con.execute("LOAD spatial")
con.execute(f"ATTACH '{REF}' AS py (READ_ONLY)")
con.execute(f"ATTACH '{OUT}' AS rs (READ_ONLY)")
bad = 0


def differ(name, select):
    """Rows of one side that are not in the other (EXCEPT ALL, both ways)."""
    global bad
    q = lambda a, b: con.execute(f"SELECT count(*) FROM ({select.format(db=a)} EXCEPT ALL {select.format(db=b)})").fetchone()[0]
    n = con.execute(f"SELECT count(*) FROM ({select.format(db='py')})").fetchone()[0]
    only_py, only_rs = q("py", "rs"), q("rs", "py")
    ok = only_py == 0 and only_rs == 0
    bad += not ok
    print(f"{'ok  ' if ok else 'DIFF'} {name}: {n} rows" + ("" if ok else f", {only_py} only in duckOSM, {only_rs} only in duckosm-rust"))


for m in MODES:
    differ(f"{m}.ways", "SELECT * EXCLUDE (tags), list_sort(map_entries(tags)) AS tags FROM {db}." + m + ".ways")
    differ(f"{m}.way_nodes", "SELECT * FROM {db}." + m + ".way_nodes")
    differ(f"{m}.virtual_nodes", "SELECT node_id, ST_AsText(geom) FROM {db}." + m + ".virtual_nodes")
if "driving" in MODES:
    differ("driving.lane_profile", "SELECT * FROM {db}.driving.lane_profile")
differ("main.global_junctions", "SELECT * FROM {db}.main.global_junctions")
differ("main.boundary", "SELECT * EXCLUDE (geom), ST_AsText(ST_Normalize(geom)) AS g FROM {db}.main.boundary")
differ("main.visualization_metadata", "SELECT center_lat, center_lon, initial_zoom, timezone, "
       "ST_AsText(ST_Normalize(ST_GeomFromGeoJSON(boundary_geojson))) AS g FROM {db}.main.visualization_metadata")
differ("mm.transfers", "SELECT * FROM {db}.mm.transfers")
differ("bus.routes", "SELECT * FROM {db}.bus.routes")
differ("bus.route_edges", "SELECT * FROM {db}.bus.route_edges")
differ("mm.edges (view)", "SELECT mode, edge_id, source, target, cost_s, length_m, highway, name FROM {db}.mm.edges")
sys.exit(1 if bad else 0)
