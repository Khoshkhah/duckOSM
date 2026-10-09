"""Tests for duckosm.sumo.to_sumo — export a duckOSM network to SUMO, keeping edge_id + turns."""
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


def _db(mode="driving"):
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute(f"CREATE SCHEMA {mode}; USE {mode}")
    con.execute("CREATE TABLE nodes(node_id BIGINT, geom GEOMETRY)")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute(f"""INSERT INTO nodes VALUES
        (1,{p('POINT(18.06 59.32)')}),(2,{p('POINT(18.07 59.32)')}),(3,{p('POINT(18.08 59.32)')})""")
    con.execute("CREATE TABLE edges(edge_id BIGINT, source BIGINT, target BIGINT, "
                "highway VARCHAR, name VARCHAR, lanes INTEGER, maxspeed_kmh FLOAT, length_m FLOAT, "
                "geometry GEOMETRY)")
    con.execute(f"""INSERT INTO edges VALUES
        ({E1},1,2,'residential','A & B',1,30,123.4,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({E2},2,3,'tertiary',NULL,2,50,567.8,{p('LINESTRING(18.07 59.32,18.08 59.32)')})""")
    # legal-successor line graph (turn restrictions already excluded): only E1 -> E2 is allowed
    con.execute("CREATE TABLE edge_graph(from_edge BIGINT, to_edge BIGINT, cost DOUBLE)")
    con.execute(f"INSERT INTO edge_graph VALUES ({E1},{E2},1.0)")
    return con


def test_plain_xml_preserves_edge_id_and_emits_connections(tmp_path):
    out = to_sumo(_db(), str(tmp_path), run_netconvert=False)
    assert out["n_nodes"] == 3 and out["n_edges"] == 2 and "net" not in out
    edg = (tmp_path / "network.edg.xml").read_text()
    assert f'id="{E1}"' in edg and f'id="{E2}"' in edg        # edge_id == SUMO edge id
    assert 'from="1"' in edg and 'to="2"' in edg
    assert 'priority=' in edg and 'numLanes="1"' in edg
    assert 'speed="8.333"' in edg                             # 30 km/h -> m/s
    assert 'length="123.40"' in edg                           # true graph length stamped
    assert 'type="residential"' in edg
    assert 'name="A &amp; B"' in edg                          # attribute properly escaped
    # connections come from edge_graph -> turn restrictions honoured
    assert out["n_connections"] == 1
    conx = (tmp_path / "network.con.xml").read_text()
    assert f'<connection from="{E1}" to="{E2}"/>' in conx
    assert f'<connection from="{E2}"/>' in conx               # no legal successor: none invented
    assert "allow=" not in edg                                # driving: every vehicle class
    nod = (tmp_path / "network.nod.xml").read_text()
    assert 'id="1"' in nod and 'x="18.06' in nod


def test_missing_values_are_omitted(tmp_path):
    out = to_sumo(_db(), str(tmp_path), run_netconvert=False)
    e2 = [ln for ln in (tmp_path / "network.edg.xml").read_text().splitlines()
          if f'id="{E2}"' in ln][0]
    assert "name=" not in e2                                  # E2 has NULL name -> omitted


def test_no_edge_graph_falls_back_to_inference(tmp_path):
    con = _db()
    con.execute("DROP TABLE edge_graph")
    out = to_sumo(con, str(tmp_path), run_netconvert=False)
    assert "con" not in out                                   # no edge_graph -> no .con.xml emitted


@pytest.mark.skipif(not HAVE_NETCONVERT,
                    reason="netconvert not installed (pip install duckosm[sumo])")
def test_netconvert_preserves_ids_and_connections(tmp_path):
    out = to_sumo(_db(), str(tmp_path), net_name="t", run_netconvert=True)
    assert out["net"].endswith("t.net.xml")
    net = (tmp_path / "t.net.xml").read_text()
    sumo_ids = set(re.findall(r'<edge id="([^":][^"]*)"', net))   # exclude internal ':' edges
    assert {str(E1), str(E2)} <= sumo_ids                         # both duckOSM ids survived
    assert "<connection " in net                                  # explicit connections kept


