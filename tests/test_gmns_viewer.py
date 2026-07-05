"""Tests for duckosm.gmns_viewer — the interactive HTML viewer for a GMNS DuckDB.

Builds a tiny GMNS db (+ meso), then checks the viewer payload (lane/section/connector features,
meso presence) and the HTML rendering (standalone vs body-only). No browser needed.
"""
import duckdb

from duckosm.gmns import to_gmns, to_meso
from duckosm.gmns_viewer import build_viewer_payload, render_viewer_html, write_viewer

A, B, AR = 6141068311830699705, 3843102655846694531, 1234567890123456789


def _source(path):
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA raw; CREATE SCHEMA driving")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR,VARCHAR))")
    con.execute("INSERT INTO raw.nodes VALUES (2,59.32,18.07,MAP{'highway':'traffic_signals'})")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR,VARCHAR), refs BIGINT[])")
    con.execute("INSERT INTO raw.ways VALUES "
                "(100, MAP{'turn:lanes':'through|right','bicycle:lanes':'no|designated'}, [1,2]),"
                "(101, MAP{}, [2,3])")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"INSERT INTO driving.nodes VALUES (1,{p('POINT(18.06 59.32)')}),"
                f"(2,{p('POINT(18.07 59.32)')}),(3,{p('POINT(18.07 59.31)')})")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, "
                "highway VARCHAR, name VARCHAR, lanes INTEGER, is_reverse BOOLEAN, length_m FLOAT, "
                "maxspeed_kmh FLOAT, geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({A},1,2,100,'primary','Main',2,false,80,50,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({B},2,3,101,'residential',NULL,1,false,110,30,{p('LINESTRING(18.07 59.32,18.07 59.31)')}),
        ({AR},2,1,100,'primary','Main',2,true,80,50,{p('LINESTRING(18.07 59.32,18.06 59.32)')})""")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, via_edge BIGINT, cost DOUBLE)")
    con.execute(f"INSERT INTO driving.edge_graph VALUES ({A},{B},{B},1.0),({A},{AR},{AR},1.0)")
    con.close()


def _gmns_db(tmp_path, meso=True):
    src = tmp_path / "src.duckdb"
    _source(src)
    out = tmp_path / "gmns.duckdb"
    to_gmns(str(src), str(out))
    if meso:
        to_meso(str(out), modes=["driving"])
    return out


def test_payload_has_lanes_and_meso(tmp_path):
    con = duckdb.connect(str(_gmns_db(tmp_path)), read_only=True)
    con.execute("LOAD spatial;")
    p = build_viewer_payload(con, mode="driving")
    assert p["has_meso"] is True
    assert len(p["lanes"]) == 5                                # lane rows: A(2) + B(1) + AR(2)
    assert p["stats"]["lanes"] == con.execute("SELECT count(*) FROM gmns_driving.lane").fetchone()[0]
    assert len(p["sections"]) == 3                             # one section per macro link
    assert len(p["conns"]) == con.execute("SELECT count(*) FROM gmns_driving.movement").fetchone()[0]
    # feature shape: connectors carry a reversible X<from>-<to> id and a movement code
    conn = p["conns"][0]
    assert conn["t"] == "conn" and conn["id"].startswith("X") and "g" in conn
    lane = p["lanes"][0]
    assert lane["t"] == "lane" and "id" in lane and "u" in lane


def test_payload_includes_micro(tmp_path):
    from duckosm.gmns import to_micro
    db = _gmns_db(tmp_path)                                     # gmns + meso
    to_micro(str(db), modes=["driving"])
    con = duckdb.connect(str(db), read_only=True)
    con.execute("LOAD spatial;")
    p = build_viewer_payload(con, mode="driving")
    assert p["has_micro"] is True
    assert len(p["micro_cells"]) > 0 and len(p["micro_conns"]) > 0   # cells + turn connectors present


def test_payload_without_meso(tmp_path):
    con = duckdb.connect(str(_gmns_db(tmp_path, meso=False)), read_only=True)
    con.execute("LOAD spatial;")
    p = build_viewer_payload(con, mode="driving")
    assert p["has_meso"] is False and p["sections"] == [] and p["conns"] == []
    assert len(p["lanes"]) > 0                                 # lanes still render


def test_render_standalone_vs_body(tmp_path):
    con = duckdb.connect(str(_gmns_db(tmp_path)), read_only=True)
    con.execute("LOAD spatial;")
    p = build_viewer_payload(con, mode="driving")
    doc = render_viewer_html(p, standalone=True, file_label="x.duckdb")
    assert doc.lstrip().startswith("<!doctype html>") and "const D=" in doc and "/*__P__*/" not in doc
    body = render_viewer_html(p, standalone=False, file_label="x.duckdb")
    assert "<!doctype" not in body and body.startswith("<title>")


def test_write_viewer_writes_file(tmp_path):
    out = tmp_path / "viewer.html"
    path = write_viewer(str(_gmns_db(tmp_path)), str(out), mode="driving")
    assert out.exists() and out.read_text().startswith("<!doctype html>")
    assert path == str(out)
