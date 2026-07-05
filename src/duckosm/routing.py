"""
Routing helper — load the edge-based routing graph as a networkx DiGraph.

The DB stores an edge-based (line) graph: `<mode>.edge_graph(from_edge, to_edge, cost)`, where
each NODE is an `edge_id` and an arc `u -> v` means you may drive from edge `u` onto edge `v`
(illegal turns are already removed). `to_networkx` loads it into a `networkx.DiGraph` so you can
run any networkx algorithm:

    import duckdb, networkx as nx
    from duckosm.routing import to_networkx
    con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)
    G = to_networkx(con, weight="time")                 # or weight="length"
    path = nx.shortest_path(G, from_edge_id, to_edge_id, weight="weight")
    secs = nx.shortest_path_length(G, from_edge_id, to_edge_id, weight="weight")

This loads the whole graph into memory — fine for a city; for country-scale prefer an on-disk
search (see docs). Every road in `edges` is part of `edge_graph`, including `highway='service'`.

For the *geographic* (node-based) view — OSM junctions as nodes, road segments as edges with
their full attribute set, in the osmnx `MultiDiGraph` layout — use `to_networkx_nodes` instead.
"""
import logging

logger = logging.getLogger("duckosm")


def to_networkx(con, mode: str = "driving", weight: str = "time", node_attrs: bool = True):
    """Return a `networkx.DiGraph` of the routing graph, with a `weight` attribute per arc.

    Parameters
    ----------
    con : a DuckDB connection to a built duckOSM database.
    mode : the mode schema to read (default ``"driving"``).
    weight : ``"time"`` → travel time in seconds (`cost_s`, the default) or
        ``"length"`` → length in metres (`length_m`). The chosen value is stored on each arc as
        the ``weight`` attribute, so `nx.shortest_path(G, a, b, weight="weight")` gives the
        fastest / shortest route between two edge ids.
    node_attrs : when True (default) attach each edge's metadata to its node — ``name``,
        ``highway``, ``length_m``, ``maxspeed_kmh``, ``cost_s`` and ``geometry`` (WKT) — so
        ``G.nodes[edge_id]`` carries the road data, not just the routing ``weight`` on arcs. Pass
        False for a bare graph (e.g. inside :func:`route`, which doesn't need it).

    Returns a DiGraph whose nodes are ``edge_id``s (with the attributes above when ``node_attrs``).
    """
    try:
        import networkx as nx
    except ImportError as e:
        raise ImportError("to_networkx needs networkx — `pip install networkx`") from e

    graph = f"{mode}.edge_graph"
    if weight == "time":
        # edge_graph.cost is already the from_edge's travel time (cost_s).
        sql = f"SELECT from_edge, to_edge, cost FROM {graph}"
    elif weight == "length":
        # length of the from_edge.
        sql = (f"SELECT g.from_edge, g.to_edge, e.length_m "
               f"FROM {graph} g JOIN {mode}.edges e ON e.edge_id = g.from_edge")
    else:
        raise ValueError("weight must be 'time' or 'length'")

    g = nx.DiGraph()
    g.add_weighted_edges_from(con.execute(sql).fetchall(), weight="weight")
    if node_attrs:
        _attach_edge_attrs(con, g, mode)
    logger.info(f"routing graph[{mode}]: {g.number_of_nodes():,} edges, "
                f"{g.number_of_edges():,} arcs, weight={weight}"
                f"{', +node attrs' if node_attrs else ''}")
    return g


def _attach_edge_attrs(con, g, mode):
    """Attach each edge's metadata onto its node (edge_id) in an edge-based graph.

    Sets name/highway/length_m/maxspeed_kmh/cost_s and geometry (WKT) on every node already in
    ``g``; isolated edges absent from the graph are skipped.
    """
    try:
        con.execute("INSTALL spatial; LOAD spatial;")
    except Exception:
        pass
    rows = con.execute(
        f"SELECT edge_id, name, highway, length_m, maxspeed_kmh, cost_s, ST_AsText(geometry) "
        f"FROM {mode}.edges").fetchall()
    for eid, name, hw, length, spd, cost, geom in rows:
        if eid in g:
            g.nodes[eid].update(name=name, highway=hw, length_m=length,
                                maxspeed_kmh=spd, cost_s=cost, geometry=geom)
    return g


