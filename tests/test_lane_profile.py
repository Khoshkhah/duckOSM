"""The lane profile's rules (docs/design/gmns_lane_profile.md), one tag case per row: OSM tags in, the lanes of the GMNS ``lane`` table out,
left to right. The Rust build writes the edges, ``duckosm.to_gmns`` makes the lanes. The unit tests of ``lanes_of`` (the source of each lane) are in rust/duckosm-core/src/lanes.rs."""
import duckdb
import pytest

import duckosm
from osm_build import build

X = 0.001   # about 81 m


def _lanes(tmp_path, tags, edge_ref="10#1f", highway="primary", ways=None):
    """The lanes ``[(lane_num, allowed_uses, width, turn)]`` of the link ``edge_ref`` of way 10 (nodes 1-2-3, about 160 m)."""
    nodes = {1: (7.0, 43.0), 2: (7.0 + X, 43.0), 3: (7.0 + 2 * X, 43.0), 5: (7.0 + X, 43.0 + X)}
    ways = ways or {10: ([1, 2, 3], {"highway": highway, **tags})}
    n = len(list(tmp_path.glob("l*.duckdb")))
    db = build(tmp_path, nodes, ways, name=f"l{n}")
    duckosm.to_gmns(str(db), str(tmp_path / f"g{n}.duckdb"))
    con = duckdb.connect(str(tmp_path / f"g{n}.duckdb"), read_only=True)
    return con.execute("SELECT l.lane_num, l.allowed_uses, l.width, l.turn FROM gmns_driving.lane l JOIN gmns_driving.link k USING (link_id) "
                       "WHERE k.edge_ref = ? ORDER BY l.lane_num", [edge_ref]).fetchall()


# (case, tags, edge_ref, [(lane_num, use)]); every road is one-way unless the case says it is a two-way road (then lanes counts both directions)
CASES = [
    ("untagged one-way: one default lane", {"oneway": "yes"}, "10#1f", [(1, "auto")]),
    ("lanes=2 one-way", {"oneway": "yes", "lanes": "2"}, "10#1f", [(1, "auto"), (2, "auto")]),
    ("more turn:lanes entries than lanes: one lane per entry", {"oneway": "yes", "lanes": "2", "turn:lanes": "left|through|right"}, "10#1f",
     [(1, "auto"), (2, "auto"), (3, "auto")]),
    ("bus:lanes marks its lane", {"oneway": "yes", "lanes": "3", "bus:lanes": "||designated"}, "10#1f", [(1, "auto"), (2, "auto"), (3, "bus")]),
    ("lanes:bus=1: the right-most motor lane", {"oneway": "yes", "lanes": "3", "lanes:bus": "1"}, "10#1f", [(1, "auto"), (2, "auto"), (3, "bus")]),
    ("busway:right=lane on the forward edge", {"oneway": "yes", "lanes": "2", "busway:right": "lane"}, "10#1f", [(1, "auto"), (2, "bus")]),
    ("busway:right=lane is not the reverse edge's", {"lanes": "2", "busway:right": "lane"}, "10#1r", [(1, "auto")]),
    ("cycleway:right=lane: a bike lane right of the motor lanes, not counted in lanes", {"oneway": "yes", "lanes": "2", "cycleway:right": "lane"}, "10#1f",
     [(1, "auto"), (2, "auto"), (3, "bike")]),
    ("cycleway:left=lane on a one-way, with the traffic: lane -1", {"oneway": "yes", "lanes": "1", "cycleway:left": "lane"}, "10#1f",
     [(-1, "bike"), (1, "auto")]),
    ("cycleway:left=lane against the traffic (oneway:bicycle=no): not on this edge", {"oneway": "yes", "oneway:bicycle": "no", "cycleway:left": "lane"},
     "10#1f", [(1, "auto")]),
    ("on a two-way road cycleway:left is the reverse edge's right", {"lanes": "2", "cycleway:left": "lane"}, "10#1r", [(1, "auto"), (2, "bike")]),
    ("a bus-only edge (contraflow bus lane), bikes sharing it", {"oneway": "yes", "oneway:bus": "no", "cycleway:left": "share_busway"}, "10#1r",
     [(1, "bus,bike")]),
    ("share_busway on a bus lane of this side", {"oneway": "yes", "lanes": "2", "bus:lanes": "|designated", "cycleway:right": "share_busway"}, "10#1f",
     [(1, "auto"), (2, "bus,bike")]),
]


@pytest.mark.parametrize("case,tags,edge_ref,want", CASES, ids=[c[0] for c in CASES])
def test_a_tag_case(tmp_path, case, tags, edge_ref, want):
    got = _lanes(tmp_path, tags, edge_ref)
    assert [(n, use) for n, use, _, _ in got] == want


def test_widths(tmp_path):
    got = _lanes(tmp_path, {"oneway": "yes", "lanes": "2", "width:lanes": "3.5|", "cycleway:right": "lane", "cycleway:right:width": "1.8"},
                 highway="residential")
    assert [w for _, _, w, _ in got] == [3.5, 3.0, 1.8]          # width:lanes, the class default, the cycleway's width
    assert _lanes(tmp_path, {}, highway="service")[0][2] == 2.5   # the default of a service road


def test_arrows_only_where_the_way_ends(tmp_path):
    tags = {"oneway": "yes", "lanes": "2", "turn:lanes": "left|right"}
    assert [t for *_, t in _lanes(tmp_path, tags)] == ["left", "right"]
    # a side street joins at node 2: the first edge ends part way along the way: same lanes, no arrows
    side = {10: ([1, 2, 3], {"highway": "primary", **tags}), 11: ([2, 5], {"highway": "residential"})}
    mid = _lanes(tmp_path, tags, "10#1f", ways=side)
    assert [(n, t) for n, _, _, t in mid] == [(1, None), (2, None)]
    side[10] = ([1, 2, 3], {"highway": "primary", "oneway": "yes", "lanes": "1", "turn:lanes": "left|right"})
    assert len(_lanes(tmp_path, {}, "10#1f", ways=side)) == 2
