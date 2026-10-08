"""`duckosm route-map`: duckOSM's route planner page over a map drawn by mapstyle
(docs/design/viz_on_mapstyle.md).

Drag a start and an end; the page routes in the browser (Drive, Walk, Cycle, and Walk + drive
when the database has `mm.*` from `duckosm multimodal`), with turn-by-turn directions. The routing
itself is duckOSM's: :func:`duckosm.route_points`, :func:`duckosm.route_multimodal_points` and
:func:`duckosm.directions` give the same routes and steps (a test keeps them equal).
"""
import json
import logging
from pathlib import Path

logger = logging.getLogger("duckosm")

MODES = ("driving", "walking", "cycling")
WARN_EDGES = 100_000


def write_route_map(db, out, mode=None, basemap=None):
    """Write the route planner for the built database ``db`` (a path) to ``out``; return its path.
    ``mode``: the first choice, and the network in front: ``driving``, ``walking``, ``cycling``, or
    ``walk+drive`` (every mode in front; needs ``mm.*`` from ``duckosm multimodal``). Default: the
    first mode of the page, walking in front."""
    import duckdb

    from duckosm.viz import mapstyle_module

    ms = mapstyle_module()
    if mode is not None and mode not in MODES + ("walk+drive",):
        raise ValueError(f"unknown mode {mode!r} (one of {', '.join(MODES)}, walk+drive)")
    con = duckdb.connect(str(db), read_only=True)
    present = {r[0] for r in con.execute(
        "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE table_name = 'edge_graph'").fetchall()}
    has_mm = con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = 'mm' "
                         "AND table_name = 'transfers'").fetchone()[0] > 0
    n = sum(con.execute(f"SELECT count(*) FROM {m}.edges").fetchone()[0] for m in MODES if m in present)
    con.close()
    if mode == "walk+drive" and not ({"walking", "driving"} <= present and has_mm):
        raise ValueError(f"walk+drive needs walking, driving and the mm tables in {db}: "
                         "run `duckosm multimodal` on it")
    if mode in MODES and mode not in present:
        raise ValueError(f"mode {mode!r} has no edge_graph in {db} (present: {sorted(present) or 'none'})")
    if n > WARN_EDGES:
        logger.warning(f"route-map: {n:,} edges make a heavy page; clip an area first "
                       "(duckosm extract) for a lighter one")
    from mapstyle.map import load_roads

    data = _planner_data(db, load_roads(str(db)))
    data["first"] = ["walking", "driving"] if mode == "walk+drive" else [mode] if mode else None
    kw = {"basemap": basemap} if basemap else {}
    front = "all" if mode == "walk+drive" else mode or "walking"
    m = ms.render_map(str(db), mode=front, name=f"{Path(db).stem}: route planner", **kw)
    page = (Path(__file__).parent / "templates" / "planner.html").read_text(encoding="utf-8")
    page = page.replace("__RM__", json.dumps(data, separators=(",", ":")).replace("</", "<\\/"))
    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(m.html.replace("</body>", page + "</body>", 1), encoding="utf-8")
    logger.info(f"route-map: {n:,} edges -> {out}")
    return out


