"""
Tests for ServiceSplitter — moves highway='service' edges out of the routing graph into
`service_edges`, prunes the derived tables, and keeps the full graph as
`edge_graph_with_service`. Run: pytest tests/ -q
"""
import duckdb

from duckosm.processors.service_splitter import ServiceSplitter


def _graph():
    """edges 1,3 = real roads, edge 2 = service. Node 50 is only on the service edge (orphaned
    once it's removed). edge_graph transitions (1,2) and (3,2) touch the service edge."""
    con = duckdb.connect()
    con.execute("CREATE TABLE edges(edge_id BIGINT, source BIGINT, target BIGINT, highway VARCHAR)")
    con.execute("INSERT INTO edges VALUES "
                "(1,10,20,'residential'),(2,20,50,'service'),(3,20,30,'residential')")
    con.execute("CREATE TABLE nodes(node_id BIGINT)")
    con.execute("INSERT INTO nodes VALUES (10),(20),(30),(50)")
    con.execute("CREATE TABLE edge_graph(from_edge BIGINT, to_edge BIGINT)")
    con.execute("INSERT INTO edge_graph VALUES (1,2),(3,2),(1,3)")
    return con


def test_service_splitter_moves_and_prunes():
    con = _graph()
    ServiceSplitter(con).run()
    # the service edge is moved out of the routing graph
    assert [r[0] for r in con.execute("SELECT edge_id FROM service_edges").fetchall()] == [2]
    assert {r[0] for r in con.execute("SELECT edge_id FROM edges").fetchall()} == {1, 3}
    # edge_graph keeps only transitions among surviving edges; the full graph is backed up
    assert {tuple(r) for r in con.execute("SELECT from_edge, to_edge FROM edge_graph").fetchall()} == {(1, 3)}
    assert con.execute("SELECT count(*) FROM edge_graph_with_service").fetchone()[0] == 3
    # the node only the service edge used is pruned; the rest stay
    assert {r[0] for r in con.execute("SELECT node_id FROM nodes").fetchall()} == {10, 20, 30}


def test_service_splitter_idempotent():
    con = _graph()
    ServiceSplitter(con).run()
    ServiceSplitter(con).run()                       # second run: nothing left to move
    assert [r[0] for r in con.execute("SELECT edge_id FROM service_edges").fetchall()] == [2]
    assert {r[0] for r in con.execute("SELECT edge_id FROM edges").fetchall()} == {1, 3}


def test_service_splitter_no_service_creates_empty_table():
    con = duckdb.connect()
    con.execute("CREATE TABLE edges(edge_id BIGINT, source BIGINT, target BIGINT, highway VARCHAR)")
    con.execute("INSERT INTO edges VALUES (1,10,20,'residential'),(2,20,30,'primary')")
    ServiceSplitter(con).run()
    assert con.execute("SELECT count(*) FROM service_edges").fetchone()[0] == 0     # empty, but exists
    assert con.execute("SELECT count(*) FROM edges").fetchone()[0] == 2             # unchanged
    # no service -> no full-graph backup table created
    assert con.execute("SELECT count(*) FROM information_schema.tables "
                       "WHERE table_name='edge_graph_with_service'").fetchone()[0] == 0
