"""duckosm.Network on Monaco (src/duckosm/network.py): the result of build and open, its tables, its map, and the
functions that take it as a path."""
import os
from pathlib import Path

import duckdb
import pytest

import duckosm

DATA = Path(__file__).parent / "data"


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    out = tmp_path_factory.mktemp("network") / "monaco.duckdb"
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving", "walking"],
                         output=out)


def test_build_and_open_give_a_network(net):
    assert isinstance(net, duckosm.Network)
    assert net == net.path and net == str(net.path) and duckosm.open(net.path) == net
    assert os.fspath(net) == str(net) == str(net.path)
    assert net.name == "monaco.duckdb" and net.exists()          # the attributes of the Path, as before
    assert net.modes == ["driving", "walking"]
    with pytest.raises(FileNotFoundError):
        duckosm.open(net.path.with_name("none.duckdb"))


def test_tables(net):
    pytest.importorskip("geopandas")
    con = duckdb.connect(str(net), read_only=True)                 # a Network is a path for duckdb too
    n, nodes = con.execute("SELECT (SELECT count(*) FROM driving.edges), (SELECT count(*) FROM driving.nodes)").fetchone()
    con.close()
    gdf = net.edges()
    assert len(gdf) == n and str(gdf.crs) == "EPSG:4326" and str(gdf.edge_id.dtype) == "int64"
    assert gdf.edge_id.is_monotonic_increasing and {"osm_id", "highway", "length_m", "maxspeed_kmh"} <= set(gdf.columns)
    main = net.edges(where="highway = 'primary'")
    assert 0 < len(main) < n and set(main.highway) == {"primary"}
    assert len(net.nodes()) == nodes and net.nodes().geometry.geom_type.eq("Point").all()
    assert net.sql("SELECT count(*) AS n FROM walking.edges").n[0] > 0
    with pytest.raises(ValueError):
        net.edges("driving; DROP TABLE x")


def test_functions_take_the_network(net):
    r = duckosm.route_points(net, (7.4155, 43.7285), (7.4335, 43.7468))   # an existing function, with os.fspath
    assert r == net.route_points((7.4155, 43.7285), (7.4335, 43.7468)) and r["edges"]
    assert duckosm.to_gis(net, os.path.join(os.path.dirname(net), "monaco.gpkg"), modes=["driving"])["layers"]["edges_driving"] == len(net.edges())
    assert net.directions(r)[-1]["type"] == "arrive"


def test_map_picture(net, monkeypatch):
    pytest.importorskip("matplotlib")
    import matplotlib
    matplotlib.use("Agg")
    monkeypatch.setenv("DUCKOSM_STATIC_MAPS", "1")
    gdf = net.edges()
    assert net.map(highlight=gdf.edge_id[:3], color_by="maxspeed_kmh") is None


def test_map_live(net, monkeypatch):
    pytest.importorskip("roadstyle")
    monkeypatch.delenv("DUCKOSM_STATIC_MAPS", raising=False)
    assert hasattr(net.map(highlight=net.edges().edge_id[:3]), "_repr_html_")
