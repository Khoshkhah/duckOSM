#!/usr/bin/env python
"""Add an ``admin_boundaries`` table (all OSM administrative levels) to a duckOSM DB.

OSM stores administrative areas as *boundary relations* whose member ways must be
stitched into rings and assembled into (multi)polygons. GDAL's OSM driver does this
assembly natively, so this script shells out to ``ogr2ogr`` to extract
``boundary='administrative'`` multipolygons, then loads them into the target DuckDB
as a single table:

    admin_boundaries(osm_id BIGINT, name VARCHAR, admin_level INTEGER,
                     geometry GEOMETRY)   -- EPSG:4326, matches the edge geometries

The table is replaced on each run. Requires ``ogr2ogr`` (GDAL) on PATH and the
``duckdb`` Python package with the spatial extension.

Examples
--------
    python scripts/add_admin_boundaries.py \\
        --pbf /path/to/sweden-latest.osm.pbf \\
        --db  data/db/sweden.duckdb

    # Reuse a previously extracted GeoPackage (skips the slow ogr2ogr step):
    python scripts/add_admin_boundaries.py --pbf ... --db ... --gpkg /tmp/sweden_admin.gpkg
"""
import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

import duckdb


def extract_boundaries(pbf: Path, gpkg: Path) -> None:
    """Extract administrative boundary multipolygons from a PBF into a GeoPackage."""
    subprocess.run(
        [
            "ogr2ogr", "-f", "GPKG", str(gpkg), str(pbf),
            "multipolygons", "-where", "boundary='administrative'",
        ],
        check=True,
    )


def load_boundaries(gpkg: Path, db: Path) -> dict:
    """Load the GeoPackage into ``admin_boundaries`` and return per-level counts."""
    con = duckdb.connect(str(db))
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute(
        f"""
        CREATE OR REPLACE TABLE admin_boundaries AS
        SELECT
            TRY_CAST(COALESCE(osm_id, osm_way_id) AS BIGINT) AS osm_id,
            name,
            TRY_CAST(admin_level AS INTEGER) AS admin_level,
            geom AS geometry
        FROM ST_Read('{gpkg}')
        """
    )
    rows = con.execute(
        "SELECT admin_level, count(*) FROM admin_boundaries "
        "GROUP BY admin_level ORDER BY admin_level NULLS LAST"
    ).fetchall()
    con.close()
    return rows


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pbf", required=True, type=Path, help="source OSM PBF file")
    ap.add_argument("--db", required=True, type=Path, help="target duckOSM .duckdb file")
    ap.add_argument("--gpkg", type=Path, default=None,
                    help="GeoPackage path; reused if it already exists, else written here "
                         "(default: a temporary file that is removed afterwards)")
    args = ap.parse_args(argv)

    if not args.pbf.exists():
        sys.exit(f"PBF not found: {args.pbf}")
    if not args.db.exists():
        sys.exit(f"DB not found: {args.db}")

    tmp = None
    if args.gpkg is not None:
        gpkg = args.gpkg
    else:
        tmp = tempfile.NamedTemporaryFile(suffix=".gpkg", delete=False)
        tmp.close()
        gpkg = Path(tmp.name)

    try:
        if gpkg.exists() and args.gpkg is not None:
            print(f"Reusing existing GeoPackage: {gpkg}")
        else:
            print(f"Extracting admin boundaries from {args.pbf} -> {gpkg} ...")
            extract_boundaries(args.pbf, gpkg)

        print(f"Loading admin_boundaries into {args.db} ...")
        rows = load_boundaries(gpkg, args.db)
        total = sum(n for _, n in rows)
        print(f"admin_boundaries: {total} features")
        for level, n in rows:
            print(f"  admin_level {level if level is not None else 'NULL':>4}: {n}")
    finally:
        if tmp is not None:
            Path(tmp.name).unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
