"""The modules of the SDK on Monaco (docs/proposals/python-sdk.md section 3): each function that exists today, called
through its module with a Network or a ModeView, gives the result of the old top-level function."""
import gzip
import xml.etree.ElementTree as ET
from pathlib import Path

import pytest

import duckosm

DATA = Path(__file__).parent / "data"
A, B = (7.4155, 43.7285), (7.4335, 43.7468)


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    out = tmp_path_factory.mktemp("modules") / "monaco.duckdb"
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving", "walking"],
                         output=out, progress=False)


def test_the_modules_load_when_used():
    for name in ("analysis", "gis", "routing", "sim", "planning", "mobility", "hdmap", "ml", "fixes"):
        assert getattr(duckosm, name).__name__ == f"duckosm.{name}"


def test_routing(net):
    r = duckosm.routing.route_points(net.driving, A, B)
    assert r == duckosm.route_points(net, A, B)                       # the old function, mode driving
    assert duckosm.routing.route(net.driving, r["edges"][0], r["edges"][-1])["edges"] == \
        duckosm.route(net, r["edges"][0], r["edges"][-1])["edges"]
    assert duckosm.routing.directions(net.driving, r) == duckosm.directions(net, r)


def test_sim(net, tmp_path):
    s = duckosm.sim.to_sumo(net.driving, tmp_path / "sumo", run_netconvert=False)
    assert s["n_edges"] == net.driving.summary().edges[0]
    m = duckosm.sim.to_matsim(net.driving, tmp_path / "network.xml.gz")
    assert len(ET.parse(gzip.open(tmp_path / "network.xml.gz")).findall(".//link")) == m["links"]


def test_planning_hdmap_gis(net, tmp_path):
    g = duckosm.planning.to_gmns(net.driving, tmp_path / "gmns.duckdb")
    assert list(g["modes"]) == ["driving"]
    assert not [c for c in duckosm.planning.check_gmns(tmp_path / "gmns.duckdb") if c[2] > 0]
    o = duckosm.hdmap.to_opendrive(net.driving, tmp_path / "monaco.xodr")
    assert o["roads"] == net.driving.summary().edges[0]
    gis = duckosm.gis.to_gis(net.walking, tmp_path / "walk.gpkg")
    assert set(gis["layers"]) >= {"edges_walking", "nodes_walking"} and "edges_driving" not in gis["layers"]
    q = duckosm.gis.quack(DATA / "monaco.osm.pbf", out=tmp_path, name="monaco", formats=["geoparquet"])
    assert q and q[0].stat().st_size > 0


def test_ml(net):
    nx = pytest.importorskip("networkx")
    pytest.importorskip("rustworkx")
    g = duckosm.ml.to_networkx(net.driving)
    assert isinstance(g, nx.DiGraph) and g.number_of_nodes() > 0
    h = duckosm.ml.to_rustworkx(net.driving)
    assert h.num_nodes() == g.number_of_nodes() and h.num_edges() == g.number_of_edges()
    assert net.driving.to_networkx().number_of_edges() == g.number_of_edges()
