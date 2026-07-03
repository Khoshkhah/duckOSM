"""Geometry foundation + layer writers for the ``features.*`` schema.

Ported from duckmap (its verified DuckDB-native core) — duckOSM's ``raw`` schema is
column-identical to duckmap's, so this moves across essentially unchanged. Two renames vs
duckmap: output schema ``basemap`` → ``features``, and the styling column ``class`` → ``kind``
(the Shortbread convention).

``ensure_foundation`` builds, once per connection:
  geom.way_pt    ordered (way, ord, point)
  geom.way_line  one LINESTRING per way (+ is_closed)
  geom.rel_area  one (multi)polygon per multipolygon/boundary relation (holes included)

The three ``build_*_layer`` helpers write ``features.<name>`` tables with the uniform contract
``(osm_id, osm_type, kind, name, tags, geom)``. ``geom`` (schema + column) is DuckDB spatial
GEOMETRY in EPSG:4326 lon/lat. ``geom.*`` is scaffolding — drop it after the features build.
"""

import logging

import duckdb

logger = logging.getLogger("duckosm")


def ensure_foundation(con: duckdb.DuckDBPyConnection) -> None:
    """Build the shared geometry tables (idempotent). Needs raw.nodes/ways/relations."""
    con.execute("CREATE SCHEMA IF NOT EXISTS geom")

    # Ordered point per way node — ordinality preserves node order along the way.
    con.execute("""
        CREATE OR REPLACE TABLE geom.way_pt AS
        SELECT w.osm_id, u.ord, ST_Point(n.lon, n.lat) AS pt
        FROM raw.ways w,
             UNNEST(w.refs) WITH ORDINALITY AS u(node_id, ord)
        JOIN raw.nodes n ON n.osm_id = u.node_id
    """)

    # One LINESTRING per way (+ closed-ring flag).
    con.execute("""
        CREATE OR REPLACE TABLE geom.way_line AS
        SELECT w.osm_id,
               w.tags,
               (w.refs[1] = w.refs[len(w.refs)]) AS is_closed,
               ST_MakeLine(list(p.pt ORDER BY p.ord)) AS geom
        FROM raw.ways w
        JOIN geom.way_pt p USING (osm_id)
        GROUP BY w.osm_id, w.tags, w.refs
    """)

    # Member way-lines of multipolygon / boundary relations (positional UNNEST aligns
    # refs / ref_types / ref_roles).
    con.execute("""
        CREATE OR REPLACE TABLE geom.rel_member_line AS
        WITH m AS (
            SELECT osm_id AS rel_id,
                   tags   AS rel_tags,
                   UNNEST(refs)      AS ref_id,
                   UNNEST(ref_types) AS ref_type,
                   UNNEST(ref_roles) AS role
            FROM raw.relations
            WHERE map_extract(tags, 'type')[1] IN ('multipolygon', 'boundary')
        )
        SELECT m.rel_id, m.rel_tags, m.role, wl.geom
        FROM m
        JOIN geom.way_line wl ON wl.osm_id = m.ref_id
        WHERE m.ref_type = 'way'
    """)

    # Assemble each relation: stitch member lines (ST_LineMerge) then build the area
    # (ST_BuildArea carves enclosed rings as holes).
    con.execute("""
        CREATE OR REPLACE TABLE geom.rel_area AS
        SELECT rel_id AS osm_id,
               any_value(rel_tags) AS tags,
               ST_BuildArea(ST_LineMerge(ST_Collect(list(geom)))) AS geom
        FROM geom.rel_member_line
        GROUP BY rel_id
    """)


def drop_foundation(con: duckdb.DuckDBPyConnection) -> None:
    """Drop the geom.* scaffolding once features.* are built (no lasting duplication)."""
    for t in ("way_pt", "way_line", "rel_member_line", "rel_area"):
        con.execute(f"DROP TABLE IF EXISTS geom.{t}")
    con.execute("DROP SCHEMA IF EXISTS geom")


def build_area_layer(con, name: str, where_sql: str, kind_sql: str) -> int:
    """``features.<name>`` = closed-way areas + relation (multi)polygons matching ``where_sql``.

    ``where_sql`` / ``kind_sql`` reference the unqualified ``tags`` MAP column.
    """
    con.execute("CREATE SCHEMA IF NOT EXISTS features")
    con.execute(f"""
        CREATE OR REPLACE TABLE features.{name} AS
        SELECT w.osm_id                          AS osm_id,
               'way'                             AS osm_type,
               {kind_sql}                        AS kind,
               map_extract(w.tags, 'name')[1]    AS name,
               w.tags                            AS tags,
               ST_MakePolygon(w.geom)            AS geom
        FROM geom.way_line w
        WHERE w.is_closed AND ST_NPoints(w.geom) >= 4 AND ({where_sql})
        UNION ALL BY NAME
        SELECT r.osm_id                          AS osm_id,
               'relation'                        AS osm_type,
               {kind_sql}                        AS kind,
               map_extract(r.tags, 'name')[1]    AS name,
               r.tags                            AS tags,
               r.geom                            AS geom
        FROM geom.rel_area r
        WHERE r.geom IS NOT NULL AND NOT ST_IsEmpty(r.geom) AND ({where_sql})
    """)
    n = con.execute(f"SELECT count(*) FROM features.{name}").fetchone()[0]
    logger.info("  features.%s: %s area features", name, f"{n:,}")
    return n


def build_line_layer(con, name: str, where_sql: str, kind_sql: str) -> int:
    """``features.<name>`` = way LINESTRINGs matching ``where_sql`` (references ``tags``)."""
    con.execute("CREATE SCHEMA IF NOT EXISTS features")
    con.execute(f"""
        CREATE OR REPLACE TABLE features.{name} AS
        SELECT osm_id,
               'way'                          AS osm_type,
               {kind_sql}                     AS kind,
               map_extract(tags, 'name')[1]   AS name,
               tags,
               geom
        FROM geom.way_line
        WHERE ({where_sql})
    """)
    n = con.execute(f"SELECT count(*) FROM features.{name}").fetchone()[0]
    logger.info("  features.%s: %s line features", name, f"{n:,}")
    return n


def build_point_layer(con, name: str, where_sql: str, kind_sql: str) -> int:
    """``features.<name>`` = tagged nodes matching ``where_sql`` (references ``tags``)."""
    con.execute("CREATE SCHEMA IF NOT EXISTS features")
    con.execute(f"""
        CREATE OR REPLACE TABLE features.{name} AS
        SELECT osm_id,
               'node'                         AS osm_type,
               {kind_sql}                     AS kind,
               map_extract(tags, 'name')[1]   AS name,
               tags,
               ST_Point(lon, lat)             AS geom
        FROM raw.nodes
        WHERE ({where_sql})
    """)
    n = con.execute(f"SELECT count(*) FROM features.{name}").fetchone()[0]
    logger.info("  features.%s: %s point features", name, f"{n:,}")
    return n
