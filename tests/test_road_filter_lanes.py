"""RoadFilter lane-count parsing — in particular `lanes:reversible` (counterflow) handling.

A shared reversible lane (e.g. Lions Gate Bridge / Stanley Park Causeway:
lanes=3, lanes:forward=1, lanes:backward=1, lanes:reversible=1) is available to each direction
at its peak, so it must be added to BOTH directions' capacity. Before the fix duckOSM ignored
`lanes:reversible` entirely and stored lanes=1 per direction.
"""
import duckdb
import pytest

from duckosm.processors.road_filter import RoadFilter


def _ways(rows):
    """Fresh connection with a raw.ways table populated from (osm_id, tags-dict) rows."""
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    for osm_id, tags in rows:
        kv = ", ".join(f"'{k}': '{v}'" for k, v in tags.items())
        con.execute(f"INSERT INTO raw.ways VALUES ({osm_id}, MAP {{{kv}}}, [1, 2])")
    RoadFilter(con, mode="driving")._create_ways_table()
    return con


def _lanes(con, osm_id):
    return con.execute(
        f"SELECT lanes_fwd, lanes_bwd FROM ways WHERE osm_id = {osm_id}").fetchone()


def test_reversible_lane_added_to_both_directions():
    # Lions Gate Bridge pattern: 1 fixed each way + 1 shared reversible -> 2 lanes each way.
    con = _ways([(1, {"highway": "trunk", "lanes": "3", "lanes:forward": "1",
                      "lanes:backward": "1", "lanes:reversible": "1", "oneway": "no"})])
    assert _lanes(con, 1) == (2, 2)


def test_reversible_only_with_total():
    # reversible present, no explicit forward/backward: split total, then add the shared lane.
    con = _ways([(1, {"highway": "primary", "lanes": "2", "lanes:reversible": "1", "oneway": "no"})])
    fwd, bwd = _lanes(con, 1)
    assert fwd == 2 and bwd == 2          # CEIL/FLOOR(2/2)=1, +1 reversible


def test_no_reversible_is_unchanged():
    # Regression guard: COALESCE(n_rev,0) is a no-op when there's no reversible tag.
    con = _ways([
        (1, {"highway": "trunk", "lanes": "2", "oneway": "no"}),          # two-way split
        (2, {"highway": "motorway", "lanes": "2", "oneway": "yes"}),      # one-way, all forward
        (3, {"highway": "residential"}),                                  # untagged -> class default
        (4, {"highway": "trunk", "lanes:forward": "3", "lanes:backward": "2", "oneway": "no"}),
    ])
    assert _lanes(con, 1) == (1, 1)
    assert _lanes(con, 2)[0] == 2
    assert _lanes(con, 3) == (1, 1)
    assert _lanes(con, 4) == (3, 2)


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-q"]))
