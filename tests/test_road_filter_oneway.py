"""RoadFilter one-way flag is mode-specific.

Pedestrians are not bound by vehicular ``oneway``: a ``oneway=yes`` street is walkable
in both directions, so the walking network must be undirected (``oneway=FALSE``) unless an
explicit ``oneway:foot`` says otherwise. Driving and cycling still honour vehicular
``oneway`` (cycling with the usual ``oneway:bicycle`` contraflow override). Regression guard
for the walking branch of ``_oneway_expression()``.

``highway=residential`` is kept by every mode's road filter, so it is the shared way used to
compare the same tags across modes.
"""
import duckdb
import pytest

from duckosm.processors.road_filter import RoadFilter


def _oneway(mode, tags):
    """Build the `ways` table for a single way under `mode`; return its `oneway` flag."""
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    kv = ", ".join(f"'{k}': '{v}'" for k, v in tags.items())
    con.execute(f"INSERT INTO raw.ways VALUES (1, MAP {{{kv}}}, [1, 2])")
    RoadFilter(con, mode=mode)._create_ways_table()
    row = con.execute("SELECT oneway FROM ways WHERE osm_id = 1").fetchone()
    assert row is not None, f"way was filtered out of the {mode} network: {tags}"
    return bool(row[0])


def test_walking_ignores_vehicular_oneway():
    # A car one-way street is walkable both ways -> the walking edge is bidirectional...
    assert not _oneway("walking", {"highway": "residential", "oneway": "yes"})
    # ...while the very same way stays one-way for driving.
    assert _oneway("driving", {"highway": "residential", "oneway": "yes"})


def test_walking_honours_explicit_oneway_foot():
    # An explicit pedestrian one-way (escalator / one-way passage) IS respected.
    assert _oneway("walking", {"highway": "footway", "oneway:foot": "yes"})
    assert _oneway("walking", {"highway": "steps", "oneway:foot": "-1"})


def test_walking_ignores_roundabout_direction():
    # Roundabouts are one-way for vehicles, but pedestrians walk them either way.
    assert not _oneway("walking", {"highway": "footway", "junction": "roundabout"})


def test_vehicular_modes_stay_directed():
    # Regression guard: the walking branch must not change driving / cycling behaviour.
    assert _oneway("driving", {"highway": "primary", "oneway": "yes"})
    assert _oneway("driving", {"highway": "motorway"})                 # implicitly one-way
    assert _oneway("cycling", {"highway": "residential", "oneway": "yes"})
    # Contraflow cycling: oneway:bicycle=no overrides the generic oneway.
    assert not _oneway("cycling", {"highway": "residential",
                                   "oneway": "yes", "oneway:bicycle": "no"})


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-q"]))