def to_networkx_nodes(con, mode: str = "driving", geometry: str = "wkt"):
    """Return a node-based ``networkx.MultiDiGraph`` of the road network for ``mode``.

    Where :func:`to_networkx` returns the *edge-based* routing (line) graph — whose nodes are
    ``edge_id``s and whose arcs are legal turns — this returns the **geographic** graph: nodes are
    OSM junction ``node_id``s and each graph edge is a road segment from ``<mode>.edges`` carrying
    its **full attribute set** (``edge_id``, ``osm_id``, ``highway``, ``name``, ``oneway``,
    ``lanes``, ``length_m``, ``maxspeed_kmh``, ``cost_s``, ``geometry``, ``is_reverse``, …).

    The layout matches osmnx: a directed multigraph keyed by ``edge_id`` (so parallel ways between
    the same two junctions are preserved), nodes carry ``x``/``y`` (lon/lat) plus any extra node
    columns (e.g. ``h3_cell``), and ``G.graph['crs'] = 'EPSG:4326'``. So osmnx / momepy tooling can
    consume it directly::

        from duckosm.routing import to_networkx_nodes
        G = to_networkx_nodes(con, mode="driving")
        G.nodes[node_id]["x"], G.nodes[node_id]["y"]       # lon, lat
        G[u][v][edge_id]["highway"]                        # full edge attrs, keyed by edge_id

    Parameters
    ----------
    con : a DuckDB connection to a built duckOSM database.
    mode : the mode schema to read (default ``"driving"``).
    geometry : how to store each edge's geometry — ``"wkt"`` (default) as a WKT string,
        ``"shapely"`` as a shapely object (needs ``shapely``; matches osmnx), or ``"none"`` to omit
        it (lighter graph). Node geometry is always reduced to ``x``/``y``.
    """
    try:
        import networkx as nx
    except ImportError as e:
        raise ImportError("to_networkx_nodes needs networkx — `pip install networkx`") from e
    if geometry not in ("wkt", "shapely", "none"):
        raise ValueError("geometry must be 'wkt', 'shapely' or 'none'")
    try:
        con.execute("INSTALL spatial; LOAD spatial;")
    except Exception:
        pass

    to_shapely = None
    if geometry == "shapely":
        try:
            from shapely import wkt as _wkt
        except ImportError as e:
            raise ImportError("geometry='shapely' needs shapely — `pip install shapely`") from e
        to_shapely = _wkt.loads

    g = nx.MultiDiGraph()
    g.graph["crs"] = "EPSG:4326"
    g.graph["mode"] = mode

    # --- nodes: node_id -> x/y (+ any other node columns, e.g. h3_cell) -------------------------
    ncols = [r[1] for r in con.execute(f"PRAGMA table_info('{mode}.nodes')").fetchall()]
    nselect = ["node_id", "ST_X(geom) AS x", "ST_Y(geom) AS y"]
    nextra = [c for c in ncols if c not in ("node_id", "geom")]
    nselect += nextra
    nrows = con.execute(f"SELECT {', '.join(nselect)} FROM {mode}.nodes WHERE geom IS NOT NULL")
    ncolnames = [d[0] for d in nrows.description]
    for row in nrows.fetchall():
        rec = dict(zip(ncolnames, row))
        nid = rec.pop("node_id")
        g.add_node(nid, **rec)

    # --- edges: one directed multigraph edge per row, keyed by edge_id, with full attributes ----
    ecols = [r[1] for r in con.execute(f"PRAGMA table_info('{mode}.edges')").fetchall()]
    eselect = []
    for c in ecols:
        if c == "geometry":
            if geometry == "none":
                continue
            eselect.append("ST_AsText(geometry) AS geometry")
        else:
            eselect.append(c)
    erows = con.execute(f"SELECT {', '.join(eselect)} FROM {mode}.edges")
    ecolnames = [d[0] for d in erows.description]
    n_edges = 0
    for row in erows.fetchall():
        rec = dict(zip(ecolnames, row))
        eid = rec["edge_id"]
        source = rec.pop("source")
        target = rec.pop("target")
        if to_shapely is not None and rec.get("geometry") is not None:
            rec["geometry"] = to_shapely(rec["geometry"])
        # source/target may be junctions pruned from `nodes` (clipped builds); add_edge re-adds them
        # as bare nodes, which is fine and matches osmnx behaviour.
        g.add_edge(source, target, key=eid, **rec)
        n_edges += 1

    logger.info(f"node graph[{mode}]: {g.number_of_nodes():,} nodes, {n_edges:,} edges, "
                f"geometry={geometry}")
    return g


