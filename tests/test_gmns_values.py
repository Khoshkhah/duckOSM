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
    ("UPDATE gmns_driving.link SET lanes = lanes + 1", "link.lanes is the number of lane rows"),
    ("UPDATE gmns_driving.lane SET lane_num = lane_num + 1 WHERE lane_num = 2", "lane_num is 1..n"),
    ("UPDATE gmns_driving.movement SET node_id = node_id + 1", "movement turns where its inbound link ends"),
    ("UPDATE gmns_driving.movement SET start_ib_lane = 1, end_ib_lane = 9, start_ob_lane = 1, end_ob_lane = 9",
     "movement inbound lanes exist"),
    ("UPDATE gmns_driving.movement SET start_ib_lane = 1, end_ib_lane = 2, start_ob_lane = 1, end_ob_lane = 1",
     "movement lane ranges are as long on both sides"),
    ("UPDATE gmns_driving.node SET x_coord = x_coord + 0.001", "link starts at its from_node"),
    ("DELETE FROM gmns_driving.lane WHERE lane_num = 1", "lane_num is 1..n"),
])
def test_each_check_fails_on_the_matching_break(out, sql, check):
    _break(out, sql)
    assert check in _failed(out)
