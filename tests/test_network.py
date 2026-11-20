"""The core of the SDK on Monaco (src/duckosm/network.py, docs/proposals/python-sdk.md section 4): Network, ModeView,
open, Area.build, the mode rules, the data, the cards, the maps and the errors."""
import os
from pathlib import Path

import duckdb
import pytest

import duckosm

DATA = Path(__file__).parent / "data"
pytest.importorskip("geopandas")
A, B = (7.4155, 43.7285), (7.4335, 43.7468)            # Fontvieille, Larvotto


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    out = tmp_path_factory.mktemp("network") / "monaco.duckdb"
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving", "walking"],
                         output=out, progress=False)


@pytest.fixture(scope="module")
def one(tmp_path_factory):
    """A file with one mode, from Area.build."""
    area = duckosm.Area(DATA / "monaco.osm.pbf", DATA / "monaco.geojson")
    return area.build(modes=["walking"], output=tmp_path_factory.mktemp("one") / "walk.duckdb", progress=False)


def test_build_and_open_give_a_network(net):
    assert isinstance(net, duckosm.Network)
    assert net == net.path and net == str(net.path) and duckosm.open(net.path) == net
    assert os.fspath(net) == str(net) == str(net.path)
    assert net.name == "monaco.duckdb" and net.exists()          # the attributes of the Path, as before
    con = duckdb.connect(str(net), read_only=True)                 # a Network is a path for duckdb too
    con.close()
    with pytest.raises(duckosm.DuckOSMError, match="duckosm.network"):      # says how to build one
        duckosm.open(net.path.with_name("none.duckdb"))


def test_modes_and_views(net, one):
    assert net.modes == ["driving", "walking"] and one.modes == ["walking"]
    assert isinstance(net.walking, duckosm.ModeView) and net["walking"].mode == "walking"
    with pytest.raises(duckosm.DuckOSMError, match="driving, walking"):      # lists the modes of the file
        net["cycling"]
    with pytest.raises(duckosm.DuckOSMError, match=r"net\['driving'\]\.route"):   # a call for one mode shows the fix
        net.route(A, B)
    assert one.route(A, B)["edges"]                                 # one mode: that one
    assert net.walking.route(A, B) == net.route(A, B, mode="walking") == duckosm.routing.route(net.walking, A, B)


def test_data(net, one):
    s = net.summary()
    assert list(s["mode"]) == ["driving", "walking"] and (s.edges > 0).all() and (s.km > 0).all()
    every = net.edges()                                             # more modes: all, with a mode column
    assert set(every["mode"]) == {"driving", "walking"} and len(every) == s.edges.sum()
    drive = net.driving.edges()
    assert "mode" not in drive and len(drive) == s.edges[0] and str(drive.crs) == "EPSG:4326"
    assert str(drive.edge_id.dtype) == "int64" and drive.length_m.dtype == "float64"
    assert drive.edge_id.is_monotonic_increasing
    assert net.sql("SELECT 44.2::FLOAT AS x").x[0] == 44.2       # FLOAT columns as float64, through their shortest text
    main = net.driving.edges(where="highway = 'primary'")
    assert 0 < len(main) < len(drive) and set(main.highway) == {"primary"}
    assert net.driving.edges(crs=3857).crs.to_epsg() == 3857 and net.driving.edges(crs="utm").crs.to_epsg() == 32632
    assert len(one.nodes()) == one.summary().nodes[0] and "mode" not in one.nodes()
    assert "driving.edges" in net.tables() and "edge_id" in set(net.columns("driving.edges").column_name)
    with pytest.raises(duckosm.DuckOSMError, match="the tables"):
        net.columns("driving.none")
    assert len(net.boundary) == 1


def test_cards(net):
    assert "<table" in net._repr_html_() and "no fixes file" in net._repr_html_()
    assert "driving 2" in repr(net) and "walking" in repr(net.walking) and "<table" in net.walking._repr_html_()
    area = duckosm.Area(DATA / "monaco.osm.pbf", DATA / "monaco.geojson")
    assert "<table>" in area._repr_html_()


def test_plot_and_static_map(net, monkeypatch):
    pytest.importorskip("matplotlib")
    import matplotlib
    matplotlib.use("Agg")
    r = net.walking.route(A, B)
    assert net.plot() is None and net.walking.plot(routes={"walk": r}) is None
    monkeypatch.setenv("DUCKOSM_MAP_SHOW", "plot")
    counts = net.driving.edges().set_index("edge_id").length_m.rename("metres")     # your own data, keyed by edge_id
    assert net.driving.map(color_by=counts) is None
    with pytest.raises(duckosm.DuckOSMError, match="colors="):
        net.driving.map(color_by="highway")


def test_live_maps(net, tmp_path, monkeypatch):
    pytest.importorskip("roadstyle")
    monkeypatch.delenv("DUCKOSM_MAP_SHOW", raising=False)
    r = net.walking.route(A, B)
    assert hasattr(net.map(), "_repr_html_")                                   # roadstyle.duckosm.page, all modes
    assert hasattr(net.walking.map(highlight=r["edges"]), "_repr_html_")
    assert hasattr(net.driving.map(color_by="maxspeed_kmh", cmap="viridis"), "_repr_html_")
    assert hasattr(net.map("walking", routes=[r]), "_repr_html_")
    assert hasattr(net.dashboard(), "_repr_html_")
    monkeypatch.setenv("DUCKOSM_MAP_SHOW", "plot")                              # a file is the live map also then
    out = net.save_map(tmp_path / "monaco.html")
    assert out.stat().st_size > 100_000 and "<html" in out.read_text()[:3000].lower()
    link = net.driving.map(show="link", path=tmp_path / "link.html")              # the saved live map and a link to it
    assert Path(link).stat().st_size > 100_000 and "Open the live map" in link._repr_html_()
    with pytest.raises(duckosm.DuckOSMError, match="show must be"):
        net.map(show="video")


def test_map_picture(net, tmp_path):
    pytest.importorskip("roadstyle")
    pytest.importorskip("playwright")
    pic = net.map(show="picture", path=tmp_path / "monaco.png")                  # a PNG of the live roadstyle map
    assert pic._repr_png_()[:8] == b"\x89PNG\r\n\x1a\n" and Path(pic).stat().st_size > 20_000
