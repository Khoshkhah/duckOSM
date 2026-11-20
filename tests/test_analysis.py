"""duckosm.analysis on Monaco: the numbers equal OSMnx's on the graph of to_osmnx (docs/reference/sdk/analysis.md)."""
import math
import shutil
from pathlib import Path

import duckdb
import numpy as np
import pytest

import duckosm
from duckosm import analysis as an

DATA = Path(__file__).parent / "data"


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    out = tmp_path_factory.mktemp("analysis") / "monaco.duckdb"
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving", "walking"],
                         output=out, progress=False)


@pytest.fixture(scope="module")
def ox():
    return pytest.importorskip("osmnx")


@pytest.fixture(scope="module")
def G(net, ox):
    return an.to_osmnx(net.driving)


def _same(a, b, key):
    if isinstance(b, dict):
        assert a.keys() == b.keys(), key
        for k in b:
            _same(a[k], b[k], f"{key}[{k}]")
    else:
        assert a == pytest.approx(b, rel=1e-9, abs=1e-9), key


def test_to_osmnx_is_an_osmnx_graph(net, ox, G):
    if hasattr(ox.convert, "validate_graph"):          # OSMnx 2.1
        ox.convert.validate_graph(G)
    e = net.edges("driving")
    assert G.number_of_edges() == len(e) and G.number_of_nodes() == len(net.nodes("driving"))
    assert sorted(d["edge_id"] for *_, d in G.edges(data=True)) == sorted(e.edge_id)


@pytest.mark.parametrize("mode", ["driving", "walking"])
def test_stats_equal_osmnx(net, ox, mode):
    G = an.to_osmnx(net[mode])
    area = 2.0e6
    s = an.stats(net[mode], area_m2=area)
    G.graph["simplified"] = False                      # to_osmnx marks it simplified: each edge has its line
    for view, graph in (("duckosm", G), ("osmnx", ox.simplify_graph(G))):
        want = ox.stats.basic_stats(graph, area=area)
        got = {k: v for k, v in s[view].items() if not (isinstance(v, float) and math.isnan(v))}
        assert got.keys() == want.keys(), view
        for k in want:
            _same(got[k], want[k], f"{mode} {view} {k}")
    # the OSMnx view drops the 2-neighbour nodes: fewer nodes and intersections
    assert s["osmnx"]["n"] < s["duckosm"]["n"]
    assert s["osmnx"]["intersection_count"] < s["duckosm"]["intersection_count"]


def test_stats_without_area_and_method(net):
    s = net.driving.stats()
    assert "intersection_density_km" not in s.index        # no area: no densities, as in OSMnx
    assert s.equals(an.stats(net.driving))


def test_streets_and_bearings_equal_osmnx(net, ox, G):
    spn = an.streets_per_node(net.driving)
    assert spn.to_dict() == ox.stats.count_streets_per_node(G)
    b = an.bearings(net.driving)
    Gb = ox.bearing.add_edge_bearings(G.copy())
    want = {d["edge_id"]: d.get("bearing", math.nan) for *_, d in Gb.edges(data=True)}
    assert b.to_numpy() == pytest.approx(np.array([want[i] for i in b.index]), abs=1e-9, nan_ok=True)
    Gu = ox.bearing.add_edge_bearings(ox.convert.to_undirected(G))
    for weight in (None, "length"):
        assert an.orientation_entropy(net.driving, weight=weight) == \
            pytest.approx(ox.bearing.orientation_entropy(Gu, weight=weight), rel=1e-9)


