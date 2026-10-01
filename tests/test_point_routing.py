"""Routing between two points (docs/design/point_routing.md): the access leg within the radius,
the parts of the first and last edges, the same-edge trip, going around, walk + drive."""
import duckdb
import pytest

from duckosm import route_multimodal_points, route_points

K = 111320.0                     # metres per degree at the equator (the tests sit at lat 0)
V = 4.5 / 3.6                    # the default walking speed, m/s


def _line(*xy):
    return "LINESTRING(" + ", ".join(f"{x} {y}" for x, y in xy) + ")"


def _db():
    """driving: edge 1 (0,0)->(0.001,0) then edge 2 (0.001,0)->(0.002,0), 100 m / 10 s each; edge 5
    far away brings you from the end of 2 back to the start of 1 (going around, 300 m / 30 s)."""
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, name VARCHAR, "
                "highway VARCHAR, length_m DOUBLE, cost_s DOUBLE, geometry GEOMETRY)")
    for e, s, t, ln, c, g in [(1, 10, 11, 100, 10, _line((0, 0), (0.001, 0))),
                              (2, 11, 12, 100, 10, _line((0.001, 0), (0.002, 0))),
                              (5, 12, 10, 300, 30, _line((0.002, 0), (0.002, 0.01), (0, 0.01), (0, 0)))]:
        con.execute(f"INSERT INTO driving.edges VALUES ({e}, {s}, {t}, 'r{e}', 'residential', {ln}, {c}, "
                    f"ST_GeomFromText('{g}'))")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, cost DOUBLE)")
    con.execute("INSERT INTO driving.edge_graph VALUES (1, 2, 10), (2, 5, 10), (5, 1, 30)")
    return con


def test_half_of_each_edge_plus_the_walks_to_the_road():
    off = 0.0001                                                  # 11.1 m beside the road
    r = route_points(_db(), (0.0005, off), (0.0015, off))
    assert r["edges"] == [1, 2]
    walk = off * K
    assert r["start"]["fraction"] == pytest.approx(0.5) and r["start"]["access_m"] == pytest.approx(walk)
    assert r["time_s"] == pytest.approx(2 * walk / V + 5 + 5)     # half of 10 s, twice
    assert r["length_m"] == pytest.approx(2 * walk + 50 + 50)
    assert [(p["from_fraction"], p["to_fraction"]) for p in r["path"]] == [(0.5, 1.0), (0.0, 0.5)]


def test_start_and_end_on_one_edge_cost_the_part_between():
    r = route_points(_db(), (0.0002, 0), (0.0008, 0))
    assert r["edges"] == [1] and r["time_s"] == pytest.approx(6) and r["length_m"] == pytest.approx(60)


def test_behind_on_a_one_way_edge_goes_around():
    r = route_points(_db(), (0.0008, 0), (0.0002, 0))
    assert r["edges"] == [1, 2, 5, 1]
    assert r["time_s"] == pytest.approx(2 + 10 + 30 + 2)          # 20 % of 1, 2, 5, 20 % of 1


def test_no_road_within_the_radius():
    with pytest.raises(ValueError, match="no road within 50 m of the start"):
        route_points(_db(), (0.0005, 0.001), (0.0015, 0))         # 111 m off
    assert route_points(_db(), (0.0005, 0.001), (0.0015, 0), radius_m=150) is not None


def test_length_weight_and_its_totals():
    r = route_points(_db(), (0.0005, 0), (0.0015, 0), weight="length")
    assert r["edges"] == [1, 2] and r["length_m"] == pytest.approx(100) and r["time_s"] == pytest.approx(10)


def _mm():
    """Walking 1 (nodes 1-2) and 3 (nodes 3-4) both ways, slow; driving 2 (2->3), fast; changing
    walk <-> drive at nodes 2 and 3 costs 60 s."""
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA walking; CREATE SCHEMA mm")
    cols = "edge_id BIGINT, source BIGINT, target BIGINT, name VARCHAR, highway VARCHAR, length_m DOUBLE, cost_s DOUBLE, geometry GEOMETRY"
    con.execute(f"CREATE TABLE walking.edges({cols})")
    con.execute(f"CREATE TABLE mm.edges(mode VARCHAR, {cols})")
    rows = [("walking", 11, 1, 2, 100, 80, _line((0, 0), (0.001, 0))),
            ("walking", 12, 2, 1, 100, 80, _line((0.001, 0), (0, 0))),
            ("walking", 31, 3, 4, 100, 80, _line((0.1, 0), (0.101, 0))),
            ("walking", 32, 4, 3, 100, 80, _line((0.101, 0), (0.1, 0))),
            ("driving", 21, 2, 3, 11000, 400, _line((0.001, 0), (0.1, 0)))]
    for m, e, s, t, ln, c, g in rows:
        vals = f"{e}, {s}, {t}, 'w{e}', 'road', {ln}, {c}, ST_GeomFromText('{g}')"
        con.execute(f"INSERT INTO mm.edges VALUES ('{m}', {vals})")
        if m == "walking":
            con.execute(f"INSERT INTO walking.edges VALUES ({vals})")
    con.execute("CREATE TABLE mm.transfers(node_id BIGINT, from_mode VARCHAR, to_mode VARCHAR, cost_s DOUBLE, kind VARCHAR)")
    con.execute("INSERT INTO mm.transfers VALUES (2, 'walking', 'driving', 60, 'park'), (3, 'driving', 'walking', 60, 'park'),"
                " (3, 'walking', 'driving', 60, 'park'), (2, 'driving', 'walking', 60, 'park')")
    return con


def test_walk_drive_walk_from_the_middle_of_walking_edges():
    r = route_multimodal_points(_mm(), (0.0005, 0), (0.1005, 0))
    assert [lg["mode"] for lg in r["legs"]] == ["walking", "driving", "walking"]
    assert r["time_s"] == pytest.approx(40 + 60 + 400 + 60 + 40)  # half of each walk, 2 changes
    assert r["length_m"] == pytest.approx(50 + 11000 + 50)
    assert r["start"]["fraction"] == pytest.approx(0.5) and "__start__" not in r["nodes"]


def test_walk_only_on_one_edge():
    r = route_multimodal_points(_mm(), (0.0002, 0), (0.0007, 0))
    assert [lg["mode"] for lg in r["legs"]] == ["walking"] and r["time_s"] == pytest.approx(40)
