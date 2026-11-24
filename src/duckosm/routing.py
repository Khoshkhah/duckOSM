"""``duckosm.routing``: routing, accessibility and logistics (docs/proposals/python-sdk.md, section 5.3).

Each function takes a ``Network``, a ``ModeView`` (``net.walking``), a path or a DuckDB connection. A place (``a``,
``b``, ``origins`` ...) is an edge id, a ``(lon, lat)`` point or a place name. The searches run in Rust on the edge
graph of ``route`` (the same turn rules); the calls for many places run in parallel.

The cost of a route (``weight``): ``"time"`` (``cost_s``, the default), ``"length"`` (``length_m``), the name of a
numeric column of ``<mode>.edges`` (for example a column that you added), or your own cost of each edge: a pandas Series
(or a dict) keyed by ``edge_id`` (tolls, CO2, danger, noise). Your own cost replaces the time of the edge (the turn costs
are not added). ``missing``: an edge with no value costs its time (``"time"``, the default) or is not used (``"skip"``).
"""
from __future__ import annotations

import json
import math

from duckosm._api import route_multimodal  # noqa: F401 -- the names of this module
# the old names of duckosm.routing: they stay, on the Rust code
from duckosm._api import Router, to_networkx, to_networkx_nodes, write_graph  # noqa: F401
from duckosm.errors import DuckOSMError, require
from duckosm.network import Network, resolve

WEIGHTS = {"time": "cost_s", "length": "length_m"}
MISSING = ("time", "skip")


def _weight(weight, missing="time"):
    """``(name, costs)`` of a weight: ``costs`` is the list of ``(edge_id, cost)`` of a Series or a dict (no NaN), else
    None (``"time"``, ``"length"`` or a column, which the Rust router reads)."""
    if missing not in MISSING:
        raise DuckOSMError(f"missing must be 'time' or 'skip', not {missing!r}")
    if isinstance(weight, str):
        return weight, None
    if hasattr(weight, "items"):
        costs = [(int(e), float(c)) for e, c in weight.items() if c is not None and c == c]
        return str(getattr(weight, "name", None) or "own"), costs
    raise DuckOSMError(f"weight must be 'time', 'length', a column name, or a Series or dict keyed by edge_id, not {weight!r}")


def _is_own(weight):
    return not (isinstance(weight, str) and weight in WEIGHTS)


# ---- the Route object ------------------------------------------------------------------------------------------------

class Route(dict):
    """One route. It is also the dict of the old ``route`` (``r["edges"]`` is the list of edge ids), so the old code
    and ``net.map(routes=r)`` still work.

    - ``.edge_ids``: the edge ids, in the order of the route.
    - ``.edges``: a GeoDataFrame of the edges, in the order of the route (``seq``, ``edge_id``, the edge columns).
    - ``.cost``, ``.weight``: the cost that the route makes smallest, and its name (``"time"``, ``"length"``, a column or
      the name of your Series; ``"own"`` for a dict). With ``"time"`` the cost is ``.time_s``, with ``"length"``
      ``.length_m``; with your own cost it is the sum of your costs of the edges (the access legs cost 0).
    - ``.length_m``, ``.time_s``: the sums of ``length_m`` and ``cost_s`` of the edges. A route between two points
      also includes the access legs: ``length_m`` includes ``start.access_m`` and ``end.access_m`` (the distance from
      each point to its road), ``time_s`` includes ``access_s``, and only the used parts of the first and the last
      edge count.
    - ``.geometry``: one shapely line (a MultiLineString where the edges do not join).
    - ``.directions()``: the turn-by-turn steps. ``.map()``: the route on the roadstyle map of the network.

    >>> r = duckosm.routing.route(net.driving, (7.4155, 43.7285), (7.4335, 43.7468))
    >>> r.time_s, r.length_m, r.map()"""

    def __init__(self, data: dict, db, mode: str, weight: str = "time"):
        super().__init__(data)
        self._db, self._mode, self._weight = db, mode, weight

    @property
    def weight(self) -> str:
        return self.get("weight", self._weight)

    @property
    def cost(self) -> float:
        if "cost" in self:
            return math.nan if self["cost"] is None else float(self["cost"])
        return self.length_m if self.weight == "length" else self.time_s

    @property
    def edge_ids(self) -> list[int]:
        return [int(e) for e in self["edges"]]

    @property
    def length_m(self) -> float:
        return float(self["length_m"])

    @property
    def time_s(self) -> float:
        return float(self["time_s"])

    def _network(self) -> Network:
        return self._db if isinstance(self._db, Network) else Network(_file(self._db))

    @property
    def edges(self):
        net = self._network()
        ids = ",".join(map(str, self.edge_ids))
        gdf = net._frame(f"{self._mode}.edges", "geometry", f"edge_id IN ({ids})", None)
        order = {e: i for i, e in enumerate(self.edge_ids)}
        gdf.insert(0, "seq", gdf["edge_id"].map(order))
        return gdf.sort_values("seq").reset_index(drop=True)

    @property
    def geometry(self):
        import shapely
        from duckosm import _api
        return shapely.from_wkb(bytes(_api._rust().route_line(_file(self._db), self._mode, self.edge_ids)))

    def directions(self) -> list[dict]:
        return directions(self._db, self, mode=self._mode)

    def map(self, **style):
        return self._network().map(self._mode, routes=self, **style)

    def __repr__(self) -> str:
        own = f", {self.weight} {self.cost:g}" if "cost" in self else ""
        return f"Route({self._mode}: {len(self['edges'])} edges, {self.length_m:.0f} m, {self.time_s:.0f} s{own})"

    def _repr_html_(self) -> str:
        return (f"<div><b>Route</b> ({self._mode})<table><tr><td>edges</td><td>{len(self['edges'])}</td></tr>"
                f"<tr><td>length</td><td>{self.length_m:,.0f} m</td></tr>"
                f"<tr><td>time</td><td>{self.time_s / 60:,.1f} min</td></tr></table></div>")


