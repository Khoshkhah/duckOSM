"""The Rust engine (``options.engine: rust``): duckosm-rust builds the network step.

``duckosm_rust`` (the wheel of the duckosm-rust repository) reads the cut PBF and writes the tables of
the network step as GeoParquet: ``edges``, ``private_edges``, ``nodes`` and ``edge_id_map`` of each mode,
with the same ``edge_id`` values as the SQL steps. The importer calls :func:`build_parquet` one time for
all modes, then :func:`load_mode` for each mode. The other steps of the build run in Python as before.
See docs/design/rust_engine.md.
"""
import logging
from pathlib import Path

logger = logging.getLogger("duckosm.rust_engine")


def module():
    """The ``duckosm_rust`` module, or a clear error when it is not installed."""
    try:
        import duckosm_rust
    except ImportError as e:
        raise RuntimeError("options.engine: rust needs the Python module duckosm_rust (the wheel of "
                           "duckosm-rust; see docs/design/rust_engine.md). Install it, or set "
                           "options.engine: python") from e
    return duckosm_rust


def build_parquet(config, pbf_path, out_dir) -> dict:
    """Run the network step of duckosm-rust for every mode of ``config`` on the cut PBF; the GeoParquet
    files go to ``out_dir``. Returns the summary dict of ``duckosm_rust.build`` (counts and seconds)."""
    o, c = config.options, config.clip
    res = module().build(
        str(pbf_path), str(out_dir), modes=list(config.modes), formats=["geoparquet"], name=config.name,
        merge_segments=o.merge_segments, global_junctions=o.global_junctions,
        keep_largest_component=False,     # ComponentFilter runs in Python after the edge graph, as before
        cycling_dismount=o.cycling_dismount, infer_separate_sidewalks=o.infer_separate_sidewalks,
        connectivity_rescue=c.connectivity_rescue, connect_snap_m=c.connect_snap_m,
        process_speeds=o.process_speeds, calculate_costs=o.calculate_costs,
        functional_types=o.functional_types, osm_overrides=config.osm_overrides)
    for what, secs in res.get("seconds", {}).items():
        logger.info(f"  duckosm-rust {what}: {secs:.2f}s")
    return res


def load_mode(con, mode: str, out_dir, merge_segments: bool) -> tuple[int, int]:
    """Create ``edges``, ``private_edges``, ``nodes`` and (with merge_segments) ``edge_id_map`` in the
    current schema from the files of ``mode``. Returns (edges, nodes). GeoParquet reads as
    ``GEOMETRY('OGC:CRS84')``; the tables have ``GEOMETRY``, so the geometry is cast."""
    def p(table):
        return str(Path(out_dir) / f"{table}_{mode}.parquet").replace("'", "''")
    con.execute(f"CREATE OR REPLACE TABLE edges AS SELECT * REPLACE (geometry::GEOMETRY AS geometry) "
                f"FROM read_parquet('{p('edges')}')")
    con.execute(f"CREATE OR REPLACE TABLE private_edges AS SELECT * REPLACE (geometry::GEOMETRY AS geometry) "
                f"FROM read_parquet('{p('private_edges')}')")
    con.execute(f"CREATE OR REPLACE TABLE nodes AS SELECT node_id, geometry::GEOMETRY AS geom "
                f"FROM read_parquet('{p('nodes')}')")
    if merge_segments:
        con.execute(f"CREATE OR REPLACE TABLE edge_id_map AS SELECT * FROM read_parquet('{p('edge_id_map')}')")
    n_edges = con.execute("SELECT count(*) FROM edges").fetchone()[0]
    n_nodes = con.execute("SELECT count(*) FROM nodes").fetchone()[0]
    return n_edges, n_nodes
