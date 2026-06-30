"""Tests for duckosm.routing.to_networkx — load the edge_graph as a weighted DiGraph."""
import duckdb
import pytest

from duckosm.routing import to_networkx, to_networkx_nodes, write_graph, route, Router


def _db():
    con = duckdb.connect()
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, length_m DOUBLE, cost_s DOUBLE)")
    con.execute("INSERT INTO driving.edges VALUES (1,100,10),(2,200,20),(3,50,5)")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, cost DOUBLE)")
    con.execute("INSERT INTO driving.edge_graph VALUES (1,2,10),(2,3,20)")   # cost = from_edge cost_s
    return con


def test_to_networkx_time_weight():
    G = to_networkx(_db(), weight="time", node_attrs=False)         # minimal db has no attr columns
    assert G.number_of_nodes() == 3 and G.number_of_edges() == 2   # nodes = edge_ids
    assert G[1][2]["weight"] == 10 and G[2][3]["weight"] == 20      # travel time of from_edge


def test_to_networkx_length_weight():
    G = to_networkx(_db(), weight="length", node_attrs=False)
    assert G[1][2]["weight"] == 100 and G[2][3]["weight"] == 200    # length of from_edge


def test_to_networkx_bad_weight():
    with pytest.raises(ValueError):
        to_networkx(_db(), weight="nope")


def test_to_networkx_attaches_node_attrs():
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, name VARCHAR, highway VARCHAR, "
                "length_m DOUBLE, maxspeed_kmh DOUBLE, cost_s DOUBLE, geometry GEOMETRY)")
    con.execute("INSERT INTO driving.edges VALUES "
                "(1,'A St','residential',100,30,12,ST_GeomFromText('LINESTRING(0 0,1 0)')),"
                "(2,'B St','primary',200,50,14,ST_GeomFromText('LINESTRING(1 0,2 0)'))")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, cost DOUBLE)")
    con.execute("INSERT INTO driving.edge_graph VALUES (1,2,12)")

    G = to_networkx(con)                                # node_attrs=True by default
    assert G.nodes[1]["name"] == "A St" and G.nodes[1]["highway"] == "residential"
    assert G.nodes[1]["length_m"] == 100 and G.nodes[1]["maxspeed_kmh"] == 30
    assert G.nodes[1]["geometry"].startswith("LINESTRING")

    bare = to_networkx(con, node_attrs=False)           # opt out -> weight only, no node metadata
    assert "name" not in bare.nodes[1]


def _node_db():
    """Minimal geographic db: 3 junctions, 2 directed road segments with full edge attrs."""
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY, h3_cell BIGINT)")
    con.execute("INSERT INTO driving.nodes VALUES "
                "(10, ST_GeomFromText('POINT(0 0)'), 100),"
                "(11, ST_GeomFromText('POINT(1 0)'), 101),"
                "(12, ST_GeomFromText('POINT(2 0)'), 102)")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, "
                "osm_id BIGINT, highway VARCHAR, name VARCHAR, oneway BOOLEAN, length_m DOUBLE, "
                "maxspeed_kmh DOUBLE, cost_s DOUBLE, is_reverse BOOLEAN, geometry GEOMETRY)")
    con.execute("INSERT INTO driving.edges VALUES "
                "(1,10,11,500,'residential','A St',FALSE,100,30,12,FALSE,"
                "ST_GeomFromText('LINESTRING(0 0,1 0)')),"
                "(2,11,12,500,'residential','A St',FALSE,200,30,24,FALSE,"
                "ST_GeomFromText('LINESTRING(1 0,2 0)'))")
    return con


def test_to_networkx_nodes_is_geographic_multidigraph():
    G = to_networkx_nodes(_node_db())
    import networkx as nx
    assert isinstance(G, nx.MultiDiGraph)
    assert G.number_of_nodes() == 3 and G.number_of_edges() == 2   # nodes = junctions
    assert G.graph["crs"] == "EPSG:4326" and G.graph["mode"] == "driving"
    # node carries lon/lat (x/y) + extra node columns
    assert G.nodes[11]["x"] == 1 and G.nodes[11]["y"] == 0 and G.nodes[11]["h3_cell"] == 101
    # edge is keyed by edge_id and carries the full attribute set
    assert G[10][11][1]["highway"] == "residential" and G[10][11][1]["name"] == "A St"
    assert G[10][11][1]["length_m"] == 100 and G[10][11][1]["edge_id"] == 1
    assert G[10][11][1]["geometry"].startswith("LINESTRING")     # WKT by default
    assert "source" not in G[10][11][1]                          # endpoints become u/v, not attrs


