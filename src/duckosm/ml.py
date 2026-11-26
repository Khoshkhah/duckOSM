"""``duckosm.ml``: graphs for graph machine learning and graph libraries (docs/proposals/python-sdk.md, section 5.8).

``to_networkx``, ``to_networkx_nodes``, ``to_rustworkx``; ``to_pyg`` (PyTorch Geometric) and ``to_dgl`` (DGL), with the
edge features and ``edge_id``. DuckDB makes the index arrays; Python only makes the tensors. pip extras:
``duckosm[graph]`` (networkx, rustworkx), ``duckosm[ml]`` (torch, torch_geometric).
"""
from duckosm.errors import require
from duckosm.network import Network, resolve


def _with_connection(db, fn):
    """``fn(con)`` with a read-only connection to a Network (closed after it), or with the connection that was given."""
    if isinstance(db, Network):
        con = db._connect()
        try:
            return fn(con)
        finally:
            con.close()
    if isinstance(db, (str, bytes)) or hasattr(db, "__fspath__"):
        return _with_connection(Network(db), fn)
    return fn(db)


def to_networkx(net, mode=None, weight="time", **kwargs):
    """The turn graph of one mode as a ``networkx.DiGraph``: a node is an ``edge_id``, an arc a legal turn with its
    ``weight`` (``"time"`` in seconds or ``"length"`` in metres). Options as :func:`duckosm.to_networkx`.

    >>> g = duckosm.ml.to_networkx(net.driving)"""
    require("networkx", "duckosm.ml.to_networkx")
    from duckosm._api import to_networkx as python
    db, mode = resolve(net, mode)
    return _with_connection(db, lambda con: python(con, mode=mode, weight=weight, **kwargs))


def to_networkx_nodes(net, mode=None, **kwargs):
    """The junction graph of one mode as a ``networkx.MultiDiGraph``: a node is a junction (``x``, ``y``), an edge a road.

    >>> g = duckosm.ml.to_networkx_nodes(net.driving)"""
    require("networkx", "duckosm.ml.to_networkx_nodes")
    from duckosm._api import to_networkx_nodes as python
    db, mode = resolve(net, mode)
    return _with_connection(db, lambda con: python(con, mode=mode, **kwargs))


def to_rustworkx(net, mode=None, graph="turns", weight="time"):
    """The graph of one mode as a ``rustworkx.PyDiGraph`` (``graph="turns"`` or ``"junctions"``);
    ``g.attrs["index"]`` gives the node index of an id.

    >>> h = duckosm.ml.to_rustworkx(net.driving)"""
    require("rustworkx", "duckosm.ml.to_rustworkx")
    from duckosm import _api
    db, mode = resolve(net, mode)
    return _api.to_rustworkx(db, mode=mode, graph=graph, weight=weight)


DEFAULT_FEATURES = ("length_m", "maxspeed_kmh", "lanes")
DEFAULT_NODE_FEATURES = ("lon", "lat")
def graph_arrays(net, mode=None, graph="nodes", features=DEFAULT_FEATURES, node_features=DEFAULT_NODE_FEATURES):
    """The arrays of :func:`to_pyg` and :func:`to_dgl`, as numpy: ``src``, ``dst`` (node indices of each arc), ``x``
    (node features), ``x_names``, ``edge_attr`` (arc features), ``edge_attr_names``, ``num_nodes``, and ``edge_id`` (of
    each arc for ``graph="nodes"``, of each node for ``"turns"``), ``node_id`` (``"nodes"`` only).

    ``graph="nodes"``: a node is a junction (features ``lon``, ``lat``), an arc an edge (the ``features`` columns of
    the edges). ``graph="turns"``: a node is an edge (the ``features`` columns), an arc a legal turn of ``edge_graph``
    (feature ``cost``, the seconds of the first edge). A text column (``highway``) becomes one 0/1 column for each
    value (``highway=primary``); a boolean column becomes 0/1.

    ``node_features`` (junction graph only): the vertex features of each node. Default ``lon``, ``lat``. Also
    ``in_degree``, ``out_degree`` and ``degree`` (the edges that arrive at, leave or touch the node), and any column of
    ``<mode>.nodes``, for example ``ele`` after :func:`duckosm.analysis.add_elevation`.

    >>> a = duckosm.ml.graph_arrays(net.driving, node_features=["lon", "lat", "in_degree", "out_degree"])"""
    import numpy as np

    from duckosm import _api
    if graph not in ("nodes", "turns"):
        raise ValueError(f"graph must be 'nodes' or 'turns', not {graph!r}")
    db, mode = resolve(net, mode)
    node_features = list(node_features)
    if graph == "turns" and node_features != list(DEFAULT_NODE_FEATURES):
        raise ValueError("node_features is for the junction graph (graph='nodes'); in the turn graph each vertex is an "
                         "edge, and features= chooses its columns")
    with _api._file(db) as path:
        src, dst, x, xn, ea, en, n, edge_id, node_id = _api._rust().graph_arrays(path, mode, graph, list(features), node_features)
    out = {"src": np.array(src, dtype="int64"), "dst": np.array(dst, dtype="int64"), "num_nodes": n, "graph": graph, "mode": mode,
           "x": np.array(x, dtype="float32").reshape(n, len(xn)), "x_names": xn,
           "edge_attr": np.array(ea, dtype="float32").reshape(len(src), len(en)), "edge_attr_names": en,
           "edge_id": np.array(edge_id, dtype="int64")}
    if graph == "nodes":
        out["node_id"] = np.array(node_id, dtype="int64")
    return out

