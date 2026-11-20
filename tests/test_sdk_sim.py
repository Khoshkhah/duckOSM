"""duckosm.sim on Monaco: sumo_net, edge_map (docs/proposals/python-sdk.md section 5.4)."""
import shutil
from pathlib import Path

import pytest

import duckosm

DATA = Path(__file__).parent / "data"


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving"],
                         output=tmp_path_factory.mktemp("sim") / "monaco.duckdb", progress=False)


def test_edge_map_matsim(net, tmp_path):
    duckosm.sim.to_matsim(net.driving, tmp_path / "network.xml.gz")
    m = duckosm.sim.edge_map(tmp_path / "network.xml.gz")
    assert set(m.edge_id) == set(net.edges().edge_id) and (m.id == m.edge_id.astype(str)).all()


def test_edge_map_names_of_netconvert(tmp_path):
    (tmp_path / "x.net.xml").write_text(
        '<net><edge id=":j_0" function="internal"/><edge id="12#1"/><edge id="34+56"/><edge id="78"/></net>')
    m = duckosm.sim.edge_map(tmp_path / "x.net.xml")
    assert list(zip(m.id, m.edge_id)) == [("12#1", 12), ("34+56", 34), ("34+56", 56), ("78", 78)]


def test_sumo_net_and_edge_map(net, tmp_path):
    sumolib = pytest.importorskip("sumolib")
    if not shutil.which("netconvert"):
        pytest.skip("no netconvert")
    files = duckosm.sim.to_sumo(net.driving, tmp_path / "sumo")
    sn = duckosm.sim.sumo_net(files)
    assert isinstance(sn, sumolib.net.Net) and len(sn.getEdges(withInternal=False)) > 0
    m = duckosm.sim.edge_map(files["net"])
    assert len(m) >= len(sn.getEdges(withInternal=False))
    assert set(m.edge_id) <= set(net.edges().edge_id)            # each SUMO edge goes back to a duckOSM edge
