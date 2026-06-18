"""Tests for duckosm.routing.to_networkx — load the edge_graph as a weighted DiGraph."""
import duckdb
import pytest

from duckosm.routing import to_networkx, route


def _db():
    con = duckdb.connect()
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, length_m DOUBLE, cost_s DOUBLE)")
    con.execute("INSERT INTO driving.edges VALUES (1,100,10),(2,200,20),(3,50,5)")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, cost DOUBLE)")
    con.execute("INSERT INTO driving.edge_graph VALUES (1,2,10),(2,3,20)")   # cost = from_edge cost_s
    return con


def test_to_networkx_time_weight():
    G = to_networkx(_db(), weight="time")
    assert G.number_of_nodes() == 3 and G.number_of_edges() == 2   # nodes = edge_ids
    assert G[1][2]["weight"] == 10 and G[2][3]["weight"] == 20      # travel time of from_edge


def test_to_networkx_length_weight():
    G = to_networkx(_db(), weight="length")
    assert G[1][2]["weight"] == 100 and G[2][3]["weight"] == 200    # length of from_edge


def test_to_networkx_bad_weight():
    with pytest.raises(ValueError):
        to_networkx(_db(), weight="nope")


def _route_db():
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, name VARCHAR, highway VARCHAR, "
                "length_m DOUBLE, cost_s DOUBLE, geometry GEOMETRY)")
    g = lambda w: f"ST_GeomFromText('{w}')"
    con.execute(f"""INSERT INTO driving.edges VALUES
        (1,'A','residential',100,10,{g('LINESTRING(0 0,1 0)')}),
        (2,'B','residential',200,20,{g('LINESTRING(1 0,2 0)')}),
        (3,'C','residential', 50, 5,{g('LINESTRING(2 0,3 0)')})""")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, cost DOUBLE)")
    con.execute("INSERT INTO driving.edge_graph VALUES (1,2,10),(2,3,20)")
    return con


def test_route_returns_ordered_path_with_totals():
    r = route(_route_db(), 1, 3)                       # defaults: time, no service
    assert r["edges"] == [1, 2, 3]
    assert r["time_s"] == 35 and r["length_m"] == 350  # door-to-door (every edge counted)
    assert [p["name"] for p in r["path"]] == ["A", "B", "C"]
    assert r["path"][0]["geometry"].startswith("LINESTRING")


def test_route_no_path_returns_none():
    con = _route_db()
    con.execute("INSERT INTO driving.edge_graph VALUES (8,9,1),(9,8,1)")   # disconnected component
    assert route(con, 1, 9) is None


def test_route_unknown_edge_raises():
    with pytest.raises(ValueError):
        route(_route_db(), 1, 999999)