@pytest.mark.skipif(not HAVE_NETCONVERT,
                    reason="netconvert not installed (pip install duckosm[sumo])")
def test_netconvert_with_a_relative_out_dir(tmp_path, monkeypatch):
    """SUMO resolves .netccfg paths against the config's folder: a relative out_dir (the CLI default,
    `sumo`) once produced `sumo/sumo/...` paths and netconvert failed with "No nodes loaded"."""
    monkeypatch.chdir(tmp_path)
    out = to_sumo(_db(), "sumo", net_name="t")
    assert (tmp_path / "sumo" / "t.net.xml").exists(), out


@pytest.mark.skipif(not HAVE_NETCONVERT,
                    reason="netconvert not installed (pip install duckosm[sumo])")
def test_netccfg_default_and_override(tmp_path):
    out = to_sumo(_db(), str(tmp_path / "a"), net_name="d")       # default config
    cfg = open(out["netccfg"]).read()
    assert "<configuration>" in cfg                               # standard SUMO config format
    assert '<geometry.remove value="false"/>' in cfg
    assert '<proj.utm value="true"/>' in cfg
    net = open(out["net"]).read()
    assert "+proj=utm +zone=34" in net                            # metres, not degrees (18.07 E)
    x1 = float(re.search(r'convBoundary="[\d.]+,[\d.]+,([\d.]+)', net).group(1))
    assert 1000 < x1 < 1200                                       # 0.02° of longitude ≈ 1.1 km
    out2 = to_sumo(_db(), str(tmp_path / "b"), net_name="d",
                   config={"geometry.remove": "true"})           # dict override merged onto default
    assert '<geometry.remove value="true"/>' in open(out2["netccfg"]).read()


def test_walking_edges_are_for_pedestrians(tmp_path):
    to_sumo(_db("walking"), str(tmp_path), mode="walking", run_netconvert=False)
    assert 'allow="pedestrian"' in (tmp_path / "network.edg.xml").read_text()


def test_a_path_is_one_lane_of_path_width(tmp_path):
    """A walking edge on a road with lanes=2 was two 3.2 m lanes (a 6.4 m footpath)."""
    out = to_sumo(_db("walking"), str(tmp_path), mode="walking", run_netconvert=False)
    assert "numLanes=" not in (tmp_path / "network.edg.xml").read_text()
    if HAVE_NETCONVERT:
        to_sumo(_db("walking"), str(tmp_path), mode="walking")
        assert '<default.lanewidth value="2.0"/>' in (tmp_path / "network.netccfg").read_text()


def test_edge_attrs_override(tmp_path):
    to_sumo(_db(), str(tmp_path), run_netconvert=False, edge_attrs={str(E1): {"numLanes": 2, "width": 3.1}})   # text ids work too
    e1 = [ln for ln in (tmp_path / "network.edg.xml").read_text().splitlines() if f'id="{E1}"' in ln][0]
    assert 'numLanes="2"' in e1 and 'numLanes="1"' not in e1 and 'width="3.1"' in e1


def test_a_one_way_road_is_centred_on_its_line(tmp_path):
    """SUMO puts an edge's lanes right of its line: right for one direction of a two-way road, but a one-way road's line is its middle."""
    con = _db()
    con.execute(f"""INSERT INTO edges VALUES (7, 2, 1, 'residential', NULL, 1, 30, 123.4,
                    ST_GeomFromText('LINESTRING(18.07 59.32,18.06 59.32)'))""")      # E1 now has its reverse twin
    to_sumo(con, str(tmp_path), run_netconvert=False)
    edg = {ln.split('"')[1]: ln for ln in (tmp_path / "network.edg.xml").read_text().splitlines() if "<edge " in ln}
    assert 'spreadType="center"' in edg[str(E2)]                    # one-way
    assert "spreadType" not in edg[str(E1)] and "spreadType" not in edg["7"]
