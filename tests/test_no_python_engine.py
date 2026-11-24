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
    "__init__.py", "__main__.py", "_api.py", "_main.py", "_progress.py",
    "admin.py", "analysis.py", "area.py", "config.py", "dem_sources.py", "edge_id.py", "errors.py", "fixes.py",
    "gis.py", "gmns.py", "gmns_check.py", "gmns_viewer.py", "hdmap.py", "ml.py", "mobility.py", "network.py",
    "planning.py", "routing.py", "sim.py", "sumo.py", "sumo_compare.py", "widget.py",
}

# the Python engine code still to port to Rust: empty since phase 3 (2026-11-23). It stays empty.
LEFT = set()


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
