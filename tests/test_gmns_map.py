"""Tests for duckosm.gmns_map — the lanestyle lane map of a GMNS DuckDB — and the link levels it reads."""
import duckdb
import pytest

from duckosm.gmns import to_gmns

A, B, AR = 6141068311830699705, 3843102655846694531, 1234567890123456789


def _gmns_db(tmp_path):
    src = tmp_path / "src.duckdb"
    con = duckdb.connect(str(src))
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"INSERT INTO driving.nodes VALUES (1,{p('POINT(18.06 59.32)')}),"
                f"(2,{p('POINT(18.07 59.32)')}),(3,{p('POINT(18.07 59.31)')})")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, "
                "highway VARCHAR, name VARCHAR, lanes INTEGER, is_reverse BOOLEAN, length_m FLOAT, "
                "maxspeed_kmh FLOAT, bridge VARCHAR, tunnel VARCHAR, layer VARCHAR, geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({A},1,2,100,'primary','Main',2,false,80,50,'yes',NULL,'1',{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({B},2,3,101,'residential',NULL,1,false,110,30,NULL,NULL,NULL,{p('LINESTRING(18.07 59.32,18.07 59.31)')}),
        ({AR},2,1,100,'primary','Main',2,true,80,50,'yes',NULL,'1',{p('LINESTRING(18.07 59.32,18.06 59.32)')})""")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, via_edge BIGINT, cost DOUBLE)")
    con.execute(f"INSERT INTO driving.edge_graph VALUES ({A},{B},{B},1.0)")
    con.close()
    out = tmp_path / "gmns.duckdb"
    to_gmns(str(src), str(out))
    return out


def test_link_carries_levels(tmp_path):
    con = duckdb.connect(str(_gmns_db(tmp_path)), read_only=True)
    got = dict(con.execute("SELECT link_id, bridge || '/' || layer FROM gmns_driving.link").fetchall())
    assert got[A] == "yes/1" and got[B] is None


def test_write_map(tmp_path):
    """The lane page of the GMNS file on the roads of the source db's level area (lanestyle.lane_page); no area: an error saying how to make one."""
    ls = pytest.importorskip("lanestyle")
    if not hasattr(ls, "lane_page"):
        pytest.skip("lanestyle before 0.3 has no lane_page")
    pytest.importorskip("scipy")
    from duckosm.gmns_map import write_map
    from duckosm.levels import make_and_solve
    gmns = _gmns_db(tmp_path)
    src = tmp_path / "src.duckdb"
    with pytest.raises(FileNotFoundError, match="duckosm levels"):
        write_map(gmns, tmp_path / "lanes.html", src)
    make_and_solve(src)
    html = write_map(gmns, tmp_path / "lanes.html", src).read_text()
    assert "maplibre" in html.lower() and "roads-simple" in html
