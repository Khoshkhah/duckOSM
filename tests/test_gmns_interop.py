"""What the GMNS README asks of the output, beyond the table shapes: a zone every node is in (so tools such as
Path4GMNS load it), a sidewalk's parent road, on-road bike lanes as explicit lanes.
"""
import duckdb
import pytest

pytest.importorskip("pandas")
from duckosm.gmns import to_gmns  # noqa: E402
from tests.test_gmns import A, AR, B, _source  # noqa: E402


def test_there_is_always_one_zone_and_every_node_is_in_it(tmp_path):
    _source(tmp_path / "src.duckdb")
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "out.duckdb"))
    con = duckdb.connect(str(tmp_path / "out.duckdb"), read_only=True)
    assert con.execute("SELECT zone_id, name FROM gmns_driving.zone").fetchall() == [(1, "Testville")]
    assert con.execute("SELECT count(*), count(zone_id), min(zone_id), max(zone_id) FROM gmns_driving.node").fetchone() == (3, 3, 1, 1)
    assert con.execute("SELECT DISTINCT zone_id FROM gmns_driving.location").fetchall() == [(1,)]
    # an area built without a boundary still has its zone, named after the output file, with no outline
    src = duckdb.connect(str(tmp_path / "src.duckdb"))
    src.execute("DROP TABLE main.boundary")
    src.close()
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "area_gmns.duckdb"))
    con = duckdb.connect(str(tmp_path / "area_gmns.duckdb"), read_only=True)
    assert con.execute("SELECT zone_id, name, boundary FROM gmns_driving.zone").fetchall() == [(1, "area", None)]


def _walking(tmp_path, drive_side="right"):
    """A and AR run east-west along y = 59.32; a sidewalk 7.8 m north of them (way 200), one 7.8 m south (201)."""
    src = tmp_path / f"src_{drive_side}.duckdb"
    _source(src)
    con = duckdb.connect(str(src))
    con.execute("LOAD spatial; CREATE SCHEMA walking")
    con.execute("CREATE TABLE walking.nodes AS SELECT * FROM driving.nodes")
    con.execute("CREATE TABLE walking.edges AS SELECT * FROM driving.edges")
    con.execute("INSERT INTO raw.ways VALUES (200, MAP{'highway':'footway','footway':'sidewalk'}, [1,2]), "
                "(201, MAP{'highway':'footway','footway':'sidewalk'}, [1,2]), (202, MAP{'highway':'footway'}, [1,2])")
    for edge_id, osm_id, y in ((9001, 200, 59.32007), (9002, 201, 59.31993), (9003, 202, 59.32007)):
        con.execute(f"INSERT INTO walking.edges (edge_id, source, target, osm_id, highway, lanes, is_reverse, length_m, maxspeed_kmh, geometry) "
                    f"VALUES ({edge_id}, 1, 2, {osm_id}, 'footway', 1, false, 80, 5, ST_GeomFromText('LINESTRING(18.06 {y}, 18.07 {y})'))")
    con.close()
    to_gmns(str(src), str(tmp_path / f"out_{drive_side}.duckdb"), modes=["walking"], drive_side=drive_side)
    c = duckdb.connect(str(tmp_path / f"out_{drive_side}.duckdb"), read_only=True)
    return dict(c.execute("SELECT link_id, parent_link_id FROM gmns_walking.link").fetchall())


def test_a_sidewalk_has_the_road_it_runs_along_as_parent(tmp_path):
    parent = _walking(tmp_path)
    # right-hand traffic: the parent is the road link the sidewalk is on the right of (the kerb side):
    # the north one is on the right of AR (heading west), the south one on the right of A (heading east)
    assert parent[9001] == AR and parent[9002] == A
    assert parent[9003] is None                        # a footway that is not tagged footway=sidewalk
    assert parent[A] is None and parent[B] is None     # roads have no parent
    assert _walking(tmp_path, "left")[9001] == A       # left-hand traffic mirrors it


def _bike_lane(tmp_path, tags, drive_side="right"):
    src = tmp_path / f"b_{drive_side}_{abs(hash(tags))}.duckdb"
    _source(src)
    con = duckdb.connect(str(src))
    con.execute(f"UPDATE raw.ways SET tags = {tags} WHERE osm_id = 101")           # way 101 is link B, one lane
    con.close()
    out = str(src).replace(".duckdb", "_gmns.duckdb")
    to_gmns(str(src), out, drive_side=drive_side)
    c = duckdb.connect(out, read_only=True)
    c.execute("LOAD spatial")
    return c


def test_cycleway_lane_is_an_explicit_bike_lane_beside_the_motor_lanes(tmp_path):
    c = _bike_lane(tmp_path, "MAP{'cycleway:right':'lane', 'cycleway:right:width':'1.8'}")
    assert c.execute(f"SELECT lane_num, allowed_uses, width FROM gmns_driving.lane WHERE link_id = {B} ORDER BY lane_num").fetchall() \
        == [(1, "auto", None), (2, "bike", 1.8)]
    assert c.execute(f"SELECT lanes FROM gmns_driving.link WHERE link_id = {B}").fetchone()[0] == 1      # motor lanes only
    # the movement into B still uses lane 1 only: the bike lane is not a place to turn into
    assert c.execute(f"SELECT start_ob_lane, end_ob_lane FROM gmns_driving.movement WHERE ob_link_id = {B}").fetchone() == (1, 1)
    # and the bike lane lies right of the car lane (B heads south: right is west, x smaller)
    x = dict(c.execute(f"SELECT lane_num, ST_X(ST_StartPoint(geom)) FROM gmns_driving.lane WHERE link_id = {B}").fetchall())
    assert x[2] < x[1]
    # the other side's tag, a track, an existing bicycle:lanes lane and left-hand traffic add nothing
    for tags, side in (("MAP{'cycleway:left':'lane'}", "right"), ("MAP{'cycleway:right':'track'}", "right"),
                       ("MAP{'cycleway:right':'lane'}", "left")):
        c = _bike_lane(tmp_path, tags, side)
        assert c.execute(f"SELECT count(*) FROM gmns_driving.lane WHERE link_id = {B}").fetchone()[0] == 1, (tags, side)


def test_path4gmns_loads_the_csv_and_routes_on_it(tmp_path):
    """The README names Path4GMNS: it refused a node.csv without any zone."""
    pg = pytest.importorskip("path4gmns")
    _source(tmp_path / "src.duckdb")
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "out.duckdb"), to_csv=str(tmp_path / "csv"))
    for f in tmp_path.glob("csv/*.csv"):
        if f.stem not in ("node", "link"):
            f.unlink()
    net = pg.read_network(input_dir=str(tmp_path / "csv"))
    assert net.find_shortest_path(1, 3, mode="auto", seq_type="node", cost_type="distance")
