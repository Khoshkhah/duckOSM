"""`duckosm route-map`: the graph embedded in the page matches the db (the in-page Dijkstra was
checked against route() by hand in a browser: docs/design/route_map.md)."""
import json

import duckdb
import pytest

pytest.importorskip("roadstyle")
pytest.importorskip("geopandas")

from duckosm.route_map import write_route_map

BIG = 2**62 + 1          # virtual node ids pass 2**53: the page must only see small indices


def _db():
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, name VARCHAR, "
                "highway VARCHAR, bridge BOOLEAN, tunnel BOOLEAN, layer INTEGER, length_m DOUBLE, "
                "cost_s DOUBLE, geometry GEOMETRY)")
    g = lambda w: f"ST_GeomFromText('{w}')"
    con.execute(f"""INSERT INTO driving.edges VALUES
        (30, 1, 2, 'A', 'residential', false, false, 0, 100, 10, {g('LINESTRING(7.40 43.73, 7.41 43.73)')}),
        (10, 2, {BIG}, 'B', 'residential', false, false, 0, 200, 20, {g('LINESTRING(7.41 43.73, 7.42 43.73)')}),
        (20, {BIG}, 4, NULL, 'service', false, false, 0, 50, 5, {g('LINESTRING(7.42 43.73, 7.43 43.73)')})""")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT)")
    con.execute("INSERT INTO driving.edge_graph VALUES (30, 10), (10, 20)")
    return con


def test_route_map_embeds_the_turn_graph(tmp_path):
    out = write_route_map(_db(), tmp_path / "rm.html", name="t")
    html = out.read_text()
    rm = json.loads(html.split("const RM = ", 1)[1].split(";</script>", 1)[0])

    assert rm["modes"] == ["driving"] and rm["n"] == 3 and rm["mm"] is None
    # features ordered by edge_id: 10, 20, 30 -> k 0, 1, 2
    assert rm["name"] == ["B", "", "A"]
    assert max(rm["src"] + rm["tgt"]) < 4                  # 4 nodes, remapped to 0..3
    assert rm["tgt"][0] == rm["src"][1]                    # B ends where the service road starts (BIG)
    d = rm["graphs"]["driving"]
    nxt = dict(zip(d["k"], d["next"]))
    assert nxt == {0: [1], 1: [], 2: [0]}                  # A -> B -> service road, as edge_graph
    assert dict(zip(d["k"], d["cost"])) == {0: 20, 1: 5, 2: 10}


def test_route_map_unknown_mode_raises(tmp_path):
    with pytest.raises(ValueError):
        write_route_map(_db(), tmp_path / "rm.html", modes=["walking"])
