"""Routing between two points, not two edges (docs/design/point_routing.md).

A point joins the network at its nearest road point within ``radius_m``: the straight walk to it
(the access leg) costs walking time at ``access_kmh``, whatever the mode, and may not cross another
road of the mode or any road for cars (Kaveh: "it can't jump from roads"); the first and last edges
count only the part travelled (a trip from the middle of an edge pays half of it). Every edge within
the radius, both directions of a two-way road, is a candidate; the route is the cheapest over all
of them, access legs included.
"""

import heapq
import math

INF = float("inf")


TOUCH_M = 0.5        # a road meeting the access leg this close to its road point only touches it


def candidates(con, schema, point, radius_m):
    """The edges of ``schema.edges`` within ``radius_m`` of ``point`` (lon, lat) that the point can
    walk straight to without crossing another road (an edge of ``schema`` or of ``driving``, the
    roads for cars), each with the nearest point on it: ``{edge_id, source, target, cost_s,
    length_m, fraction, access_m, road_point}``. ``fraction`` is how far along the edge (by length)
    that point lies, 0 to 1."""
    from shapely import STRtree, wkb
    from shapely.geometry import LineString, Point

    lon, lat = point
    ky = 111320.0
    kx = ky * math.cos(math.radians(lat))
    dx, dy = radius_m / kx, radius_m / ky
    rows = con.execute(
        f"SELECT edge_id, source, target, cost_s, length_m, ST_AsWKB(geometry) FROM {schema}.edges "
        f"WHERE ST_Intersects(geometry, ST_MakeEnvelope(?, ?, ?, ?))",
        [lon - dx, lat - dy, lon + dx, lat + dy]).fetchall()
    to_local = lambda g: LineString([((x - lon) * kx, (y - lat) * ky) for x, y in wkb.loads(bytes(g)).coords])  # noqa: E731
    here, out = Point(0.0, 0.0), []
    for eid, s, t, cost, length, geom in rows:
        local = to_local(geom)
        d = local.distance(here)
        if d > radius_m or local.length == 0:
            continue
        at = local.project(here)
        out.append({"edge_id": eid, "source": s, "target": t, "cost_s": float(cost or 0.0),
                    "length_m": float(length or 0.0), "fraction": at / local.length,
                    "access_m": d, "_p": local.interpolate(at)})
    # the roads the walk may not cross: the mode's own, and the roads for cars
    schemas = {r[0] for r in con.execute("SELECT table_schema FROM information_schema.tables "
                                         "WHERE table_name = 'edges'").fetchall()}
    blockers = [to_local(g) for sch in {schema, "driving"} & schemas for (g,) in con.execute(
        f"SELECT ST_AsWKB(geometry) FROM {sch}.edges WHERE ST_Intersects(geometry, ST_MakeEnvelope(?, ?, ?, ?))",
        [lon - dx, lat - dy, lon + dx, lat + dy]).fetchall()]
    tree = STRtree(blockers) if blockers else None
    keep = []
    for c in out:
        p = c.pop("_p")
        walk = LineString([(0.0, 0.0), (p.x, p.y)])
        crossed = tree is not None and walk.length > 0 and any(
            not walk.intersection(blockers[i]).difference(p.buffer(TOUCH_M)).is_empty
            for i in tree.query(walk, predicate="intersects"))
        if not crossed:
            c["road_point"] = (p.x / kx + lon, p.y / ky + lat)
            keep.append(c)
    return keep


def _ends(con, schema, a, b, radius_m):
    con.execute("INSTALL spatial; LOAD spatial;")
    S, T = candidates(con, schema, a, radius_m), candidates(con, schema, b, radius_m)
    if not S:
        raise ValueError(f"no road within {radius_m:g} m of the start")
    if not T:
        raise ValueError(f"no road within {radius_m:g} m of the end")
    return S, T


def _end_info(point, c, v):
    return {"point": tuple(point), "road_point": c["road_point"], "edge_id": c["edge_id"],
            "fraction": c["fraction"], "access_m": c["access_m"], "access_s": c["access_m"] / v}


