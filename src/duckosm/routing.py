"""``duckosm.routing``: routing, accessibility and logistics (docs/proposals/python-sdk.md, section 5.3).

Each function takes a ``Network``, a ``ModeView`` (``net.walking``), a path or a DuckDB connection. A place (``a``,
``b``, ``origins`` ...) is an edge id, a ``(lon, lat)`` point or a place name. The searches run in Rust on the edge
graph of ``route`` (the same turn rules); the calls for many places run in parallel.
"""
from __future__ import annotations

import json
import math
import urllib.parse
import urllib.request

from duckosm._api import route_multimodal  # noqa: F401 -- the names of this module
# the old names of duckosm.routing (the Python code): they stay. Router is the Python Router (it has .graph); the Rust
# one is duckosm.Router.
from duckosm._routing_py import Router, node_branches, node_out_degree, to_networkx, to_networkx_nodes, write_graph  # noqa: F401
from duckosm.errors import DuckOSMError, require
from duckosm.network import Network, resolve

WEIGHTS = {"time": "cost_s", "length": "length_m"}


# ---- the Route object ------------------------------------------------------------------------------------------------

class Route(dict):
    """One route. It is also the dict of the old ``route`` (``r["edges"]`` is the list of edge ids), so the old code
    and ``net.map(routes=r)`` still work.

    - ``.edge_ids``: the edge ids, in the order of the route.
    - ``.edges``: a GeoDataFrame of the edges, in the order of the route (``seq``, ``edge_id``, the edge columns).
    - ``.length_m``, ``.time_s``: the sums of ``length_m`` and ``cost_s`` of the edges. A route between two points
      also includes the access legs: ``length_m`` includes ``start.access_m`` and ``end.access_m`` (the distance from
      each point to its road), ``time_s`` includes ``access_s``, and only the used parts of the first and the last
      edge count.
    - ``.geometry``: one shapely line (a MultiLineString where the edges do not join).
    - ``.directions()``: the turn-by-turn steps. ``.map()``: the route on the roadstyle map of the network.

    >>> r = duckosm.routing.route(net.driving, (7.4155, 43.7285), (7.4335, 43.7468))
    >>> r.time_s, r.length_m, r.map()"""

    def __init__(self, data: dict, db, mode: str):
        super().__init__(data)
        self._db, self._mode = db, mode

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
        from shapely.ops import linemerge
        return linemerge(list(self.edges.geometry))

    def directions(self) -> list[dict]:
        return directions(self._db, self, mode=self._mode)

    def map(self, **style):
        return self._network().map(self._mode, routes=self, **style)

    def __repr__(self) -> str:
        return f"Route({self._mode}: {len(self['edges'])} edges, {self.length_m:.0f} m, {self.time_s:.0f} s)"

    def _repr_html_(self) -> str:
        return (f"<div><b>Route</b> ({self._mode})<table><tr><td>edges</td><td>{len(self['edges'])}</td></tr>"
                f"<tr><td>length</td><td>{self.length_m:,.0f} m</td></tr>"
                f"<tr><td>time</td><td>{self.time_s / 60:,.1f} min</td></tr></table></div>")


def _route(data, db, mode):
    return None if data is None else Route(data, db, mode)


# ---- places: edge ids, points and place names --------------------------------------------------------------------

def _is_point(x):
    return isinstance(x, (tuple, list)) and len(x) == 2 and all(isinstance(v, (int, float)) for v in x) \
        and not all(isinstance(v, int) for v in x)


def geocode(net, name: str, mode: str | None = None) -> tuple[float, float]:
    """The ``(lon, lat)`` of a place name: the middle of the longest edge of that name in the network, else the first hit of
    OpenStreetMap's Nominatim in the box of the network (this needs the internet).

    >>> duckosm.routing.geocode(net.driving, "Boulevard Albert 1er")"""
    db, mode = resolve(net, mode)
    net = db if isinstance(db, Network) else Network(_file(db))
    row = net.sql(f"SELECT ST_X(c), ST_Y(c) FROM (SELECT ST_LineInterpolatePoint(geometry, 0.5) AS c FROM {mode}.edges "
                  "WHERE lower(name) = lower($n) ORDER BY length_m DESC, edge_id LIMIT 1)", {"n": name})
    if len(row):
        return float(row.iloc[0, 0]), float(row.iloc[0, 1])
    w, s, e, n = net.sql(f"SELECT min(ST_XMin(geometry)), min(ST_YMin(geometry)), max(ST_XMax(geometry)), "
                         f"max(ST_YMax(geometry)) FROM {mode}.edges").iloc[0]
    query = urllib.parse.urlencode({"q": name, "format": "json", "limit": 1, "viewbox": f"{w},{n},{e},{s}", "bounded": 1})
    req = urllib.request.Request(f"https://nominatim.openstreetmap.org/search?{query}", headers={"User-Agent": "duckosm"})
    with urllib.request.urlopen(req, timeout=30) as f:
        hits = json.load(f)
    if not hits:
        raise DuckOSMError(f"no place {name!r} in the network or in Nominatim inside the box of the network")
    return float(hits[0]["lon"]), float(hits[0]["lat"])


