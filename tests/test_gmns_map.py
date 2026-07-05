"""Tests for duckosm.gmns_map — pretty road / lane HTML maps of a GMNS DuckDB."""
import duckdb
import pytest

from duckosm.gmns import to_gmns, to_meso
from duckosm.gmns_map import build_road_payload, build_lane_payload, write_map

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
                "maxspeed_kmh FLOAT, geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({A},1,2,100,'primary','Main',2,false,80,50,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({B},2,3,101,'residential',NULL,1,false,110,30,{p('LINESTRING(18.07 59.32,18.07 59.31)')}),
        ({AR},2,1,100,'primary','Main',2,true,80,50,{p('LINESTRING(18.07 59.32,18.06 59.32)')})""")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, via_edge BIGINT, cost DOUBLE)")
    con.execute(f"INSERT INTO driving.edge_graph VALUES ({A},{B},{B},1.0)")
    con.close()
    out = tmp_path / "gmns.duckdb"
    to_gmns(str(src), str(out))
    to_meso(str(out), modes=["driving"])
    return out


def test_road_payload(tmp_path):
    con = duckdb.connect(str(_gmns_db(tmp_path)), read_only=True)
    con.execute("LOAD spatial;")
    p = build_road_payload(con, mode="driving")
    assert p["n"]["roads"] == 3 and p["gpm"] > 0                # 3 directed carriageways
    assert len(p["conn"]) >= 1                                  # A->B turn connector present
    assert all(len(r) > 2 for r in p["roads"])                 # [classIdx, width, ...coords]


def test_lane_payload(tmp_path):
    con = duckdb.connect(str(_gmns_db(tmp_path)), read_only=True)
    con.execute("LOAD spatial;")
    p = build_lane_payload(con, mode="driving")
    assert p["n"]["lanes"] == 5                                 # A(2)+B(1)+AR(2) lane rows
    assert all(len(L) > 2 for L in p["lanes"])                 # [useIdx, width_m, ...coords]


def test_write_map_both_styles(tmp_path):
    db = _gmns_db(tmp_path)
    for style in ("road", "lane"):
        out = tmp_path / f"{style}.html"
        write_map(str(db), str(out), style=style)
        h = out.read_text()
        assert h.startswith("<!doctype html>") and "const D=" in h and "/*__P__*/" not in h


def test_bad_style(tmp_path):
    with pytest.raises(ValueError, match="style"):
        write_map(str(_gmns_db(tmp_path)), str(tmp_path / "x.html"), style="wobble")
