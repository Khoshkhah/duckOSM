"""Tests for duckosm.matsim_lanes — MATSim lanes.xml + signals from a GMNS db (XSD-validated)."""
from pathlib import Path

import duckdb
import pytest

from duckosm.gmns import to_gmns
from duckosm.matsim_lanes import to_matsim_lanes

_XSD = Path(__file__).parent / "fixtures" / "matsim_xsd"       # vendored MATSim v2.0 schemas
A, B, C, AR = 6141068311830699705, 3843102655846694531, 5555555555555555555, 1234567890123456789


def _gmns(tmp_path, signal=False):
    """Build a GMNS db from a tiny source with a turn choice A→{B,C} at node 2."""
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
    if signal:
        w = duckdb.connect(str(gmns))
        w.execute("UPDATE gmns_driving.node SET ctrl_type='signal' WHERE node_id=2")
        w.close()
    return gmns


def _xsd_valid(path, xsd_name):
    etree = pytest.importorskip("lxml.etree")
    schema = etree.XMLSchema(etree.parse(str(_XSD / xsd_name)))
    tree = etree.parse(str(path))
    ok = schema.validate(tree)
    if not ok:
        pytest.fail(f"{path.name} not valid against {xsd_name}:\n{schema.error_log}")
    return ok


def test_lanes_xml(tmp_path):
    res = to_matsim_lanes(str(_gmns(tmp_path)), out_dir=tmp_path, mode="driving", signals=False)
    assert res["lanes_links"] >= 1 and "signal_systems" not in res
    lanes = tmp_path / "lanes.xml"
    assert _xsd_valid(lanes, "laneDefinitions_v2.0.xsd")
    text = lanes.read_text()
    assert f'linkIdRef="{A}"' in text                          # inbound link A gets an assignment
    assert f'refId="{B}"' in text and f'refId="{C}"' in text   # lane leads to both legal turns
    # signals not requested → no signal files
    assert not (tmp_path / "signalSystems.xml").exists()


def test_signals_xsd_valid(tmp_path):
    res = to_matsim_lanes(str(_gmns(tmp_path, signal=True)), out_dir=tmp_path, mode="driving",
                          signals=True, cycle_s=90)
    assert res["signal_systems"] == 1                          # node 2 is signalised
    for f, xsd in (("signalSystems.xml", "signalSystems_v2.0.xsd"),
                   ("signalGroups.xml", "signalGroups_v2.0.xsd"),
                   ("signalControl.xml", "signalControl_v2.0.xsd")):
        assert _xsd_valid(tmp_path / f, xsd)
    systems = (tmp_path / "signalSystems.xml").read_text()
    assert 'signalSystem id="2"' in systems and f'linkIdRef="{A}"' in systems
    control = (tmp_path / "signalControl.xml").read_text()
    assert 'cycleTime sec="90"' in control and "DefaultPlanbasedSignalSystemController" in control


def test_signalised_node_only(tmp_path):
    # no signalised node → no signal systems even with signals=True
    res = to_matsim_lanes(str(_gmns(tmp_path, signal=False)), out_dir=tmp_path, signals=True)
    assert res["signal_systems"] == 0


def test_bad_db_raises(tmp_path):
    empty = tmp_path / "empty.duckdb"
    duckdb.connect(str(empty)).close()
    with pytest.raises(ValueError, match="movement"):
        to_matsim_lanes(str(empty), out_dir=tmp_path)
