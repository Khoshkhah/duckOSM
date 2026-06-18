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
search (see docs). Service roads are excluded by default; pass `with_service=True` to route over
them (`edge_graph_with_service`).
"""
import logging

logger = logging.getLogger("duckosm")


def to_networkx(con, mode: str = "driving", weight: str = "time", with_service: bool = False):
    """Return a `networkx.DiGraph` of the routing graph, with a `weight` attribute per arc.

    Parameters
    ----------
    con : a DuckDB connection to a built duckOSM database.
    mode : the mode schema to read (default ``"driving"``).
    weight : ``"time"`` → travel time in seconds (`cost_s`, the default) or
        ``"length"`` → length in metres (`length_m`). The chosen value is stored on each arc as
        the ``weight`` attribute, so `nx.shortest_path(G, a, b, weight="weight")` gives the
        fastest / shortest route between two edge ids.
    with_service : use ``edge_graph_with_service`` (includes ``service`` roads) instead of the
        default service-free ``edge_graph``.

    Returns a DiGraph whose nodes are ``edge_id``s.
    """
    try:
        import networkx as nx
    except ImportError as e:
        raise ImportError("to_networkx needs networkx — `pip install networkx`") from e

    graph = f"{mode}.edge_graph_with_service" if with_service else f"{mode}.edge_graph"
    if weight == "time":
        # edge_graph.cost is already the from_edge's travel time (cost_s).
        sql = f"SELECT from_edge, to_edge, cost FROM {graph}"
    elif weight == "length":
        # length of the from_edge; resolve from edges (+ service_edges when routing with service).
        if with_service:
            esrc = (f"(SELECT edge_id, length_m FROM {mode}.edges "
                    f"UNION ALL SELECT edge_id, length_m FROM {mode}.service_edges)")
        else:
            esrc = f"{mode}.edges"
        sql = (f"SELECT g.from_edge, g.to_edge, e.length_m "
               f"FROM {graph} g JOIN {esrc} e ON e.edge_id = g.from_edge")
    else:
        raise ValueError("weight must be 'time' or 'length'")

    g = nx.DiGraph()
    g.add_weighted_edges_from(con.execute(sql).fetchall(), weight="weight")
    logger.info(f"routing graph[{mode}]: {g.number_of_nodes():,} edges, "
                f"{g.number_of_edges():,} arcs, weight={weight}"
                f"{', with service' if with_service else ''}")
    return g


def route(con, from_edge, to_edge, mode: str = "driving", weight: str = "time",
          with_service: bool = False, graph=None):
    """Shortest route between two edge ids.

    Returns a dict ``{edges, time_s, length_m, path}`` where ``edges`` is the ordered list of
    edge_ids, ``time_s``/``length_m`` are the door-to-door totals (summed over every edge on the
    route), and ``path`` is those edges in order with ``name``/``highway``/``length_m``/``cost_s``/
    ``geometry`` (WKT). Returns ``None`` if the two edges aren't connected.

    Defaults: fastest route (``weight="time"``), service roads excluded. Pass a prebuilt ``graph``
    (from :func:`to_networkx`) to route many times without rebuilding it.
    """
    try:
        import networkx as nx
    except ImportError as e:
        raise ImportError("route needs networkx — `pip install duckosm[routing]`") from e

    g = graph if graph is not None else to_networkx(con, mode=mode, weight=weight,
                                                    with_service=with_service)
    missing = [e for e in (from_edge, to_edge) if e not in g]
    if missing:
        raise ValueError(f"edge id(s) not in the {mode} routing graph: {missing}")
    try:
        path = nx.shortest_path(g, from_edge, to_edge, weight="weight")
    except nx.NetworkXNoPath:
        return None

    # per-edge detail in path order (edges + service_edges when routing with service)
    try:
        con.execute("INSTALL spatial; LOAD spatial;")
    except Exception:
        pass
    src = (f"(SELECT * FROM {mode}.edges UNION ALL SELECT * FROM {mode}.service_edges)"
           if with_service else f"{mode}.edges")
    ids = ", ".join(str(e) for e in path)
    detail = {r[0]: r for r in con.execute(
        f"SELECT edge_id, name, highway, length_m, cost_s, ST_AsText(geometry) AS wkt "
        f"FROM {src} WHERE edge_id IN ({ids})").fetchall()}
    rows = [{"edge_id": e, "name": detail[e][1], "highway": detail[e][2],
             "length_m": detail[e][3], "cost_s": detail[e][4], "geometry": detail[e][5]}
            for e in path]
    return {
        "edges": path,
        "time_s": sum(r["cost_s"] for r in rows if r["cost_s"] is not None),
        "length_m": sum(r["length_m"] for r in rows if r["length_m"] is not None),
        "path": rows,
    }
