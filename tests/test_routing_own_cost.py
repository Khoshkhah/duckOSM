"""Your own cost in duckosm.routing on Monaco: a column or a Series as ``weight``, and ``missing="time" | "skip"``."""
import math
from pathlib import Path

import numpy as np
import pandas as pd
import pytest

import duckosm
from duckosm import routing
from duckosm.errors import DuckOSMError

DATA = Path(__file__).parent / "data"
A, B = (7.4155, 43.7285), (7.4335, 43.7468)


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    out = tmp_path_factory.mktemp("own_cost") / "monaco.duckdb"
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving", "walking"],
                         output=out, progress=False)


@pytest.fixture(scope="module")
def ids(net):
    every = net.sql(f"SELECT DISTINCT from_edge FROM {routing.routed_graph(net.driving)} ORDER BY 1").from_edge.astype(int).tolist()
    return every[:: len(every) // 20][:20]


@pytest.fixture(scope="module")
def time_s(net):
    """The time of each driving edge, as a Series keyed by edge_id."""
    df = net.sql("SELECT edge_id, cost_s::DOUBLE AS cost_s FROM driving.edges")
    return pd.Series(df.cost_s.to_numpy(), index=df.edge_id.astype(int), name="cost_s")


def test_column_equals_length(net, ids):
    for a, b in zip(ids[:5], ids[10:15]):
        want, got = routing.route(net.driving, a, b, weight="length"), routing.route(net.driving, a, b, weight="length_m")
        assert (want is None) == (got is None)
        if want is not None:
            assert got["edges"] == want["edges"] and got.weight == "length_m" and got.cost == pytest.approx(want.length_m)


def test_time_and_length_keep_their_fields(net, ids):
    r = routing.route(net.driving, ids[0], ids[5])
    assert "cost" not in r and "weight" not in r and r.cost == r.time_s and r.weight == "time"
    assert routing.route(net.driving, ids[0], ids[5], weight="length").cost == routing.route(
        net.driving, ids[0], ids[5], weight="length").length_m


def test_toll_changes_the_route(net, ids, time_s):
    """A large toll on an edge in the middle of the fastest route: the cheapest route goes around it."""
    found = 0
    for a, b in zip(ids[:10], ids[10:]):
        fast = routing.route(net.driving, a, b)
        if fast is None or len(fast.edge_ids) < 5:
            continue
        toll_edge = fast.edge_ids[len(fast.edge_ids) // 2]
        toll = time_s.copy()
        toll[toll_edge] += 1e6
        cheap = routing.route(net.driving, a, b, weight=toll)
        assert cheap is not None and cheap.weight == "cost_s"
        if toll_edge not in cheap.edge_ids:
            found += 1
            assert cheap.cost == pytest.approx(sum(time_s[e] for e in cheap.edge_ids))
            assert cheap.cost >= fast.time_s - 1e-6
    assert found > 0


def test_dict_and_skip(net, ids, time_s):
    a, b = ids[1], ids[12]
    fast = routing.route(net.driving, a, b)
    only = {e: time_s[e] for e in fast.edge_ids}
    r = routing.route(net.driving, a, b, weight=only, missing="skip")          # only the edges of the route
    assert r["edges"] == fast["edges"] and r.weight == "own" and r.cost == pytest.approx(fast.time_s)
    middle = fast.edge_ids[len(fast.edge_ids) // 2]
    del only[middle]
    assert routing.route(net.driving, a, b, weight=only, missing="skip") is None   # the one way through is gone
    r = routing.route(net.driving, a, b, weight=only)                          # missing="time": the edge costs its time
    assert r["edges"] == fast["edges"]


def test_same_costs_same_results(net, ids, time_s):
    """A Series of the times gives the matrix, the routes and the k paths of weight="time"."""
    o, d = ids[:6], ids[6:12]
    np.testing.assert_allclose(net.driving.matrix(o, d, weight=time_s).to_numpy(), net.driving.matrix(o, d).to_numpy(),
                               rtol=1e-9)
    many = net.driving.routes(o, d, weight=time_s)
    for a, b, r in zip(o, d, many):
        one = routing.route(net.driving, a, b)
        assert (r is None) == (one is None) and (r is None or r.cost == pytest.approx(one.time_s))
    ks = net.driving.k_shortest_paths(ids[2], ids[15], k=3, weight=time_s)
    assert [r["edges"] for r in ks] == [r["edges"] for r in net.driving.k_shortest_paths(ids[2], ids[15], k=3)]
    acc = net.driving.accessibility(o, d, minutes=[1, 3], weight=time_s)
    assert acc.equals(net.driving.accessibility(o, d, minutes=[1, 3]))
    iso = net.driving.isochrones(ids[:1], minutes=[1, 2], weight=time_s)
    assert iso.n_edges.tolist() == net.driving.isochrones(ids[:1], minutes=[1, 2]).n_edges.tolist()


def test_route_points_own(net, time_s):
    r = net.driving.route(A, B, weight=time_s * 2)
    fast = net.driving.route(A, B)
    assert r.weight == "cost_s" and r["start"]["edge_id"] is not None
    assert r.cost == pytest.approx(2 * sum(p["cost_s"] for p in r["path"]))  # the access legs cost 0
    assert r.cost <= 2 * (fast.time_s - fast["start"]["access_s"] - fast["end"]["access_s"]) + 1e-6


def test_bad_weights(net, ids):
    with pytest.raises(Exception, match="not 'time', 'length' or a column"):
        routing.route(net.driving, ids[0], ids[5], weight="no_such_column")
    with pytest.raises(Exception, match="not numeric"):
        routing.route(net.driving, ids[0], ids[5], weight="highway")
    with pytest.raises(DuckOSMError, match="missing"):
        routing.route(net.driving, ids[0], ids[5], weight={ids[0]: 1.0}, missing="drop")
    with pytest.raises(Exception, match="0 or more"):
        routing.route(net.driving, ids[0], ids[5], weight={ids[0]: -1.0})
    assert math.isnan(routing.matrix(net.driving, [ids[0]], [ids[5]], weight={}, missing="skip").iloc[0, 0])


def test_add_multimodal_node_costs(net):
    """Network.add_multimodal(node_costs=..., only_listed_nodes=...): the cost of a change at each junction (the last
    test: it changes mm.* of the file)."""
    net.add_multimodal()
    node = int(net.sql("SELECT min(node_id) AS n FROM mm.transfers").n[0])
    net.add_multimodal(node_costs={node: 7})                                    # a dict: each change at the junction
    rows = net.sql(f"SELECT DISTINCT cost_s FROM mm.transfers WHERE node_id = {node}").cost_s.tolist()
    assert rows == [7.0] and net.sql("SELECT count(*) AS n FROM mm.transfers WHERE cost_s = 60").n[0] > 0
    df = pd.DataFrame({"node_id": [node], "cost_s": [9], "from_mode": ["walking"], "to_mode": ["driving"]})
    net.add_multimodal(node_costs=df, only_listed_nodes=True)
    assert net.sql("SELECT node_id, from_mode, to_mode, cost_s FROM mm.transfers").values.tolist() == [
        [node, "walking", "driving", 9.0]]
    with pytest.raises(DuckOSMError, match="needs node_costs"):
        net.add_multimodal(only_listed_nodes=True)
