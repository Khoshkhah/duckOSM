"""Cycling dismount edges: footway/pedestrian enter the cycling graph as push-the-bike
edges — kept by the filter, bidirectional, flagged dismount=TRUE, costed at walking speed,
classified cycle_type='dismount'. Gated by options.cycling_dismount (default on); flag off
restores the pre-feature graph. See docs/design/cycling_dismount_edges.md."""
import duckdb
import pytest

from duckosm.processors.dismount import DismountMarker
from duckosm.processors.functional_type import FunctionalType
from duckosm.processors.road_filter import RoadFilter
from duckosm.processors.speed import SpeedProcessor


def _ways_row(mode, tags, cycling_dismount=True):
    """Run RoadFilter._create_ways_table on a single way; return its (highway, oneway) or None."""
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    kv = ", ".join(f"'{k}': '{v}'" for k, v in tags.items())
    con.execute(f"INSERT INTO raw.ways VALUES (1, MAP {{{kv}}}, [1, 2])")
    RoadFilter(con, mode=mode, cycling_dismount=cycling_dismount)._create_ways_table()
    return con.execute("SELECT highway, oneway FROM ways WHERE osm_id = 1").fetchone()


# ---- filter: what enters the cycling graph ------------------------------------------

def test_footway_kept_as_dismount_even_bicycle_no():
    # bicycle=no forbids RIDING; pushing is walking, so the way still enters the graph.
    assert _ways_row("cycling", {"highway": "footway", "bicycle": "no"}) is not None
    assert _ways_row("cycling", {"highway": "pedestrian"}) is not None


def test_private_or_foot_no_still_excluded():
    assert _ways_row("cycling", {"highway": "footway", "access": "private"}) is None
    assert _ways_row("cycling", {"highway": "footway", "foot": "no"}) is None


def test_flag_off_restores_old_filter():
    assert _ways_row("cycling", {"highway": "footway"}, cycling_dismount=False) is None
    # rideable footway is kept regardless of the flag (pre-existing bicycle=yes path)
    assert _ways_row("cycling", {"highway": "footway", "bicycle": "yes"},
                     cycling_dismount=False) is not None


def test_steps_stay_out():
    assert _ways_row("cycling", {"highway": "steps"}) is None


# ---- oneway: dismount ways are always bidirectional ----------------------------------

def test_dismount_way_ignores_oneway():
    hw, oneway = _ways_row("cycling", {"highway": "footway", "oneway": "yes"})
    assert not oneway
    # ...but a RIDEABLE footway keeps the vehicular rules
    hw, oneway = _ways_row("cycling", {"highway": "footway", "bicycle": "yes", "oneway": "yes"})
    assert oneway


# ---- marker + speed + cycle_type on the built edges -----------------------------------

def _edges_db():
    """raw.ways + a minimal built cycling `edges` table (one dismount, one rideable)."""
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR))")
    con.execute("INSERT INTO raw.ways VALUES "
                "(1, MAP {'highway': 'footway'}), "
                "(2, MAP {'highway': 'footway', 'bicycle': 'yes'}), "
                "(3, MAP {'highway': 'cycleway'})")
    con.execute("CREATE TABLE edges(edge_id BIGINT, osm_id BIGINT, highway VARCHAR, "
                "is_reverse BOOLEAN, maxspeed VARCHAR, length_m DOUBLE)")
    con.execute("INSERT INTO edges VALUES "
                "(10, 1, 'footway',  false, NULL, 100), "   # dismount
                "(20, 2, 'footway',  false, NULL, 100), "   # rideable footway
                "(30, 3, 'cycleway', false, NULL, 100)")
    return con


def test_marker_speed_and_cycle_type():
    con = _edges_db()
    DismountMarker(con).run()
    flags = dict(con.execute("SELECT edge_id, dismount FROM edges").fetchall())
    assert flags == {10: True, 20: False, 30: False}

    SpeedProcessor(con, mode="cycling").run()
    speeds = dict(con.execute("SELECT edge_id, maxspeed_kmh FROM edges").fetchall())
    assert speeds[10] == 5.0 and speeds[20] == 15.0 and speeds[30] == 15.0

    FunctionalType(con, mode="cycling").run()
    types = dict(con.execute("SELECT edge_id, cycle_type FROM edges").fetchall())
    assert types[10] == "dismount" and types[30] == "cycleway"


def test_speed_without_dismount_column_unchanged():
    con = _edges_db()          # no DismountMarker run -> no dismount column
    SpeedProcessor(con, mode="cycling").run()
    assert {r[0] for r in con.execute("SELECT maxspeed_kmh FROM edges").fetchall()} == {15.0}


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-q"]))