# file extensions we recognise per format, for inferring `fmt` from the output path.
_GRAPH_EXTS = {".graphml": "graphml", ".gpickle": "gpickle", ".pkl": "gpickle",
               ".pickle": "gpickle"}


def _graphml_safe(g):
    """In-place: coerce attribute values GraphML can't serialise (lists, None, other objects).

    GraphML only stores scalars (str/int/float/bool). Lists (e.g. the ``refs`` array) and any other
    object become their ``str``; ``None``-valued keys are dropped (GraphML has no null). Done on a
    shallow copy of each attr dict so the caller's graph is untouched only if they pass a copy — we
    mutate ``g`` directly here, so this is for write-paths that build the graph fresh.
    """
    def fix(d):
        for k in list(d):
            v = d[k]
            if v is None:
                del d[k]
            elif not isinstance(v, (str, int, float, bool)):
                d[k] = str(v)
    for _, data in g.nodes(data=True):
        fix(data)
    # MultiDiGraph edge views accept keys=True; plain DiGraph (the edge graph) does not.
    edges = g.edges(data=True, keys=True) if g.is_multigraph() else g.edges(data=True)
    for *_, data in edges:
        fix(data)


def write_graph(con, path, mode: str = "driving", graph: str = "node", fmt=None,
                weight: str = "time", geometry: str = "wkt"):
    """Build a networkx graph for ``mode`` and write it to ``path``. Returns a small summary dict.

    Parameters
    ----------
    path : output file. ``fmt`` is inferred from its extension when not given.
    graph : ``"node"`` → the geographic node-based ``MultiDiGraph`` (:func:`to_networkx_nodes`,
        full edge info) or ``"edge"`` → the edge-based routing ``DiGraph`` (:func:`to_networkx`).
    fmt : ``"graphml"`` or ``"gpickle"``. Default: inferred from the path extension, else
        ``"graphml"``. GraphML is portable but scalar-only (lists like ``refs`` and shapely
        geometry are stringified, nulls dropped); gpickle round-trips the graph losslessly
        (including shapely objects) but is Python-only.
    weight, geometry : forwarded to the underlying builder (``weight`` only for ``graph="edge"``;
        ``geometry`` only for ``graph="node"``).
    """
    from pathlib import Path
    path = Path(path)
    fmt = fmt or _GRAPH_EXTS.get(path.suffix.lower(), "graphml")
    if fmt not in ("graphml", "gpickle"):
        raise ValueError("fmt must be 'graphml' or 'gpickle'")
    if graph not in ("node", "edge"):
        raise ValueError("graph must be 'node' or 'edge'")
    try:
        import networkx as nx
    except ImportError as e:
        raise ImportError("write_graph needs networkx — `pip install networkx`") from e

    if graph == "node":
        # GraphML can't hold shapely objects; keep geometry as WKT for that format.
        geom = "wkt" if (fmt == "graphml" and geometry == "shapely") else geometry
        g = to_networkx_nodes(con, mode=mode, geometry=geom)
    else:
        g = to_networkx(con, mode=mode, weight=weight)

    path.parent.mkdir(parents=True, exist_ok=True)
    if fmt == "graphml":
        _graphml_safe(g)
        nx.write_graphml(g, path)
    else:
        import pickle
        with open(path, "wb") as f:
            pickle.dump(g, f)
    summary = {"path": str(path), "fmt": fmt, "graph": graph, "mode": mode,
               "n_nodes": g.number_of_nodes(), "n_edges": g.number_of_edges()}
    logger.info(f"wrote {graph} graph[{mode}] -> {path} ({fmt}: "
                f"{summary['n_nodes']:,} nodes, {summary['n_edges']:,} edges)")
    return summary


