"""Check the base-map layers of duckosm-rust (docs/features.md): load the raw.* files of `duckosm-rust --raw`, run
rust/duckosm-build/sql/features.sql, and compare the result, row by row,
- with a duckOSM database (--duckosm): raw.* and each features.* table;
- with the features_<table>.parquet files of the page (--page, a folder that www/e2e.py wrote).

    python tools/compare_features.py OUT_DIR [--duckosm monaco.duckdb] [--page PAGE_DIR]

Rows are compared on osm_id, osm_type, kind, name, tags and the WKB of geom; traffic.bearing to 1e-9 degrees. The
page's files have no tags (as duckOSM's GeoParquet export). features.ocean (from Overture, not from OSM) is not
compared. Needs the duckdb Python package with the spatial extension.
"""
import argparse
import os
import pathlib
import sys

import duckdb

a = argparse.ArgumentParser()
a.add_argument("out", help="the output folder of duckosm-rust --raw")
a.add_argument("--duckosm", help="a duckOSM database of the same area")
a.add_argument("--page", help="a folder with the page's features_<table>.parquet files")
args = a.parse_args()
SQL = pathlib.Path(__file__).resolve().parent.parent / "rust/duckosm-build/sql/features.sql"
page = pathlib.Path(args.page).resolve() if args.page else None
con = duckdb.connect()
os.chdir(pathlib.Path(args.out).resolve())     # features.sql reads raw_*.parquet from the current folder
con.execute(SQL.read_text())
layers = [r[0] for r in con.sql("SELECT table_name FROM information_schema.tables WHERE table_catalog = 'memory' "
                                "AND table_schema = 'features' ORDER BY 1").fetchall()]
bad = 0


def diff(name, mine, theirs):
    """Rows of one in the other, both ways."""
    global bad
    a = con.sql(f"SELECT count(*) FROM ({mine} EXCEPT ALL {theirs})").fetchone()[0]
    b = con.sql(f"SELECT count(*) FROM ({theirs} EXCEPT ALL {mine})").fetchone()[0]
    n = con.sql(f"SELECT count(*) FROM ({mine})").fetchone()[0]
    print(f"{name}: {n} rows, {a} not in the other, {b} of the other missing")
    bad += a + b


def row(t, tags=True):
    return f"osm_id, osm_type, kind, name{', tags' if tags else ''}, ST_AsWKB(geom){', round(bearing, 9)' if t == 'traffic' else ''}"


if args.duckosm:
    print("against duckOSM:")
    con.execute(f"ATTACH '{args.duckosm}' AS py (READ_ONLY)")
    for t in ("nodes", "ways", "relations"):
        diff(f"  raw.{t}", f"FROM raw.{t}", f"FROM py.raw.{t}")
    for t in layers:
        diff(f"  features.{t}", f"SELECT {row(t)} FROM features.{t}", f"SELECT {row(t)} FROM py.features.{t}")
    theirs = {r[0] for r in con.sql("SELECT table_name FROM information_schema.tables WHERE table_catalog = 'py' "
                                    "AND table_schema = 'features'").fetchall()}
    if theirs - set(layers) - {"ocean"}:
        print("  duckOSM tables that are not built:", sorted(theirs - set(layers) - {"ocean"}))
        bad += 1
if page:
    print("against the page:")
    files = sorted(p.name for p in page.glob("features_*.parquet"))
    if files != [f"features_{t}.parquet" for t in layers]:
        print("  the page's files:", files)
        bad += 1
    for t in layers:
        f = page / f"features_{t}.parquet"
        if f.exists():
            diff(f"  features_{t}.parquet", f"SELECT {row(t, False)} FROM features.{t}", f"SELECT {row(t, False)} FROM read_parquet('{f}')")
sys.exit(1 if bad else 0)
