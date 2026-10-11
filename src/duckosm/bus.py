"""Bus routes on the driving network: each OSM bus route relation as the ordered, directed driving edges its buses travel.

A route relation lists its ways in travel order (roles '' / forward / backward; its stops and platforms are skipped). Each way is
travelled from the node it shares with the previous way to the node it shares with the next one, so its direction comes from its
neighbours; a member role ``forward`` / ``backward`` decides it where a neighbour cannot (the first or last way of a run). The
route's node pairs are then looked up in the driving edges (``driving.edges``, and ``driving.private_edges`` with ``access = 'bus'``:
bus lanes and bus-only roads), so a two-way street gives its ``#nf`` or ``#nr`` edge by the direction travelled.

Nothing is skipped silently: what cannot be ordered or matched is a row of ``df.attrs["gaps"]``.
"""
from __future__ import annotations

import pandas as pd

ROUTES = ("bus", "trolleybus", "share_taxi")
ROLES = {"": None, "forward": 1, "backward": -1}      # the route's direction on the way: along its drawing, against it
GAP_KINDS = {
    "outside": "the member way is not in the file (outside the area)",
    "role": "a member way with a role that is not '', forward or backward",
    "no_connection": "the way shares no node with the next member way",
    "role_conflict": "the role says one direction, the neighbouring ways the other (the neighbours are followed)",
    "ambiguous": "the first or last way of a run, two-way, no role: its direction cannot be told",
    "no_edge": "no driving edge (nor bus lane) in the direction travelled",
    "private": "the edge travelled is a private road (driving.private_edges, access not bus)",
}


def _oneway(tags):
    """+1 / -1 for a one-way way (along / against its drawing), else None."""
    if tags.get("oneway") == "-1":
        return -1
    if tags.get("oneway") in ("yes", "1", "true") or tags.get("junction") in ("roundabout", "circular"):
        return 1
    return None


def _link(a, b, entry):
    """The node where the route leaves way ``a`` for way ``b``: a shared node, ends of both first, never ``entry`` if another will do."""
    shared = set(a) & set(b)
    if not shared:
        return None
    ends_a, ends_b = {a[0], a[-1]}, {b[0], b[-1]}
    # ponytail: a tie (two ways that share two nodes) takes the first in a's order
    return min(shared, key=lambda n: (not (n in ends_a and n in ends_b), not (n in ends_a or n in ends_b), n == entry, a.index(n)))


def _path(nodes, entry, exit_, role, oneway):
    """The nodes of the way the route travels, in its order: (path, gap kind or None)."""
    want = role or oneway                                    # the direction where the neighbours cannot tell
    if nodes[0] == nodes[-1] and len(nodes) > 2:             # a closed way (a roundabout): from entry round to exit
        if entry is None or exit_ is None or want is None:
            return None, "ambiguous"
        ring = nodes[:-1]
        i, j, k = ring.index(entry), ring.index(exit_), len(ring)
        return [ring[(i + want * s) % k] for s in range(((j - i) * want) % k + 1)], None
    last = len(nodes) - 1
    if entry is not None and exit_ is not None:
        i, j = nodes.index(entry), nodes.index(exit_)
    elif entry is not None:                                  # the last way of a run: on to an end
        i = nodes.index(entry)
        j = last if i == 0 else 0 if i == last else None if want is None else (last if want == 1 else 0)
    elif exit_ is not None:                                  # the first way of a run: from an end
        j = nodes.index(exit_)
        i = 0 if j == last else last if j == 0 else None if want is None else (0 if want == 1 else last)
    else:                                                    # a run of one way
        i, j = (None, None) if want is None else (0, last) if want == 1 else (last, 0)
    if i is None or j is None:
        return None, "ambiguous"
    path = nodes[i:j + 1] if i <= j else nodes[j:i + 1][::-1]
    return path, ("role_conflict" if role and i != j and role != (1 if j > i else -1) else None)