def _compass_bearing(a, b):
    """Initial compass bearing (deg, 0 = north, clockwise) from ``a`` to ``b`` (``[lng, lat]``)."""
    import math
    lon1, lat1, lon2, lat2 = map(math.radians, (a[0], a[1], b[0], b[1]))
    dlon = lon2 - lon1
    y = math.sin(dlon) * math.cos(lat2)
    x = math.cos(lat1) * math.sin(lat2) - math.sin(lat1) * math.cos(lat2) * math.cos(dlon)
    return round((math.degrees(math.atan2(y, x)) + 360.0) % 360.0, 1)


def node_branches(con, node_ids, mode: str = "driving"):
    """For each node, the LEAVING bearing of every edge whose ``source`` is that node.

    Returns ``{node_id: [(edge_id, bearing_deg), …]}`` — the full branch structure at a junction, so
    guidance can reason about **forks** ("keep left/right" when two branches both go ~forward) and
    **roundabout exits**. Bearing = compass heading over the first ~2 segments of each branch. Same
    source data as :func:`node_out_degree`; no new tables.
    """
    ids = {int(n) for n in node_ids if n is not None}
    if not ids:
        return {}
    rows = con.execute(f"""
        SELECT source, edge_id,
               ST_X(ST_PointN(geometry, 1)), ST_Y(ST_PointN(geometry, 1)),
               ST_X(ST_PointN(geometry, CAST(LEAST(ST_NPoints(geometry), 3) AS INTEGER))),
               ST_Y(ST_PointN(geometry, CAST(LEAST(ST_NPoints(geometry), 3) AS INTEGER)))
        FROM {mode}.edges WHERE source IN ({",".join(map(str, ids))})
    """).fetchall()
    out = {}
    for src, eid, x1, y1, x2, y2 in rows:
        out.setdefault(int(src), []).append((str(eid), _compass_bearing((x1, y1), (x2, y2))))
    return out


def node_out_degree(con, node_ids, mode: str = "driving"):
    """Out-degree of each node in ``<mode>.edges`` — how many edges LEAVE it (``source = node``).

    A junction hint for turn-by-turn guidance: out-degree ``<= 2`` is a **through-node** (the road just
    continues, or a dead-end — no real choice, so a bend there isn't a "turn"); ``>= 3`` is a **genuine
    junction** where turns/forks are worth announcing. Out-degree (not undirected degree) is the right
    measure: a two-way road passing straight through has out-degree 2 (continue + the way back), while a
    3-way junction has 3 — undirected degree would double-count every two-way road.

    Returns ``{node_id: out_degree}`` for the requested nodes (absent nodes omitted). No new tables —
    computed straight from ``source``.
    """
    ids = {int(n) for n in node_ids if n is not None}
    if not ids:
        return {}
    idlist = ",".join(map(str, ids))
    rows = con.execute(
        f"SELECT source, COUNT(*) FROM {mode}.edges WHERE source IN ({idlist}) GROUP BY source"
    ).fetchall()
    return {int(n): int(d) for n, d in rows}


