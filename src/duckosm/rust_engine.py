"""The Rust engine (``options.engine: rust``): the Rust code of this repository (``rust/``) builds the network step.

``duckosm._rust`` (the module of the wheel, crate ``rust/duckosm-py``) reads the cut PBF and writes the tables of
the network step as GeoParquet: ``edges``, ``private_edges``, ``nodes`` and ``edge_id_map`` of each mode,
with the same ``edge_id`` values as the SQL steps. The importer calls :func:`build_parquet` one time for
all modes, then :func:`load_mode` for each mode. The other steps of the build run in Python as before.
See docs/design/rust_engine.md and docs/design/rust_engine_move.md.
"""
import logging
from pathlib import Path

logger = logging.getLogger("duckosm.rust_engine")

GRAPH_TABLES = ("edge_graph", "turn_permission", "turn_restrictions", "turn_path_restrictions")


def module():
    """The ``duckosm._rust`` module, or a clear error when this install has no Rust module."""
    try:
        from duckosm import _rust
    except ImportError as e:
        raise RuntimeError("options.engine: rust needs the module duckosm._rust. It is in every wheel of duckosm; "
                           "a source install must build it (pip install -e . needs a Rust toolchain). "
                           "Or set options.engine: python") from e
    return _rust


def build_parquet(config, pbf_path, out_dir) -> dict:
    """Run the network step of the Rust engine for every mode of ``config`` on the cut PBF; the GeoParquet
    files go to ``out_dir``. Returns the summary dict of ``duckosm._rust.build`` (counts and seconds)."""
    o, c = config.options, config.clip
    res = module().build(
        str(pbf_path), str(out_dir), modes=list(config.modes), formats=["geoparquet"], name=config.name,
        merge_segments=o.merge_segments, global_junctions=o.global_junctions,
        # ComponentFilter runs in Python after the edge graph, as before. With the walls, the Rust step also keeps only the largest
        # component (as on the page), so that a wall does not face a street piece that ComponentFilter removes later
        keep_largest_component=o.building_faces and c.keep_largest_component,
        cycling_dismount=o.cycling_dismount, infer_separate_sidewalks=o.infer_separate_sidewalks,
        connectivity_rescue=c.connectivity_rescue, connect_snap_m=c.connect_snap_m,
        process_speeds=o.process_speeds, calculate_costs=o.calculate_costs,
        functional_types=o.functional_types, osm_overrides=config.osm_overrides,
        build_graph=o.build_graph, extract_restrictions=o.extract_restrictions,
        # the walls need raw.* of the area, which the module reads only with the boundary (the cut PBF is cut again, with no change)
        building_faces=o.building_faces, boundary=config.effective_boundary_path if o.building_faces else None, bus_routes=False)
    for what, secs in res.get("seconds", {}).items():
        logger.info(f"  rust {what}: {secs:.2f}s")
    return res


# The load step (docs/design/streaming_load.md): the Rust engine writes raw_nodes, raw_ways and raw_relations as Parquet
# while it reads the file, in a memory budget; DuckDB reads them into raw.* with the types of ST_READOSM.
RAW_SQL = {
    "nodes": "SELECT osm_id, lat, lon, tags FROM read_parquet('{d}/raw_nodes.parquet')",
    "ways": "SELECT osm_id, tags, refs FROM read_parquet('{d}/raw_ways.parquet')",
    "relations": "SELECT osm_id, tags, refs, ref_roles, "
                 "CAST(ref_types AS ENUM('node', 'way', 'relation')[]) AS ref_types FROM read_parquet('{d}/raw_relations.parquet')",
}


def load_raw(con, pbf_path, folder, name: str, memory_limit=None) -> dict:
    """Create raw.nodes, raw.ways and raw.relations from ``pbf_path`` with the load step of the Rust engine. The Parquet
    files wait in a temporary folder in ``folder`` and are deleted after DuckDB reads them. ``memory_limit`` (as DuckDB
    writes it, for example "2GB") is the memory budget of the load; None: 1 GB. Returns the summary dict of
    ``duckosm._rust.load_raw``."""
    import shutil
    import tempfile
    tmp = Path(tempfile.mkdtemp(prefix=f".{name}.raw-", dir=folder))
    try:
        res = module().load_raw(str(pbf_path), str(tmp), memory=memory_limit)
        d = str(tmp).replace("'", "''")
        for table, sql in RAW_SQL.items():
            con.execute(f"CREATE OR REPLACE TABLE raw.{table} AS " + sql.format(d=d))
        return res
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


