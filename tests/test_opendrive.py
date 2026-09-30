"""Tests for duckosm.opendrive — ASAM OpenDRIVE .xodr export (Phase 1: roads + lanes + geometry).

The ASAM OpenDRIVE XSD is behind GitLab auth (not freely fetchable), so — as the design doc allows —
these are structural assertions on a well-formed .xodr (esmini load is the documented manual check)."""
import math
import xml.etree.ElementTree as ET

import duckdb
import pytest

from duckosm.gmns import to_gmns
from duckosm.opendrive import to_opendrive

A, B = 6141068311830699705, 3843102655846694531
C, AR = 5555555555555555555, 1234567890123456789


def _src(path):
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, name VARCHAR, "
                "lanes INTEGER, length_m FLOAT, geometry GEOMETRY)")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({A},1,2,'Main St',2,120,{p('LINESTRING(18.06 59.32, 18.065 59.32, 18.07 59.321)')}),
        ({B},2,3,NULL,1,80,{p('LINESTRING(18.07 59.321, 18.07 59.31)')})""")   # 3-vertex + 2-vertex
    con.close()
    return path


def _roads(path):
    root = ET.parse(str(path)).getroot()
    assert root.tag == "OpenDRIVE"
    return root, {r.get("id"): r for r in root.findall("road")}


def test_structure_and_counts(tmp_path):
    res = to_opendrive(str(_src(tmp_path / "s.duckdb")), tmp_path / "n.xodr")
    assert res["roads"] == 2 and res["crs"].startswith("EPSG:326")    # default: UTM zone of the data
    assert "+proj=utm" in (tmp_path / "n.xodr").read_text()             # geoReference says so
    root, roads = _roads(tmp_path / "n.xodr")
    assert set(roads) == {str(A), str(B)}
    a = roads[str(A)]
    assert a.get("junction") == "-1"
    assert len(a.findall("./planView/geometry")) == 2          # 3 vertices → 2 line segments
    assert all(g.find("line") is not None for g in a.findall("./planView/geometry"))
    right = a.findall("./lanes/laneSection/right/lane")
    assert [l.get("id") for l in right] == ["-1", "-2"]        # 2 driving lanes, decreasing negative
    assert all(l.get("type") == "driving" and l.find("width").get("a") == "3.25" for l in right)
    assert a.find("./lanes/laneSection/center/lane").get("id") == "0"
    assert roads[str(B)].findall("./lanes/laneSection/right/lane")[0].get("id") == "-1"   # 1 lane


def test_road_length_equals_planview(tmp_path):
    to_opendrive(str(_src(tmp_path / "s.duckdb")), tmp_path / "n.xodr")
    _, roads = _roads(tmp_path / "n.xodr")
    for r in roads.values():
        seglen = sum(float(g.get("length")) for g in r.findall("./planView/geometry"))
        assert float(r.get("length")) == pytest.approx(seglen, abs=1e-3)


def test_no_elevation_profile_without_z(tmp_path):
    # the standard fixture has no z_from/z_to → flat roads, no <elevationProfile>
    to_opendrive(str(_src(tmp_path / "s.duckdb")), tmp_path / "n.xodr")
    _, roads = _roads(tmp_path / "n.xodr")
    assert all(r.find("elevationProfile") is None for r in roads.values())


def test_elevation_profile_from_z(tmp_path):
    # z_from/z_to (from `duckosm elevation`) → a linear <elevationProfile> between planView and lanes
    path = tmp_path / "s.duckdb"
    con = duckdb.connect(str(path)); con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, name VARCHAR, "
                "lanes INTEGER, length_m FLOAT, z_from DOUBLE, z_to DOUBLE, geometry GEOMETRY)")
    con.execute(f"INSERT INTO driving.edges VALUES ({A},1,2,'Hill',1,100,10.0,40.0,"
                f"ST_GeomFromText('LINESTRING(18.06 59.32, 18.07 59.32)'))")
    con.close()
    to_opendrive(str(path), tmp_path / "n.xodr")
    r = _roads(tmp_path / "n.xodr")[1][str(A)]
    e = r.find("./elevationProfile/elevation")
    assert e is not None and float(e.get("a")) == pytest.approx(10.0)      # z_from at s=0
    assert float(e.get("b")) == pytest.approx((40.0 - 10.0) / float(r.get("length")), abs=1e-5)  # grade (6 dp)
    tags = [c.tag for c in r]
    assert tags.index("planView") < tags.index("elevationProfile") < tags.index("lanes")   # schema order


def test_georeference_and_metric_coords(tmp_path):
    to_opendrive(str(_src(tmp_path / "s.duckdb")), tmp_path / "n.xodr", crs="EPSG:3006")
    root, roads = _roads(tmp_path / "n.xodr")
    geo = root.find("header/geoReference").text
    assert "proj=tmerc" in geo and "lon_0=15" in geo         # exact SWEREF99 TM, not a UTM approx
    g0 = roads[str(A)].find("./planView/geometry")
    assert abs(float(g0.get("x"))) > 1000 and abs(float(g0.get("y"))) > 1000   # metric, not degrees
    # heading is a sane radian value
    assert -math.pi <= float(g0.get("hdg")) <= math.pi


def test_bad_mode_raises(tmp_path):
    with pytest.raises(ValueError, match="cycling"):
        to_opendrive(str(_src(tmp_path / "s.duckdb")), tmp_path / "n.xodr", mode="cycling")


# ---- Phase 2: routable junctions (from a GMNS db) ----

def _gmns(tmp_path):
    """A GMNS db from a source with a turn choice A→{B,C} at node 2 (→ node 2 is a junction)."""
    src = tmp_path / "src.duckdb"
    con = duckdb.connect(str(src))
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"INSERT INTO driving.nodes VALUES (1,{p('POINT(18.06 59.32)')}),(2,{p('POINT(18.07 59.32)')}),"
                f"(3,{p('POINT(18.07 59.31)')}),(4,{p('POINT(18.08 59.32)')})")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, "
                "highway VARCHAR, name VARCHAR, lanes INTEGER, is_reverse BOOLEAN, length_m FLOAT, "
                "maxspeed_kmh FLOAT, geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({A},1,2,100,'primary','Main',2,false,80,50,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({B},2,3,101,'residential',NULL,1,false,110,30,{p('LINESTRING(18.07 59.32,18.07 59.31)')}),
        ({C},2,4,102,'residential',NULL,1,false,110,30,{p('LINESTRING(18.07 59.32,18.08 59.32)')}),
        ({AR},2,1,100,'primary','Main',2,true,80,50,{p('LINESTRING(18.07 59.32,18.06 59.32)')})""")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, via_edge BIGINT, cost DOUBLE)")
    con.execute(f"INSERT INTO driving.edge_graph VALUES ({A},{B},{B},1.0),({A},{C},{C},1.0)")
    con.close()
    gmns = tmp_path / "gmns.duckdb"
    to_gmns(str(src), str(gmns))
    return gmns


