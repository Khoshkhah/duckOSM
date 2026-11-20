"""duckosm.mobility on Monaco: snap, map matching of a trace made from a known route, join, aggregate and the edge id
map between two builds (docs/reference/sdk/mobility.md)."""
from pathlib import Path

import numpy as np
import pandas as pd
import pytest

import duckosm
from duckosm import analysis as an
from duckosm import mobility as mob

DATA = Path(__file__).parent / "data"
A, B = (7.4155, 43.7285), (7.4335, 43.7468)


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    out = tmp_path_factory.mktemp("mobility") / "monaco.duckdb"
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving", "walking"],
                         output=out, progress=False)


def _trace(net, edges, step_m=15.0, noise_m=3.0, seed=0):
    """GPS points every ``step_m`` along the edges of a route, ``noise_m`` off the line, with times. The first and the
    last point are 10 m (at most half the edge) inside the route: a point at a junction can belong to the edge before."""
    gpd = pytest.importorskip("geopandas")
    from shapely.ops import linemerge
    e = net.edges("driving", crs="utm").set_index("edge_id").loc[list(edges)]
    line = linemerge(list(e.geometry))
    rng = np.random.default_rng(seed)
    first, last = min(10, e.geometry.iloc[0].length / 2), min(10, e.geometry.iloc[-1].length / 2)
    d = np.linspace(first, line.length - last, int(line.length // step_m) + 1)
    pts = gpd.GeoSeries([line.interpolate(x) for x in d], crs=e.crs)
    pts = gpd.GeoSeries(gpd.points_from_xy(pts.x + rng.normal(0, noise_m, len(d)), pts.y + rng.normal(0, noise_m, len(d))),
                        crs=e.crs).to_crs("EPSG:4326")
    return pd.DataFrame({"x": pts.x, "y": pts.y, "time": pd.Timestamp("2026-11-20 08:00") + pd.to_timedelta(d / 10, "s")})


def test_snap_equals_nearest_edges(net):
    pts = an.sample_points(net.driving, 50, seed=3)
    s = mob.snap(net.driving, pts)
    assert list(s.edge_id) == list(an.nearest_edges(net.driving, pts.geometry.x, pts.geometry.y))
    assert s.distance_m.max() < 0.5 and s.fraction.between(0, 1).all()
    far = mob.snap(net.driving, [(7.50, 43.70)], radius_m=30)        # in the sea
    assert far.edge_id.isna().all()


def test_match_gives_back_the_route(net):
    r = net.driving.route(A, B)
    gps = _trace(net, r["edges"])
    m = mob.match(net.driving, gps.sample(frac=1, random_state=1))     # the rows in any order: time orders them
    assert list(m.edges.edge_id) == list(r["edges"])
    assert m.points.edge_id.notna().all() and m.points.edge_id.isin(r["edges"]).all()
    assert m.points.distance_m.max() < 20


def test_match_on_20_noisy_traces(net):
    """3 m GPS noise: a point near a junction at the start or the end can add the edge before or after, and a parallel
    road can take a point. Over 20 traces of each route, nearly all route edges come back, with few others."""
    for a, b in ((A, B), (B, A)):
        r = net.driving.route(a, b)["edges"]
        found = [set(mob.match(net.driving, _trace(net, r, seed=s)).edges.edge_id) for s in range(20)]
        assert np.mean([len(f & set(r)) / len(r) for f in found]) > 0.99            # recall
        assert np.mean([len(f & set(r)) / len(f) for f in found]) > 0.98            # precision


def test_two_traces_and_aggregate(net):
    r1, r2 = net.driving.route(A, B), net.driving.route(B, A)
    gps = pd.concat([_trace(net, r1["edges"]).assign(trace_id="a"), _trace(net, r2["edges"], seed=1).assign(trace_id="b")])
    m = mob.match(net.driving, gps)
    assert list(m.edges[m.edges.trace_id == "a"].edge_id) == list(r1["edges"])
    assert list(m.edges[m.edges.trace_id == "b"].edge_id) == list(r2["edges"])
    agg = mob.aggregate(net.driving, m.points, time="hour")
    assert agg["count"].sum() == len(gps) and set(agg.columns) == {"edge_id", "time", "count"}
    on = mob.join(net.driving, agg.groupby("edge_id")["count"].sum())
    assert len(on) == agg.edge_id.nunique() and "geometry" in on


def test_edge_id_map(net, tmp_path):
    assert (mob.edge_id_map(net.driving, net.driving).share == 1).all()
    # an OSM update: the dead-end street 778494662 goes, so its T junction goes, and the road it met is one edge in
    # each direction, with a new edge_id
    fixes = tmp_path / "fixes.yaml"
    fixes.write_text("overrides:\n  - osm_id: 778494662\n    remove: true\n")
    new = duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving"],
                        output=tmp_path / "new.duckdb", fixes=fixes, progress=False)
    m = mob.edge_id_map(net.driving, new.driving)
    assert set(m.new_edge_id) <= set(new.edges().edge_id)
    joined = m[m.old_edge_id != m.new_edge_id]
    assert len(joined) == 4 and joined.new_edge_id.nunique() == 2 and (joined.share == 1).all()
    assert not (net.edges("driving").set_index("edge_id").osm_id[m.old_edge_id] == 778494662).any()   # the street went
    # the other way round: each new (long) edge is split over the two old edges
    back = mob.edge_id_map(new.driving, net.driving)
    split = back[back.old_edge_id.isin(joined.new_edge_id)]
    assert len(split) == 4 and split.groupby("old_edge_id").share.sum().tolist() == [1.0, 1.0]
