"""What the GMNS README asks of the output, beyond the table shapes: a zone every node is in (so tools such as
Path4GMNS load it), a sidewalk's parent road, on-road bike lanes as explicit lanes, the lanes an untagged one-way road takes from the road
it continues. The zone, sidewalk and sign tests read a small hand-made source db (``tests.test_gmns._source``); the bike lane and lane count
tests build their network from OSM ways with the Rust build (``tests/osm_build.py``).
"""
import duckdb
import pytest
from osm_build import build

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


def _bike_lane(tmp_path, b_tags, drive_side="right"):
    """A built network: way 100 (A, two-way primary, nodes 1-2) and way 101 (B, nodes 2-3, heading south) with the tags ``b_tags``.
    Returns a connection to the GMNS db and the link ids of B and of A."""
    n = len(list(tmp_path.glob("b_*.duckdb")))
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 3: (18.07, 59.31)}
    ways = {100: ([1, 2], {"highway": "primary", "lanes": "2"}), 101: ([2, 3], {"highway": "residential", **b_tags})}
    src = build(tmp_path, nodes, ways, name=f"b_{n}")
    out = str(tmp_path / f"b_{n}_gmns.duckdb")
    to_gmns(str(src), out, drive_side=drive_side)
    c = duckdb.connect(out, read_only=True)
    c.execute("LOAD spatial")
    link = lambda ref: c.execute("SELECT link_id FROM gmns_driving.link WHERE edge_ref = ?", [ref]).fetchone()[0]
    return c, link("101#1f"), link("100#1f")


def test_cycleway_lane_is_an_explicit_bike_lane_beside_the_motor_lanes(tmp_path):
    c, b, a = _bike_lane(tmp_path, {"cycleway:right": "lane", "cycleway:right:width": "1.8"})
    lanes = c.execute(f"SELECT lane_num, allowed_uses, width FROM gmns_driving.lane WHERE link_id = {b} ORDER BY lane_num").fetchall()
    assert [(n, u) for n, u, _ in lanes] == [(1, "auto"), (2, "bike")] and lanes[1][2] == 1.8
    assert c.execute(f"SELECT lanes FROM gmns_driving.link WHERE link_id = {b}").fetchone()[0] == 1      # motor lanes only
    # the movement into B still uses lane 1 only: the bike lane is not a place to turn into
    assert c.execute(f"SELECT DISTINCT start_ob_lane, end_ob_lane FROM gmns_driving.movement WHERE ob_link_id = {b}").fetchall() == [(1, 1)]
    # and the bike lane lies right of the car lane (B heads south: right is west, x smaller)
    x = dict(c.execute(f"SELECT lane_num, ST_X(ST_StartPoint(geom)) FROM gmns_driving.lane WHERE link_id = {b}").fetchall())
    assert x[2] < x[1]
    # the other side's tag and a track add nothing
    for tags in ({"cycleway:left": "lane"}, {"cycleway:right": "track"}):
        c, b, _ = _bike_lane(tmp_path, tags)
        assert c.execute(f"SELECT count(*) FROM gmns_driving.lane WHERE link_id = {b}").fetchone()[0] == 1, tags


@pytest.mark.xfail(strict=True, reason="With drive_side=left the GMNS export still adds the bike lane from the lane profile in the built db "
                   "(the right-hand rule in gmns.rs bike_extra is only used when the db has no lane profile).")
def test_left_hand_traffic_adds_no_bike_lane(tmp_path):
    c, b, _ = _bike_lane(tmp_path, {"cycleway:right": "lane"}, "left")
    assert c.execute(f"SELECT count(*) FROM gmns_driving.lane WHERE link_id = {b}").fetchone()[0] == 1


def test_a_left_bike_lane_on_a_one_way_road_is_lane_minus_one(tmp_path):
    """cycleway:left=lane on a one-way road (2026-10-10, Avenue Princesse Grace): a bike lane LEFT of the motor lanes, GMNS lane -1 (left of the
    left-most through lane, the spec's negative numbers), so the motor lanes keep 1..n and the movements their numbers."""
    c, b, _ = _bike_lane(tmp_path, {"oneway": "yes", "cycleway:left": "lane"})
    lanes = c.execute(f"SELECT lane_num, allowed_uses, width FROM gmns_driving.lane WHERE link_id = {b} ORDER BY lane_num").fetchall()
    assert [(n, u) for n, u, _ in lanes] == [(-1, "bike"), (1, "auto")] and lanes[0][2] < lanes[1][2]
    assert c.execute(f"SELECT DISTINCT start_ob_lane, end_ob_lane FROM gmns_driving.movement WHERE ob_link_id = {b}").fetchall() == [(1, 1)]
    # B heads south: its left is east (x larger)
    x = dict(c.execute(f"SELECT lane_num, ST_X(ST_StartPoint(geom)) FROM gmns_driving.lane WHERE link_id = {b}").fetchall())
    assert x[-1] > x[1]


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


def _link_lanes(tmp_path, ways):
    """``{osm_id: lanes}`` of the GMNS links of one-way ways ``{osm_id: (refs, tags)}`` (nodes on a line, about 80 m apart)."""
    n = len(list(tmp_path.glob("i_*.duckdb")))
    ids = sorted({r for refs, _ in ways.values() for r in refs})
    nodes = {r: (7.0 + 0.001 * k, 43.0 + 0.0004 * (k % 2)) for k, r in enumerate(ids)}
    src = build(tmp_path, nodes, ways, name=f"i_{n}")
    out = str(tmp_path / f"i_{n}_gmns.duckdb")
    to_gmns(str(src), out)
    c = duckdb.connect(out, read_only=True)
    return dict(c.execute("SELECT osm_id, lanes FROM gmns_driving.link WHERE edge_ref LIKE '%f'").fetchall())


def test_an_untagged_one_way_road_takes_the_lanes_of_the_road_it_continues(tmp_path):
    def way(refs, lanes=None, cls="secondary", oneway=True):
        return refs, {"highway": cls, **({"lanes": str(lanes)} if lanes else {}), **({"oneway": "yes"} if oneway else {})}
    # a 2-lane road, then two untagged pieces (a tunnel under another name): piece by piece
    chain = {1: way([1, 2], 2), 2: way([2, 3]), 3: way([3, 4])}
    assert _link_lanes(tmp_path, chain) == {1: 2, 2: 2, 3: 2}
    assert _link_lanes(tmp_path, {1: way([1, 2], 2), 2: way([2, 3], 1)})[2] == 1                       # a tagged lanes=1 stays
    assert _link_lanes(tmp_path, {1: way([1, 2], 2, cls="primary"), 2: way([2, 3])})[2] == 1           # another class
    assert _link_lanes(tmp_path, {1: way([1, 2], 2), 4: way([4, 2], 3), 2: way([2, 3])})[2] == 1       # a junction: two roads lead in
    assert _link_lanes(tmp_path, {1: way([1, 2], 2), 2: way([2, 3]), 5: way([2, 6], 1, oneway=False)})[2] == 1   # a side road at the node
