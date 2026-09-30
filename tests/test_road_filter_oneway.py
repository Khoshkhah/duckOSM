"""RoadFilter one-way flag is mode-specific.

Pedestrians are not bound by vehicular ``oneway``: a ``oneway=yes`` street is walkable
in both directions, so the walking network must be undirected (``oneway=FALSE``) unless an
explicit ``oneway:foot`` says otherwise. Driving and cycling still honour vehicular
``oneway`` (cycling with the usual ``oneway:bicycle`` contraflow override). Regression guard
for the walking branch of ``_direction_expression()``.

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


def _refs(mode, tags):
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    kv = ", ".join(f"'{k}': '{v}'" for k, v in tags.items())
    con.execute(f"INSERT INTO raw.ways VALUES (1, MAP {{{kv}}}, [1, 2, 3])")
    RoadFilter(con, mode=mode)._create_ways_table()
    return con.execute("SELECT refs, oneway, lanes_fwd FROM ways WHERE osm_id = 1").fetchone()


def test_oneway_minus_one_is_turned_round():
    """OSM oneway=-1 is one-way AGAINST the drawing: the way is reversed, so its single edge runs
    the legal way (it used to run as drawn: the wrong way). Its lanes are the backward ones."""
    assert _refs("driving", {"highway": "primary", "oneway": "-1", "lanes:backward": "2"}) == ([3, 2, 1], True, 2)
    assert _refs("driving", {"highway": "primary", "oneway": "yes"})[0] == [1, 2, 3]
    assert _refs("driving", {"highway": "motorway"})[0] == [1, 2, 3]
    assert _refs("cycling", {"highway": "residential", "oneway:bicycle": "-1"})[0] == [3, 2, 1]
    assert _refs("cycling", {"highway": "residential", "oneway": "-1", "oneway:bicycle": "no"})[:2] == ([1, 2, 3], False)
    assert _refs("walking", {"highway": "steps", "oneway:foot": "-1"})[0] == [3, 2, 1]
    assert _refs("walking", {"highway": "residential", "oneway": "-1"})[:2] == ([1, 2, 3], False)


def _access(mode, tags):
    """(kept?, effective access) of one way in `mode`."""
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    kv = ", ".join(f"'{k}': '{v}'" for k, v in tags.items())
    con.execute(f"INSERT INTO raw.ways VALUES (1, MAP {{{kv}}}, [1, 2])")
    RoadFilter(con, mode=mode)._create_ways_table()
    row = con.execute("SELECT access FROM ways WHERE osm_id = 1").fetchone()
    return (row is not None, row[0] if row else None)


def test_access_most_specific_tag_decides():
    """docs/design/access_private.md: forbidden -> out of that mode; private -> kept, marked
    'private' (the build moves it to private_edges); a more specific tag reopens a road."""
    res = {"highway": "residential"}
    assert _access("driving", {**res, "access": "no"}) == (False, None)
    assert _access("driving", {**res, "motor_vehicle": "no"}) == (False, None)
    assert _access("driving", {**res, "access": "psv"}) == (False, None)            # bus-only
    assert _access("driving", {**res, "access": "private"}) == (True, "private")
    assert _access("driving", {**res, "access": "private", "motor_vehicle": "yes"}) == (True, "yes")
    assert _access("driving", {**res, "access": "no", "motorcar": "destination"}) == (True, "destination")
    assert _access("walking", {**res, "motor_vehicle": "no"}) == (True, None)        # not about walkers
    assert _access("walking", {**res, "foot": "no"}) == (False, None)
    assert _access("walking", {**res, "access": "private", "foot": "yes"}) == (True, "yes")
    assert _access("cycling", {**res, "vehicle": "no"}) == (False, None)             # was ignored
    assert _access("cycling", {**res, "vehicle": "no", "bicycle": "yes"}) == (True, "yes")
    assert _access("cycling", {**res, "access": "private"}) == (True, "private")