def _walk(members, ways):
    """One relation's member ways [(member, way_id, role)] -> steps [(member, way_id, u, v)] and gaps [(member, way_id, kind)]."""
    steps, gaps, run = [], [], []

    def flush():
        entry = None
        for k, (m, w, role) in enumerate(run):
            nodes, tags = ways[w]
            exit_ = _link(nodes, ways[run[k + 1][1]][0], entry) if k + 1 < len(run) else None
            path, kind = _path(nodes, entry, exit_, role, _oneway(tags))
            if kind:
                gaps.append((m, w, kind))
            if path:
                steps.extend((m, w, u, v) for u, v in zip(path, path[1:]))
            entry = exit_
        run.clear()

    for m, w, role in members:
        if w not in ways:
            flush()
            gaps.append((m, w, "outside"))
            continue
        if run and _link(ways[run[-1][1]][0], ways[w][0], None) is None:
            gaps.append((run[-1][0], run[-1][1], "no_connection"))
            flush()
        run.append((m, w, role))
    flush()
    return steps, gaps


def match_bus_routes(con):
    """The driving edges every bus route relation in ``raw.relations`` travels (``route`` = bus, trolleybus, share_taxi), in order.

    One row per route and edge: ``relation_id``, ``route``, ``ref``, ``name``, ``from``, ``to``, ``operator``, ``seq`` (1, 2, … along
    the route), ``member`` (the way's position in the relation's member list, 1-based), ``way_id``, ``edge_id``, ``edge_ref`` and
    ``bus_lane`` (the edge is in ``driving.private_edges``: a bus lane or a bus-only road). ``df.attrs["gaps"]``: one row per member
    way (or run of node pairs of one way) the route could not be ordered or matched on, ``relation_id``, ``ref``, ``member``,
    ``way_id``, ``kind`` (``GAP_KINDS``)."""
    rels = con.execute(f"""
        SELECT osm_id, tags, refs, ref_roles, ref_types FROM raw.relations
        WHERE tags['route'] IN {ROUTES} ORDER BY osm_id""").fetchall()
    ways = {w: (list(r), dict(t)) for w, r, t in con.execute(f"""
        SELECT w.osm_id, w.refs, w.tags FROM raw.ways w
        WHERE w.osm_id IN (SELECT unnest(refs) FROM raw.relations WHERE tags['route'] IN {ROUTES})""").fetchall()}

    info, steps, gaps = [], [], []
    for rid, tags, refs, roles, types in rels:
        tags = dict(tags)
        info.append((rid, tags.get("route"), tags.get("ref"), tags.get("name"), tags.get("from"), tags.get("to"), tags.get("operator")))
        members = []
        for m, (ref, role, typ) in enumerate(zip(refs, roles, types, strict=True), start=1):
            role = role or ""
            if typ != "way" or role.startswith(("platform", "stop")):
                continue                                     # a stop or a platform, not the road
            if role not in ROLES:
                gaps.append((rid, m, ref, "role"))
                continue
            members.append((m, ref, ROLES[role]))
        s, g = _walk(members, ways)
        steps += [(rid, i, *x) for i, x in enumerate(s)]
        gaps += [(rid, *x) for x in g]

    step_df = pd.DataFrame(steps, columns=["relation_id", "step", "member", "way_id", "u", "v"])
    con.register("_bus_steps", step_df)
    try:
        matched = con.execute("""
            WITH e AS (
                SELECT edge_id, edge_ref, osm_id, refs, false AS bus_lane, NULL AS access FROM driving.edges
                UNION ALL SELECT edge_id, edge_ref, osm_id, refs, true, access FROM driving.private_edges),
            pairs AS (SELECT edge_id, edge_ref, osm_id, bus_lane, access, unnest(refs[:-1]) AS u, unnest(refs[2:]) AS v FROM e)
            SELECT s.*, p.edge_id, p.edge_ref, p.bus_lane, p.access
            FROM _bus_steps s LEFT JOIN pairs p ON p.osm_id = s.way_id AND p.u = s.u AND p.v = s.v
            ORDER BY s.relation_id, s.step""").df()
    finally:
        con.unregister("_bus_steps")
    if len(matched) != len(step_df):
        raise RuntimeError("a node pair of a way lies on two driving edges in the same direction: the route cannot pick one")

    private = matched["bus_lane"].fillna(False).astype(bool) & (matched["access"] != "bus")
    bad = matched["edge_id"].isna() | private
    kind = pd.Series("no_edge", index=matched.index).where(~private, "private")
    miss = matched[bad].assign(kind=kind[bad])
    miss = miss[(miss[["relation_id", "member", "kind"]] != miss[["relation_id", "member", "kind"]].shift()).any(axis=1)]   # one row per run
    gaps += list(miss[["relation_id", "member", "way_id", "kind"]].itertuples(index=False, name=None))

    ok = matched[~bad]
    ok = ok[(ok[["relation_id", "edge_id"]] != ok[["relation_id", "edge_id"]].shift()).any(axis=1)]   # an edge once per stretch travelled
    ok = ok.assign(seq=ok.groupby("relation_id").cumcount() + 1)
    meta = pd.DataFrame(info, columns=["relation_id", "route", "ref", "name", "from", "to", "operator"])
    df = meta.merge(ok[["relation_id", "seq", "member", "way_id", "edge_id", "edge_ref", "bus_lane"]], on="relation_id")
    df["edge_id"] = df["edge_id"].astype("int64")
    df["bus_lane"] = df["bus_lane"].astype(bool)
    g = pd.DataFrame(gaps, columns=["relation_id", "member", "way_id", "kind"]).sort_values(["relation_id", "member"], kind="stable")
    df.attrs["gaps"] = meta[["relation_id", "ref"]].merge(g, on="relation_id")[["relation_id", "ref", "member", "way_id", "kind"]].reset_index(drop=True)
    return df


