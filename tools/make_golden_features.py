"""Write the golden file of the H3 columns and the base-map layers from a duckOSM database (rust/duckosm-build/tests/features_h3.rs).

    python tools/make_golden_features.py golden_features.duckdb tests/data/monaco_features_h3_summary.tsv

Build the database with tests/data/golden_monaco_features.yaml (see the comments there). Each row of the file is a table or a
group of columns, its number of rows, and the MD5 of its rows as text (sorted). The SQL is the same as in the Rust test:
if you change one, change the other. Needs DuckDB's spatial extension.
"""
import sys

import duckdb

MODES = ("driving", "walking", "cycling")
LAYERS = ("buildings", "land", "paved", "place_labels", "pois", "public_transport", "sites", "streets", "traffic",
          "water_lines", "water_polygons")
RAW = {
    "nodes": "SELECT concat_ws('|', osm_id, lat, lon, tags::VARCHAR) FROM raw.nodes",
    "ways": "SELECT concat_ws('|', osm_id, tags::VARCHAR, refs::VARCHAR) FROM raw.ways",
    "relations": "SELECT concat_ws('|', osm_id, tags::VARCHAR, refs::VARCHAR, ref_roles::VARCHAR, ref_types::VARCHAR) FROM raw.relations",
}


def tables():
    """(name, a SELECT of one text for each row)"""
    for t in LAYERS:
        bearing = ", round(bearing, 6)" if t == "traffic" else ""
        yield f"features.{t}", (f"SELECT concat_ws('|', osm_id, osm_type, kind, name, tags::VARCHAR, hex(ST_AsWKB(geom)){bearing}) "
                                f"FROM features.{t}")
    for t, sql in RAW.items():
        yield f"raw.{t}", sql
    for m in MODES:
        yield f"{m}.nodes.h3", f"SELECT concat_ws('|', node_id, h3_cell) FROM {m}.nodes"
        yield f"{m}.edges.h3", f"SELECT concat_ws('|', edge_id, from_cell, to_cell, lca_res) FROM {m}.edges"


if __name__ == "__main__":
    con = duckdb.connect(sys.argv[1], read_only=True)
    con.execute("INSTALL spatial; LOAD spatial")
    with open(sys.argv[2], "w") as f:
        f.write("what\trows\tmd5\n")
        for name, select in tables():
            n, md5 = con.execute(f"SELECT count(*), md5(string_agg(s, chr(10) ORDER BY s)) FROM ({select}) x(s)").fetchone()
            f.write(f"{name}\t{n}\t{md5}\n")