def route(con, from_edge, to_edge, mode: str = "driving", weight: str = "time", graph=None):
    """Shortest route between two edge ids.

    Returns a dict ``{edges, time_s, length_m, path}`` where ``edges`` is the ordered list of
    edge_ids, ``time_s``/``length_m`` are the door-to-door totals (summed over every edge on the
    route), and ``path`` is those edges in order with ``name``/``highway``/``length_m``/``cost_s``/
    ``geometry`` (WKT). Returns ``None`` if the two edges aren't connected.

    Defaults: fastest route (``weight="time"``). Pass a prebuilt ``graph`` (from
    :func:`to_networkx`) to route many times without rebuilding it.
    """
    try:
        import networkx as nx
    except ImportError as e:
        raise ImportError("route needs networkx — `pip install duckosm[routing]`") from e

    g = graph if graph is not None else to_networkx(con, mode=mode, weight=weight,
                                                    node_attrs=False)
    missing = [e for e in (from_edge, to_edge) if e not in g]
    if missing:
        raise ValueError(f"edge id(s) not in the {mode} routing graph: {missing}")
    try:
        path = nx.shortest_path(g, from_edge, to_edge, weight="weight")
    except nx.NetworkXNoPath:
        return None

    # per-edge detail in path order
    try:
        con.execute("INSTALL spatial; LOAD spatial;")
    except Exception:
        pass
    ids = ", ".join(str(e) for e in path)
    detail = {r[0]: r for r in con.execute(
        f"SELECT edge_id, name, highway, length_m, cost_s, ST_AsText(geometry) AS wkt "
        f"FROM {mode}.edges WHERE edge_id IN ({ids})").fetchall()}
    rows = [{"edge_id": e, "name": detail[e][1], "highway": detail[e][2],
             "length_m": detail[e][3], "cost_s": detail[e][4], "geometry": detail[e][5]}
            for e in path]
    return {
        "edges": path,
        "time_s": sum(r["cost_s"] for r in rows if r["cost_s"] is not None),
        "length_m": sum(r["length_m"] for r in rows if r["length_m"] is not None),
        "path": rows,
    }


class Router:
    """Build the routing graph ONCE, then answer many shortest-path queries cheaply.

        r = Router(con)                 # builds the in-memory graph once (time)
        r.route(from_edge, to_edge)     # reuses it -> same dict as routing.route()
        r.route(c, d)                   # no rebuild

    Defaults match :func:`route` (fastest); set ``weight="length"`` per Router. The prebuilt graph
    is on ``.graph``.
    """

    def __init__(self, con, mode: str = "driving", weight: str = "time"):
        self.con = con
        self.mode = mode
        self.weight = weight
        self.graph = to_networkx(con, mode=mode, weight=weight, node_attrs=False)

    def route(self, from_edge, to_edge):
        """Shortest route between two edge ids, reusing the prebuilt graph."""
        return route(self.con, from_edge, to_edge, mode=self.mode, weight=self.weight,
                     graph=self.graph)


# ---- multimodal (intermodal) routing over the mm.* layered graph ------------------------------

# The pedestrian layer every mode-change routes through (matches MultimodalBuilder). A trip is
# walk* (drive|cycle)* walk*: you can only *enter* a vehicle from walking and *leave* it back to
# walking, so every vehicular leg is bracketed by walking.
_HUB_MODE = "walking"