def _file(db):
    from duckosm import _api
    path = _api._db(db)
    if path is None:
        raise DuckOSMError("this call needs a database file, not an in-memory database")
    return path


def _edge_ids(db, mode, places) -> list[int]:
    """The edge id of each place: an edge id stays; a point (or a geocoded name) takes its nearest edge."""
    places = list(places)
    out = [None if isinstance(p, str) or _is_point(p) else int(p) for p in places]
    todo = [i for i, e in enumerate(out) if e is None]
    if todo:
        gpd = require("geopandas", "routing from points")
        net = db if isinstance(db, Network) else Network(_file(db))
        pts = [geocode(net, places[i], mode) if isinstance(places[i], str) else places[i] for i in todo]
        edges = net.edges(mode, where=f"edge_id IN (SELECT from_edge FROM {_routed(net, mode)})")[["edge_id", "geometry"]]
        crs = edges.estimate_utm_crs()
        at = gpd.GeoDataFrame({"i": todo}, geometry=gpd.points_from_xy(*zip(*pts)), crs="EPSG:4326").to_crs(crs)
        hit = gpd.sjoin_nearest(at, edges.to_crs(crs), how="left").drop_duplicates("i")
        for i, e in zip(hit["i"], hit["edge_id"]):
            out[i] = int(e)
    return out


def routed_graph(net, mode: str | None = None) -> str:
    """The SQL relation of the rows of ``<mode>.edge_graph`` that the mode routes on (its own ``uses``): the graph of
    :func:`route`. ``SELECT from_edge FROM {routed_graph(net.driving)}`` gives the edges that a route can start on.

    >>> net.sql(f"SELECT count(*) FROM {duckosm.routing.routed_graph(net.driving)}")"""
    db, mode = resolve(net, mode)
    return _routed(db if isinstance(db, Network) else Network(_file(db)), mode)


def _routed(net, mode):
    use = {"driving": "car", "walking": "walk", "cycling": "bike"}.get(mode)
    has = len(net.sql("SELECT 1 FROM information_schema.columns WHERE table_schema = $m AND table_name = 'edge_graph' "
                      "AND column_name = 'uses'", {"m": mode}))
    return f"(SELECT * FROM {mode}.edge_graph WHERE uses = '{use}')" if has and use else f"{mode}.edge_graph"


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


def _router(db, mode, weight):
    from duckosm import _api
    if weight not in WEIGHTS:
        raise DuckOSMError(f"weight must be one of {', '.join(WEIGHTS)}, not {weight!r}")
    return _api._rust().Router(_file(db), mode, weight)


def _routes_of(db, mode, paths) -> list:
    """The Route of each path (a list of edge ids, or None), with the edge rows of the old ``route``."""
    ids = sorted({e for p in paths if p for e in p})
    rows = {}
    if ids:
        net = db if isinstance(db, Network) else Network(_file(db))
        df = net.sql(f"SELECT edge_id, name, highway, length_m::DOUBLE AS length_m, cost_s::DOUBLE AS cost_s, "
                     f"ST_AsText(geometry) AS geometry FROM {mode}.edges WHERE list_contains($ids, edge_id)", {"ids": ids})
        rows = {int(r["edge_id"]): {k: (None if v != v else v) for k, v in r.items()} for r in df.to_dict("records")}
    out = []
    for p in paths:
        if p is None:
            out.append(None)
            continue
        path = [{**rows[e], "edge_id": e} for e in p]
        out.append(Route({"edges": list(p), "time_s": sum(r["cost_s"] or 0 for r in path),
                          "length_m": sum(r["length_m"] or 0 for r in path), "path": path}, db, mode))
    return out


