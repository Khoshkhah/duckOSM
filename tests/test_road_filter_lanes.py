"""Lane counts per direction in the Rust build (`driving.ways.lanes_fwd` / `lanes_bwd`).

A shared reversible lane (Lions Gate Bridge: lanes=3, lanes:forward=1, lanes:backward=1,
lanes:reversible=1) is open to each direction at its peak, so it counts in BOTH directions.
"""
import duckdb

from osm_build import build


def _lanes(tmp_path, tagsets):
    """One short way per tag set (ids 1, 2, ...): {id: (lanes_fwd, lanes_bwd)} from driving.ways."""
    nodes, ways = {}, {}
    for i, tags in enumerate(tagsets, 1):
        nodes[i * 10], nodes[i * 10 + 1] = (7.0, 43 + i * 0.01), (7.0012, 43 + i * 0.01)
        ways[i] = ([i * 10, i * 10 + 1], tags)
    con = duckdb.connect(str(build(tmp_path, nodes, ways)), read_only=True)
    return {r[0]: r[1:] for r in con.execute("SELECT osm_id, lanes_fwd, lanes_bwd FROM driving.ways").fetchall()}


def test_reversible_lane_added_to_both_directions(tmp_path):
    # 1 fixed lane each way + 1 shared reversible lane -> 2 lanes each way.
    got = _lanes(tmp_path, [{"highway": "trunk", "lanes": "3", "lanes:forward": "1",
                             "lanes:backward": "1", "lanes:reversible": "1", "oneway": "no"}])
    assert got[1] == (2, 2)


def test_reversible_only_with_total(tmp_path):
    # reversible present, no forward/backward: split the total, then add the shared lane.
    got = _lanes(tmp_path, [{"highway": "primary", "lanes": "2", "lanes:reversible": "1", "oneway": "no"}])
    assert got[1] == (2, 2)


def test_no_reversible_is_unchanged(tmp_path):
    got = _lanes(tmp_path, [
        {"highway": "trunk", "lanes": "2", "oneway": "no"},        # two-way split
        {"highway": "motorway", "lanes": "2", "oneway": "yes"},    # one-way, all forward
        {"highway": "residential"},                                # untagged -> class default
        {"highway": "trunk", "lanes:forward": "3", "lanes:backward": "2", "oneway": "no"},
    ])
    assert got[1] == (1, 1)
    assert got[2][0] == 2
    assert got[3] == (1, 1)
    assert got[4] == (3, 2)
