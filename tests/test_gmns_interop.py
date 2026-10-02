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
    src = tmp_path / f"b_{len(list(tmp_path.glob('b_*.duckdb')))}.duckdb"
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
        == [(1, "auto", 3.25), (2, "bike", 1.8)]
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


def _signs(tmp_path, tags, signal=False):
    """A sign on way 100's node 6 at (18.0695, 59.32): on link A (east), 28 m before node 2; node 2 has no signal."""
    src = tmp_path / f"sign_{len(list(tmp_path.glob('sign_*.duckdb')))}.duckdb"
    _source(src)
    con = duckdb.connect(str(src))
    con.execute("LOAD spatial")
    if not signal:
        con.execute("UPDATE raw.nodes SET tags = MAP{} WHERE osm_id = 2")
    con.execute(f"INSERT INTO raw.nodes VALUES (6, 59.32, 18.0695, {tags})")
    con.close()
    out = str(src).replace(".duckdb", "_gmns.duckdb")
    to_gmns(str(src), out)
    return duckdb.connect(out, read_only=True)


def test_a_give_way_or_stop_sign_on_the_approach_sets_the_movement_and_node_control(tmp_path):
    for tags, want in (("MAP{'highway':'give_way','direction':'forward'}", "yield"),
                       ("MAP{'highway':'stop','direction':'forward'}", "stop"),
                       ("MAP{'highway':'give_way','direction':'both'}", "yield")):
        c = _signs(tmp_path, tags)
        assert {r[0] for r in c.execute(f"SELECT ctrl_type FROM gmns_driving.movement WHERE ib_link_id = {A}").fetchall()} == {want}
        # node 2 has one approach (A) and it stops: every approach, fewer than four -> 'stop'; a yield stays 'yield'
        assert c.execute("SELECT ctrl_type FROM gmns_driving.node WHERE node_id = 2").fetchone()[0] == want
        assert c.execute(f"SELECT count(ctrl_type) FROM gmns_driving.movement WHERE ib_link_id <> {A}").fetchone()[0] == 0


def test_signs_that_say_nothing_about_this_approach_are_left_out(tmp_path):
    from tests.test_gmns import AR
    for tags in ("MAP{'highway':'give_way'}",                                  # no direction
                 "MAP{'highway':'give_way','direction':'backward'}",           # faces the other way: AR's, not A's
                 "MAP{'highway':'traffic_calming'}"):
        c = _signs(tmp_path, tags)
        assert c.execute(f"SELECT count(ctrl_type) FROM gmns_driving.movement WHERE ib_link_id = {A}").fetchone()[0] == 0
    c = _signs(tmp_path, "MAP{'highway':'give_way','direction':'backward'}")
    # AR runs west from node 2; the sign is 28 m after node 2 along it, not before a junction: no control either
    assert c.execute(f"SELECT count(ctrl_type) FROM gmns_driving.movement WHERE ib_link_id = {AR}").fetchone()[0] == 0
    # a signalised node keeps its signal whatever signs there are
    c = _signs(tmp_path, "MAP{'highway':'stop','direction':'forward'}", signal=True)
    assert {r[0] for r in c.execute(f"SELECT ctrl_type FROM gmns_driving.movement WHERE ib_link_id = {A}").fetchall()} == {"signal"}
    assert c.execute("SELECT ctrl_type FROM gmns_driving.node WHERE node_id = 2").fetchone()[0] == "signal"


def test_node_types_use_osm_names(tmp_path):
    _source(tmp_path / "src.duckdb")
    con = duckdb.connect(str(tmp_path / "src.duckdb"))
    con.execute("INSERT INTO raw.nodes VALUES (3, 59.31, 18.07, MAP{'highway':'turning_circle'})")
    con.close()
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "out.duckdb"))
    t = dict(duckdb.connect(str(tmp_path / "out.duckdb"), read_only=True).execute(
        "SELECT node_id, node_type FROM gmns_driving.node").fetchall())
    assert t == {1: "dead_end", 2: None, 3: "turning_circle"}      # 2 is a cut in a road: two neighbours, no type


def test_csv_extensions_are_kept_as_u_columns_only_when_asked(tmp_path):
    import csv
    _source(tmp_path / "src.duckdb")
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "plain.duckdb"), to_csv=str(tmp_path / "plain"))
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "ext.duckdb"), to_csv=str(tmp_path / "ext"), csv_extensions=True)
    head = lambda d, t: next(csv.reader(open(tmp_path / d / f"{t}.csv", newline="")))   # noqa: E731
    for table, extras in (("link", ["u_osm_id", "u_edge_ref", "u_footway", "u_crossing", "u_crossing_markings", "u_along_link_id", "u_along_mode", "u_along_gap_m", "u_along_kind", "u_bridge", "u_tunnel", "u_layer"]), ("lane", ["u_turn"]),
                          ("location", ["u_osm_id", "u_name"]), ("signal_controller", ["u_node_id", "u_control_type"])):
        plain, ext = head("plain", table), head("ext", table)
        assert ext == plain + extras, table                       # the standard columns first, unchanged
        assert not any(c.startswith("u_") for c in plain)
    assert "u_geom" not in head("ext", "link")                    # native geometry does not go to a CSV


def test_default_lane_width_by_class_and_connector_fit():
    from duckosm.gmns import _default_lane_w, _fit_width
    assert (_default_lane_w("service"), _default_lane_w("residential_link"), _default_lane_w("primary")) == (2.5, 3.0, 3.25)
    import math
    arc = lambda r: [(r * math.cos(t / 12 * math.pi), r * math.sin(t / 12 * math.pi)) for t in range(13)]   # a half circle
    assert _fit_width(arc(5), 3.25) == 3.25 and abs(_fit_width(arc(1), 3.25) - 1.95) < 1e-9     # roomy / tight: floor 60 %
    assert abs(_fit_width(arc(1.5), 3.25) - 2.7) < 0.01 and _fit_width([(0, 0), (1, 0), (2, 0)], 3.0) == 3.0


def test_an_untagged_one_way_road_takes_the_lanes_of_the_road_it_continues():
    from duckosm.gmns import _inherit_lanes

    def e(i, a, b, lanes, tagged, cls="secondary", oneway=True):
        return dict(edge_id=i, source=a, target=b, cls=cls, lanes=lanes, tagged=tagged, oneway=oneway)
    chain = [e(1, 1, 2, 2, True), e(2, 2, 3, 1, False), e(3, 3, 4, 1, False)]          # a 2-lane road, then two untagged pieces (a tunnel under another name)
    assert _inherit_lanes(chain) == {2: 2, 3: 2}                                         # piece by piece
    assert _inherit_lanes([e(1, 1, 2, 2, True), e(2, 2, 3, 1, True)]) == {}               # a tagged lanes=1 stays
    assert _inherit_lanes([e(1, 1, 2, 2, True, cls="primary"), e(2, 2, 3, 1, False)]) == {}   # another class
    assert _inherit_lanes([e(1, 1, 2, 2, True), e(4, 4, 2, 3, True), e(2, 2, 3, 1, False)]) == {}   # a junction: two roads lead in
    assert _inherit_lanes([e(1, 1, 2, 2, True), e(2, 2, 3, 1, False), e(5, 2, 6, 1, True, oneway=False)]) == {}   # a side road at the node
