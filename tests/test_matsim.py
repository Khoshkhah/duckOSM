"""Tests for duckosm.matsim — MATSim network.xml export from a built duckOSM db."""
import gzip
import xml.etree.ElementTree as ET
from pathlib import Path

import duckdb
import pytest

from duckosm.matsim import to_matsim

_DTD = Path(__file__).parent / "fixtures" / "network_v2.dtd"   # vendored official MATSim network_v2 DTD

A, B, AR = 6141068311830699705, 3843102655846694531, 1234567890123456789
SH, D_ONLY, C_ONLY, W_ONLY = 111111, 222222, 333333, 444444   # multimodal edge_ids


def _build_multi(path, nodes, per_mode):
    """Build a multi-mode source: nodes shared, per_mode = {mode: [edge tuples]} (edge tuple as _build)."""
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial;")
    g = "ST_GeomFromText('LINESTRING(18.06 59.32,18.07 59.32)')"
    for m, edges in per_mode.items():
        con.execute(f"CREATE SCHEMA {m}")
        con.execute(f"CREATE TABLE {m}.nodes(node_id BIGINT, geom GEOMETRY)")
        for nid, lon, lat in nodes:
            con.execute(f"INSERT INTO {m}.nodes VALUES ({nid}, ST_GeomFromText('POINT({lon} {lat})'))")
        con.execute(f"CREATE TABLE {m}.edges(edge_id BIGINT, source BIGINT, target BIGINT, highway VARCHAR, "
                    f"lanes INTEGER, maxspeed_kmh FLOAT, length_m FLOAT, cost_s FLOAT, geometry GEOMETRY)")
        for eid, src, tgt, hw, lanes, spd, length, cost in edges:
            lv, sv = ("NULL" if lanes is None else str(lanes)), ("NULL" if spd is None else str(spd))
            con.execute(f"INSERT INTO {m}.edges VALUES ({eid},{src},{tgt},'{hw}',{lv},{sv},{length},{cost},{g})")
    con.close()
    return path


def _multi_src(path):
    nodes = [(1, 18.06, 59.32), (2, 18.07, 59.32), (3, 18.07, 59.31), (4, 18.06, 59.31)]
    return _build_multi(path, nodes, {
        "driving": [(SH, 1, 2, "primary", 2, 50, 80, 6), (D_ONLY, 2, 3, "residential", 1, None, 50, 6)],
        "cycling": [(SH, 1, 2, "primary", 2, 50, 80, 6), (C_ONLY, 3, 4, "cycleway", None, None, 40, 8)],
        "walking": [(W_ONLY, 1, 4, "footway", None, None, 60, 12)],
    })


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


def _build(path, nodes, edges):
    """Build a minimal driving-only source db from (node_id, lon, lat) + edge tuples
    (edge_id, source, target, highway, lanes, maxspeed_kmh, length_m, cost_s)."""
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    for nid, lon, lat in nodes:
        con.execute(f"INSERT INTO driving.nodes VALUES ({nid}, ST_GeomFromText('POINT({lon} {lat})'))")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, highway VARCHAR, "
                "lanes INTEGER, maxspeed_kmh FLOAT, length_m FLOAT, cost_s FLOAT, geometry GEOMETRY)")
    g = "ST_GeomFromText('LINESTRING(18.06 59.32,18.07 59.32)')"
    for eid, src, tgt, hw, lanes, spd, length, cost in edges:
        lv, sv = ("NULL" if lanes is None else str(lanes)), ("NULL" if spd is None else str(spd))
        con.execute(f"INSERT INTO driving.edges VALUES ({eid},{src},{tgt},'{hw}',{lv},{sv},{length},{cost},{g})")
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


def test_dtd_valid(tmp_path):
    """The emitted network validates against the official MATSim network_v2 DTD (vendored)."""
    lxml_etree = pytest.importorskip("lxml.etree")
    to_matsim(str(_src(tmp_path / "s.duckdb")), tmp_path / "net.xml", gzip=False)
    dtd = lxml_etree.DTD(str(_DTD))
    tree = lxml_etree.parse(str(tmp_path / "net.xml"))
    assert dtd.validate(tree), "network_v2.dtd validation failed:\n" + "\n".join(
        e.message for e in dtd.error_log)


def test_isolated_node_dropped(tmp_path):
    # node 99 has no incident link → must not appear in <nodes>
    src = _build(tmp_path / "s.duckdb", [(1, 18.06, 59.32), (2, 18.07, 59.32), (99, 18.08, 59.33)],
                 [(A, 1, 2, "primary", 2, 50, 80, 6)])
    res = to_matsim(str(src), tmp_path / "n.xml", gzip=False)
    ids = {n.get("id") for n in _parse(tmp_path / "n.xml", gz=False).findall("./nodes/node")}
    assert ids == {"1", "2"} and res["nodes"] == 2


