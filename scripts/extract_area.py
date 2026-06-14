#!/usr/bin/env python
"""Extract a sub-area from an existing duckOSM database into a new, self-contained DB.

Instead of re-running the OSM pipeline, this slices a built database (e.g. a whole
country) down to one area — selected by name (looked up in the source's
``admin_boundaries`` table) or by a GeoJSON boundary file — and writes a new duckOSM
database containing just that area's network.

Clip rule: an edge is kept **whole** if its geometry intersects the area (geometry is
not cut), preserving routing connectivity at the border. For every mode schema the
referenced ``nodes``, ``edge_graph``, ``turn_restrictions``, ``ways`` and
``way_nodes`` are carried along. ``main.boundary``, a fresh
``main.visualization_metadata`` (incl. an IANA ``timezone`` when ``timezonefinder``
is installed; disable with ``--no-timezone``) and any intersecting
``admin_boundaries`` are added. The large ``raw.*`` tables are not copied.

Examples
--------
    # By name (needs admin_boundaries in the source)
    python scripts/extract_area.py --source data/db/sweden.duckdb \\
        --db data/db/sodermalm.duckdb --name "Sodermalm"

    # By a specific boundary osm_id, or by a GeoJSON file
    python scripts/extract_area.py --source ... --db ... --osm-id 5691336
    python scripts/extract_area.py --source ... --db ... --boundary area.geojson
"""
import argparse
import sys
from pathlib import Path

import duckdb

# Per-mode tables and how each is filtered to the kept edges.
MODE_TABLES = {
    "nodes":             "node_id IN (SELECT source FROM {sch}.edges UNION SELECT target FROM {sch}.edges)",
    "edge_graph":        "from_edge IN (SELECT edge_id FROM {sch}.edges) AND to_edge IN (SELECT edge_id FROM {sch}.edges)",
    "turn_restrictions": "from_edge_id IN (SELECT edge_id FROM {sch}.edges) AND to_edge_id IN (SELECT edge_id FROM {sch}.edges)",
    "ways":              "osm_id IN (SELECT DISTINCT osm_id FROM {sch}.edges)",
    "way_nodes":         "way_id IN (SELECT DISTINCT osm_id FROM {sch}.edges)",
}


def build_clip(out, name, osm_id, boundary):
    """Create a temp table `_clip(geom)` with the single area polygon."""
    if boundary:
        out.execute(
            "CREATE TEMP TABLE _clip AS "
            f"SELECT ST_Union_Agg(geom) AS geom FROM ST_Read('{Path(boundary).resolve()}')"
        )
        label = Path(boundary).name
    else:
        # name / osm_id -> look up in the source admin_boundaries table
        cols = [r[0] for r in out.execute(
            "SELECT column_name FROM duckdb_columns() "
            "WHERE database_name='src' AND schema_name='main' "
            "AND table_name='admin_boundaries'").fetchall()]
        if not cols:
            sys.exit("Source has no main.admin_boundaries table — use --boundary instead.")
        if osm_id is not None:
            out.execute(
                "CREATE TEMP TABLE _clip AS "
                "SELECT geometry AS geom, name, admin_level FROM src.main.admin_boundaries "
                f"WHERE osm_id = {int(osm_id)}")
        else:
            has_en = "name_en" in cols
            en = "strip_accents(lower(coalesce(name_en,'')))" if has_en else "''"
            out.execute(
                "CREATE TEMP TABLE _clip AS "
                "SELECT geometry AS geom, name, admin_level FROM src.main.admin_boundaries "
                f"WHERE strip_accents(lower(name)) LIKE '%' || strip_accents(lower($q)) || '%' "
                f"   OR {en} LIKE '%' || strip_accents(lower($q)) || '%' "
                f"ORDER BY (strip_accents(lower(name)) = strip_accents(lower($q)) "
                f"          OR {en} = strip_accents(lower($q))) DESC, ST_Area(geometry) DESC "
                "LIMIT 1", {"q": name})
        row = out.execute("SELECT name, admin_level FROM _clip").fetchone()
        if row is None:
            sys.exit(f"No admin boundary matched '{name or osm_id}'.")
        label = f"{row[0]} (admin_level {row[1]})"

    n = out.execute("SELECT count(*) FROM _clip WHERE geom IS NOT NULL").fetchone()[0]
    if n == 0:
        sys.exit("Resolved boundary is empty.")
    return label


