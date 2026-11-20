"""duckosm.routing on Monaco (docs/reference/sdk/routing.md): each new call gives the result of ``route`` on the same
pairs. Matrix cells are route times, the first of the k shortest paths is the route, the isochrone reach is the matrix
within the time."""
import math
from pathlib import Path

import numpy as np
import pytest

import duckosm
from duckosm import routing

DATA = Path(__file__).parent / "data"
A, B = (7.4155, 43.7285), (7.4335, 43.7468)


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    out = tmp_path_factory.mktemp("routing") / "monaco.duckdb"
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving", "walking"],
                         output=out, progress=False)


@pytest.fixture(scope="module")
def ids(net):
    """20 driving edges spread over the network."""
    all_ids = net.sql(f"SELECT DISTINCT from_edge FROM {routing.routed_graph(net.driving)} ORDER BY 1").from_edge.astype(int).tolist()
    return all_ids[:: len(all_ids) // 20][:20]


def _route_time(net, a, b, weight="time"):
    r = routing.route(net.driving, a, b, weight=weight)
    return math.nan if r is None else r[{"time": "time_s", "length": "length_m"}[weight]]


def test_route_object(net, ids):
    r = net.driving.route(A, B)
    assert isinstance(r, routing.Route) and r == duckosm.route_points(net, A, B)      # still the dict of route_points
    e = routing.route(net.driving, ids[0], ids[5])
    assert e["edges"] == duckosm.route(net, ids[0], ids[5])["edges"]
    gdf = e.edges
    assert gdf.edge_id.tolist() == e.edge_ids and gdf.seq.tolist() == list(range(len(gdf)))
    assert e.length_m == pytest.approx(gdf.length_m.sum()) and e.time_s == pytest.approx(gdf.cost_s.sum())
    assert e.geometry.length > 0 and e.directions() == duckosm.directions(net, e)
    assert "Route" in repr(e) and "<table>" in e._repr_html_()


def test_route_place_name(net):
    r = net.driving.route("Boulevard Albert 1er", "Avenue Princesse Grace")
    assert isinstance(r, routing.Route) and r.time_s > 0


def test_routes_equal_route(net, ids):
    pairs = list(zip(ids[:10], ids[10:]))
    many = net.driving.routes([a for a, _ in pairs], [b for _, b in pairs])
    for (a, b), r in zip(pairs, many):
        one = routing.route(net.driving, a, b)
        assert (r is None) == (one is None)
        if r is not None:
            assert r["edges"] == one["edges"] and r.time_s == pytest.approx(one.time_s)


def test_k_shortest_paths(net, ids):
    a, b = ids[2], ids[15]
    ks = net.driving.k_shortest_paths(a, b, k=4)
    assert ks[0]["edges"] == routing.route(net.driving, a, b)["edges"]
    assert 1 < len(ks) <= 4
    assert len({tuple(r["edges"]) for r in ks}) == len(ks)                     # each path once
    assert all(len(set(r["edges"])) == len(r["edges"]) for r in ks)          # no edge twice
    graph = [round(r.time_s - r.edges.cost_s.iloc[-1], 6) for r in ks]       # the graph cost: without the last edge
    assert graph == sorted(graph)


@pytest.mark.parametrize("weight", ["time", "length"])
def test_matrix_equals_route(net, ids, weight):
    o, d = ids[:8], ids[8:16]
    m = net.driving.matrix(o, d, weight=weight)
    assert m.shape == (8, 8) and list(m.index) == o and list(m.columns) == d
    want = np.array([[_route_time(net, a, b, weight) for b in d] for a in o])
    np.testing.assert_allclose(m.to_numpy(), want, rtol=1e-9)
    sq = routing.matrix(net.driving, o)                                       # destinations=None: the origins
    assert sq.shape == (8, 8) and sq.iloc[0, 0] == pytest.approx(_route_time(net, o[0], o[0]))


def test_matrix_points_and_limit(net, ids):
    m = net.driving.matrix([A, B], ids[:3], max_cost=120)
    assert list(m.index) == [0, 1] and (m.fillna(0) <= 120).all().all()


def test_matrix_edge_not_in_graph(net):
    out = net.sql(f"SELECT edge_id FROM walking.edges WHERE edge_id NOT IN (SELECT from_edge FROM "
                  f"{routing.routed_graph(net.walking)}) AND edge_id NOT IN (SELECT to_edge FROM "
                  f"{routing.routed_graph(net.walking)}) LIMIT 1").edge_id.astype(int).tolist()
    if not out:
        pytest.skip("each walking edge is in the graph")
    w = net.sql(f"SELECT DISTINCT from_edge FROM {routing.routed_graph(net.walking)} ORDER BY 1 LIMIT 2").from_edge.astype(int).tolist()
    m = net.walking.matrix(out + w, w + out)
    assert m.iloc[0].isna().all() and m.iloc[:, -1].isna().all() and m.iloc[1:, :2].notna().any().any()


def test_isochrones_equal_matrix(net, ids):
    g = routing.routed_graph(net.driving)                     # all graph nodes: a dead-end edge is only a to_edge
    every = net.sql(f"SELECT from_edge AS e FROM {g} UNION SELECT to_edge FROM {g} ORDER BY 1").e.astype(int).tolist()
    iso = net.driving.isochrones(ids[:2], minutes=[1, 2])
    assert len(iso) == 4 and iso.crs.to_epsg() == 4326 and iso.geometry.is_valid.all()
    m = net.driving.matrix(ids[:2], every)
    for row in iso.itertuples():
        assert row.n_edges == int((m.loc[row.origin] <= row.minutes * 60).sum())
    small, big = iso[iso.origin == ids[0]].geometry
    assert big.area >= small.area


def test_accessibility(net, ids):
    o, t = ids[:6], ids[6:]
    acc = net.driving.accessibility(o, t, minutes=[1, 3])
    m = net.driving.matrix(o, t)
    assert acc["n_3min"].tolist() == (m <= 180).sum(axis=1).tolist()
    assert acc["n_1min"].tolist() == (m <= 60).sum(axis=1).tolist()
    for origin, nearest, nearest_s in zip(acc.index, acc.nearest, acc.nearest_s):
        within = m.loc[origin][m.loc[origin] <= 180]
        if within.empty:
            assert nearest is None
        else:
            assert nearest_s == pytest.approx(within.min()) and m.loc[origin, nearest] == pytest.approx(within.min())