def _own_weight(db, mode, weight, ids) -> dict:
    """The weight (``cost_s`` or ``length_m``) of each edge: the cost of the last edge of a route, which the graph cost
    does not hold (an arc of the edge graph costs its from-edge)."""
    net = db if isinstance(db, Network) else Network(_file(db))
    df = net.sql(f"SELECT edge_id, coalesce({WEIGHTS[weight]}, 0)::DOUBLE AS w FROM {mode}.edges "
                 "WHERE list_contains($ids, edge_id)", {"ids": sorted(set(ids))})
    return dict(zip(df["edge_id"].astype(int), df["w"]))


# ---- routes ----------------------------------------------------------------------------------------------------------

def route(net, a, b, mode=None, weight="time", graph=None):
    """The fastest route (``weight="length"``: the shortest) from ``a`` to ``b``, as a :class:`Route`, or None when no
    route connects them. ``a`` and ``b``: two edge ids, two ``(lon, lat)`` points (then :func:`route_points`: each point
    joins its nearest road; ``length_m`` and ``time_s`` then include the distance and the walking time from each point
    to its road, ``access_m`` and ``access_s`` of ``start`` and ``end``) or place names (:func:`geocode`). ``graph``: a
    networkx graph of :func:`duckosm.ml.to_networkx` (the Python code).

    >>> duckosm.routing.route(net.driving, (7.4155, 43.7285), (7.4335, 43.7468))
    >>> duckosm.routing.route(net.walking, "Boulevard Albert 1er", "Avenue de la Costa")"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    a, b = (geocode(db, x, mode) if isinstance(x, str) else x for x in (a, b))
    if _is_point(a) and _is_point(b):
        return _route(_api.route_points(db, a, b, mode=mode, weight=weight), db, mode)
    if _is_point(a) or _is_point(b):
        a, b = _edge_ids(db, mode, [a, b])
    return _route(_api.route(db, a, b, mode=mode, weight=weight, graph=graph), db, mode)


def route_points(net, a, b, mode=None, weight="time", radius_m=50.0, access_kmh=4.5):
    """The fastest route between two ``(lon, lat)`` points: each joins the network at its nearest road within
    ``radius_m``. A :class:`Route` (also ``start``, ``end``), or None.

    >>> duckosm.routing.route_points(net.walking, (7.4155, 43.7285), (7.4335, 43.7468))"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    return _route(_api.route_points(db, a, b, mode=mode, weight=weight, radius_m=radius_m, access_kmh=access_kmh), db, mode)


def routes(net, origins, destinations, mode=None, weight="time") -> list:
    """The route from each origin to the destination at the same place in the list (``zip``), in parallel. A list of
    :class:`Route` (None: not connected). A point takes its nearest edge.

    >>> rs = duckosm.routing.routes(net.driving, [e1, e2], [e3, e4])"""
    db, mode = resolve(net, mode)
    origins, destinations = list(origins), list(destinations)
    if len(origins) != len(destinations):
        raise DuckOSMError(f"{len(origins)} origins and {len(destinations)} destinations: give the same number")
    ids = _edge_ids(db, mode, origins + destinations)
    pairs = list(zip(ids[:len(origins)], ids[len(origins):]))
    return _routes_of(db, mode, _router(db, mode, weight).paths(pairs))


def k_shortest_paths(net, a, b, k=3, mode=None, weight="time") -> list:
    """The ``k`` fastest routes (``weight="length"``: shortest) from ``a`` to ``b`` that visit no edge twice, the best
    first (Yen's algorithm, on the graph and the turn rules of :func:`route`). The first equals :func:`route` of two edge
    ids. A list of :class:`Route`; fewer than ``k`` when the graph has fewer.

    >>> for r in duckosm.routing.k_shortest_paths(net.driving, a, b, k=3): print(r.time_s)"""
    db, mode = resolve(net, mode)
    a, b = _edge_ids(db, mode, [a, b])
    return _routes_of(db, mode, _router(db, mode, weight).k_shortest_paths(a, b, int(k)))


