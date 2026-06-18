"""Tests for duckosm.routing.to_networkx — load the edge_graph as a weighted DiGraph."""
import duckdb
import pytest

from duckosm.routing import to_networkx


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
