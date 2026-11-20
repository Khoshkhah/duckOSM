"""duckosm.planning on Monaco: to_path4gmns, to_aequilibrae (docs/proposals/python-sdk.md section 5.5)."""
import shutil
from pathlib import Path

import pytest

import duckosm

DATA = Path(__file__).parent / "data"


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving"],
                         output=tmp_path_factory.mktemp("planning") / "monaco.duckdb", progress=False)


def test_to_path4gmns(net, tmp_path):
    pytest.importorskip("path4gmns")
    if not shutil.which("netconvert"):
        pytest.skip("no netconvert (GMNS needs it)")
    duckosm.planning.to_gmns(net.driving, tmp_path / "gmns.duckdb")
    pnet = duckosm.planning.to_path4gmns(tmp_path / "gmns.duckdb")
    r = net.sql("SELECT source, target, length_m FROM driving.edges ORDER BY length_m DESC LIMIT 1")
    d = pnet.find_shortest_path(int(r.source[0]), int(r.target[0]), mode="auto", seq_type="link", cost_type="distance")
    assert d and (tmp_path / "path4gmns" / "link.csv").is_file()


def test_to_aequilibrae(net, tmp_path):
    pytest.importorskip("aequilibrae")
    out = duckosm.planning.to_aequilibrae(net.driving, tmp_path / "aeq")
    s = net.driving.summary()
    assert out["links"] == s.edges[0] and out["nodes"] == s.nodes[0]
    from aequilibrae import Project
    p = Project()
    p.open(str(tmp_path / "aeq"))
    try:
        links = p.network.links.data
        assert len(links) == s.edges[0] and set(links.edge_id) == set(net.edges().edge_id)
        p.network.build_graphs(modes=["c"])
        assert p.network.graphs["c"].num_links > 0
    finally:
        p.close()