def test_to_networkx_nodes_geometry_none_omits_geometry():
    G = to_networkx_nodes(_node_db(), geometry="none")
    assert "geometry" not in G[10][11][1]


def test_to_networkx_nodes_bad_geometry_raises():
    with pytest.raises(ValueError):
        to_networkx_nodes(_node_db(), geometry="nope")


def test_write_graph_graphml_roundtrips(tmp_path):
    import networkx as nx
    out = tmp_path / "g.graphml"
    res = write_graph(_node_db(), out)                  # format inferred from .graphml
    assert res["fmt"] == "graphml" and res["graph"] == "node" and res["n_nodes"] == 3
    # force_multigraph: read_graphml only auto-detects a multigraph when parallel edges exist
    G = nx.read_graphml(out, force_multigraph=True)     # portable read-back
    assert isinstance(G, nx.MultiDiGraph) and G.number_of_edges() == 2
    u, v, d = next(iter(G.edges(data=True)))
    assert d["highway"] == "residential" and d["geometry"].startswith("LINESTRING")


def test_write_graph_gpickle_roundtrips_lossless(tmp_path):
    import pickle
    out = tmp_path / "g.gpickle"
    res = write_graph(_node_db(), out)                  # .gpickle -> pickle
    assert res["fmt"] == "gpickle"
    with open(out, "rb") as f:
        G = pickle.load(f)
    assert G.number_of_nodes() == 3 and G.number_of_edges() == 2


def test_write_graph_edge_kind(tmp_path):
    # edge-based to_networkx attaches node attrs (name/highway/length_m/maxspeed_kmh/cost_s/geom).
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, name VARCHAR, highway VARCHAR, "
                "length_m DOUBLE, maxspeed_kmh DOUBLE, cost_s DOUBLE, geometry GEOMETRY)")
    con.execute("INSERT INTO driving.edges VALUES "
                "(1,'A','residential',100,30,12,ST_GeomFromText('LINESTRING(0 0,1 0)')),"
                "(2,'B','residential',200,30,24,ST_GeomFromText('LINESTRING(1 0,2 0)'))")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, cost DOUBLE)")
    con.execute("INSERT INTO driving.edge_graph VALUES (1,2,12)")
    out = tmp_path / "edge.graphml"
    res = write_graph(con, out, graph="edge")           # edge-based routing graph
    assert res["graph"] == "edge" and res["n_nodes"] == 2   # nodes = edge_ids


def test_write_graph_graphml_stringifies_lists_and_drops_none(tmp_path):
    # `refs` is a list and `name` NULL — neither is GraphML-serialisable as-is.
    import networkx as nx
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute("INSERT INTO driving.nodes VALUES (10, ST_GeomFromText('POINT(0 0)')),"
                "(11, ST_GeomFromText('POINT(1 0)'))")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, "
                "name VARCHAR, refs BIGINT[], geometry GEOMETRY)")
    con.execute("INSERT INTO driving.edges VALUES "
                "(1,10,11,NULL,[10,11],ST_GeomFromText('LINESTRING(0 0,1 0)'))")
    out = tmp_path / "g.graphml"
    write_graph(con, out)                               # must not raise on list/None attrs
    G = nx.read_graphml(out)
    d = next(iter(G.edges(data=True)))[2]
    assert isinstance(d["refs"], str) and "name" not in d   # list stringified, NULL dropped


def test_write_graph_bad_format_raises(tmp_path):
    with pytest.raises(ValueError):
        write_graph(_node_db(), tmp_path / "g.xyz", fmt="nope")


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
    r = route(_route_db(), 1, 3)                       # defaults: fastest by time
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


def test_router_builds_once_and_reuses_graph():
    r = Router(_route_db())
    g = r.graph
    assert r.route(1, 3)["edges"] == [1, 2, 3]
    assert r.route(2, 3)["edges"] == [2, 3]
    assert r.graph is g                              # graph built once, not rebuilt per query
