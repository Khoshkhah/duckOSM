"""duckOSM's GMNS values agree with each other (src/duckosm/gmns_check.py).

Builds the tiny network of test_gmns, expects every check to pass, then breaks one thing at a time and
expects the matching check to fail, so a check that can never fail is caught.
"""
import duckdb
import pytest

pytest.importorskip("pandas")
from duckosm.gmns import to_gmns  # noqa: E402
from duckosm.gmns_check import check_gmns  # noqa: E402
from tests.test_gmns import _source  # noqa: E402


@pytest.fixture
def out(tmp_path):
    _source(tmp_path / "src.duckdb")
    path = tmp_path / "out.duckdb"
    to_gmns(str(tmp_path / "src.duckdb"), str(path))
    return path


def _failed(path):
    return {name for _, name, bad, _ in check_gmns(path) if bad}


def _break(path, sql):
    con = duckdb.connect(str(path))
    con.execute("LOAD spatial")
    con.execute(sql)
    con.close()


def test_a_fresh_build_passes_every_check(out):
    rows = check_gmns(out)
    assert len(rows) >= 16 and all(schema == "gmns_driving" for schema, *_ in rows)
    assert _failed(out) == set()
    assert any(total > 0 for *_, total in rows)         # not passing because there is nothing to check


@pytest.mark.parametrize("sql, check", [
    ("UPDATE gmns_driving.link SET length = length * 1.2", "link.length is its geometry's length"),
    ("UPDATE gmns_driving.link SET lanes = lanes + 5", "link.lanes is not more than the lane rows"),
    ("UPDATE gmns_driving.lane SET lane_num = lane_num + 1 WHERE lane_num = 2", "lane_num is 1..n"),
    ("UPDATE gmns_driving.movement SET node_id = node_id + 1", "movement turns where its inbound link ends"),
    ("UPDATE gmns_driving.movement SET start_ib_lane = 1, end_ib_lane = 9, start_ob_lane = 1, end_ob_lane = 9",
     "movement inbound lanes exist"),
    ("UPDATE gmns_driving.movement SET start_ib_lane = 1, end_ib_lane = 2, start_ob_lane = 1, end_ob_lane = 1",
     "movement lane ranges are as long on both sides"),
    ("UPDATE gmns_driving.node SET x_coord = x_coord + 0.001", "link starts at its from_node"),
    ("DELETE FROM gmns_driving.lane WHERE lane_num = 1", "lane_num is 1..n"),
    ("UPDATE gmns_driving.location SET lr = lr + 5000", "location lies on its link"),
])
def test_each_check_fails_on_the_matching_break(out, sql, check):
    _break(out, sql)
    assert check in _failed(out)


def test_a_way_tagged_with_more_lanes_than_turn_entries_still_gets_every_lane(tmp_path):
    """OSM lanes=3 with two `turn:lanes` entries: the third lane has no arrow but is still a lane."""
    from tests.test_gmns import A
    _source(tmp_path / "src.duckdb")
    con = duckdb.connect(str(tmp_path / "src.duckdb"))
    con.execute(f"UPDATE driving.edges SET lanes = 3 WHERE edge_id = {A}")           # its turn:lanes has 2
    con.close()
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "out.duckdb"))
    con = duckdb.connect(str(tmp_path / "out.duckdb"), read_only=True)
    assert con.execute(f"SELECT count(*), max(lane_num) FROM gmns_driving.lane WHERE link_id = {A}").fetchone() == (3, 3)
    assert _failed(tmp_path / "out.duckdb") == set()


@pytest.mark.parametrize("drive_side, left_of_lane_2", [("right", True), ("left", True)])
def test_lane_1_is_the_leftmost_lane_whichever_side_traffic_drives(tmp_path, drive_side, left_of_lane_2):
    """Lane 1 is the leftmost lane in the direction of travel (OSM's turn:lanes, the movements and the GMNS
    convention count from the left): with left-hand traffic the lanes lie left of the centre line, so lane 1 is
    the one farthest from it. Two-way road A has 2 lanes; its two lanes must not swap places between sides."""
    from shapely import wkt
    from tests.test_gmns import A
    _source(tmp_path / "src.duckdb")
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "out.duckdb"), drive_side=drive_side)
    con = duckdb.connect(str(tmp_path / "out.duckdb"), read_only=True)
    con.execute("LOAD spatial")
    link = wkt.loads(con.execute(f"SELECT ST_AsText(geom) FROM gmns_driving.link WHERE link_id = {A}").fetchone()[0])
    (x0, y0), (x1, y1) = link.coords[0], link.coords[-1]
    lane = {n: wkt.loads(g) for n, g in con.execute(
        f"SELECT lane_num, ST_AsText(geom) FROM gmns_driving.lane WHERE link_id = {A} ORDER BY lane_num").fetchall()}
    side = lambda g: (x1 - x0) * (g.centroid.y - y0) - (y1 - y0) * (g.centroid.x - x0)   # > 0: left of travel
    assert len(lane) == 2 and side(lane[1]) > side(lane[2])        # lane 1 is further left than lane 2
    # and the pair sits on the traffic side of the road line: right of it for right-hand traffic
    assert (side(lane[1]) + side(lane[2]) < 0) == (drive_side == "right")


def test_graph_report_counts_strongly_connected_parts_and_dead_ends(out):
    from duckosm.gmns_check import _strong_components, graph_report
    # A and AR join nodes 1 and 2 both ways; B leaves node 2 for node 3 and nothing comes back
    [(schema, nodes, links, big, share, parts, dead)] = graph_report(out)
    assert (schema, nodes, links, big, parts, dead) == ("gmns_driving", 3, 3, 2, 2, 1)
    assert share == pytest.approx(2 / 3)
    # a ring of three, a tail into it, a loop on its own, a pair one way
    ring = [(1, 2), (2, 3), (3, 1), (4, 1), (5, 5), (6, 7)]
    assert sorted(_strong_components(ring)) == [1, 1, 1, 1, 3]


def test_the_graph_report_survives_a_long_chain(out):
    from duckosm.gmns_check import _strong_components
    assert _strong_components([(i, i + 1) for i in range(200000)]) == [1] * 200001      # no recursion limit
