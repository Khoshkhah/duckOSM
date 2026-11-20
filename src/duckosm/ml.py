"""``duckosm.ml``: graphs for graph machine learning and graph libraries (docs/proposals/python-sdk.md, section 5.8).

Today: ``to_networkx``, ``to_networkx_nodes``, ``to_rustworkx``. Later: ``to_pyg``, ``to_dgl``. pip extra:
``duckosm[graph]`` (networkx, rustworkx).
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
