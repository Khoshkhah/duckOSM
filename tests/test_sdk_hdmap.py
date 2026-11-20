"""duckosm.hdmap on Monaco: to_commonroad (docs/proposals/python-sdk.md section 5.7)."""
import shutil
from pathlib import Path

import pytest

import duckosm

DATA = Path(__file__).parent / "data"


@pytest.fixture(scope="module")
def gmns(tmp_path_factory):
    if not shutil.which("netconvert"):
        pytest.skip("no netconvert (GMNS lanes need it)")
    d = tmp_path_factory.mktemp("hdmap")
    net = duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving"],
                        output=d / "monaco.duckdb", progress=False)
    duckosm.planning.to_gmns(net.driving, d / "gmns.duckdb")
    return d / "gmns.duckdb"


def test_to_commonroad(gmns, tmp_path):
    pytest.importorskip("commonroad")
    from commonroad.common.file_reader import CommonRoadFileReader
    out = duckosm.hdmap.to_commonroad(gmns, tmp_path / "monaco.xml")
    scenario, _ = CommonRoadFileReader(str(out["path"])).open()
    lanelets = scenario.lanelet_network.lanelets
    assert len(lanelets) == out["lanelets"] > 0.95 * (out["lanes"] + out["connectors"])
    assert sum(bool(x.successor) for x in lanelets) > 0.8 * len(lanelets)          # most lanelets go on
    assert any(x.adj_right for x in lanelets)                                         # lanes side by side
    ids = out["ids"]
    import duckdb
    links = {r[0] for r in duckdb.connect(str(gmns), read_only=True).execute("SELECT link_id FROM gmns_driving.link").fetchall()}
    assert set(ids.edge_id) <= links                                                  # each lanelet goes back to a link
