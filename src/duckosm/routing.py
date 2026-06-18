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
