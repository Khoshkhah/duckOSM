"""Tests for duckosm.sumo.to_sumo — export a duckOSM network to SUMO, keeping edge_id."""
import re

import duckdb
import pytest

from duckosm.sumo import to_sumo, _find_netconvert

# two edges with realistic content-hash ids — the whole point is that these survive into SUMO
E1, E2 = 6141068311830699705, 3843102655846694531


def _have_netconvert():
    try:
        _find_netconvert()
        return True
    except Exception:
        return False


HAVE_NETCONVERT = _have_netconvert()


def _db():
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute(f"""INSERT INTO driving.nodes VALUES
        (1,{p('POINT(18.06 59.32)')}),(2,{p('POINT(18.07 59.32)')}),(3,{p('POINT(18.08 59.32)')})""")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, "
                "highway VARCHAR, name VARCHAR, lanes INTEGER, maxspeed_kmh FLOAT, geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({E1},1,2,'residential','A & B',1,30,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({E2},2,3,'tertiary',NULL,2,50,{p('LINESTRING(18.07 59.32,18.08 59.32)')})""")
    return con


def test_plain_xml_preserves_edge_id(tmp_path):
    out = to_sumo(_db(), str(tmp_path), run_netconvert=False)
    assert out["n_nodes"] == 3 and out["n_edges"] == 2
    assert "net" not in out                                   # netconvert not run
    edg = (tmp_path / "network.edg.xml").read_text()
    assert f'id="{E1}"' in edg and f'id="{E2}"' in edg        # edge_id == SUMO edge id
    assert 'from="1"' in edg and 'to="2"' in edg
    assert 'type="residential"' in edg and 'numLanes="1"' in edg
    assert 'speed="8.333"' in edg                             # 30 km/h -> m/s
    assert 'name="A &amp; B"' in edg                         # attribute properly escaped
    nod = (tmp_path / "network.nod.xml").read_text()
    assert 'id="1"' in nod and 'x="18.06' in nod


def test_missing_values_are_omitted(tmp_path):
    out = to_sumo(_db(), str(tmp_path), run_netconvert=False)
    edg = (tmp_path / "network.edg.xml").read_text()
    e2 = [ln for ln in edg.splitlines() if f'id="{E2}"' in ln][0]
    assert "name=" not in e2                                  # E2 has NULL name -> omitted


@pytest.mark.skipif(not HAVE_NETCONVERT,
                    reason="netconvert not installed (pip install duckosm[sumo])")
def test_netconvert_preserves_all_edge_ids(tmp_path):
    out = to_sumo(_db(), str(tmp_path), net_name="t", run_netconvert=True)
    assert out["net"].endswith("t.net.xml")
    net = (tmp_path / "t.net.xml").read_text()
    sumo_ids = set(re.findall(r'<edge id="([^":][^"]*)"', net))   # exclude internal ':' edges
    assert {str(E1), str(E2)} <= sumo_ids                         # both duckOSM ids survived
