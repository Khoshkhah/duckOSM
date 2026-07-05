"""Tests for duckosm.matsim — MATSim network.xml export from a built duckOSM db."""
import gzip
import xml.etree.ElementTree as ET

import duckdb
import pytest

from duckosm.matsim import to_matsim

A, B, AR = 6141068311830699705, 3843102655846694531, 1234567890123456789


def _src(path):
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"INSERT INTO driving.nodes VALUES (1,{p('POINT(18.06 59.32)')}),"
                f"(2,{p('POINT(18.07 59.32)')}),(3,{p('POINT(18.07 59.31)')})")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, highway VARCHAR, "
                "lanes INTEGER, maxspeed_kmh FLOAT, length_m FLOAT, cost_s FLOAT, geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({A},1,2,'primary',2,50,80,6,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({B},2,3,'residential',1,NULL,100,12,{p('LINESTRING(18.07 59.32,18.07 59.31)')}),
        ({AR},2,1,'primary',2,50,80,6,{p('LINESTRING(18.07 59.32,18.06 59.32)')})""")
    con.close()
    return path


def _parse(path, gz=True):
    if gz:
        with gzip.open(path, "rt") as f:
            return ET.parse(f).getroot()
    return ET.parse(path).getroot()


def test_structure_and_counts(tmp_path):
    src = _src(tmp_path / "s.duckdb")
    res = to_matsim(str(src), tmp_path / "net.xml.gz")
    assert res == {"nodes": 3, "links": 3}
    root = _parse(tmp_path / "net.xml.gz")
    assert root.tag == "network"
    assert root.find("./attributes/attribute").text == "EPSG:3006"
    nodes = root.findall("./nodes/node"); links = root.findall("./links/link")
    assert len(nodes) == 3 and len(links) == 3
    nid = {n.get("id") for n in nodes}
    assert all(l.get("from") in nid and l.get("to") in nid for l in links)   # referential integrity
    # metric (projected) coords, not degrees
    assert all(abs(float(n.get("x"))) > 1000 for n in nodes)


def test_link_attributes(tmp_path):
    to_matsim(str(_src(tmp_path / "s.duckdb")), tmp_path / "net.xml.gz")
    links = {l.get("id"): l for l in _parse(tmp_path / "net.xml.gz").findall("./links/link")}
    a = links[str(A)]
    assert abs(float(a.get("freespeed")) - 50/3.6) < 1e-3          # from maxspeed 50 km/h
    assert float(a.get("capacity")) == 1600 * 2                    # _CAPACITY[primary] × permlanes
    assert a.get("permlanes") == "2" and a.get("modes") == "car"
    b = links[str(B)]
    assert abs(float(b.get("freespeed")) - 100/12) < 1e-3          # maxspeed NULL → length/cost_s
    assert all(float(l.get("freespeed")) > 0 for l in links.values())


def test_no_gzip_and_dtd(tmp_path):
    to_matsim(str(_src(tmp_path / "s.duckdb")), tmp_path / "net.xml", gzip=False)
    text = (tmp_path / "net.xml").read_text()
    assert text.startswith("<?xml") and "network_v2.dtd" in text
    assert _parse(tmp_path / "net.xml", gz=False).tag == "network"


def test_crs_override(tmp_path):
    to_matsim(str(_src(tmp_path / "s.duckdb")), tmp_path / "net.xml", crs="EPSG:32635", gzip=False)
    root = _parse(tmp_path / "net.xml", gz=False)
    assert root.find("./attributes/attribute").text == "EPSG:32635"


def test_bad_mode(tmp_path):
    with pytest.raises(ValueError, match="cycling"):
        to_matsim(str(_src(tmp_path / "s.duckdb")), tmp_path / "net.xml", mode="cycling")
