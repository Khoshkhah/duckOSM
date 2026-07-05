"""Tests for duckosm.opendrive — ASAM OpenDRIVE .xodr export (Phase 1: roads + lanes + geometry).

The ASAM OpenDRIVE XSD is behind GitLab auth (not freely fetchable), so — as the design doc allows —
these are structural assertions on a well-formed .xodr (esmini load is the documented manual check)."""
import math
import xml.etree.ElementTree as ET

import duckdb
import pytest

from duckosm.opendrive import to_opendrive

A, B = 6141068311830699705, 3843102655846694531


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
    assert res == {"roads": 2}
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