def to_pyg(net, mode=None, graph="nodes", features=DEFAULT_FEATURES, node_features=DEFAULT_NODE_FEATURES):
    """The graph of one mode as a ``torch_geometric.data.Data``: ``edge_index``, ``x``, ``edge_attr``, ``num_nodes``,
    ``edge_id`` (a tensor: of each arc for ``graph="nodes"``, of each node for ``"turns"``), ``node_id``
    (``"nodes"``), and the names of the feature columns ``x_names``, ``edge_attr_names``. See :func:`graph_arrays` for
    the two graphs and the features.

    >>> data = duckosm.ml.to_pyg(net.driving, graph="turns", features=["length_m", "maxspeed_kmh", "highway"])"""
    import warnings

    import numpy as np
    with warnings.catch_warnings():      # torch_geometric's import uses torch.jit.script, which new torch marks as old
        warnings.simplefilter("ignore", FutureWarning)
        require("torch_geometric", "duckosm.ml.to_pyg")
        from torch_geometric.data import Data
    import torch
    a = graph_arrays(net, mode=mode, graph=graph, features=features, node_features=node_features)
    data = Data(edge_index=torch.tensor(np.stack([a["src"], a["dst"]])), x=torch.tensor(a["x"]),
                edge_attr=torch.tensor(a["edge_attr"]), num_nodes=a["num_nodes"], edge_id=torch.tensor(a["edge_id"]))
    if "node_id" in a:
        data.node_id = torch.tensor(a["node_id"])
    data.x_names, data.edge_attr_names = a["x_names"], a["edge_attr_names"]
    return data


def to_dgl(net, mode=None, graph="nodes", features=DEFAULT_FEATURES, node_features=DEFAULT_NODE_FEATURES):
    """The graph of one mode as a ``dgl.DGLGraph``: ``g.ndata["x"]``, ``g.edata["edge_attr"]``, and ``edge_id`` in
    ``g.edata`` (``graph="nodes"``) or ``g.ndata`` (``"turns"``), ``node_id`` in ``g.ndata`` (``"nodes"``). See
    :func:`graph_arrays`.

    >>> g = duckosm.ml.to_dgl(net.driving)"""
    require("dgl", "duckosm.ml.to_dgl")
    import dgl
    import torch
    a = graph_arrays(net, mode=mode, graph=graph, features=features, node_features=node_features)
    g = dgl.graph((torch.tensor(a["src"]), torch.tensor(a["dst"])), num_nodes=a["num_nodes"])
    g.ndata["x"], g.edata["edge_attr"] = torch.tensor(a["x"]), torch.tensor(a["edge_attr"])
    (g.edata if graph == "nodes" else g.ndata)["edge_id"] = torch.tensor(a["edge_id"])
    if "node_id" in a:
        g.ndata["node_id"] = torch.tensor(a["node_id"])
    return g
