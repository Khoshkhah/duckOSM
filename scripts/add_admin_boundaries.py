#!/usr/bin/env python
"""Add an ``admin_boundaries`` table (all OSM administrative levels) to a duckOSM DB.

OSM stores administrative areas as *boundary relations* whose member ways must be
stitched into rings and assembled into (multi)polygons. GDAL's OSM driver does this
assembly natively, so this script shells out to ``ogr2ogr`` to extract
``boundary='administrative'`` multipolygons, then loads them into the target DuckDB
as a single table:

    admin_boundaries(osm_id BIGINT, name VARCHAR, name_en VARCHAR,
                     admin_level INTEGER, parent_osm_id BIGINT,
                     geometry GEOMETRY)  -- EPSG:4326

``name`` is the local (OSM) name; ``name_en`` is the English name (OSM ``name:en``)
where present, so searches like "Sweden" can match "Sverige".

``parent_osm_id`` is the immediate enclosing boundary (the containing boundary with
the highest ``admin_level`` below this one's), derived by spatial containment, so the
hierarchy (e.g. kommun -> county -> country) can be traversed with self-joins. It is
NULL for the country root and for the few boundaries whose interior point is not
covered by any parent (border/coastline artifacts).

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


def compute_parents(con) -> None:
    """Set ``parent_osm_id`` to each boundary's immediate enclosing boundary.

    Containment is tested with a guaranteed-interior point (ST_PointOnSurface) against
    the geometry of candidate parents (those with a lower admin_level), pre-filtered by
    bounding box for speed. The immediate parent is the containing boundary with the
    highest admin_level below the child's (smallest by area to break ties).
    """
    con.execute("ALTER TABLE admin_boundaries ADD COLUMN IF NOT EXISTS parent_osm_id BIGINT")
    con.execute("""
        CREATE OR REPLACE TEMP TABLE _b AS
        SELECT osm_id, admin_level, geometry,
               ST_X(ST_PointOnSurface(geometry)) AS px,
               ST_Y(ST_PointOnSurface(geometry)) AS py,
               ST_XMin(geometry) xmin, ST_XMax(geometry) xmax,
               ST_YMin(geometry) ymin, ST_YMax(geometry) ymax
        FROM admin_boundaries
    """)
    con.execute("""
        CREATE OR REPLACE TEMP TABLE _parent AS
        WITH cand AS (
            SELECT c.osm_id AS child_id, p.osm_id AS parent_id,
                   p.admin_level AS plvl, ST_Area(p.geometry) AS parea
            FROM _b c
            JOIN _b p
              ON p.osm_id <> c.osm_id
             AND (p.admin_level < c.admin_level OR c.admin_level IS NULL)
             AND c.px BETWEEN p.xmin AND p.xmax
             AND c.py BETWEEN p.ymin AND p.ymax
             AND ST_Contains(p.geometry, ST_Point(c.px, c.py))
        )
        SELECT child_id, parent_id FROM (
            SELECT child_id, parent_id,
                   row_number() OVER (
                       PARTITION BY child_id ORDER BY plvl DESC NULLS LAST, parea ASC) rn
            FROM cand
        ) WHERE rn = 1
    """)
    con.execute("""
        UPDATE admin_boundaries SET parent_osm_id = _parent.parent_id
        FROM _parent WHERE admin_boundaries.osm_id = _parent.child_id
    """)
    con.execute("DROP TABLE IF EXISTS _b")
    con.execute("DROP TABLE IF EXISTS _parent")


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
            -- English name (OSM name:en), parsed from GDAL's hstore other_tags.
            -- Only present for areas that carry a name:en tag (country, counties,
            -- big cities); NULL otherwise.
            NULLIF(regexp_extract(other_tags, '"name:en"=>"([^"]*)"', 1), '') AS name_en,
            TRY_CAST(admin_level AS INTEGER) AS admin_level,
            geom AS geometry
        FROM ST_Read('{gpkg}')
        """
    )
    compute_parents(con)
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
