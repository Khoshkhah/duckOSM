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
