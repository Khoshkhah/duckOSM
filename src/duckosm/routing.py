"""``duckosm.routing``: routing, accessibility and logistics (docs/proposals/python-sdk.md, section 5.3).

Today: ``route``, ``route_points``, ``route_multimodal``, ``route_multimodal_points`` and ``directions``.
Later (PR 3 of the design): ``routes``, ``k_shortest_paths``, ``matrix``, ``isochrones``, ``accessibility`` and a
``Route`` object. Each function takes a ``Network``, a ``ModeView`` (``net.walking``), a path or a DuckDB connection.
"""
from duckosm._api import route_multimodal  # noqa: F401 -- the names of this module
# the old names of duckosm.routing (the Python code): they stay. Router is the Python Router (it has .graph); the Rust
# one is duckosm.Router.
from duckosm._routing_py import Router, node_branches, node_out_degree, to_networkx, to_networkx_nodes, write_graph  # noqa: F401
from duckosm.network import resolve


def _is_point(x):
    return isinstance(x, (tuple, list)) and len(x) == 2 and all(isinstance(v, float) for v in x)


def route(net, a, b, mode=None, weight="time", graph=None):
    """The fastest route (``weight="length"``: the shortest) from ``a`` to ``b``: two edge ids, or two ``(lon, lat)``
    points (then :func:`route_points`). Returns ``{edges, time_s, length_m, path}``, or None when no route connects them.
    ``graph``: a networkx graph of :func:`duckosm.ml.to_networkx` (the Python code).

    >>> duckosm.routing.route(net.driving, (7.4155, 43.7285), (7.4335, 43.7468))
    >>> duckosm.routing.route("monaco.duckdb", from_edge, to_edge)"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    if _is_point(a) and _is_point(b):
        return _api.route_points(db, a, b, mode=mode, weight=weight)
    return _api.route(db, a, b, mode=mode, weight=weight, graph=graph)


def route_points(net, a, b, mode=None, weight="time", radius_m=50.0, access_kmh=4.5):
    """The fastest route between two ``(lon, lat)`` points: each joins the network at its nearest road within
    ``radius_m``. Returns ``{time_s, length_m, edges, start, end, path}``, or None.

    >>> duckosm.routing.route_points(net.walking, (7.4155, 43.7285), (7.4335, 43.7468))"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    return _api.route_points(db, a, b, mode=mode, weight=weight, radius_m=radius_m, access_kmh=access_kmh)


def route_multimodal_points(net, a, b, **kwargs):
    """A route across the modes between two ``(lon, lat)`` points (it can walk to a car and drive). Needs the multimodal
    graph ``mm`` (a build with ``multimodal: {enabled: true}``). Returns the legs of each mode.

    >>> duckosm.routing.route_multimodal_points(net, (7.4155, 43.7285), (7.4335, 43.7468))"""
    from duckosm import _api
    db = net.network if hasattr(net, "network") else net
    return _api.route_multimodal_points(db, a, b, **kwargs)


def directions(net, route, mode=None):
    """The turn-by-turn steps of a route (``text``, ``length_m``, ``type`` ...).

    >>> for step in duckosm.routing.directions(net.driving, r): print(step["text"])"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    return _api.directions(db, route, mode=mode)