def _route(data, db, mode, weight="time"):
    return None if data is None else Route(data, db, mode, weight if isinstance(weight, str) else "own")


# ---- places: edge ids, points and place names --------------------------------------------------------------------

def _is_point(x):
    return isinstance(x, (tuple, list)) and len(x) == 2 and all(isinstance(v, (int, float)) for v in x) \
        and not all(isinstance(v, int) for v in x)


def geocode(net, name: str, mode: str | None = None) -> tuple[float, float]:
    """The ``(lon, lat)`` of a place name: the middle of the longest edge of that name in the network, else the first hit of
    OpenStreetMap's Nominatim in the box of the network (this needs the internet).

    >>> duckosm.routing.geocode(net.driving, "Boulevard Albert 1er")"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    try:
        return _api._rust().geocode(_file(db), mode, name)
    except RuntimeError as e:
        raise DuckOSMError(str(e)) from None


def _file(db):
    """The database file of ``db``. An in-memory database is copied to a temporary file, which stays until Python exits (the
    router keeps it open)."""
    from duckosm import _api
    path = _api._db(db)
    if path is None:
        import atexit
        import shutil
        f = _api._file(db)
        path = f.__enter__()
        atexit.register(shutil.rmtree, f.tmp, True)
    return path


def _edge_ids(db, mode, places, router=None) -> list[int]:
    """The edge id of each place: an edge id stays; a point (or a geocoded name) takes its nearest edge of the graph."""
    places = list(places)
    out = [None if isinstance(p, str) or _is_point(p) else int(p) for p in places]
    todo = [i for i, e in enumerate(out) if e is None]
    if todo:
        router = router or _router(db, mode, "time")
        pts = [geocode(db, places[i], mode) if isinstance(places[i], str) else tuple(places[i]) for i in todo]
        for i, e in zip(todo, router.nearest(pts)):
            out[i] = int(e)
    return out


def routed_graph(net, mode: str | None = None) -> str:
    """The SQL relation of the rows of ``<mode>.edge_graph`` that the mode routes on (its own ``uses``): the graph of
    :func:`route`. ``SELECT from_edge FROM {routed_graph(net.driving)}`` gives the edges that a route can start on.

    >>> net.sql(f"SELECT count(*) FROM {duckosm.routing.routed_graph(net.driving)}")"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    return _api._rust().routed_relation(_file(db), mode)


def _labels(places):
    places = list(places)
    return places if all(isinstance(p, (int, str)) for p in places) else list(range(len(places)))


def _named(places):
    """The places and their labels: a dict ``{label: place}`` gives its keys as the labels; a list gives the places
    themselves (edge ids, names) or their numbers (points)."""
    if isinstance(places, dict):
        return list(places.values()), list(places.keys())
    places = list(places)
    return places, _labels(places)


