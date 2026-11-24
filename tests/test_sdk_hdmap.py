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
    duckosm.planning.to_gmns(net.driving, d / "gmns.duckdb", lane_source="netconvert")   # the fixture needs netconvert: its lanes
    return d / "gmns.duckdb"


def test_to_commonroad(gmns, tmp_path):
    """A file name with "_" (the map name of the scenario id): commonroad-io 2022 failed to read it back."""
    pytest.importorskip("commonroad")
    from commonroad.common.file_reader import CommonRoadFileReader
    out = duckosm.hdmap.to_commonroad(gmns, tmp_path / "monaco_commonroad.xml")
    scenario, _ = CommonRoadFileReader(str(out["path"])).open()
    lanelets = scenario.lanelet_network.lanelets
    assert len(lanelets) == out["lanelets"] > 0.95 * (out["lanes"] + out["connectors"])
    assert sum(bool(x.successor) for x in lanelets) > 0.8 * len(lanelets)          # most lanelets go on
    assert any(x.adj_right for x in lanelets)                                         # lanes side by side
    ids = out["ids"]
    import duckdb
    links = {r[0] for r in duckdb.connect(str(gmns), read_only=True).execute("SELECT link_id FROM gmns_driving.link").fetchall()}
    assert set(ids.edge_id) <= links                                                  # each lanelet goes back to a link
    assert out["dropped"] == out["lanes"] + out["connectors"] - out["lanelets"]       # the rows with no lanelet
    assert len(ids) == out["lanelets"] and set(ids.lanelet_id) == {x.lanelet_id for x in lanelets}


def test_to_opendrive_junctions_from_a_network(gmns, tmp_path):
    """junctions=True with a mode view: the GMNS file is made in a temporary folder; the same numbers as from the GMNS file."""
    net = duckosm.Network(gmns.parent / "monaco.duckdb")
    a = duckosm.hdmap.to_opendrive(net.driving, tmp_path / "a.xodr", junctions=True)
    own = duckosm.planning.to_gmns(net.driving, tmp_path / "gmns.duckdb")["path"]   # the default lanes, as to_opendrive makes them
    b = duckosm.hdmap.to_opendrive(own, tmp_path / "b.xodr", mode="driving", junctions=True)
    assert a == b and a["junctions"] > 0


def test_surfaces(gmns):
    """One polygon for each lane with a cut line (area = length x width) and one for each junction node."""
    import duckdb
    lanes, junctions = duckosm.hdmap.surfaces(gmns)
    con = duckdb.connect(str(gmns), read_only=True)
    n_lanes = con.execute("SELECT count(*) FROM gmns_driving.lane WHERE geom_cut IS NOT NULL").fetchone()[0]
    nodes = {r[0] for r in con.execute("SELECT node_id FROM gmns_driving.node").fetchall()}
    links = {r[0] for r in con.execute("SELECT link_id FROM gmns_driving.link").fetchall()}
    con.close()
    assert len(lanes) == n_lanes > 0 and set(lanes.edge_id) <= links
    assert len(junctions) > 0 and set(junctions.node_id) <= nodes
    assert lanes.crs == junctions.crs == "EPSG:4326" and lanes.is_valid.mean() > 0.99
    m = lanes.to_crs(lanes.estimate_utm_crs())
    length = m.geometry.boundary.length / 2 - lanes.width    # a strip with flat ends: border = 2 x (length + width)
    ratio = (m.area / (length * lanes.width)).median()
    assert 0.9 < ratio < 1.1