def test_nearest_equal_osmnx(net, ox, G):
    gpd = pytest.importorskip("geopandas")
    rng = np.random.default_rng(7)
    w, s, e, n = net.edges("driving").total_bounds
    xs, ys = rng.uniform(w, e, 100), rng.uniform(s, n, 100)
    Gp = ox.project_graph(G)
    P = gpd.GeoSeries(gpd.points_from_xy(xs, ys), crs=4326).to_crs(Gp.graph["crs"])
    # nodes: the same node, or one at the same distance (within 1 cm: the flat map of Rust and UTM differ)
    ids, dist = an.nearest_nodes(net.driving, xs, ys, return_dist=True)
    want = ox.distance.nearest_nodes(Gp, P.x, P.y)
    xy = {k: (d["x"], d["y"]) for k, d in Gp.nodes(data=True)}
    for p, a, b in zip(P, ids, want):
        assert a == b or abs(p.distance(gpd.points_from_xy(*zip(xy[a]))[0]) - p.distance(gpd.points_from_xy(*zip(xy[b]))[0])) < 0.01
    # edges: the same line (a two-way road has two rows on one line), at the same distance
    eids, edist = an.nearest_edges(net.driving, xs, ys, return_dist=True)
    lines = net.edges("driving", crs=Gp.graph["crs"]).set_index("edge_id").geometry
    for p, a, (u, v, k), d in zip(P, eids, ox.distance.nearest_edges(Gp, P.x, P.y), edist):
        b = Gp.edges[u, v, k]["edge_id"]
        assert a == b or abs(p.distance(lines[a]) - p.distance(lines[b])) < 0.01
        assert d == pytest.approx(p.distance(lines[a]), abs=0.5)
    assert isinstance(an.nearest_edges(net.driving, float(xs[0]), float(ys[0])), int)


def test_components_and_largest(net, ox, G, tmp_path):
    nx = pytest.importorskip("networkx")
    c = an.components(net.driving)
    big = max(nx.strongly_connected_components(G), key=len)
    assert set(c.index[c == 0]) == big
    assert c.nunique() == nx.number_strongly_connected_components(G)
    assert an.components(net.driving, strong=False).nunique() == nx.number_weakly_connected_components(G)
    sub = an.largest_component(net.driving, out=tmp_path / "big.duckdb")
    assert len(sub.nodes("driving")) == len(big)
    assert set(an.components(sub.driving)) == {0}
    assert len(sub.edges("walking")) == len(net.edges("walking"))


def test_clip(net, tmp_path):
    small = an.clip(net, point=(7.4280, 43.7393), dist_m=300, out=tmp_path / "clip.duckdb")
    assert 0 < len(small.edges("driving")) < len(net.edges("driving"))
    box = an.clip(net, bbox=(7.42, 43.73, 7.43, 43.74), out=tmp_path / "box.duckdb")
    w, s, e, n = box.edges("driving").total_bounds
    assert w < 7.43 and e > 7.42 and s < 43.74 and n > 43.73


def test_sample_points(net):
    pts = an.sample_points(net.walking, 200, seed=1)
    assert len(pts) == 200 and pts.crs.to_epsg() == 4326
    lines = net.edges("walking", crs="utm").set_index("edge_id").geometry
    p = pts.to_crs(lines.crs)
    assert max(pt.distance(lines[e]) for pt, e in zip(p.geometry, pts.edge_id)) < 0.01
    assert an.sample_points(net.walking, 5, seed=1).equals(an.sample_points(net.walking, 5, seed=1))


def test_grades(net, tmp_path):
    with pytest.raises(duckosm.DuckOSMError, match="add_elevation"):
        an.grades(net.driving)
    db = tmp_path / "z.duckdb"
    shutil.copyfile(net.path, db)
    con = duckdb.connect(str(db))
    con.load_extension("spatial")
    con.execute("ALTER TABLE driving.edges ADD COLUMN z_from DOUBLE; ALTER TABLE driving.edges ADD COLUMN z_to DOUBLE;"
                "UPDATE driving.edges SET z_from = 10, z_to = 10 + length_m::DOUBLE * 0.05")
    con.close()
    g = an.grades(duckosm.open(db).driving).dropna()          # NaN: an edge of length 0
    assert len(g) > 0 and g.to_numpy() == pytest.approx(0.05, abs=1e-6)


def test_network_is_callable_after_a_module_import():
    import importlib
    importlib.import_module("duckosm.network")        # the submodule shadows the function duckosm.network
    assert callable(duckosm.network)