def _router(db, mode, weight, missing="time"):
    """The Rust router of ``weight``: an arc of the edge graph costs the cost of its from-edge."""
    from duckosm import _api
    name, costs = _weight(weight, missing)
    if costs is None and name in WEIGHTS:
        return _api._rust().Router(_file(db), mode, name)
    return _api._rust().Router(_file(db), mode, name, costs, missing)


def _routes(db, mode, data, weight="time") -> list:
    """The Route of each dict of a JSON list of the Rust router (None: not connected)."""
    return [_route(d, db, mode, weight) for d in json.loads(data)]


# ---- routes ----------------------------------------------------------------------------------------------------------

def route(net, a, b, mode=None, weight="time", missing="time"):
    """The fastest route (``weight="length"``: the shortest; a column or a Series: the cheapest in your cost, see the
    module text) from ``a`` to ``b``, as a :class:`Route`, or None when no route connects them. ``a`` and ``b``: two edge ids, two ``(lon, lat)`` points (then :func:`route_points`: each point
    joins its nearest road; ``length_m`` and ``time_s`` then include the distance and the walking time from each point
    to its road, ``access_m`` and ``access_s`` of ``start`` and ``end``) or place names (:func:`geocode`).

    >>> duckosm.routing.route(net.driving, (7.4155, 43.7285), (7.4335, 43.7468))
    >>> duckosm.routing.route(net.walking, "Boulevard Albert 1er", "Avenue de la Costa")
    >>> duckosm.routing.route(net.driving, a, b, weight=toll)  # toll: a Series keyed by edge_id"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    a, b = (geocode(db, x, mode) if isinstance(x, str) else x for x in (a, b))
    if _is_point(a) and _is_point(b):
        return route_points(db, a, b, mode=mode, weight=weight, missing=missing)
    router = _router(db, mode, weight, missing)
    a, b = _edge_ids(db, mode, [a, b], router)
    try:
        return _routes(db, mode, router.routes([(int(a), int(b))]), weight)[0]
    except RuntimeError as e:
        if "not in the" in str(e):
            raise ValueError(str(e)) from None
        raise


def route_points(net, a, b, mode=None, weight="time", radius_m=50.0, access_kmh=4.5, missing="time"):
    """The fastest route between two ``(lon, lat)`` points: each joins the network at its nearest road within
    ``radius_m``. A :class:`Route` (also ``start``, ``end``), or None. With your own weight, the access legs cost 0.

    >>> duckosm.routing.route_points(net.walking, (7.4155, 43.7285), (7.4335, 43.7468))"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    if not _is_own(weight):
        return _route(_api.route_points(db, a, b, mode=mode, weight=weight, radius_m=radius_m, access_kmh=access_kmh),
                      db, mode, weight)
    name, costs = _weight(weight, missing)
    data = json.loads(_api._rust().route_points(_file(db), tuple(a), tuple(b), mode, name, float(radius_m),
                                                float(access_kmh), costs, missing))
    return _route(data, db, mode, weight)


def routes(net, origins, destinations, mode=None, weight="time", missing="time") -> list:
    """The route from each origin to the destination at the same place in the list (``zip``), in parallel. A list of
    :class:`Route` (None: not connected). A point takes its nearest edge.

    >>> rs = duckosm.routing.routes(net.driving, [e1, e2], [e3, e4])"""
    db, mode = resolve(net, mode)
    origins, destinations = list(origins), list(destinations)
    if len(origins) != len(destinations):
        raise DuckOSMError(f"{len(origins)} origins and {len(destinations)} destinations: give the same number")
    router = _router(db, mode, weight, missing)
    ids = _edge_ids(db, mode, origins + destinations, router)
    pairs = list(zip(ids[:len(origins)], ids[len(origins):]))
    return _routes(db, mode, router.routes(pairs), weight)


def k_shortest_paths(net, a, b, k=3, mode=None, weight="time", missing="time") -> list:
    """The ``k`` fastest routes (``weight="length"``: shortest) from ``a`` to ``b`` that visit no edge twice, the best
    first (Yen's algorithm, on the graph and the turn rules of :func:`route`). The first equals :func:`route` of two edge
    ids. A list of :class:`Route`; fewer than ``k`` when the graph has fewer.

    >>> for r in duckosm.routing.k_shortest_paths(net.driving, a, b, k=3): print(r.time_s)"""
    db, mode = resolve(net, mode)
    router = _router(db, mode, weight, missing)
    a, b = _edge_ids(db, mode, [a, b], router)
    return _routes(db, mode, router.k_shortest_routes(a, b, int(k)), weight)


