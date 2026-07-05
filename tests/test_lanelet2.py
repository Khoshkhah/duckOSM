"""Tests for duckosm.lanelet2 — Lanelet2 (.osm) HD-map export from a GMNS db.

Lanelet2 is OSM XML (no XSD); structural assertions on a well-formed map (Autoware/lanelet2 load is
the documented manual check)."""
import xml.etree.ElementTree as ET

import duckdb
import pytest

from duckosm.gmns import to_gmns
from duckosm.lanelet2 import to_lanelet2

A, B, AR = 6141068311830699705, 3843102655846694531, 1234567890123456789


def _gmns(tmp_path):
    """A GMNS db from a 2-lane primary edge A (→ 2 lanelets) continuing to B."""
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
    return out


def _osm(path):
    root = ET.parse(str(path)).getroot()
    assert root.tag == "osm" and root.get("version") == "0.6"
    return root


def test_lanelets_structure(tmp_path):
    res = to_lanelet2(str(_gmns(tmp_path)), tmp_path / "m.osm")
    assert res["lanelets"] >= 3 and res["ways"] == 2 * res["lanelets"]   # A(2)+B(1)+AR(2) lanes
    root = _osm(tmp_path / "m.osm")
    ways = {w.get("id") for w in root.findall("way")}
    nodes = {n.get("id") for n in root.findall("node")}
    lls = [r for r in root.findall("relation")
           if any(t.get("k") == "type" and t.get("v") == "lanelet" for t in r.findall("tag"))]
    assert len(lls) == res["lanelets"]
    for ll in lls:
        roles = [m.get("role") for m in ll.findall("member")]
        assert roles.count("left") == 1 and roles.count("right") == 1    # exactly one of each
        assert all(m.get("ref") in ways for m in ll.findall("member"))   # members resolve
    # every boundary way references existing nodes and is a line_thin
    for w in root.findall("way"):
        assert all(nd.get("ref") in nodes for nd in w.findall("nd"))
        assert any(t.get("k") == "type" and t.get("v") == "line_thin" for t in w.findall("tag"))


def test_lanelet_tags(tmp_path):
    to_lanelet2(str(_gmns(tmp_path)), tmp_path / "m.osm")
    root = _osm(tmp_path / "m.osm")
    ll = next(r for r in root.findall("relation")
              if any(t.get("k") == "type" and t.get("v") == "lanelet" for t in r.findall("tag")))
    tags = {t.get("k"): t.get("v") for t in ll.findall("tag")}
    assert tags["subtype"] == "road" and tags["one_way"] == "yes" and tags["location"] == "urban"
    assert tags["speed_limit"] == "50" and tags["duckosm:edge_id"] in {str(A), str(B), str(AR)}


def test_node_snapping_dedups(tmp_path):
    # the two lanes of edge A share a boundary → snapping must reuse nodes (fewer nodes than raw points)
    res = to_lanelet2(str(_gmns(tmp_path)), tmp_path / "m.osm")
    total_pts = sum(len(w.findall("nd")) for w in _osm(tmp_path / "m.osm").findall("way"))
    assert res["nodes"] < total_pts                                       # dedup happened


def test_bad_db_raises(tmp_path):
    bare = tmp_path / "bare.duckdb"
    duckdb.connect(str(bare)).close()
    with pytest.raises(ValueError, match="lane"):
        to_lanelet2(str(bare), tmp_path / "m.osm")