def matrix(net, origins, destinations=None, mode=None, weight="time", max_cost=None):
    """The travel time in seconds (``weight="length"``: the distance in metres) from each origin to each destination,
    as a DataFrame: a row for each origin, a column for each destination. ``destinations=None``: the origins. A cell is
    the ``time_s`` (``length_m``) of :func:`route` of that pair: from the start of the origin edge to the end of the
    destination edge. NaN: not connected, more than ``max_cost``, or an edge that is not in the graph of the mode. One Dijkstra search for each origin, in parallel
    (Rust); ``max_cost`` stops each search early.

    Origins and destinations are lists of places (edge ids, ``(lon, lat)`` points, names), or a dict
    ``{label: place}``: then the labels name the rows and the columns.

    >>> m = duckosm.routing.matrix(net.driving, edge_ids[:100])"""
    import numpy as np
    import pandas as pd
    db, mode = resolve(net, mode)
    origins, o_labels = _named(origins)
    destinations, d_labels = (origins, o_labels) if destinations is None else _named(destinations)
    ids = _edge_ids(db, mode, origins + destinations)
    o, d = ids[:len(origins)], ids[len(origins):]
    own = _own_weight(db, mode, weight, d)
    limit = math.inf if max_cost is None else float(max_cost)
    cost = np.array(_router(db, mode, weight).costs(o, d, limit), dtype=float).reshape(len(o), len(d))
    cost = cost + np.array([own.get(e, 0.0) for e in d])
    cost[cost > limit] = np.nan
    return pd.DataFrame(cost, index=o_labels, columns=d_labels)


def isochrones(net, origins, minutes=(5, 10, 15), mode=None, buffer_m=25.0):
    """The area reached from each origin within each number of minutes, as a GeoDataFrame of polygons (``origin``,
    ``minutes``, ``n_edges``, ``area_km2``, ``geometry``). An edge is reached when the whole edge is reached: its :func:`matrix` cell is
    at most the time. The polygon is the union of the reached edges, each with a buffer of ``buffer_m`` metres (in the UTM
    zone of the area).

    >>> iso = duckosm.routing.isochrones(net.walking, [(7.4246, 43.7384)], minutes=[5, 10])"""
    gpd = require("geopandas", "isochrones")
    import pandas as pd
    from shapely import Polygon, union_all
    db, mode = resolve(net, mode)
    (origins, labels), minutes = _named(origins), sorted(float(m) for m in ([minutes] if isinstance(minutes, (int, float)) else minutes))
    ids = _edge_ids(db, mode, origins)
    reach = _router(db, mode, "time").reach(ids, max(minutes) * 60)
    every = sorted({e for hit, _ in reach for e in hit})
    own = _own_weight(db, mode, "time", every)
    edges = Network(_file(db)).edges(mode, where=f"edge_id IN ({','.join(map(str, every)) or 'NULL'})", crs="utm")
    geom = dict(zip(edges["edge_id"].astype(int), edges.buffer(buffer_m, resolution=4)))
    rows = []
    for label, (hit, cost) in zip(labels, reach):
        total = pd.Series(cost, index=hit) + pd.Series([own[e] for e in hit], index=hit)
        poly, done = None, 0.0
        for m in minutes:                   # each polygon: the one before, and the edges of the next minutes
            ring = total.index[(total > done * 60) & (total <= m * 60)]
            parts = [geom[e] for e in ring] + ([poly] if poly is not None else [])
            poly, done = union_all(parts) if parts else Polygon(), m
            rows.append({"origin": label, "minutes": m, "n_edges": int((total <= m * 60).sum()),
                         "area_km2": round(poly.area / 1e6, 3), "geometry": poly})        # the area in the UTM zone
    return gpd.GeoDataFrame(rows, geometry="geometry", crs=edges.crs).to_crs("EPSG:4326")


def accessibility(net, origins, targets, minutes=15, mode=None):
    """For each origin: the number of targets reached within each number of minutes (a column ``n_<m>min``), and the
    nearest target (``nearest``, ``nearest_s``). It uses :func:`matrix` (one search for each origin, stopped at the
    largest time). A DataFrame with a row for each origin.

    >>> acc = duckosm.routing.accessibility(net.walking, homes, shops, minutes=[5, 10])"""
    import numpy as np
    import pandas as pd
    minutes = sorted(float(m) for m in ([minutes] if isinstance(minutes, (int, float)) else minutes))
    m = matrix(net, origins, targets, mode=mode, max_cost=max(minutes) * 60)
    out = {f"n_{x:g}min": (m <= x * 60).sum(axis=1).astype(int) for x in minutes}
    best = m.to_numpy()
    reached = ~np.isnan(best).all(axis=1)
    pick = np.where(reached, np.nanargmin(np.where(np.isnan(best), np.inf, best), axis=1), 0)
    out["nearest"] = pd.Series([m.columns[j] if ok else None for j, ok in zip(pick, reached)], index=m.index, dtype=object)
    out["nearest_s"] = [best[i, j] if ok else np.nan for i, (j, ok) in enumerate(zip(pick, reached))]
    return pd.DataFrame(out, index=m.index)


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
