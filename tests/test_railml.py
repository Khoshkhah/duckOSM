"""Tests for duckosm.railml — railML 2.4 rail-infrastructure export from raw OSM.

No freely-available railML XSD → structural assertions on a well-formed railML 2.4 file
(OpenTrack/railVIVID load is the documented manual check)."""
import xml.etree.ElementTree as ET

import duckdb
import pytest

from duckosm.railml import to_railml

NS = {"r": "https://www.railml.org/schemas/2013"}


def _src(path, rail=True):
    """Two rail ways sharing a switch node (3), with a signal (2) and a station (10)."""
    con = duckdb.connect(str(path))
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR,VARCHAR), refs BIGINT[])")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR,VARCHAR))")
    if rail:
        con.execute("INSERT INTO raw.ways VALUES "
                    "(100, MAP(['railway'],['rail']), [1,2,3]), (101, MAP(['railway'],['rail']), [3,4,5])")
        con.execute("INSERT INTO raw.nodes VALUES "
                    "(1,59.320,18.060,MAP(['x'],['y'])), (2,59.321,18.061,MAP(['railway'],['signal'])), "
                    "(3,59.322,18.062,MAP(['railway'],['switch'])), (4,59.323,18.070,MAP(['x'],['y'])), "
                    "(5,59.324,18.080,MAP(['x'],['y'])), "
                    "(10,59.320,18.050,MAP(['railway','name'],['station','Söder']))")
    con.close()
    return path


def _root(path):
    root = ET.parse(str(path)).getroot()
    assert root.tag.endswith("railml") and root.get("version") == "2.4"
    return root


def test_infrastructure_counts(tmp_path):
    res = to_railml(str(_src(tmp_path / "s.duckdb")), tmp_path / "r.xml")
    assert res == {"tracks": 2, "switches": 1, "signals": 1, "ocps": 1}   # split at switch node 3
    root = _root(tmp_path / "r.xml")
    assert len(root.findall(".//r:track", NS)) == 2
    assert root.find(".//r:ocp", NS).get("name") == "Söder"
    assert root.find(".//r:ocp/r:geoCoord", NS).get("epsgCode") == "4326"


def test_topology_cross_refs(tmp_path):
    to_railml(str(_src(tmp_path / "s.duckdb")), tmp_path / "r.xml")
    root = _root(tmp_path / "r.xml")
    # every track has a begin + end; each connection's ref resolves to a real connection id
    for t in root.findall(".//r:track", NS):
        assert t.find("./r:trackTopology/r:trackBegin", NS) is not None
        assert t.find("./r:trackTopology/r:trackEnd", NS) is not None
    conns = root.findall(".//r:connection", NS)
    ids = {c.get("id") for c in conns}
    assert conns and {c.get("ref") for c in conns}.issubset(ids)   # topology wires the two tracks


def test_switch_and_signal_placement(tmp_path):
    to_railml(str(_src(tmp_path / "s.duckdb")), tmp_path / "r.xml")
    root = _root(tmp_path / "r.xml")
    sw = root.find(".//r:switch", NS); sig = root.find(".//r:signal", NS)
    assert sw.get("id") == "sw_3" and float(sw.get("pos")) >= 0
    assert sig.get("id") == "sig_2" and sig.get("dir") == "up"


def test_no_rail_raises_or_empty(tmp_path):
    # a db with a raw schema but no rail ways → zero tracks (not an error)
    src = tmp_path / "empty.duckdb"
    _src(src, rail=False)
    assert to_railml(str(src), tmp_path / "r.xml")["tracks"] == 0


def test_missing_raw_raises(tmp_path):
    bare = tmp_path / "bare.duckdb"
    duckdb.connect(str(bare)).close()
    with pytest.raises(ValueError, match="raw.ways"):
        to_railml(str(bare), tmp_path / "r.xml")
