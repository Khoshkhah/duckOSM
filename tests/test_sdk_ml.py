"""duckosm.ml on Monaco: graph_arrays, to_pyg, to_dgl (docs/proposals/python-sdk.md section 5.8)."""
from pathlib import Path

import pytest

import duckosm

DATA = Path(__file__).parent / "data"


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving"],
                         output=tmp_path_factory.mktemp("ml") / "monaco.duckdb", progress=False)


def test_node_graph(net):
    a = duckosm.ml.graph_arrays(net.driving, features=["length_m", "highway", "oneway"])
    s = net.driving.summary()
    assert a["num_nodes"] == s.nodes[0] and len(a["src"]) == len(a["edge_id"]) == s.edges[0]
    assert a["x"].shape == (s.nodes[0], 2) and "highway=primary" in a["edge_attr_names"]
    assert a["edge_attr"].shape == (s.edges[0], len(a["edge_attr_names"]))
    e = net.sql("SELECT edge_id, source, target FROM driving.edges").set_index("edge_id")
    k = 7                                                         # an arc goes from the source to the target of its edge
    assert a["node_id"][a["src"][k]] == e.source[a["edge_id"][k]] and a["node_id"][a["dst"][k]] == e.target[a["edge_id"][k]]


def test_turn_graph(net):
    a = duckosm.ml.graph_arrays(net.driving, graph="turns")
    turns = net.sql("SELECT count(*) n FROM driving.edge_graph WHERE uses IS NULL OR uses = 'car'").n[0]
    assert a["num_nodes"] == net.driving.summary().edges[0] and len(a["src"]) == turns
    assert a["x_names"] == ["length_m", "maxspeed_kmh", "lanes"] and a["edge_attr_names"] == ["cost"]


def test_to_pyg(net):
    pytest.importorskip("torch_geometric")
    d = duckosm.ml.to_pyg(net.driving)
    assert d.num_nodes == net.driving.summary().nodes[0] and d.edge_index.shape == (2, d.edge_attr.shape[0])
    assert d.edge_id.dtype.is_floating_point is False and d.validate()
    t = duckosm.ml.to_pyg(net.driving, graph="turns")
    assert t.num_nodes == len(t.edge_id) == net.driving.summary().edges[0]


def test_to_dgl(net):
    pytest.importorskip("dgl")
    g = duckosm.ml.to_dgl(net.driving)
    assert g.num_nodes() == net.driving.summary().nodes[0] and "edge_id" in g.edata


def test_node_features_of_the_junction_graph(net):
    """node_features: the computed degrees, and the default lon, lat; not for the turn graph."""
    a = duckosm.ml.graph_arrays(net.driving, node_features=["lon", "lat", "in_degree", "out_degree", "degree"])
    assert a["x"].shape == (a["num_nodes"], 5) and a["x_names"] == ["lon", "lat", "in_degree", "out_degree", "degree"]
    assert a["x"][:, 2].sum() == a["x"][:, 3].sum() == len(a["src"])          # each arc arrives once and leaves once
    assert duckosm.ml.graph_arrays(net.driving)["x_names"] == ["lon", "lat"]
    with pytest.raises(ValueError, match="junction graph"):
        duckosm.ml.graph_arrays(net.driving, graph="turns", node_features=["degree"])