def write_bus_routes(con):
    """Store the bus routes in schema ``bus`` (docs/concepts/networks.md#bus-routes), replacing what is there; no geometry copied:
    ``bus.routes`` (one row per route relation: ``osm_id``, ``route``, ``ref``, ``name``, ``operator``, ``from``, ``to``, ``network``,
    ``edges``, ``outside`` (member ways outside the file) and ``gaps`` (``'<way_id> <kind>'`` for the rest, empty when the route is
    matched fully)) and ``bus.route_edges`` (``route_id``, ``ref``, ``seq``, ``edge_id``, ``edge_ref``, ``bus_lane``), which points at
    ``driving.edges`` / ``driving.private_edges``. Returns ``bus.routes`` as a DataFrame."""
    df = match_bus_routes(con)
    g = df.attrs["gaps"]
    routes = con.execute(f"""
        SELECT osm_id, tags['route'] AS route, tags['ref'] AS ref, tags['name'] AS name, tags['operator'] AS operator,
               tags['from'] AS "from", tags['to'] AS "to", tags['network'] AS network
        FROM raw.relations WHERE tags['route'] IN {ROUTES} ORDER BY osm_id""").df()
    routes["edges"] = routes["osm_id"].map(df.groupby("relation_id").size()).fillna(0).astype("int32")
    routes["outside"] = routes["osm_id"].map(g[g.kind == "outside"].groupby("relation_id").size()).fillna(0).astype("int32")
    inside = g[g.kind != "outside"]
    listed = inside.assign(t=inside.way_id.astype(str) + " " + inside.kind).groupby("relation_id")["t"].agg(list)
    routes["gaps"] = [listed.get(r, []) for r in routes["osm_id"]]
    edges = df.rename(columns={"relation_id": "route_id"})[["route_id", "ref", "seq", "edge_id", "edge_ref", "bus_lane"]]
    con.execute("CREATE SCHEMA IF NOT EXISTS bus")
    con.register("_routes", routes)
    con.register("_route_edges", edges)
    try:
        con.execute("CREATE OR REPLACE TABLE bus.routes AS SELECT * REPLACE (gaps::VARCHAR[] AS gaps) FROM _routes")
        con.execute("CREATE OR REPLACE TABLE bus.route_edges AS SELECT * REPLACE (seq::INTEGER AS seq) FROM _route_edges")
    finally:
        con.unregister("_routes")
        con.unregister("_route_edges")
    return routes