def route_multimodal(con, src_node, dst_node, schema: str = "mm", start_mode: str = "walking",
                     end_mode: str = "walking", enforce_sequence: bool = True, allowed_modes=None):
    """Fastest **intermodal** route between two junction ``node_id``s over the ``mm.*`` graph.

    Where :func:`route` stays in one mode end-to-end, this routes across the *layered* graph built
    by :class:`duckosm.processors.multimodal.MultimodalBuilder` — vertices are ``(node_id, mode)``,
    intra-mode arcs are ``mm.edges`` (weight ``cost_s``) and inter-mode arcs are ``mm.transfers``
    (a transfer penalty in seconds). So a single trip can **switch mode** (walk → drive → walk).

    Everything is weighted in **seconds**, so ``time_s`` is exactly ``Σ edge cost_s + Σ transfer
    cost_s``. A node-based Dijkstra (``heapq``); needs no networkx.

    Parameters
    ----------
    con : DuckDB connection to a db that has an ``mm`` schema (run ``duckosm multimodal`` first).
    src_node, dst_node : OSM junction ``node_id``s (as in ``<mode>.nodes`` / ``mm.edges``).
    schema : the multimodal schema (default ``"mm"``).
    start_mode, end_mode : the mode you begin / end the trip in (default walking on both ends).
    enforce_sequence : when True (default) restrict the trip to ``walk* (drive|cycle)* walk*`` — at
        most one contiguous vehicular segment, entered and left via walking. False = plain layered
        Dijkstra (any mode alternation the transfers allow).
    allowed_modes : optional iterable of modes to restrict the trip to (e.g. ``{"walking", "cycling"}``
        for a **car-free** route). ``None`` = all modes present in the graph. ``start_mode``/``end_mode``
        must be in it.

    Returns a dict::

        { "time_s":    float,                       # door-to-door seconds (edges + transfers)
          "length_m":  float,                       # summed edge length
          "edges":     [(mode, edge_id), …],        # full ordered path
          "legs":      [{"mode","edges","time_s","path"}, …],   # grouped into same-mode legs
          "transfers": [{"node_id","from_mode","to_mode","cost_s","kind"}, …],
          "nodes":     [node_id, …] }               # junction path (consecutive dups collapsed)

    ``None`` if the two nodes aren't connected (e.g. no transfers → the layers are disconnected).
    Raises ``ValueError`` if an endpoint isn't present in its mode's layer.
    """
    import heapq

    try:
        con.execute("INSTALL spatial; LOAD spatial;")
    except Exception:
        pass

    allow = set(allowed_modes) if allowed_modes else None   # None = all modes

    # --- load adjacency (two SQL reads), skipping any mode not in `allow` -----------------------
    adj: dict = {}                       # (mode, node) -> [(nbr_node, edge_id, cost_s)]
    nodes_by_mode: dict = {}             # mode -> {node_id}
    for mode, s, t, eid, cost in con.execute(
            f"SELECT mode, source, target, edge_id, cost_s FROM {schema}.edges").fetchall():
        if allow is not None and mode not in allow:
            continue
        adj.setdefault((mode, s), []).append((t, eid, float(cost) if cost is not None else 0.0))
        nm = nodes_by_mode.setdefault(mode, set())
        nm.add(s)
        nm.add(t)
    tadj: dict = {}                      # (from_mode, node) -> [(to_mode, cost_s, kind)]
    for nid, fm, tm, cost, kind in con.execute(
            f"SELECT node_id, from_mode, to_mode, cost_s, kind FROM {schema}.transfers").fetchall():
        if allow is not None and (fm not in allow or tm not in allow):
            continue
        tadj.setdefault((fm, nid), []).append((tm, float(cost) if cost is not None else 0.0, kind))

    if src_node not in nodes_by_mode.get(start_mode, set()):
        raise ValueError(f"src node {src_node} not in the '{start_mode}' layer of {schema}.edges")
    if dst_node not in nodes_by_mode.get(end_mode, set()):
        raise ValueError(f"dst node {dst_node} not in the '{end_mode}' layer of {schema}.edges")

    # --- Dijkstra over states (node_id, mode, phase) --------------------------------------------
    INF = float("inf")
    init_phase = (0 if start_mode == _HUB_MODE else 1) if enforce_sequence else 0
    start = (src_node, start_mode, init_phase)
    dist = {start: 0.0}
    prev: dict = {}                      # state -> (prev_state, step)
    pq = [(0.0, src_node, start_mode, init_phase)]
    goal = None
    while pq:
        d, node, mode, phase = heapq.heappop(pq)
        state = (node, mode, phase)
        if d > dist.get(state, INF):
            continue
        if node == dst_node and mode == end_mode:
            goal = state
            break
        # intra-mode edges — never change mode/phase
        for nbr, eid, cost in adj.get((mode, node), ()):
            ns = (nbr, mode, phase)
            nd = d + cost
            if nd < dist.get(ns, INF):
                dist[ns] = nd
                prev[ns] = (state, ("edge", eid, mode, cost))
                heapq.heappush(pq, (nd, nbr, mode, phase))
        # inter-mode transfers — same node, change mode (+ enforce the walk* veh* walk* phase)
        for tm, cost, kind in tadj.get((mode, node), ()):
            nphase = phase
            if enforce_sequence:
                entering = mode == _HUB_MODE and tm != _HUB_MODE
                leaving = mode != _HUB_MODE and tm == _HUB_MODE
                if entering:
                    if phase != 0:
                        continue            # already used (or past) the one vehicular segment
                    nphase = 1
                elif leaving:
                    if phase != 1:
                        continue
                    nphase = 2
            ns = (node, tm, nphase)
            nd = d + cost
            if nd < dist.get(ns, INF):
                dist[ns] = nd
                prev[ns] = (state, ("transfer", mode, tm, cost, kind))
                heapq.heappush(pq, (nd, node, tm, nphase))

    if goal is None:
        return None

    # --- reconstruct the ordered state path, then walk the steps between states ------------------
    state_path = [goal]
    s = goal
    while s in prev:
        s = prev[s][0]
        state_path.append(s)
    state_path.reverse()

    edges_seq: list = []                 # (mode, edge_id) in order
    legs: list = []
    transfers: list = []
    nodes: list = [state_path[0][0]]
    for b in state_path[1:]:
        a, step = prev[b]
        if a[0] != nodes[-1]:
            nodes.append(a[0])
        if step[0] == "edge":
            _, eid, mode, cost = step
            edges_seq.append((mode, eid))
            if not legs or legs[-1]["mode"] != mode:
                legs.append({"mode": mode, "edges": [], "time_s": 0.0})
            legs[-1]["edges"].append(eid)
            legs[-1]["time_s"] += cost
        else:
            _, fm, tm, cost, kind = step
            transfers.append({"node_id": a[0], "from_mode": fm, "to_mode": tm,
                              "cost_s": cost, "kind": kind})
        if b[0] != nodes[-1]:
            nodes.append(b[0])

    # --- per-edge detail (name/highway/length/cost/geometry) for each leg ------------------------
    detail: dict = {}
    ids = {e for _, e in edges_seq}
    if ids:
        idlist = ", ".join(str(e) for e in ids)
        for row in con.execute(
                f"SELECT mode, edge_id, name, highway, length_m, cost_s, "
                f"ST_AsText(geometry) AS wkt FROM {schema}.edges "
                f"WHERE edge_id IN ({idlist})").fetchall():
            detail[(row[0], row[1])] = row
    length_m = 0.0
    for leg in legs:
        leg["path"] = []
        for eid in leg["edges"]:
            r = detail.get((leg["mode"], eid))
            if r is None:
                continue
            leg["path"].append({"edge_id": eid, "name": r[2], "highway": r[3],
                                "length_m": r[4], "cost_s": r[5], "geometry": r[6]})
            if r[4] is not None:
                length_m += r[4]

    time_s = sum(leg["time_s"] for leg in legs) + sum(t["cost_s"] for t in transfers)
    logger.info(f"route_multimodal: {src_node} -> {dst_node}: {len(legs)} leg(s), "
                f"{len(transfers)} transfer(s), {time_s:.1f}s")
    return {"time_s": time_s, "length_m": length_m, "edges": edges_seq, "legs": legs,
            "transfers": transfers, "nodes": nodes}
