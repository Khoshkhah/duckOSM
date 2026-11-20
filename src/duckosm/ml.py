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
    from duckosm._routing_py import to_networkx as python
    db, mode = resolve(net, mode)
    return _with_connection(db, lambda con: python(con, mode=mode, weight=weight, **kwargs))


def to_networkx_nodes(net, mode=None, **kwargs):
    """The junction graph of one mode as a ``networkx.MultiDiGraph``: a node is a junction (``x``, ``y``), an edge a road.

    >>> g = duckosm.ml.to_networkx_nodes(net.driving)"""
    require("networkx", "duckosm.ml.to_networkx_nodes")
    from duckosm._routing_py import to_networkx_nodes as python
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


def graph_arrays(net, mode=None, graph="nodes", features=DEFAULT_FEATURES):
    """The arrays of :func:`to_pyg` and :func:`to_dgl`, as numpy: ``src``, ``dst`` (node indices of each arc), ``x``
    (node features), ``x_names``, ``edge_attr`` (arc features), ``edge_attr_names``, ``num_nodes``, and ``edge_id`` (of
    each arc for ``graph="nodes"``, of each node for ``"turns"``), ``node_id`` (``"nodes"`` only).

    ``graph="nodes"``: a node is a junction (features ``lon``, ``lat``), an arc an edge (the ``features`` columns of
    the edges). ``graph="turns"``: a node is an edge (the ``features`` columns), an arc a legal turn of ``edge_graph``
    (feature ``cost``, the seconds of the first edge). A text column (``highway``) becomes one 0/1 column for each
    value (``highway=primary``); a boolean column becomes 0/1."""
    import numpy as np
    import pandas as pd
    if graph not in ("nodes", "turns"):
        raise ValueError(f"graph must be 'nodes' or 'turns', not {graph!r}")
    db, mode = resolve(net, mode)
    cols = "".join(f', "{c}"' for c in features)

    def read(con):
        e = f'"{mode}".edges'
        if graph == "nodes":
            nodes = con.execute(f"""SELECT node_id, ST_X(geom) AS lon, ST_Y(geom) AS lat,
                                           row_number() OVER (ORDER BY node_id) - 1 AS i
                                    FROM "{mode}".nodes ORDER BY node_id""").df()
            arcs = con.execute(f"""WITH n AS (SELECT node_id, row_number() OVER (ORDER BY node_id) - 1 AS i FROM "{mode}".nodes)
                                   SELECT s.i AS src, t.i AS dst, e.edge_id{cols} FROM {e} e
                                   JOIN n s ON s.node_id = e.source JOIN n t ON t.node_id = e.target
                                   ORDER BY e.edge_id""").df()
            return nodes, arcs
        nodes = con.execute(f"SELECT edge_id{cols} FROM {e} ORDER BY edge_id").df()
        uses = {"driving": "car", "walking": "walk", "cycling": "bike"}.get(mode, mode)
        arcs = con.execute(f"""WITH n AS (SELECT edge_id, row_number() OVER (ORDER BY edge_id) - 1 AS i FROM {e})
                               SELECT a.i AS src, b.i AS dst, g.cost FROM "{mode}".edge_graph g
                               JOIN n a ON a.edge_id = g.from_edge JOIN n b ON b.edge_id = g.to_edge
                               WHERE g.uses IS NULL OR g.uses = '{uses}' ORDER BY a.i, b.i""").df()
        return nodes, arcs
    nodes, arcs = _with_connection(db, read)

    def matrix(df):
        df = df.copy()
        for c in df.columns:
            if df[c].dtype == bool:
                df[c] = df[c].astype("float32")
        text = [c for c in df.columns if not pd.api.types.is_numeric_dtype(df[c])]
        if text:
            df = pd.get_dummies(df, columns=text, prefix_sep="=", dtype="float32")
        return df.fillna(0).to_numpy(dtype="float32"), list(df.columns)

    out = {"src": arcs.src.to_numpy("int64"), "dst": arcs.dst.to_numpy("int64"), "num_nodes": len(nodes),
           "graph": graph, "mode": mode}
    if graph == "nodes":
        out["x"], out["x_names"] = matrix(nodes[["lon", "lat"]])
        out["edge_attr"], out["edge_attr_names"] = matrix(arcs[list(features)])
        out["edge_id"], out["node_id"] = arcs.edge_id.to_numpy("int64"), nodes.node_id.to_numpy("int64")
    else:
        out["x"], out["x_names"] = matrix(nodes[list(features)])
        out["edge_attr"], out["edge_attr_names"] = matrix(arcs[["cost"]])
        out["edge_id"] = nodes.edge_id.to_numpy("int64")
    return out


def to_pyg(net, mode=None, graph="nodes", features=DEFAULT_FEATURES):
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
    a = graph_arrays(net, mode=mode, graph=graph, features=features)
    data = Data(edge_index=torch.tensor(np.stack([a["src"], a["dst"]])), x=torch.tensor(a["x"]),
                edge_attr=torch.tensor(a["edge_attr"]), num_nodes=a["num_nodes"], edge_id=torch.tensor(a["edge_id"]))
    if "node_id" in a:
        data.node_id = torch.tensor(a["node_id"])
    data.x_names, data.edge_attr_names = a["x_names"], a["edge_attr_names"]
    return data


def to_dgl(net, mode=None, graph="nodes", features=DEFAULT_FEATURES):
    """The graph of one mode as a ``dgl.DGLGraph``: ``g.ndata["x"]``, ``g.edata["edge_attr"]``, and ``edge_id`` in
    ``g.edata`` (``graph="nodes"``) or ``g.ndata`` (``"turns"``), ``node_id`` in ``g.ndata`` (``"nodes"``). See
    :func:`graph_arrays`.

    >>> g = duckosm.ml.to_dgl(net.driving)"""
    require("dgl", "duckosm.ml.to_dgl")
    import dgl
    import torch
    a = graph_arrays(net, mode=mode, graph=graph, features=features)
    g = dgl.graph((torch.tensor(a["src"]), torch.tensor(a["dst"])), num_nodes=a["num_nodes"])
    g.ndata["x"], g.edata["edge_attr"] = torch.tensor(a["x"]), torch.tensor(a["edge_attr"])
    (g.edata if graph == "nodes" else g.ndata)["edge_id"] = torch.tensor(a["edge_id"])
    if "node_id" in a:
        g.ndata["node_id"] = torch.tensor(a["node_id"])
    return g