def route_points(con, a, b, mode="driving", weight="time", radius_m=50.0, access_kmh=4.5):
    """Fastest (``weight="time"``) or shortest (``"length"``) route in one mode between two points
    ``(lon, lat)``. Returns ``None`` if no candidate pair is connected, else::

        {"time_s", "length_m",              # door to door: access legs + the parts of edges used
         "edges": [edge_id, …],             # in order, the first and last possibly in part
         "start", "end": {point, road_point, edge_id, fraction, access_m, access_s},
         "path": [{edge_id, name, highway, from_fraction, to_fraction, length_m, cost_s}, …]}

    Raises ``ValueError`` when a point has no road of the mode within ``radius_m``. Turn
    restrictions apply (the route follows ``edge_graph``).
    """
    if weight not in ("time", "length"):
        raise ValueError("weight must be 'time' or 'length'")
    S, T = _ends(con, mode, a, b, radius_m)
    v = access_kmh / 3.6
    key = "cost_s" if weight == "time" else "length_m"
    acc = (lambda c: c["access_m"] / v) if weight == "time" else (lambda c: c["access_m"])  # noqa: E731
    Tby = {c["edge_id"]: c for c in T}

    best = None                                          # (total, path, start candidate)
    for s in S:                                          # start and end on one edge, forward
        t = Tby.get(s["edge_id"])
        if t is not None and s["fraction"] <= t["fraction"]:
            total = acc(s) + (t["fraction"] - s["fraction"]) * s[key] + acc(t)
            if best is None or total < best[0]:
                best = (total, [s["edge_id"]], s)

    # ponytail: reads the whole edge graph per call; a Router-like prebuilt graph when routing many
    W = dict(con.execute(f"SELECT edge_id, {key} FROM {mode}.edges").fetchall())
    nxt: dict = {}
    from duckosm.processors.edge_graph import routed_graph
    for f, t in con.execute(f"SELECT from_edge, to_edge FROM {routed_graph(con, mode)}").fetchall():
        nxt.setdefault(f, []).append(t)

    # edge-based Dijkstra; a label is the cost at the END of an edge (the start edges: their part)
    lab, prev, start_of, heap = {}, {}, {}, []
    for s in S:
        e, d = s["edge_id"], acc(s) + (1.0 - s["fraction"]) * s[key]
        if d < lab.get(e, INF):
            lab[e], prev[e], start_of[e] = d, None, s
            heapq.heappush(heap, (d, e))
    arrive_from = {}                                     # end edge -> the edge it is entered from
    arrive = {}
    while heap:
        d, e = heapq.heappop(heap)
        if d > lab.get(e, INF):
            continue
        if best is not None and d >= best[0]:
            break                                        # nothing cheaper can still arrive
        for n in nxt.get(e, ()):
            t = Tby.get(n)
            if t is not None and d < arrive.get(n, INF):
                arrive[n], arrive_from[n] = d, e
                total = d + t["fraction"] * t[key] + acc(t)
                if best is None or total < best[0]:
                    best = (total, n, None)
            nd = d + (W.get(n) or 0.0)
            if nd < lab.get(n, INF):
                lab[n], prev[n] = nd, e
                heapq.heappush(heap, (nd, n))
    if best is None:
        return None

    if best[2] is not None:                              # the same-edge trip
        path, s, t = best[1], best[2], Tby[best[1][0]]
        parts = [(s["fraction"], t["fraction"])]
    else:
        last = best[1]
        path, e = [last], arrive_from[last]
        while e is not None:
            path.append(e)
            e = prev[e]
        path.reverse()
        s, t = start_of[path[0]], Tby[last]
        parts = ([(s["fraction"], 1.0)] + [(0.0, 1.0)] * (len(path) - 2) + [(0.0, t["fraction"])])

    ids = ", ".join(str(e) for e in set(path))
    info = {r[0]: r for r in con.execute(
        f"SELECT edge_id, name, highway, length_m, cost_s FROM {mode}.edges WHERE edge_id IN ({ids})"
    ).fetchall()}
    rows = []
    for e, (f0, f1) in zip(path, parts, strict=True):
        r, share = info[e], f1 - f0
        rows.append({"edge_id": e, "name": r[1], "highway": r[2], "from_fraction": f0,
                     "to_fraction": f1, "length_m": (r[3] or 0.0) * share,
                     "cost_s": (r[4] or 0.0) * share})
    start, end = _end_info(a, s, v), _end_info(b, t, v)
    return {"time_s": start["access_s"] + sum(r["cost_s"] for r in rows) + end["access_s"],
            "length_m": start["access_m"] + sum(r["length_m"] for r in rows) + end["access_m"],
            "edges": path, "start": start, "end": end, "path": rows}


def route_multimodal_points(con, a, b, radius_m=50.0, access_kmh=4.5, schema="mm",
                            allowed_modes=None, enforce_sequence=True):
    """:func:`duckosm.route_multimodal` between two points ``(lon, lat)``: the trip starts and ends
    walking, so each point joins its walking edges (both ends of each: walking goes both ways, a
    part of the edge each way), plus a direct walk when both points are on one edge. Returns the
    same dict as ``route_multimodal`` (``time_s`` includes the access legs, ``length_m`` their
    metres) plus ``start`` / ``end`` as in :func:`route_points`; ``None`` if not connected."""
    from duckosm.routing import route_multimodal

    S, T = _ends(con, "walking", a, b, radius_m)
    v = access_kmh / 3.6
    A, B = "__start__", "__end__"                        # temporary nodes
    arcs = []
    for c in S:
        f, x = c["fraction"], c["access_m"] / v
        arcs += [("walking", A, c["source"], c["edge_id"], f * c["cost_s"], f, x),
                 ("walking", A, c["target"], c["edge_id"], (1 - f) * c["cost_s"], 1 - f, x)]
    for c in T:
        g, x = c["fraction"], c["access_m"] / v
        arcs += [("walking", c["source"], B, c["edge_id"], g * c["cost_s"], g, x),
                 ("walking", c["target"], B, c["edge_id"], (1 - g) * c["cost_s"], 1 - g, x)]
    Tby = {c["edge_id"]: c for c in T}
    for c in S:                                          # both points on one walking edge
        t = Tby.get(c["edge_id"])
        if t is not None:
            share = abs(t["fraction"] - c["fraction"])
            arcs.append(("walking", A, B, c["edge_id"], share * c["cost_s"], share,
                         (c["access_m"] + t["access_m"]) / v))
    r = route_multimodal(con, A, B, schema=schema, allowed_modes=allowed_modes,
                         enforce_sequence=enforce_sequence, _virtual=arcs)
    if r is None:
        return None
    # which candidates the route used: its first and last walking edges
    first, last = r["edges"][0][1], r["edges"][-1][1]
    s = next(c for c in S if c["edge_id"] == first)
    t = Tby.get(last) or next(c for c in T if c["edge_id"] == last)
    r["start"], r["end"] = _end_info(a, s, v), _end_info(b, t, v)
    r["length_m"] += r["start"]["access_m"] + r["end"]["access_m"]
    r["nodes"] = [n for n in r["nodes"] if n not in (A, B)]
    return r
