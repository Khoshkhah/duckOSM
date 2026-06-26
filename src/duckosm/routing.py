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