def add_timezone(out) -> None:
    """Add an IANA timezone (from the centroid) to main.visualization_metadata.

    Graceful: skips with a note if `timezonefinder` isn't installed or lookup fails.
    """
    try:
        from timezonefinder import TimezoneFinder
    except ImportError:
        print("  (timezone skipped — timezonefinder not installed)")
        return
    try:
        row = out.execute(
            "SELECT center_lat, center_lon FROM main.visualization_metadata").fetchone()
        if not row or row[0] is None:
            return
        tz = TimezoneFinder().timezone_at(lat=row[0], lng=row[1])
        if not tz:
            return
        cols = [r[0] for r in out.execute("DESCRIBE main.visualization_metadata").fetchall()]
        if "timezone" not in cols:
            out.execute("ALTER TABLE main.visualization_metadata ADD COLUMN timezone VARCHAR")
        out.execute("UPDATE main.visualization_metadata SET timezone = ?", [tz])
        print(f"  Timezone: {tz}")
    except Exception as e:
        print(f"  (timezone skipped — {e})")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--source", required=True, type=Path, help="source duckOSM .duckdb")
    ap.add_argument("--db", required=True, type=Path, help="output .duckdb (overwritten)")
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--name", help="area name, matched in the source admin_boundaries")
    g.add_argument("--osm-id", type=int, help="admin boundary osm_id to use as the area")
    g.add_argument("--boundary", type=Path, help="GeoJSON boundary file")
    ap.add_argument("--timezone", action=argparse.BooleanOptionalAction, default=True,
                    help="add IANA timezone to visualization_metadata (needs timezonefinder)")
    args = ap.parse_args(argv)

    if not args.source.exists():
        sys.exit(f"Source not found: {args.source}")
    if args.db.exists():
        args.db.unlink()
    args.db.parent.mkdir(parents=True, exist_ok=True)

    out = duckdb.connect(str(args.db))
    out.execute("INSTALL spatial; LOAD spatial;")
    out.execute(f"ATTACH '{args.source.resolve()}' AS src (READ_ONLY)")

    label = build_clip(out, args.name, args.osm_id, args.boundary)
    print(f"Area: {label}")

    # Bounding box of the clip, to pre-filter edges cheaply before ST_Intersects.
    xmin, ymin, xmax, ymax = out.execute(
        "SELECT ST_XMin(geom), ST_YMin(geom), ST_XMax(geom), ST_YMax(geom) FROM _clip").fetchone()
    bbox = (f"ST_XMax(geometry) >= {xmin} AND ST_XMin(geometry) <= {xmax} "
            f"AND ST_YMax(geometry) >= {ymin} AND ST_YMin(geometry) <= {ymax}")

    # Which source schemas are transport modes (exclude raw/main/system)?
    src_tables = out.execute(
        "SELECT schema_name, table_name FROM duckdb_tables() WHERE database_name='src'").fetchall()
    present = {}
    for sch, tbl in src_tables:
        present.setdefault(sch, set()).add(tbl)
    modes = [s for s in present
             if s not in ("information_schema", "pg_catalog", "main", "raw")
             and "edges" in present[s]]

    summary = []
    for sch in modes:
        out.execute(f"CREATE SCHEMA IF NOT EXISTS {sch}")
        out.execute(
            f"CREATE OR REPLACE TABLE {sch}.edges AS "
            f"SELECT * FROM src.{sch}.edges "
            f"WHERE {bbox} AND ST_Intersects(geometry, (SELECT geom FROM _clip))")
        n_edges = out.execute(f"SELECT count(*) FROM {sch}.edges").fetchone()[0]
        for tbl, pred in MODE_TABLES.items():
            if tbl in present[sch]:
                out.execute(
                    f"CREATE OR REPLACE TABLE {sch}.{tbl} AS "
                    f"SELECT * FROM src.{sch}.{tbl} WHERE {pred.format(sch=sch)}")
        summary.append((sch, n_edges))

    # main schema: boundary, intersecting admin_boundaries, fresh viz metadata
    out.execute("CREATE SCHEMA IF NOT EXISTS main")
    out.execute("CREATE OR REPLACE TABLE main.boundary AS SELECT geom AS geometry FROM _clip")
    if "admin_boundaries" in present.get("main", set()):
        out.execute(
            "CREATE OR REPLACE TABLE main.admin_boundaries AS "
            "SELECT * FROM src.main.admin_boundaries "
            "WHERE ST_Intersects(geometry, (SELECT geom FROM _clip))")
    out.execute(
        "CREATE OR REPLACE TABLE main.visualization_metadata AS "
        "SELECT ST_AsGeoJSON(geom) AS boundary_geojson, "
        "       ST_Y(ST_Centroid(geom)) AS center_lat, "
        "       ST_X(ST_Centroid(geom)) AS center_lon, "
        "       CAST(LEAST(16, GREATEST(1, "
        "            LOG2(360.0 / NULLIF(ST_XMax(geom) - ST_XMin(geom), 0)))) AS INTEGER) "
        "            AS initial_zoom "
        "FROM _clip")

    if args.timezone:
        add_timezone(out)

    out.execute("DROP TABLE _clip")
    out.execute("DETACH src")
    out.execute("CHECKPOINT")
    out.close()

    print(f"Wrote {args.db}")
    for sch, n in summary:
        print(f"  {sch}: {n:,} edges")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