def test_capacity_default_and_permlanes_floor(tmp_path):
    # unknown highway class → capacity default 800; lanes NULL → permlanes floored to 1
    src = _build(tmp_path / "s.duckdb", [(1, 18.06, 59.32), (2, 18.07, 59.32)],
                 [(A, 1, 2, "pedestrian", None, None, 50, 6)])
    to_matsim(str(src), tmp_path / "n.xml", gzip=False)
    link = _parse(tmp_path / "n.xml", gz=False).find("./links/link")
    assert float(link.get("capacity")) == 800.0 and link.get("permlanes") == "1"
    assert float(link.get("freespeed")) == pytest.approx(50 / 6, abs=1e-3)   # maxspeed NULL → length/cost_s (4dp)


def test_link_with_missing_endpoint_skipped(tmp_path):
    # edge B → node 3 which doesn't exist: B is dropped, A survives (no crash)
    src = _build(tmp_path / "s.duckdb", [(1, 18.06, 59.32), (2, 18.07, 59.32)],
                 [(A, 1, 2, "primary", 2, 50, 80, 6), (B, 2, 3, "primary", 1, 50, 80, 6)])
    res = to_matsim(str(src), tmp_path / "n.xml", gzip=False)
    ids = {l.get("id") for l in _parse(tmp_path / "n.xml", gz=False).findall("./links/link")}
    assert ids == {str(A)} and res["links"] == 1


def test_gzip_roundtrip(tmp_path):
    # gzip output re-opens and parses to the same counts
    to_matsim(str(_src(tmp_path / "s.duckdb")), tmp_path / "n.xml.gz", gzip=True)
    root = _parse(tmp_path / "n.xml.gz", gz=True)
    assert len(root.findall("./links/link")) == 3 and len(root.findall("./nodes/node")) == 3


def test_multimodal_merge(tmp_path):
    res = to_matsim(str(_multi_src(tmp_path / "s.duckdb")), tmp_path / "n.xml", mode="all", gzip=False)
    assert res == {"nodes": 4, "links": 4}                       # 4 distinct edge_ids, node union
    links = {l.get("id"): l for l in _parse(tmp_path / "n.xml", gz=False).findall("./links/link")}
    sh = links[str(SH)]                                          # shared driving+cycling segment
    assert sh.get("modes") == "car,bike"                        # union of modes on one link
    assert sh.get("permlanes") == "2" and float(sh.get("capacity")) == 1600 * 2   # car attrs
    assert links[str(C_ONLY)].get("modes") == "bike"            # cycling-only
    assert links[str(W_ONLY)].get("modes") == "walk"           # walking-only
    assert links[str(D_ONLY)].get("modes") == "car"


def test_multimodal_subset(tmp_path):
    # --mode driving,cycling drops the walk-only link
    to_matsim(str(_multi_src(tmp_path / "s.duckdb")), tmp_path / "n.xml", mode="driving,cycling", gzip=False)
    ids = {l.get("id") for l in _parse(tmp_path / "n.xml", gz=False).findall("./links/link")}
    assert ids == {str(SH), str(D_ONLY), str(C_ONLY)}          # no W_ONLY


def test_node_z_from_ele(tmp_path):
    # nodes carrying `ele` (from `duckosm elevation`) → each <node> gets a z attribute (2 dp)
    path = tmp_path / "s.duckdb"
    con = duckdb.connect(str(path)); con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY, ele DOUBLE)")
    con.execute("INSERT INTO driving.nodes VALUES (1, ST_GeomFromText('POINT(18.06 59.32)'), 12.5),"
                "(2, ST_GeomFromText('POINT(18.07 59.32)'), 40.0)")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, highway VARCHAR, "
                "lanes INTEGER, maxspeed_kmh FLOAT, length_m FLOAT, cost_s FLOAT, geometry GEOMETRY)")
    con.execute(f"INSERT INTO driving.edges VALUES ({A},1,2,'primary',2,50,80,6,"
                f"ST_GeomFromText('LINESTRING(18.06 59.32,18.07 59.32)'))")
    con.close()
    to_matsim(str(path), tmp_path / "n.xml", gzip=False)
    nodes = {n.get("id"): n for n in _parse(tmp_path / "n.xml", gz=False).findall("./nodes/node")}
    assert nodes["1"].get("z") == "12.50" and nodes["2"].get("z") == "40.00"
    lxml_etree = pytest.importorskip("lxml.etree")             # z is DTD-valid (network_v2 #IMPLIED)
    dtd = lxml_etree.DTD(str(_DTD))
    assert dtd.validate(lxml_etree.parse(str(tmp_path / "n.xml")))


def test_no_node_z_without_ele(tmp_path):
    # the standard fixture has no `ele` column → nodes have no z attribute (unchanged output)
    to_matsim(str(_src(tmp_path / "s.duckdb")), tmp_path / "n.xml", gzip=False)
    assert all(n.get("z") is None for n in _parse(tmp_path / "n.xml", gz=False).findall("./nodes/node"))


def test_multimodal_dtd_valid(tmp_path):
    lxml_etree = pytest.importorskip("lxml.etree")
    to_matsim(str(_multi_src(tmp_path / "s.duckdb")), tmp_path / "n.xml", mode="all", gzip=False)
    dtd = lxml_etree.DTD(str(_DTD))
    tree = lxml_etree.parse(str(tmp_path / "n.xml"))
    assert dtd.validate(tree), "\n".join(e.message for e in dtd.error_log)   # modes="car,bike,walk" valid CDATA
