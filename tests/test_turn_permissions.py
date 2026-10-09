"""Turn restrictions with exceptions, conditions and vehicle classes (docs/design/turn_permissions.md): edge_graph keeps the turns
open to all traffic, turn_permission says what edge_graph does not, and the SUMO export writes it as connection permissions."""
import re

import duckdb
import pytest

from duckosm.processors.edge_graph import EdgeGraphBuilder
from duckosm.processors.restrictions import RestrictionProcessor
from duckosm.sumo import _find_netconvert, to_sumo

# a junction at node 2: one road in (way 100, edge 1) and three out: left (way 200, edge 2), right (way 300, edge 3),
# straight (way 400, edge 4)
WAYS = {1: (100, 1, 2), 2: (200, 2, 3), 3: (300, 2, 4), 4: (400, 2, 5)}
PTS = {1: (18.0700, 59.3190), 2: (18.0700, 59.3200), 3: (18.0680, 59.3200), 4: (18.0720, 59.3200), 5: (18.0700, 59.3210)}
RULES = [  # (relation, tags, from way, to way)
    (11, {"restriction": "no_left_turn", "except": "psv"}, 100, 200),                          # banned, except buses and taxis
    (12, {"restriction:conditional": "no_right_turn @ (Mo-Fr 07:00-09:00)"}, 100, 300),        # banned at rush hour only
    (13, {"restriction:hgv": "no_straight_on"}, 100, 400),                                     # banned for lorries only
]


def _db():
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA raw; CREATE SCHEMA driving")
    con.execute("CREATE TABLE raw.relations(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[], ref_roles VARCHAR[], ref_types VARCHAR[])")
    for rid, tags, fw, tw in RULES:
        con.execute("INSERT INTO raw.relations VALUES (?, MAP(?, ?), ?, ?, ?)",
                    [rid, ["type", *tags], ["restriction", *tags.values()], [fw, 2, tw], ["from", "via", "to"], ["way", "node", "way"]])
    con.execute("USE driving")
    con.execute("CREATE TABLE nodes(node_id BIGINT, geom GEOMETRY)")
    con.executemany("INSERT INTO nodes VALUES (?, ST_Point(?, ?))", [(n, x, y) for n, (x, y) in PTS.items()])
    con.execute("CREATE TABLE edges(edge_id BIGINT, osm_id BIGINT, source BIGINT, target BIGINT, refs BIGINT[], cost_s DOUBLE, highway VARCHAR, "
                "name VARCHAR, lanes INTEGER, maxspeed_kmh FLOAT, length_m FLOAT, geometry GEOMETRY)")
    con.execute("CREATE TABLE way_nodes(way_id BIGINT, node_id BIGINT, seq INTEGER)")
    for eid, (way, a, b) in WAYS.items():
        (x1, y1), (x2, y2) = PTS[a], PTS[b]
        con.execute(f"INSERT INTO edges VALUES ({eid}, {way}, {a}, {b}, [{a}, {b}], 10, 'primary', NULL, 1, 50, 110, "
                    f"ST_GeomFromText('LINESTRING({x1} {y1}, {x2} {y2})'))")
        con.executemany("INSERT INTO way_nodes VALUES (?, ?, ?)", [(way, a, 0), (way, b, 1)])
    RestrictionProcessor(con).run()
    EdgeGraphBuilder(con).run()
    return con


def test_rules_keep_their_exceptions_conditions_and_vehicles():
    con = _db()
    rules = {r[0]: r[1:] for r in con.execute(
        "SELECT restriction_id, restriction_type, except_vehicles, applies_to, condition FROM turn_restrictions").fetchall()}
    assert rules == {11: ("no_left_turn", "psv", None, None), 12: ("no_right_turn", None, None, "Mo-Fr 07:00-09:00"),
                     13: ("no_straight_on", None, "hgv", None)}


def test_edge_graph_keeps_the_turns_open_to_all_and_turn_permission_the_rest():
    con = _db()
    assert {r[0] for r in con.execute("SELECT to_edge FROM edge_graph WHERE from_edge = 1").fetchall()} == {3, 4}   # not the left
    perm = {r[0]: r[1:] for r in con.execute(
        "SELECT to_edge, allowed, vehicles, except_vehicles, condition FROM turn_permission WHERE from_edge = 1").fetchall()}
    assert perm == {2: (True, "psv", None, None),                    # the left: open to buses and taxis
                    3: (False, None, None, "Mo-Fr 07:00-09:00"),     # the right: closed to all at rush hour
                    4: (False, "hgv", None, None)}                   # straight on: closed to lorries