def _planner_data(db, roads):
    """The planner's data (the page's ``RM``): per mode its turn graph (``edge_graph``) over the
    indices of ``roads`` (mapstyle's ``load_roads``: one row per edge_id), walk + drive from ``mm.*``
    when the db has it, and ``eid`` (each index's edge_id, by which the page finds the map's road).
    Node ids become small integers (some pass 2**53). Copied from mapstyle's demo planner."""
    import duckdb

    from duckosm.processors.edge_graph import routed_graph

    con = duckdb.connect(str(db), read_only=True)
    try:
        con.execute("INSTALL spatial; LOAD spatial;")
        have = {tuple(r) for r in con.execute(
            "SELECT table_schema, table_name FROM information_schema.tables").fetchall()}
        modes = [m for m in MODES if (m, "edges") in have and (m, "edge_graph") in have]
        if not modes:
            raise ValueError(f"{db}: no mode with edges + edge_graph to route on")
        k_of = {int(e): k for k, e in enumerate(roads["edge_id"])}
        ends = {}                                   # edge_id -> (source, target, length_m, junction)
        for tb in ("edges", "private_edges"):          # private_edges: drawn, never routed
            for m in MODES:
                if (m, tb) not in have:
                    continue
                jn = "junction" if con.execute(
                    "SELECT count(*) FROM information_schema.columns WHERE table_schema = ? "
                    "AND table_name = ? AND column_name = 'junction'", [m, tb]).fetchone()[0] else "NULL"
                for e, s, t, ln, j in con.execute(
                        f"SELECT edge_id, source, target, length_m, {jn} FROM {m}.{tb}").fetchall():
                    ends.setdefault(e, (s, t, ln, j))
        node_ids = sorted({x for s, t, *_ in ends.values() for x in (s, t)})
        n_of = {n: i for i, n in enumerate(node_ids)}
        row = [ends[int(e)] for e in roads["edge_id"]]
        data = {"modes": modes, "n": len(roads), "eid": [str(e) for e in roads["edge_id"]],
                "src": [n_of[r[0]] for r in row], "tgt": [n_of[r[1]] for r in row], "name": list(roads["name"]),
                "len": [round(r[2] or 0.0, 1) for r in row],
                "hw": [h or "" for h in roads["highway"]],       # for the turn-by-turn directions
                "rb": [k for k, r in enumerate(row) if r[3] in ("roundabout", "circular")],
                "graphs": {}, "mm": None}
        for m in modes:                                        # edge-based graph of legal turns
            es = con.execute(f"SELECT edge_id, cost_s, length_m FROM {m}.edges ORDER BY edge_id").fetchall()
            nxt = {}
            for f, t in con.execute(f"SELECT from_edge, to_edge FROM {routed_graph(con, m)}").fetchall():
                if f in k_of and t in k_of:
                    nxt.setdefault(k_of[f], []).append(k_of[t])
            data["graphs"][m] = {"k": [k_of[e] for e, _, _ in es],
                                 "cost": [round(c or 0.0, 3) for _, c, _ in es],
                                 "len": [round(ln or 0.0, 2) for _, _, ln in es],
                                 "next": [nxt.get(k_of[e], []) for e, _, _ in es]}
        # walk + drive over duckOSM's intermodal graph (`duckosm multimodal`), when it's there;
        # walk + cycle needs bike stations to mean anything (bike anywhere), so not offered
        if {"walking", "driving"} <= set(modes) and ("mm", "edges") in have and ("mm", "transfers") in have:
            mi = {m: i for i, m in enumerate(modes) if m in ("walking", "driving")}
            mm_edges = [[mi[md], n_of[s], n_of[t], k_of[e], round(c or 0.0, 3)]
                        for md, s, t, e, c in con.execute(
                            "SELECT mode, source, target, edge_id, cost_s FROM mm.edges").fetchall()
                        if md in mi and e in k_of and s in n_of and t in n_of]
            mm_tr = [[n_of[n], mi[fm], mi[tm], round(c or 0.0, 3)]
                     for n, fm, tm, c in con.execute(
                         "SELECT node_id, from_mode, to_mode, cost_s FROM mm.transfers").fetchall()
                     if fm in mi and tm in mi and n in n_of]
            data["mm"] = {"edges": mm_edges, "transfers": mm_tr, "nodes": len(node_ids)}
        # the page opens with a route: markers on the roads nearest 30 % and 70 % along the diagonal
        x0, y0, x1, y1 = con.execute(
            f"SELECT min(ST_XMin(geometry)), min(ST_YMin(geometry)), max(ST_XMax(geometry)), "
            f"max(ST_YMax(geometry)) FROM {modes[0]}.edges").fetchone()
        near = (f"SELECT ST_X(p), ST_Y(p) FROM (SELECT ST_LineInterpolatePoint(geometry, 0.5) AS p "
                f"FROM {modes[0]}.edges ORDER BY ST_Distance(ST_Centroid(geometry), ST_Point(?, ?)) LIMIT 1)")
        data["start"], data["end"] = (list(con.execute(near, [x0 + (x1 - x0) * f, y0 + (y1 - y0) * f]).fetchone())
                                      for f in (0.3, 0.7))
    finally:
        con.close()
    return data
