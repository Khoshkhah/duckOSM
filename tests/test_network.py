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


def test_errors_show_the_home_folder_as_tilde():
    missing = Path.home() / "no-such-folder" / "none.duckdb"
    with pytest.raises(duckosm.DuckOSMError) as e:
        duckosm.open(missing)
    assert "~/no-such-folder/none.duckdb" in str(e.value) and str(Path.home()) not in str(e.value)


def test_a_reader_of_connect_does_not_stop_sql(net):
    """In DuckDB 1.5, .arrow() is a lazy reader. A reader not read to the end must not lock the file for net.sql."""
    with net.connect() as con:
        reader = con.sql("SELECT * FROM driving.edges").arrow()
        assert con.sql("SELECT current_database()").fetchone()[0] == "monaco"
    assert net.sql("SELECT count(*) AS n FROM driving.edges").n[0] > 0
    del reader


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
    cols = net.columns("driving.edges")                       # name, type and the description that the build writes
    assert list(cols.columns) == ["column_name", "column_type", "description"]
    s = net.schema("driving")                                 # one row for each table, with exact row counts
    assert list(s.columns) == ["schema", "table", "rows", "columns", "description"]
    assert int(s[s.table == "edges"].rows.iloc[0]) == int(net.sql("SELECT count(*) AS n FROM driving.edges").n[0])
    with pytest.raises(duckosm.DuckOSMError, match="no schema"):
        net.schema("nothing")
    summ = net.summarize("driving.edges")                     # DuckDB SUMMARIZE, without the geometry
    assert "edge_id" in set(summ.column_name) and "geometry" not in set(summ.column_name)
    with net.connect() as con:                                # a read-only DuckDB connection with spatial
        assert con.sql("SELECT count(*) FROM driving.edges").fetchone()[0] == len(net.driving.edges())
    with pytest.raises(duckosm.DuckOSMError, match="the tables"):
        net.columns("driving.none")
    assert len(net.boundary) == 1


def test_cards(net):
    assert "<table" in net._repr_html_() and "<td>fixes</td><td>none</td>" in net._repr_html_()
    assert "driving 2" in repr(net) and "walking" in repr(net.walking) and "<table" in net.walking._repr_html_()
    area = duckosm.Area(DATA / "monaco.osm.pbf", DATA / "monaco.geojson")
    assert "<table>" in area._repr_html_()


def test_plot_and_static_map(net, monkeypatch):
    pytest.importorskip("matplotlib")
    import matplotlib
    matplotlib.use("Agg")
    r = net.walking.route(A, B)
    assert net.plot() is None and net.walking.plot(routes={"walk": r}) is None
    assert net.plot(layers=False) is None and net.plot(layers=["water", "buildings"]) is None    # the base-map layers
    roads, _ = net._colored(None, None, {})                  # all modes: each road once, with the set of its modes
    from duckosm.network import MODES_COLORS
    assert roads.edge_id.is_unique and set(roads.modes) <= set(MODES_COLORS)
    with pytest.raises(duckosm.DuckOSMError, match="plot draws the layers"):
        net.plot(layers=["parking"])
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
    import pandas as pd
    assert hasattr(net.dashboard("walking", color_by=pd.Series({int(r["edges"][0]): 1.0}), cmap="viridis"), "_repr_html_")   # your data
    assert hasattr(net.map(layers=False), "_repr_html_") and hasattr(net.map(layers=["water"]), "_repr_html_")
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


def test_add_multimodal_needs_no_new_build(net, one, tmp_path):
    import shutil
    copy = duckosm.open(shutil.copy(net.path, tmp_path / "mm_copy.duckdb"))
    assert not copy.has_elevation() and not copy.has_fixes() and copy.has_raw()
    assert all(isinstance(f(), bool) for f in (copy.has_features, copy.has_levels, copy.has_bus_routes, copy.has_admin))
    assert not copy.has_features("no_such_layer")
    assert not copy.has_multimodal() and copy.has_mode("driving", "walking") and not copy.has_mode("cycling")
    assert copy.add_multimodal(transfer_s=90) is copy and copy.has_multimodal()
    n = copy.sql("SELECT (SELECT count(*) FROM mm.edges) AS e, (SELECT count(*) FROM mm.transfers) AS t, "
                 "(SELECT min(cost_s) FROM mm.transfers) AS c")
    assert n.e[0] > 0 and n.t[0] > 0 and n.c[0] == 90
    with pytest.raises(duckosm.DuckOSMError, match="needs walking and one more mode"):
        one.add_multimodal()
