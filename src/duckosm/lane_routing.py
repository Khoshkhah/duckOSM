"""
Lane-level routing — a **lane graph** (lane→lane adjacency) + a Dijkstra router over it, so a route is
planned *per lane* (which lane, when to change), not just road-to-road. Mirrors duckOSM's
``edge_graph`` → ``Router`` pattern one level down. See https://khoshkhah.github.io/duckOSM/exports/lane-routing/.

Nodes are ``gmns_<mode>.lane`` rows; edges are **turns** (lane→lane across a junction, from
``movement``) and **lane-changes** (adjacent lanes on a link). ``build_lane_graph`` persists them as
``lane_<mode>.lane_edges``; ``route_lanes`` routes over them (building the graph in-memory if the table
isn't there) → a lane sequence + concatenated geometry + a maneuver list.

    from duckosm import build_lane_graph, route_lanes
    build_lane_graph("sodermalm_pbf_gmns.duckdb")
    r = route_lanes("sodermalm_pbf_gmns.duckdb", from_lane, to_lane)   # {lanes, cost, geometry, maneuvers}
"""
import logging

logger = logging.getLogger("duckosm")

_LC_PEN = 25.0                                          # lane-change penalty (m-equivalent)
_TURN_PEN = {"left": 30.0, "uturn": 60.0, "right": 10.0, "thru": 3.0}
_LABEL = {"left": "left turn", "right": "right turn", "uturn": "U-turn", "lane_change": "lane change"}


def _lc_sql(g):
    return (f"SELECT a.lane_id, b.lane_id, 'lane_change', ST_Length(b.geom) * 111320 + {_LC_PEN} "
            f"FROM {g}.lane a JOIN {g}.lane b ON a.link_id = b.link_id AND abs(a.lane_num - b.lane_num) = 1")


def _turn_sql(g):
    case = " ".join(f"WHEN '{k}' THEN {v}" for k, v in _TURN_PEN.items())
    return (f"SELECT il.lane_id, ol.lane_id, m.type, "
            f"ST_Length(ol.geom) * 111320 + CASE m.type {case} ELSE 15.0 END "
            f"FROM {g}.movement m "
            f"JOIN {g}.lane il ON il.link_id = m.ib_link_id "
            f"  AND (m.start_ib_lane IS NULL OR il.lane_num BETWEEN m.start_ib_lane AND m.end_ib_lane) "
            f"JOIN {g}.lane ol ON ol.link_id = m.ob_link_id "
            f"WHERE m.ib_link_id IS NOT NULL AND m.ob_link_id IS NOT NULL")


def _require_lane(con, g, gmns_db):
    if con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name=? AND table_name='lane'",
                   [g]).fetchone()[0] == 0:
        con.close()
        raise ValueError(f"no '{g}.lane' in {gmns_db} — build a GMNS db first (duckosm gmns)")


def build_lane_graph(gmns_db, mode="driving"):
    """Build ``lane_<mode>.lane_edges(from_lane, to_lane, kind, cost)`` in the GMNS db. Returns counts."""
    import duckdb

    con = duckdb.connect(str(gmns_db))
    con.execute("INSTALL spatial; LOAD spatial;")
    g, ls = f"gmns_{mode}", f"lane_{mode}"
    _require_lane(con, g, gmns_db)
    con.execute(f"DROP SCHEMA IF EXISTS {ls} CASCADE")
    con.execute(f"CREATE SCHEMA {ls}")
    con.execute(f"CREATE TABLE {ls}.lane_edges(from_lane VARCHAR, to_lane VARCHAR, kind VARCHAR, cost DOUBLE)")
    con.execute(f"INSERT INTO {ls}.lane_edges {_lc_sql(g)}")
    con.execute(f"INSERT INTO {ls}.lane_edges {_turn_sql(g)}")
    r = {"lanes": con.execute(f"SELECT count(*) FROM {g}.lane").fetchone()[0],
         "lane_change": con.execute(f"SELECT count(*) FROM {ls}.lane_edges WHERE kind='lane_change'").fetchone()[0],
         "turn": con.execute(f"SELECT count(*) FROM {ls}.lane_edges WHERE kind!='lane_change'").fetchone()[0]}
    r["edges"] = r["lane_change"] + r["turn"]
    con.close()
    logger.info(f"lane graph[{mode}]: {r['lanes']:,} lanes, {r['edges']:,} edges "
                f"({r['turn']:,} turn + {r['lane_change']:,} lane-change) -> {ls}.lane_edges")
    return r


def _coords(wkt):
    b = wkt[wkt.index("(") + 1:wkt.rindex(")")]
    return [tuple(float(v) for v in p.split()[:2]) for p in b.split(",")]


def route_lanes(gmns_db, from_lane, to_lane, mode="driving"):
    """Shortest lane path from ``from_lane`` to ``to_lane`` (each a ``lane_id`` or an ``edge_id`` →
    its lane 1). Returns ``{lanes, cost, geometry, maneuvers}`` (empty/None if unreachable)."""
    import duckdb
    import networkx as nx

    con = duckdb.connect(str(gmns_db), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    g, ls = f"gmns_{mode}", f"lane_{mode}"
    _require_lane(con, g, gmns_db)
    if con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name=? AND table_name='lane_edges'",
                   [ls]).fetchone()[0]:
        edges = con.execute(f"SELECT from_lane, to_lane, kind, cost FROM {ls}.lane_edges").fetchall()
    else:                                              # graph not persisted → build it in-memory
        edges = con.execute(f"{_lc_sql(g)} UNION ALL {_turn_sql(g)}").fetchall()
    lanes = {r[0] for r in con.execute(f"SELECT lane_id FROM {g}.lane").fetchall()}

    def _resolve(x):
        x = str(x)
        return x if x in lanes else (f"{x}_1" if f"{x}_1" in lanes else x)

    src, dst = _resolve(from_lane), _resolve(to_lane)
    G = nx.DiGraph()
    for f, t, kind, cost in edges:
        G.add_edge(f, t, kind=kind, cost=cost)
    if src not in G or dst not in G or not nx.has_path(G, src, dst):
        con.close()
        return {"lanes": [], "cost": None, "geometry": None, "maneuvers": []}
    path = nx.dijkstra_path(G, src, dst, weight="cost")
    cost = nx.dijkstra_path_length(G, src, dst, weight="cost")
    maneuvers = [_LABEL[k] for a, b in zip(path, path[1:])
                 for k in [G[a][b]["kind"]] if k in _LABEL]
    geom = {lid: wkt for lid, wkt in con.execute(
        f"SELECT lane_id, ST_AsText(geom) FROM {g}.lane WHERE lane_id IN "
        f"({','.join('?' * len(path))})", path).fetchall()}
    con.close()
    pts = []
    for lid in path:
        for p in _coords(geom.get(lid, "LINESTRING()")):
            if not pts or pts[-1] != p:
                pts.append(p)
    wkt = "LINESTRING (" + ", ".join(f"{x} {y}" for x, y in pts) + ")" if len(pts) >= 2 else None
    logger.info(f"route_lanes[{mode}]: {src} -> {dst}: {len(path)} lanes, cost {cost:.0f}, "
                f"{len(maneuvers)} maneuvers")
    return {"lanes": path, "cost": cost, "geometry": wkt, "maneuvers": maneuvers}
