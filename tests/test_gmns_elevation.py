"""`duckosm elevation` heights reach the GMNS output: node.z_coord, link.grade, location.z_coord."""
import duckdb
import pytest

pytest.importorskip("pandas")
from duckosm.gmns import to_gmns  # noqa: E402
from tests.test_gmns import A, AR, B, _source  # noqa: E402


def _build(tmp_path, elevation=True):
    src = tmp_path / "src.duckdb"
    _source(src)
    if elevation:
        con = duckdb.connect(str(src))
        con.execute("ALTER TABLE driving.nodes ADD COLUMN ele DOUBLE")
        con.execute("UPDATE driving.nodes SET ele = node_id * 10.0")                 # nodes 1, 2, 3 -> 10, 20, 30 m
        con.execute("ALTER TABLE driving.edges ADD COLUMN z_from DOUBLE; ALTER TABLE driving.edges ADD COLUMN z_to DOUBLE; "
                    "ALTER TABLE driving.edges ADD COLUMN tunnel VARCHAR")
        con.execute(f"UPDATE driving.edges SET z_from = 10, z_to = 20 WHERE edge_id = {A}")        # 1 -> 2: climbs
        con.execute(f"UPDATE driving.edges SET z_from = 20, z_to = 10 WHERE edge_id = {AR}")       # 2 -> 1: falls
        con.execute(f"UPDATE driving.edges SET z_from = 20, z_to = 90, tunnel = 'yes' WHERE edge_id = {B}")  # the hill above
        con.close()
    to_gmns(str(src), str(tmp_path / "out.duckdb"))
    c = duckdb.connect(str(tmp_path / "out.duckdb"), read_only=True)
    c.execute("LOAD spatial")
    return c


def test_heights_become_z_coord_grade_and_location_z(tmp_path):
    c = _build(tmp_path)
    q = lambda s: c.execute(s).fetchall()
    assert dict(q("SELECT node_id, z_coord FROM gmns_driving.node")) == {1: 10.0, 2: 20.0, 3: 30.0}
    length = dict(q("SELECT link_id, length FROM gmns_driving.link"))
    grade = dict(q("SELECT link_id, grade FROM gmns_driving.link"))
    assert grade[A] == pytest.approx(100 * 10 / length[A], rel=1e-6) and grade[AR] == pytest.approx(-grade[A])
    assert grade[B] is None                                   # a tunnel: the height is the hill above, not the road
    # the crossing half way along A is half way between its end heights
    z = dict(q("SELECT link_id, z_coord FROM gmns_driving.location WHERE osm_id = 4"))
    assert z[A] == pytest.approx(15.0, abs=0.1) and z[AR] == pytest.approx(15.0, abs=0.1)


def test_a_grade_over_100_percent_is_left_empty(tmp_path):
    c = _build(tmp_path)
    con = duckdb.connect(str(tmp_path / "src.duckdb"))
    con.execute(f"UPDATE driving.edges SET z_from = 0, z_to = 5000 WHERE edge_id = {A}")        # 5 km up in 570 m
    con.close()
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "out2.duckdb"))
    c = duckdb.connect(str(tmp_path / "out2.duckdb"), read_only=True)
    assert c.execute(f"SELECT grade FROM gmns_driving.link WHERE link_id = {A}").fetchone()[0] is None


def test_without_heights_both_stay_empty(tmp_path):
    c = _build(tmp_path, elevation=False)
    assert c.execute("SELECT count(z_coord) FROM gmns_driving.node").fetchone()[0] == 0
    assert c.execute("SELECT count(grade) FROM gmns_driving.link").fetchone()[0] == 0
    assert c.execute("SELECT count(z_coord) FROM gmns_driving.location").fetchone()[0] == 0
