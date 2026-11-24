"""duckOSM's library is Rust (AGENTS.md, "The library is Rust"): Python is only a thin layer on ``duckosm._rust``.

Every module of ``src/duckosm`` is in one of two lists:

- ``THIN``: a thin layer that calls the Rust code (or returns a Python object, such as a networkx graph). It may stay.
- ``LEFT``: Python engine code (build steps, exporters, routers, checks) that is still to port to Rust and then delete.

A module in neither list fails: a new Python file must be a thin layer, or the work belongs in Rust. A module in ``LEFT`` that
is gone fails too: take it off the list. ``LEFT`` only shrinks; the goal is an empty list.
"""
from pathlib import Path

SRC = Path(__file__).parents[1] / "src" / "duckosm"

THIN = {
    "sumo.py", "sumo_compare.py",
    "gmns.py", "gmns_check.py", "gmns_viewer.py",
    "__init__.py", "__main__.py", "_api.py", "_main.py", "_progress.py",
    "admin.py", "area.py", "config.py", "dem_sources.py", "errors.py",
    "features/__init__.py", "gis.py", "hdmap.py", "ml.py", "mobility.py",
    "network.py", "routing.py", "sim.py", "studio.py", "widget.py",
}

LEFT = {
    "_levels.py", "analysis.py", "cli.py",
    "edge_id.py", "edit.py", "elevation.py",
    "features/builder.py", "features/check.py", "features/geometry.py", "features/layers.py", "features/ocean.py",
    "fixes.py", "gis_debug.py", "gmns_map.py",
    "importer.py", "importer_snippet.py",
    "levels_edit.py",
    "planning.py", "processors/__init__.py", "processors/base.py",
    "processors/boundary_cells.py", "processors/cost.py", "processors/dismount.py", "processors/duckdb_clipper.py", "processors/edge_graph.py",
    "processors/functional_type.py", "processors/global_junctions.py", "processors/graph_builder.py", "processors/graph_simplifier.py", "processors/h3_indexer.py",
    "processors/lane_profile.py", "processors/osm_overrides.py", "processors/path_connector.py", "processors/restrictions.py", "processors/road_filter.py",
    "processors/separate_sidewalks.py", "processors/speed.py", "query.py", "report.py",
    "route_map.py", "rust_engine.py", "utils/__init__.py",
    "utils/h3_calc.py", "validate.py", "viz.py",
}

def _modules():
    return {f.relative_to(SRC).as_posix() for f in SRC.rglob("*.py")}


def test_every_module_is_thin_or_still_to_port():
    other = _modules() - THIN - LEFT
    assert not other, f"{sorted(other)}: Python engine code is not allowed; write it in Rust, or make the module a thin layer on duckosm._rust and add it to THIN"


def test_the_list_of_leftovers_only_shrinks():
    gone = LEFT - _modules()
    assert not gone, f"{sorted(gone)} are gone: take them off LEFT"


def test_no_module_is_in_both_lists():
    assert not THIN & LEFT
