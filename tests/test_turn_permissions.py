"""Turn restrictions with exceptions, conditions and vehicle classes (docs/design/turn_permissions.md): edge_graph keeps the turns
open to all traffic, turn_permission says what edge_graph does not, and the SUMO export writes it as connection permissions.
The roads are OSM data written by the test and built by the Rust build (tests/osm_build.py)."""
import re
import sys
from pathlib import Path

import duckdb
import pytest

sys.path.insert(0, str(Path(__file__).parent))
from osm_build import build
from duckosm.sumo import _find_netconvert, to_sumo


def _restriction(rid, tags, via, f, t):
    return rid, ([("way", f, "from"), (via[0], via[1], "via"), ("way", t, "to")], {"type": "restriction", **tags})


def _ids(db):
    """``{edge_ref: edge_id}``: the edge ids are hashes, the refs (``<osm id>#<n>f`` or ``r``) are not."""
    con = duckdb.connect(str(db), read_only=True)
    return {r: i for r, i in con.execute("SELECT edge_ref, edge_id FROM driving.edges").fetchall()}


# a junction at node 2: one road in (way 100) and three out: left (way 200), right (way 300), straight (way 400)
@pytest.fixture(scope="module")
def junction(tmp_path_factory):
    nodes = {1: (7.0, 43.0), 2: (7.0, 43.001), 3: (6.999, 43.001), 4: (7.001, 43.001), 5: (7.0, 43.002)}
    ways = {w: (refs, {"highway": "primary"}) for w, refs in {100: [1, 2], 200: [2, 3], 300: [2, 4], 400: [2, 5]}.items()}
    rels = dict([
        _restriction(11, {"restriction": "no_left_turn", "except": "psv"}, ("node", 2), 100, 200),                       # banned, except buses and taxis
        _restriction(12, {"restriction:conditional": "no_right_turn @ (Mo-Fr 07:00-09:00)"}, ("node", 2), 100, 300),   # banned at rush hour only
        _restriction(13, {"restriction:hgv": "no_straight_on"}, ("node", 2), 100, 400),                              # banned for lorries only
    ])
    db = build(tmp_path_factory.mktemp("perm"), nodes, ways, rels)
    return db, _ids(db), duckdb.connect(str(db), read_only=True)


def test_rules_keep_their_exceptions_conditions_and_vehicles(junction):
    con = junction[2]
    rules = {r[0]: r[1:] for r in con.execute(
        "SELECT restriction_id, restriction_type, except_vehicles, applies_to, condition FROM driving.turn_restrictions").fetchall()}
    assert rules == {11: ("no_left_turn", "psv", None, None), 12: ("no_right_turn", None, None, "Mo-Fr 07:00-09:00"),
                     13: ("no_straight_on", None, "hgv", None)}


def test_edge_graph_keeps_the_turns_open_to_all_and_turn_permission_the_rest(junction):
    _, ids, con = junction
    i = ids["100#1f"]
    # not the left (200); the U-turn onto 100#1r is there
    assert {r[0] for r in con.execute("SELECT to_edge FROM driving.edge_graph WHERE from_edge = ?", [i]).fetchall()} \
        == {ids["300#1f"], ids["400#1f"], ids["100#1r"]}
    perm = {r[0]: r[1:] for r in con.execute(
        "SELECT to_edge, allowed, vehicles, except_vehicles, condition FROM driving.turn_permission WHERE from_edge = ?", [i]).fetchall()}
    assert perm == {ids["200#1f"]: (True, "psv", None, None),                    # the left: open to buses and taxis
                    ids["300#1f"]: (False, None, None, "Mo-Fr 07:00-09:00"),     # the right: closed to all at rush hour
                    ids["400#1f"]: (False, "hgv", None, None)}                   # straight on: closed to lorries


def test_sumo_writes_the_permissions(junction, tmp_path):
    try:
        _find_netconvert()
    except Exception:
        pytest.skip("netconvert not installed (pip install duckosm[sumo])")
    db, ids, _ = junction
    f, left, right, straight = (ids[k] for k in ("100#1f", "200#1f", "300#1f", "400#1f"))
    to_sumo(str(db), str(tmp_path), net_name="p", run_netconvert=True)
    con_xml = (tmp_path / "p.con.xml").read_text()
    assert re.search(rf'from="{f}" to="{left}" fromLane="\d+" toLane="\d+" allow="bus coach taxi"', con_xml)   # the left, buses and taxis only
    assert re.search(rf'from="{f}" to="{straight}" fromLane="\d+" toLane="\d+" disallow="trailer truck"', con_xml)  # straight on, no lorries
    assert f'from="{f}" to="{right}"' not in con_xml        # the rush-hour ban written in force: SUMO has no time
    net = (tmp_path / "p.net.xml").read_text()
    assert set(re.findall(rf'<connection from="{f}" to="(\d+)"', net)) == {str(left), str(straight), str(ids["100#1r"])}   # and the U-turn


# A crossroads mapped as two nodes 6 m apart (2 south, 3 north) joined by a link way 200; two-way roads. Relation 21:
# no left turn from the south road (way 100) via the link (way 200) onto the west road (way 400), except buses.
@pytest.fixture(scope="module")
def split_junction(tmp_path_factory):
    nodes = {1: (7.0, 43.0), 2: (7.0, 43.001), 3: (7.0, 43.001054), 4: (7.0, 43.002), 5: (6.998, 43.001054), 6: (7.002, 43.001)}
    ways = {w: (refs, {"highway": "primary"}) for w, refs in {100: [1, 2], 200: [2, 3], 300: [3, 4], 400: [3, 5], 500: [2, 6]}.items()}
    rels = dict([_restriction(21, {"restriction": "no_left_turn", "except": "bus"}, ("way", 200), 100, 400)])
    db = build(tmp_path_factory.mktemp("split"), nodes, ways, rels)
    return db, _ids(db), duckdb.connect(str(db), read_only=True)


def test_a_via_way_restriction_is_a_path(split_junction):
    _, ids, con = split_junction
    assert con.execute("SELECT restriction_type, except_vehicles, from_edge, via_edges, to_edge FROM driving.turn_path_restrictions").fetchall() \
        == [("no_left_turn", "bus", ids["100#1f"], [ids["200#1f"]], ids["400#1f"])]
    a, b, c = ids["100#1f"], ids["200#1f"], ids["400#1f"]
    assert con.execute(f"SELECT count(*) FROM driving.edge_graph WHERE (from_edge, to_edge) IN (({a}, {b}), ({b}, {c}))").fetchone()[0] == 2   # untouched


def test_sumo_applies_a_via_way_restriction_in_a_joined_junction(split_junction, tmp_path):
    try:
        _find_netconvert()
    except Exception:
        pytest.skip("netconvert not installed (pip install duckosm[sumo])")
    db, ids, _ = split_junction
    f, left, straight = ids["100#1f"], ids["400#1f"], ids["300#1f"]
    out = to_sumo(str(db), str(tmp_path), net_name="v", config={"junctions.join": "true"}, run_netconvert=True)
    assert out["n_path_restrictions"] == 1
    con_xml = (tmp_path / "v.con.xml").read_text()
    assert re.search(rf'from="{f}" to="{left}" fromLane="\d+" toLane="\d+" allow="bus coach"', con_xml)   # the left: buses only
    assert re.search(rf'<connection from="{f}" to="{straight}"/>|from="{f}" to="{straight}" fromLane="\d+" toLane="\d+"/>', con_xml)  # straight: all
