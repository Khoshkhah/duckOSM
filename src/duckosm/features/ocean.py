"""``features.ocean``: the sea, which OSM only implies by its coastline lines (docs/design/sea.md).

Read from Overture Maps' ``base/water`` (subtype ``ocean``, built from OSM's coastline, ODbL): its
public GeoParquet on S3, of which DuckDB reads only the row groups whose ``bbox`` overlaps the area.
The area is the boundary's bounding box (without a boundary, the extract's) grown by 10 % (at least
500 m), so the sea also fills the view a little past the edge. Never aborts the build: offline or on
any error, a warning and an empty table.
"""

import logging
import math

logger = logging.getLogger("duckosm")

OVERTURE = "s3://overturemaps-us-west-2/release"
COLUMNS = "osm_id BIGINT, osm_type VARCHAR, kind VARCHAR, name VARCHAR, source VARCHAR, geom GEOMETRY"


def area_box(con, grow=0.10, min_m=500.0):
    """The area's bounding box (lon / lat) grown by ``grow`` of its size, at least ``min_m``: the
    boundary's, or, in a build without one, the extract's nodes'."""
    has = con.execute("SELECT count(*) FROM information_schema.tables "
                      "WHERE table_schema = 'main' AND table_name = 'boundary'").fetchone()[0]
    x0, y0, x1, y1 = con.execute(
        "SELECT min(ST_XMin(geom)), min(ST_YMin(geom)), max(ST_XMax(geom)), max(ST_YMax(geom)) "
        "FROM main.boundary" if has else
        "SELECT min(lon)::DOUBLE, min(lat)::DOUBLE, max(lon)::DOUBLE, max(lat)::DOUBLE FROM raw.nodes").fetchone()
    my = max(grow * (y1 - y0), min_m / 111320.0)
    mx = max(grow * (x1 - x0), min_m / (111320.0 * math.cos(math.radians((y0 + y1) / 2))))
    return x0 - mx, y0 - my, x1 + mx, y1 + my


def latest_release(remote):
    """The newest Overture release name, e.g. ``2026-09-23.1`` (they change monthly)."""
    rows = remote.execute(
        f"SELECT DISTINCT regexp_extract(file, 'release/([^/]+)/', 1) "
        f"FROM glob('{OVERTURE}/*/theme=base/type=water/*')").fetchall()
    return max(r[0] for r in rows)


def build_ocean(con, release=None) -> int:
    """Create ``features.ocean`` on ``con`` (a built db with ``main.boundary``); the row count."""
    import duckdb

    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA IF NOT EXISTS features")
    con.execute(f"CREATE OR REPLACE TABLE features.ocean ({COLUMNS})")
    try:
        x0, y0, x1, y1 = area_box(con)
        remote = duckdb.connect()                 # its own connection: S3 settings stay out of the db
        remote.execute("INSTALL httpfs; LOAD httpfs; INSTALL spatial; LOAD spatial; "
                       "SET s3_region = 'us-west-2'")
        release = release or latest_release(remote)
        env = f"ST_MakeEnvelope({x0}, {y0}, {x1}, {y1})"
        rows = remote.execute(f"""
            SELECT ST_AsWKB(g) FROM (
                SELECT ST_Intersection(geometry, {env}) AS g
                FROM read_parquet('{OVERTURE}/{release}/theme=base/type=water/*', hive_partitioning = 1)
                WHERE subtype = 'ocean'
                  AND bbox.xmin < {x1} AND bbox.xmax > {x0} AND bbox.ymin < {y1} AND bbox.ymax > {y0})
            WHERE NOT ST_IsEmpty(g)""").fetchall()
        remote.close()
    except Exception as e:  # noqa: BLE001 - offline / S3 / schema change: no sea, never a failed build
        logger.warning("[features] ocean skipped: %s", str(e).splitlines()[0])
        return 0
    src = f"Overture Maps {release} (base/water ocean, from OSM coastlines)"
    for (wkb,) in rows:
        con.execute("INSERT INTO features.ocean VALUES (NULL, NULL, 'ocean', NULL, ?, ST_GeomFromWKB(?))",
                    [src, bytes(wkb)])
    logger.info("[features] ocean: %d polygon(s) from %s", len(rows), src)
    return len(rows)