def test_sumo_writes_the_permissions(tmp_path):
    try:
        _find_netconvert()
    except Exception:
        pytest.skip("netconvert not installed (pip install duckosm[sumo])")
    to_sumo(_db(), str(tmp_path), net_name="p")
    con_xml = (tmp_path / "p.con.xml").read_text()
    assert re.search(r'from="1" to="2" fromLane="\d+" toLane="\d+" allow="bus coach taxi"', con_xml)   # the left, buses and taxis only
    assert re.search(r'from="1" to="4" fromLane="\d+" toLane="\d+" disallow="trailer truck"', con_xml)  # straight on, no lorries
    assert 'from="1" to="3"' not in con_xml        # the rush-hour ban written in force: SUMO has no time
    net = (tmp_path / "p.net.xml").read_text()
    assert set(re.findall(r'<connection from="1" to="(\d+)"', net)) == {"2", "4"}


def _split_junction():
    """A crossroads mapped as two nodes 6 m apart (A=2 south, B=3 north) joined by a link way 200; two-way roads. Relation 21:
    no left turn from the south road (way 100) via the link (way 200) onto the west road (way 400), except buses."""
    pts = {1: (18.0700, 59.3190), 2: (18.0700, 59.3200), 3: (18.0700, 59.320054), 4: (18.0700, 59.3210),
           5: (18.0680, 59.320054), 6: (18.0720, 59.3200)}
    legs = {11: (100, 1, 2), 12: (200, 2, 3), 13: (300, 3, 4), 14: (400, 3, 5), 15: (500, 2, 6)}
    legs.update({20 + k: (w, b, a) for k, (w, a, b) in legs.items()})
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA raw; CREATE SCHEMA driving")
    con.execute("CREATE TABLE raw.relations(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[], ref_roles VARCHAR[], ref_types VARCHAR[])")
    con.execute("INSERT INTO raw.relations VALUES (21, MAP(['type', 'restriction', 'except'], ['restriction', 'no_left_turn', 'bus']), "
                "[100, 200, 400], ['from', 'via', 'to'], ['way', 'way', 'way'])")
    con.execute("USE driving")
    con.execute("CREATE TABLE nodes(node_id BIGINT, geom GEOMETRY)")
    con.executemany("INSERT INTO nodes VALUES (?, ST_Point(?, ?))", [(n, x, y) for n, (x, y) in pts.items()])
    con.execute("CREATE TABLE edges(edge_id BIGINT, osm_id BIGINT, source BIGINT, target BIGINT, refs BIGINT[], cost_s DOUBLE, highway VARCHAR, "
                "name VARCHAR, lanes INTEGER, maxspeed_kmh FLOAT, length_m FLOAT, geometry GEOMETRY)")
    con.execute("CREATE TABLE way_nodes(way_id BIGINT, node_id BIGINT, seq INTEGER)")
    for eid, (way, a, b) in legs.items():
        (x1, y1), (x2, y2) = pts[a], pts[b]
        length = 6 if way == 200 else 110
        con.execute(f"INSERT INTO edges VALUES ({eid}, {way}, {a}, {b}, [{a}, {b}], 10, 'primary', NULL, 1, 50, {length}, "
                    f"ST_GeomFromText('LINESTRING({x1} {y1}, {x2} {y2})'))")
        if eid < 20:
            con.executemany("INSERT INTO way_nodes VALUES (?, ?, ?)", [(way, a, 0), (way, b, 1)])
    RestrictionProcessor(con).run()
    EdgeGraphBuilder(con).run()
    return con


def test_a_via_way_restriction_is_a_path():
    con = _split_junction()
    assert con.execute("SELECT restriction_type, except_vehicles, from_edge, via_edges, to_edge FROM turn_path_restrictions").fetchall() \
        == [("no_left_turn", "bus", 11, [12], 14)]
    assert con.execute("SELECT count(*) FROM edge_graph WHERE (from_edge, to_edge) IN ((11, 12), (12, 14))").fetchone()[0] == 2   # untouched


def test_sumo_applies_a_via_way_restriction_in_a_joined_junction(tmp_path):
    try:
        _find_netconvert()
    except Exception:
        pytest.skip("netconvert not installed (pip install duckosm[sumo])")
    out = to_sumo(_split_junction(), str(tmp_path), net_name="v", config={"junctions.join": "true"})
    assert out["n_path_restrictions"] == 1
    con_xml = (tmp_path / "v.con.xml").read_text()
    assert re.search(r'from="11" to="14" fromLane="\d+" toLane="\d+" allow="bus coach"', con_xml)   # the left: buses only
    assert re.search(r'<connection from="11" to="13"/>|from="11" to="13" fromLane="\d+" toLane="\d+"/>', con_xml)  # straight: all