def matrix(net, origins, destinations=None, mode=None, weight="time", max_cost=None, missing="time"):
    """The travel time in seconds (``weight="length"``: the distance in metres; your own weight: your cost) from each
    origin to each destination,
    as a DataFrame: a row for each origin, a column for each destination. ``destinations=None``: the origins. A cell is
    the ``time_s`` (``length_m``) of :func:`route` of that pair: from the start of the origin edge to the end of the
    destination edge. NaN: not connected, more than ``max_cost``, or an edge that is not in the graph of the mode. One Dijkstra search for each origin, in parallel
    (Rust); ``max_cost`` stops each search early.

    Origins and destinations are lists of places (edge ids, ``(lon, lat)`` points, names), or a dict
    ``{label: place}``: then the labels name the rows and the columns.

    >>> m = duckosm.routing.matrix(net.driving, edge_ids[:100])"""
    import pandas as pd
    db, mode = resolve(net, mode)
    origins, o_labels = _named(origins)
    destinations, d_labels = (origins, o_labels) if destinations is None else _named(destinations)
    router = _router(db, mode, weight, missing)
    ids = _edge_ids(db, mode, origins + destinations, router)
    o, d = ids[:len(origins)], ids[len(origins):]
    limit = math.inf if max_cost is None else float(max_cost)
    return pd.DataFrame(router.matrix(o, d, limit), index=o_labels, columns=d_labels, dtype=float)


def isochrones(net, origins, minutes=(5, 10, 15), mode=None, buffer_m=25.0, weight="time", missing="time"):
    """The area reached from each origin within each number of minutes, as a GeoDataFrame of polygons (``origin``,
    ``minutes``, ``n_edges``, ``area_km2``, ``geometry``). With another ``weight``, a minute is 60 units of that cost. An edge is reached when the whole edge is reached: its :func:`matrix` cell is
    at most the time. The polygon is the union of the reached edges, each with a buffer of ``buffer_m`` metres (in the UTM
    zone of the area).

    >>> iso = duckosm.routing.isochrones(net.walking, [(7.4246, 43.7384)], minutes=[5, 10])"""
    gpd = require("geopandas", "isochrones")
    import shapely
    db, mode = resolve(net, mode)
    (origins, labels), minutes = _named(origins), sorted(float(m) for m in ([minutes] if isinstance(minutes, (int, float)) else minutes))
    router = _router(db, mode, weight, missing)
    rows = router.isochrones(_edge_ids(db, mode, origins, router), [m * 60 for m in minutes], float(buffer_m))
    return gpd.GeoDataFrame(
        [{"origin": labels[i], "minutes": limit / 60, "n_edges": n, "area_km2": area,
          "geometry": shapely.from_wkb(bytes(wkb)) if wkb is not None else shapely.Polygon()} for i, limit, n, area, wkb in rows],
        geometry="geometry", crs="EPSG:4326")


def accessibility(net, origins, targets, minutes=15, mode=None, weight="time", missing="time"):
    """For each origin: the number of targets reached within each number of minutes (a column ``n_<m>min``), and the
    nearest target (``nearest``, ``nearest_s``). With another ``weight``, a minute is 60 units of that cost. It uses :func:`matrix` (one search for each origin, stopped at the
    largest time). A DataFrame with a row for each origin.

    >>> acc = duckosm.routing.accessibility(net.walking, homes, shops, minutes=[5, 10])"""
    import pandas as pd
    db, mode = resolve(net, mode)
    minutes = sorted(float(m) for m in ([minutes] if isinstance(minutes, (int, float)) else minutes))
    (origins, o_labels), (targets, t_labels) = _named(origins), _named(targets)
    router = _router(db, mode, weight, missing)
    ids = _edge_ids(db, mode, origins + targets, router)
    counts, nearest, best = router.accessibility(ids[:len(origins)], ids[len(origins):], [m * 60 for m in minutes])
    out = {f"n_{x:g}min": [c[k] for c in counts] for k, x in enumerate(minutes)}
    out["nearest"] = pd.Series([None if j is None else t_labels[j] for j in nearest], index=o_labels, dtype=object)
    out["nearest_s"] = best
    return pd.DataFrame(out, index=o_labels)


# ---- across modes, and the steps -------------------------------------------------------------------------------------

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
    return _api.directions(db, dict(route), mode=mode)