def test_junctions_structure(tmp_path):
    res = to_opendrive(str(_gmns(tmp_path)), tmp_path / "n.xodr", junctions=True)
    assert res["junctions"] == 1 and res["connecting_roads"] >= 2   # node 2, turns A→B and A→C
    root, roads = _roads(tmp_path / "n.xodr")
    junc = root.findall("junction")
    assert len(junc) == 1 and junc[0].get("id") == "2"
    conns = junc[0].findall("connection")
    assert len(conns) >= 2
    c0 = conns[0]
    assert c0.get("incomingRoad") == str(A) and c0.find("laneLink") is not None
    # its connecting road exists, is tagged junction=2, and links ib→ob
    cr = roads[c0.get("connectingRoad")]
    assert cr.get("junction") == "2"
    assert cr.find("./link/predecessor").get("elementId") == str(A)
    assert cr.find("./link/successor").get("elementId") in {str(B), str(C)}
    # main road A links forward into the junction
    assert roads[str(A)].find("./link/successor").get("elementType") == "junction"
    assert roads[str(A)].find("./link/successor").get("elementId") == "2"


def test_junctions_requires_gmns(tmp_path):
    # core db (no gmns_driving.movement) → --junctions raises
    with pytest.raises(ValueError, match="GMNS"):
        to_opendrive(str(_src(tmp_path / "core.duckdb")), tmp_path / "n.xodr", junctions=True)