# main.building_entrances: the doors of the area. A door is a node with `entrance` or `door`; its building is the building way that has the node in its outline (the smallest id when
# two buildings share it; NULL when none). The same SQL as the page of Studio (docs/rust/tables.md), so the editor reads one table everywhere.
BUILDING_ENTRANCES_SQL = """
CREATE OR REPLACE TABLE main.building_entrances AS
WITH e AS (
  SELECT osm_id, tags, lon, lat FROM raw.nodes WHERE map_extract(tags, 'entrance')[1] IS NOT NULL OR map_extract(tags, 'door')[1] IS NOT NULL
), j AS (
  SELECT node_id, min(building_id) AS building_id FROM (
    SELECT w.osm_id AS building_id, unnest(w.refs) AS node_id FROM raw.ways w WHERE map_extract(w.tags, 'building')[1] IS NOT NULL)
  WHERE node_id IN (SELECT osm_id FROM e) GROUP BY node_id)
SELECT e.osm_id, 'node' AS osm_type, coalesce(map_extract(e.tags, 'entrance')[1], 'door') AS kind, map_extract(e.tags, 'door')[1] AS door,
       map_extract(e.tags, 'access')[1] AS access, map_extract(e.tags, 'addr:housenumber')[1] AS housenumber, map_extract(e.tags, 'addr:street')[1] AS street,
       map_extract(e.tags, 'level')[1] AS level, map_extract(e.tags, 'wheelchair')[1] AS wheelchair, map_extract(e.tags, 'name')[1] AS name,
       j.building_id, e.tags, ST_Point(e.lon, e.lat) AS geom
FROM e LEFT JOIN j ON j.node_id = e.osm_id"""


def make_building_entrances(con) -> int:
    """Create ``main.building_entrances`` from ``raw.*`` (``options.building_faces``: the doors go with the walls). Returns the rows."""
    con.execute(BUILDING_ENTRANCES_SQL)
    return con.execute("SELECT count(*) FROM main.building_entrances").fetchone()[0]


def load_building_faces(con, out_dir) -> int:
    """Create ``main.building_faces`` from ``building_faces_main.parquet`` (``options.building_faces``). Returns the rows."""
    p = str(Path(out_dir) / "building_faces_main.parquet").replace("'", "''")
    con.execute(f"CREATE OR REPLACE TABLE main.building_faces AS SELECT * REPLACE (geometry::GEOMETRY AS geometry) FROM read_parquet('{p}')")
    return con.execute("SELECT count(*) FROM main.building_faces").fetchone()[0]


def load_mode(con, mode: str, out_dir, merge_segments: bool) -> tuple[int, int]:
    """Create ``edges``, ``private_edges``, ``nodes``, ``virtual_nodes``, (with merge_segments) ``edge_id_map``, and the tables of
    the edge graph and the turn restrictions that exist, in the current schema from the files of ``mode``. Returns (edges, nodes). GeoParquet reads as
    ``GEOMETRY('OGC:CRS84')``; the tables have ``GEOMETRY``, so the geometry is cast."""
    def p(table):
        return str(Path(out_dir) / f"{table}_{mode}.parquet").replace("'", "''")
    con.execute(f"CREATE OR REPLACE TABLE edges AS SELECT * REPLACE (geometry::GEOMETRY AS geometry) "
                f"FROM read_parquet('{p('edges')}')")
    con.execute(f"CREATE OR REPLACE TABLE private_edges AS SELECT * REPLACE (geometry::GEOMETRY AS geometry) "
                f"FROM read_parquet('{p('private_edges')}')")
    con.execute(f"CREATE OR REPLACE TABLE nodes AS SELECT node_id, geometry::GEOMETRY AS geom "
                f"FROM read_parquet('{p('nodes')}')")
    con.execute(f"CREATE OR REPLACE TABLE virtual_nodes AS SELECT node_id, geometry::GEOMETRY AS geom "
                f"FROM read_parquet('{p('virtual_nodes')}')")     # always empty, but the Python build has the table
    if merge_segments:
        con.execute(f"CREATE OR REPLACE TABLE edge_id_map AS SELECT * FROM read_parquet('{p('edge_id_map')}')")
    for t in GRAPH_TABLES:              # the files that the options made (build_graph, extract_restrictions)
        if Path(p(t)).exists():
            con.execute(f"CREATE OR REPLACE TABLE {t} AS SELECT * FROM read_parquet('{p(t)}')")
    n_edges = con.execute("SELECT count(*) FROM edges").fetchone()[0]
    n_nodes = con.execute("SELECT count(*) FROM nodes").fetchone()[0]
    return n_edges, n_nodes
