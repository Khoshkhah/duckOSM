"""
SUMO exporter — turn a built duckOSM database into a SUMO network whose edges keep their duckOSM
``edge_id`` AND whose junction movements honour the turn restrictions.

The DB stores a directed-edge road network (`<mode>.edges` + `<mode>.nodes`) plus the legal-turn
line graph (`<mode>.edge_graph`, with illegal turns / OSM turn restrictions already removed).
`to_sumo` writes SUMO *plain-XML* —

  * `.nod.xml` — nodes (junction positions),
  * `.edg.xml` — edges, ``id`` = ``edge_id`` (1:1), with lanes / speed / true length / shape, and
  * `.con.xml` — the legal ``from_edge -> to_edge`` successors taken from ``edge_graph`` —

plus a standard **netconvert config** (`.netccfg`), then runs ``netconvert -c <net>.netccfg`` to
assemble the `.net.xml`. Because netconvert keeps the ids and the explicit connections:

  * every SUMO edge id equals the duckOSM ``edge_id`` (per-edge data maps on by identity), and
  * the only movements allowed at each junction are the legal successors in ``edge_graph`` — i.e.
    **turn restrictions are respected** (netconvert is not left to guess connections from geometry).

Painted turn arrows (``turn:lanes``): SUMO's plain XML has no field for them; only its OSM import reads them (``osm.turn-lanes``).
GMNS passes the arrows that fit their link's exits in ``lanes=`` (``gmns_sumo._arrow_fit``); for an edge whose lanes have one, netconvert first reads the same network as an OSM file (our lanes as
tags, our road types: every vehicle class, the plain XML's priorities) and its lane connections from those edges go into the
``.con.xml`` as explicit lane-to-lane connections; every other edge keeps the edge-level successors (``_arrow_connections``).

    import duckdb
    from duckosm.sumo import to_sumo
    con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)
    out = to_sumo(con, "sumo/")            # sumo/network.{nod,edg,con}.xml + .netccfg + .net.xml

The netconvert options come from a built-in default (``DEFAULT_NETCFG``); pass ``config=`` to
override them — a dict of ``{netconvert-option: value}`` merged onto the default, or a path to your
own ``.netccfg`` to drive netconvert directly. netconvert ships with SUMO (``pip install
duckosm[sumo]``). Coordinates are geographic; the default config projects them to the UTM zone of
the data (``--proj.utm``).
"""
import logging
import os
import shutil
import subprocess
from xml.sax.saxutils import quoteattr

logger = logging.getLogger("duckosm")

# which SUMO vehicle classes may use an edge of a non-driving mode (driving: SUMO's default, all)
_MODE_ALLOW = {"walking": "pedestrian", "cycling": "bicycle"}

# the SUMO vehicle classes of a lane of driving.lane_profile, by its use (an `auto` lane: SUMO's default, every road vehicle)
_LANE_ALLOW = {"bus": "bus", "bike": "bicycle", "bus,bike": "bus bicycle"}

# right-of-way hint for netconvert's junction building (explicit connections still gate movements).
_HIGHWAY_PRIORITY = {
    "motorway": 6, "motorway_link": 6, "trunk": 5, "trunk_link": 5,
    "primary": 4, "primary_link": 4, "secondary": 3, "secondary_link": 3,
    "tertiary": 2, "tertiary_link": 2, "unclassified": 1, "residential": 1,
    "living_street": 1, "service": 0,
}

# OSM vehicle classes (except=, restriction:<vehicle>) -> SUMO vehicle classes (docs/design/turn_permissions.md)
_SUMO_CLASS = {"psv": "bus coach taxi", "bus": "bus coach", "taxi": "taxi", "bicycle": "bicycle", "hgv": "truck trailer",
               "goods": "delivery", "emergency": "emergency", "motorcar": "passenger", "motorcycle": "motorcycle",
               "moped": "moped", "hov": "hov", "agricultural": "agricultural", "motor_vehicle":
               "passenger bus coach taxi truck trailer delivery motorcycle moped hov emergency evehicle"}


def _classes(osm):
    """SUMO vehicle classes of an OSM ';'-separated vehicle list (unknown ones left out)."""
    return {c for v in (osm or "").split(";") for c in _SUMO_CLASS.get(v.strip(), "").split()}


def _apply_permissions(cs, perm):
    """Connections as (from, to, allow, disallow): allow None = every vehicle, else the SUMO classes only; disallow: classes kept
    out. `perm`: <mode>.turn_permission rows (from, to, allowed, vehicles, except_vehicles, condition). SUMO has no time: a
    conditional ban is written in force."""
    conn = {(fe, te): [None, set()] for fe, te in cs}
    for fe, te, allowed, veh, exc, cond in perm:
        if allowed:                                     # closed to all traffic, open to these
            if (fe, te) not in conn:
                conn[(fe, te)] = [set(), set()]
            if conn[(fe, te)][0] is not None:
                conn[(fe, te)][0] |= _classes(veh)
        elif veh is None and (fe, te) in conn:          # closed to all (but `exc`) while `cond` holds
            if exc and _classes(exc):
                conn[(fe, te)] = [_classes(exc), set()]
            else:
                del conn[(fe, te)]
        elif (fe, te) in conn:                          # closed to one class
            conn[(fe, te)][1] |= _classes(veh)
    return [(fe, te, frozenset(a) if a is not None else None, frozenset(d)) for (fe, te), (a, d) in conn.items()
            if a is None or a]


# a walking / cycling edge is one path, not a road: its width (netconvert's default lane width is 3.2 m)
_MODE_LANEWIDTH = {"walking": 2.0, "cycling": 1.5}

# Default netconvert options, written into the generated `.netccfg` (standard SUMO config format).
# Tuned to keep the duckOSM topology verbatim: 1:1 edge ids (no merge), explicit turns honoured.
DEFAULT_NETCFG = {
    "proj.utm": "true",                      # node/edge coords are lon/lat: project them to UTM
    "geometry.remove": "false",              # do NOT merge degree-2 ways -> edge_id stays 1:1
    "junctions.join": "false",               # do NOT merge nearby nodes
    "no-turnarounds.except-deadend": "true",  # honour the explicit topology (no invented U-turns)
    "numerical-ids": "false",                # keep our ids verbatim
}


def _find_netconvert(explicit=None):
    """Locate netconvert: explicit → eclipse-sumo wheel → sumolib → $SUMO_HOME/bin → PATH.

    Importing the ``sumo`` (eclipse-sumo) package also sets ``$SUMO_HOME``, which netconvert needs
    at runtime for its bundled type maps — so the subprocess inherits it.
    """
    if explicit:
        return explicit
    try:
        import sumo
        cand = os.path.join(os.path.dirname(sumo.__file__), "bin", "netconvert")
        if os.path.exists(cand):
            os.environ.setdefault("SUMO_HOME", os.path.dirname(sumo.__file__))
            return cand
    except Exception:
        pass
    try:
        import sumolib
        return sumolib.checkBinary("netconvert")
    except Exception:
        pass
    sumo_home = os.environ.get("SUMO_HOME")
    if sumo_home:
        cand = os.path.join(sumo_home, "bin", "netconvert")
        if os.path.exists(cand):
            return cand
    found = shutil.which("netconvert")
    if found:
        return found
    raise FileNotFoundError(
        "netconvert not found. Install SUMO (system package) or `pip install duckosm[sumo]`, "
        "or pass netconvert_bin=..., or run with run_netconvert=False to emit only plain-XML.")


def _linestring_points(wkt):
    """'LINESTRING(lon lat, ...)' -> 'lon,lat lon,lat ...' for a SUMO edge shape (or None)."""
    if not isinstance(wkt, str) or "(" not in wkt:
        return None
    body = wkt[wkt.index("(") + 1:wkt.rindex(")")]
    pts = []
    for p in body.split(","):
        xy = p.split()
        if len(xy) >= 2:
            pts.append(f"{float(xy[0])},{float(xy[1])}")
    return " ".join(pts) if pts else None


def _table_exists(con, qualified):
    """True if a ``schema.table`` exists in the connection."""
    schema, _, table = qualified.partition(".")
    try:
        return con.execute(
            "SELECT count(*) FROM information_schema.tables "
            "WHERE table_schema = ? AND table_name = ?", [schema, table]).fetchone()[0] > 0
    except Exception:
        return False


def _write_netccfg(cfg_path, nod, edg, con_xml, net, opts):
    """Write a standard SUMO netconvert config (`.netccfg`) wiring the inputs/output + options.

    Paths are written relative to the config's own folder: that is how SUMO resolves them (a path
    relative to the working directory, e.g. ``sumo/x.nod.xml`` for out_dir ``sumo``, made netconvert
    look for ``sumo/sumo/x.nod.xml`` and fail with "No nodes loaded")."""
    here = os.path.dirname(os.path.abspath(cfg_path))
    rel = lambda p: os.path.relpath(os.path.abspath(p), here)
    lines = ['<?xml version="1.0" encoding="UTF-8"?>', "<configuration>",
             f'  <node-files value="{rel(nod)}"/>', f'  <edge-files value="{rel(edg)}"/>']
    if con_xml:
        lines.append(f'  <connection-files value="{rel(con_xml)}"/>')
    lines.append(f'  <output-file value="{rel(net)}"/>')
    for key, val in opts.items():
        lines.append(f'  <{key} value="{val}"/>')
    lines += ["</configuration>", ""]
    with open(cfg_path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))


def _joined_groups(path):
    """The node groups netconvert joins into one junction (its ``--junctions.join-output``): [[node id, ...]]."""
    import xml.etree.ElementTree as ET
    return [j.get("nodes").split() for j in ET.parse(path).getroot().iter("join")]


def _merge_joined(nodes, rows, cs, stuck, groups, prules=()):
    """Merge each group of nodes into one junction node ``cluster_<ids>`` (netconvert's own name), at their mean position. An edge
    between two nodes of one group is gone (netconvert drops it too); every legal path from an edge entering a group through such
    edges to an edge leaving it (successors in ``cs``, no node twice) becomes one connection, open to the vehicles every step lets
    through and closed to those any step keeps out. Returns nodes, rows (with each edge's
    original source and target appended), connections, from-only connections, and how many via-way restriction paths (``prules``:
    type, from edge, via edges, to edge, except, vehicle class) applied: a path whose via edges the group swallowed is one connection,
    banned (no_*: onto its to edge; only_*: onto any other) for all but the exempt, or for its one vehicle class."""
    of = {n: "cluster_" + "_".join(g) for g in groups for n in g}
    pos = {n: (x, y) for n, x, y in nodes}
    merged = [(n, x, y) for n, x, y in nodes if str(n) not in of]
    for g in groups:
        ps = [pos[int(n)] for n in g if int(n) in pos]
        if ps:
            merged.append((of[g[0]], sum(p[0] for p in ps) / len(ps), sum(p[1] for p in ps) / len(ps)))
    j = lambda n: of.get(str(n), n)
    inside = {r[0] for r in rows if str(r[1]) in of and j(r[1]) == j(r[2])}
    rows2 = [(r[0], j(r[1]), j(r[2]), *r[3:9], r[1], r[2]) for r in rows if r[0] not in inside]
    ends = {r[0]: (r[1], r[2]) for r in rows}
    succ = {}
    for fe, te, allow, disallow in cs:
        succ.setdefault(fe, []).append((te, allow, disallow))
    both = lambda a, b: b if a is None else (a if b is None else a & b)     # vehicles allowed on both steps
    cs2 = {}

    def keep(fe, te, allow, disallow):      # one move reached along two paths: open to whoever either path lets through
        if (fe, te) in cs2:
            a0, d0 = cs2[(fe, te)]
            allow, disallow = (None if a0 is None or allow is None else a0 | allow), d0 & disallow
        cs2[(fe, te)] = (allow, disallow)
    by_path = {}
    for rtype, pfe, via, pte, exc, veh in prules:
        by_path.setdefault((pfe, tuple(via)), []).append((rtype, pte, exc, veh))
    applied = set()

    def path_rules(fe, inner, nxt, a, d):   # a via-way restriction on exactly this path: (allow, disallow), or None if banned to all
        for rtype, pte, exc, veh in by_path.get((fe, tuple(inner)), []):
            if (rtype.startswith("no_") and pte == nxt) or (rtype.startswith("only_") and pte != nxt):
                applied.add((fe, tuple(inner)))
                if veh:
                    d = d | _classes(veh)
                elif exc and _classes(exc):
                    a = both(a, frozenset(_classes(exc)))
                else:
                    return None
        return a, d
    for fe, te, allow, disallow in cs:
        if fe in inside:
            continue
        if te not in inside:
            keep(fe, te, allow, disallow)
            continue
        todo = [(te, {ends[fe][1], ends[te][1]}, allow, disallow, [te])]  # through the swallowed edges, never back to a node passed
        while todo:
            e, seen, a, d, inner = todo.pop()
            for nxt, a2, d2 in succ.get(e, []):
                if nxt in inside:
                    if ends[nxt][1] not in seen:
                        todo.append((nxt, seen | {ends[nxt][1]}, both(a, a2), d | d2, inner + [nxt]))
                elif ends[nxt][1] not in seen and ends[nxt][1] != ends[fe][0]:      # not a way back where it came from
                    ad = path_rules(fe, inner, nxt, both(a, a2), d | d2)
                    if ad is not None:
                        keep(fe, nxt, *ad)
    cs2 = [(fe, te, a, d) for (fe, te), (a, d) in sorted(cs2.items()) if a is None or a]
    has = {c[0] for c in cs2}
    stuck2 = [(fe,) for (fe,) in stuck if fe not in inside] + [(r[0],) for r in rows2 if r[0] not in has and r[1] != r[2]
                                                                and any(c[0] == r[0] for c in cs)]
    return merged, rows2, cs2, stuck2, len(applied)


def _arrow_connections(con, mode, bus, rows, profile, arrowed, cs, nodes, out_dir, net_name, binary, config):
    """The lane connections netconvert's OSM import builds from the edges in ``arrowed`` (their lanes have turn arrows), as
    ``{edge_id: [(fromLane, to_edge, toLane)]}`` in SUMO's lane indices (from the right). The network is written as an OSM file
    ``<net_name>.osm``: a forward edge is a way with its id (its reverse edge on the same line, the way's backward direction: SUMO's
    ``-<id>``), its lanes as tags (``lanes``, ``turn:lanes``, ``width:lanes``, ``access:lanes`` / ``bus:lanes`` / ``bicycle:lanes``);
    the road types ``<net_name>.osm.typ.xml``: every vehicle class, the plain XML's priorities; the legal successors as there."""
    import xml.etree.ElementTree as ET
    from xml.sax.saxutils import quoteattr as q
    where = f"SELECT edge_id, osm_id, is_reverse FROM {mode}.edges" + (" UNION ALL SELECT edge_id, osm_id, is_reverse FROM driving.private_edges "
                                                                         "WHERE access = 'bus'" if bus else "")
    way_of = {e: (osm, bool(rev)) for e, osm, rev in con.execute(where).fetchall()}
    by_ends = {(r[1], r[2], way_of[r[0]][0]): r for r in rows if r[1] != r[2]}
    back = {}                                            # a forward edge -> the reverse edge on its line (the way's backward lanes)
    for r in by_ends.values():
        p = by_ends.get((r[2], r[1], way_of[r[0]][0]))
        if way_of[r[0]][1] and p and not way_of[p[0]][1]:
            back[p[0]] = r[0]
    reverse = set(back.values())
    sumo_id = {}                                         # SUMO's edge id -> our edge_id

    def lane_tags(ls, sfx):
        t = {f"lanes{sfx}": str(len(ls)), f"width:lanes{sfx}": "|".join(f"{float(w):.2f}" for _, w, _ in ls)}
        if any(x[2] for x in ls):
            t[f"turn:lanes{sfx}"] = "|".join(x[2] or "" for x in ls)
        if any(u != "auto" for u, _, _ in ls):           # a bus or bike lane: no one else, as in the plain XML's lane allow
            t[f"access:lanes{sfx}"] = "|".join("yes" if u == "auto" else "no" for u, _, _ in ls)
            for k, v in (("bus", "bus"), ("bicycle", "bike")):
                t[f"{k}:lanes{sfx}"] = "|".join("designated" if v in u else "yes" if u == "auto" else "no" for u, _, _ in ls)
        return t

    osm_path = os.path.join(out_dir, f"{net_name}.osm")
    shape_id = 10**15
    with open(osm_path, "w", encoding="utf-8") as f:
        f.write('<?xml version="1.0" encoding="UTF-8"?>\n<osm version="0.6">\n')
        for n, (x, y) in nodes.items():
            f.write(f'  <node id="{n}" lat="{y:.7f}" lon="{x:.7f}"/>\n')
        ways = []
        for eid, a, b, highway, name, _, spd, _, wkt in by_ends.values():
            if eid in reverse:
                continue
            refs = [a]
            for pt in (_linestring_points(wkt) or "").split()[1:-1]:
                shape_id += 1
                while shape_id in nodes:
                    shape_id += 1
                x, y = pt.split(",")
                f.write(f'  <node id="{shape_id}" lat="{y}" lon="{x}"/>\n')
                refs.append(shape_id)
            refs.append(b)
            tags = {"highway": (highway or "road").split(";")[0]}
            if name:
                tags["name"] = name
            if spd and float(spd) > 0:
                tags["maxspeed"] = str(round(float(spd)))
            sumo_id[str(eid)] = eid
            if eid in back:
                tags.update({**lane_tags(profile[eid], ":forward"), **lane_tags(profile[back[eid]], ":backward"), "oneway": "no",
                             "lanes": str(len(profile[eid]) + len(profile[back[eid]]))})
                sumo_id[f"-{eid}"] = back[eid]
            else:
                tags.update({**lane_tags(profile[eid], ""), "oneway": "yes"})
            ways.append(f'  <way id="{eid}">' + "".join(f'<nd ref="{n}"/>' for n in refs)
                        + "".join(f"<tag k={q(k)} v={q(v)}/>" for k, v in tags.items()) + "</way>\n")
        f.write("".join(ways) + "</osm>\n")
    ours = {v: k for k, v in sumo_id.items()}
    typ_path = os.path.join(out_dir, f"{net_name}.osm.typ.xml")
    with open(typ_path, "w", encoding="utf-8") as f:
        f.write('<?xml version="1.0" encoding="UTF-8"?>\n<types>\n')
        for hw in sorted({(r[3] or "road").split(";")[0] for r in rows}):
            f.write(f'  <type id="highway.{hw}" priority="{_HIGHWAY_PRIORITY.get(hw, 1)}" numLanes="1" speed="13.89" oneway="false"/>\n')
        f.write("</types>\n")
    con_path = os.path.join(out_dir, f"{net_name}.osm.con.xml")
    has = {a for a, _ in cs}
    with open(con_path, "w", encoding="utf-8") as f:
        f.write('<?xml version="1.0" encoding="UTF-8"?>\n<connections>\n')
        for a, b in cs:
            if a in ours and b in ours:
                f.write(f'  <connection from="{ours[a]}" to="{ours[b]}"/>\n')
        for e in ours:
            if e not in has:
                f.write(f'  <connection from="{ours[e]}"/>\n')
        f.write("</connections>\n")
    net_path = os.path.join(out_dir, f"{net_name}.osm.net.xml")
    osm_opts = ["--osm-files", osm_path, "--type-files", typ_path, "--connection-files", con_path, "--output-file", net_path,
                "--osm.turn-lanes", "true", "--osm.lane-access", "true"]
    if isinstance(config, str):
        cmd = [binary, "-c", config, *osm_opts]
    else:
        opts = {**DEFAULT_NETCFG, **(config or {})}
        cmd = [binary, *osm_opts, *[x for k, v in opts.items() for x in (f"--{k}", str(v))]]
    res = subprocess.run(cmd, capture_output=True, text=True)
    if res.returncode != 0:
        raise RuntimeError(f"netconvert (OSM input, for the turn arrows) failed (exit {res.returncode}):\n{res.stderr[-2000:]}")
    got = {e: [] for e in arrowed if e in ours}
    for _, el in ET.iterparse(net_path):
        if el.tag == "edge" and el.get("function") != "internal" and el.get("id") not in sumo_id:
            raise RuntimeError(f"netconvert's OSM import built edge {el.get('id')!r}, not one of ours (a way it split): cannot map it back")
        if el.tag == "connection" and not el.get("from").startswith(":") and sumo_id[el.get("from")] in got:
            got[sumo_id[el.get("from")]].append((int(el.get("fromLane")), sumo_id[el.get("to")], int(el.get("toLane"))))
    logger.info(f"SUMO connections[{mode}]: {len(got)} edges with turn arrows connected lane by lane by netconvert's OSM import ({net_path})")
    return got


def to_sumo(con, out_dir, mode: str = "driving", net_name: str = "network",
            run_netconvert: bool = True, netconvert_bin: str = None,
            connections: bool = True, config=None, bus_edges: bool = False, edge_attrs: dict = None, lanes: dict = None):
    """Export the duckOSM network to SUMO, preserving ``edge_id`` and the turn restrictions.

    Parameters
    ----------
    con : a DuckDB connection to a built duckOSM database (spatial extension is loaded here).
    out_dir : directory for the output files (created if needed).
    mode : the mode schema to read (default ``"driving"``).
    net_name : basename for the outputs (``<net_name>.nod.xml`` / ``.edg.xml`` / ``.con.xml`` /
        ``.netccfg`` / ``.net.xml``).
    run_netconvert : if True (default) assemble the ``.net.xml`` with netconvert; if False only the
        plain-XML (+ the ``.netccfg``) are written.
    netconvert_bin : explicit path to netconvert (else auto-located).
    connections : if True (default) emit ``.con.xml`` from ``<mode>.edge_graph`` so junction
        movements are restricted to the legal successors (turn restrictions honoured); falls back to
        netconvert inference (with a warning) if the edge_graph table is absent.
    config : netconvert configuration. ``None`` → the built-in :data:`DEFAULT_NETCFG`; a ``dict`` of
        ``{netconvert-option: value}`` is merged onto the default; a ``str`` path to a ``.netccfg``
        is used as-is (inputs/output are still set by this function). The effective config is written
        in SUMO's standard ``.netccfg`` format and run with ``netconvert -c``.
    edge_attrs : ``{edge_id: {sumo-edge-attribute: value}}`` set on those edges, over what duckOSM writes: e.g. lane counts,
        ``{eid: {"numLanes": 2, "width": 3.1}}`` (urbanstyle's measured lane counts and widths).
    lanes : ``{edge_id: [(use, width[, turn]), ...]}`` left to right, each edge's lanes instead of ``driving.lane_profile``'s (GMNS gives
        its own, in every mode, so SUMO's lanes are GMNS's one to one); ``use`` as the profile's (``auto`` / ``bus`` / ``bike`` /
        ``bus,bike``); ``turn`` its arrow (``turn:lanes`` entry), applied by netconvert's OSM import (see the module docstring).
    bus_edges : driving only: also the bus-only edges (``private_edges`` with ``access = 'bus'``: a contraflow bus lane, a bus-only
        road) and the buses' turns of ``edge_graph`` (its ``bus`` rows), so their lanes get connections too (GMNS, docs/design/gmns_lane_profile.md).

    Returns ``{"nod","edg","con"(if written),"netccfg"(if built),"net"(if built),"n_nodes",
    "n_edges","n_connections"}``. Every (non-internal) SUMO edge id equals the duckOSM ``edge_id``.
    """
    try:
        con.execute("INSTALL spatial; LOAD spatial;")
    except Exception:
        pass
    os.makedirs(out_dir, exist_ok=True)
    nod_path = os.path.join(out_dir, f"{net_name}.nod.xml")
    edg_path = os.path.join(out_dir, f"{net_name}.edg.xml")
    con_path = os.path.join(out_dir, f"{net_name}.con.xml")
    net_path = os.path.join(out_dir, f"{net_name}.net.xml")

    # ---- read the network: nodes (geographic; netconvert projects them), edges, legal successors ----
    nodes = con.execute(
        f"SELECT node_id, ST_X(geom), ST_Y(geom) FROM {mode}.nodes WHERE geom IS NOT NULL").fetchall()
    bus = bus_edges and mode == "driving" and _table_exists(con, "driving.private_edges")
    if bus:
        # a bus-only edge's end that no driving edge reaches (a bus-only road's far end): its own line's end point
        have = {n for n, _, _ in nodes}
        for n, x, y in con.execute("""SELECT source, ST_X(ST_StartPoint(geometry)), ST_Y(ST_StartPoint(geometry)) FROM driving.private_edges WHERE access = 'bus'
                                      UNION ALL SELECT target, ST_X(ST_EndPoint(geometry)), ST_Y(ST_EndPoint(geometry)) FROM driving.private_edges WHERE access = 'bus'""").fetchall():
            if n not in have:
                nodes.append((n, x, y))
                have.add(n)
    cols = "edge_id, source, target, highway, name, lanes, maxspeed_kmh, length_m, ST_AsText(geometry)"
    rows = con.execute(f"SELECT {cols} FROM {mode}.edges" + (f" UNION ALL SELECT {cols} FROM driving.private_edges WHERE access = 'bus'" if bus else "")).fetchall()
    both_ways = {(r[1], r[2]) for r in rows}
    extra = {str(k): v for k, v in (edge_attrs or {}).items()}      # keyed by edge_id, as a number or as text
    # each edge's lanes from driving.lane_profile (docs/design/gmns_lane_profile.md), left to right: one <lane> each with its vehicle
    # classes and width, so netconvert gets the lanes duckOSM decided (as its own OSM import would get them from the tags), not a count
    profile = {}
    if lanes is not None:
        profile = {int(k): [(*x, None)[:3] for x in v] for k, v in lanes.items()}
    elif mode == "driving" and _table_exists(con, "driving.lane_profile"):
        for eid, use, width in con.execute("SELECT edge_id, use, width_m FROM driving.lane_profile ORDER BY edge_id, lane_num").fetchall():
            profile.setdefault(eid, []).append((use, width, None))   # no arrows: only GMNS knows which fit their exits (gmns_sumo._arrow_fit)
    graph_tbl = f"{mode}.edge_graph"
    use_conns = connections and _table_exists(con, graph_tbl)
    if connections and not use_conns:
        logger.warning(f"{graph_tbl} absent — letting netconvert INFER connections "
                       f"(turn restrictions NOT enforced)")
    cs, stuck, prules, lane_cs = [], [], [], {}
    if use_conns:
        from duckosm.processors.edge_graph import routed_graph
        graph_tbl = routed_graph(con, mode)              # the cars' turns, not the bus rows
        if bus:                                          # and the buses', where a bus-only edge is in it
            graph_tbl = (f"(SELECT * FROM {mode}.edge_graph WHERE from_edge IN (SELECT edge_id FROM {mode}.edges UNION ALL SELECT edge_id FROM "
                         f"driving.private_edges WHERE access = 'bus') AND to_edge IN (SELECT edge_id FROM {mode}.edges UNION ALL SELECT edge_id "
                         f"FROM driving.private_edges WHERE access = 'bus'))")
        cs = con.execute(f"SELECT from_edge, to_edge FROM {graph_tbl}").fetchall()
        perm = (con.execute(f"SELECT from_edge, to_edge, allowed, vehicles, except_vehicles, condition FROM {mode}.turn_permission").fetchall()
                if _table_exists(con, f"{mode}.turn_permission") else [])
        cs = _apply_permissions(cs, perm)        # (from, to, allow, disallow)
        # via-way restrictions (a path: from edge, via edges, to edge); SUMO can say them only where the via edges vanish into a
        # joined junction and the path becomes one connection
        prules = (con.execute(f"SELECT restriction_type, from_edge, via_edges, to_edge, except_vehicles, applies_to "
                              f"FROM {mode}.turn_path_restrictions").fetchall()
                  if _table_exists(con, f"{mode}.turn_path_restrictions") else [])
        # an edge with no legal successor gets a from-only connection ("no connections"), or
        # netconvert would invent its own there (it once brought back a banned left turn)
        has = {c[0] for c in cs}
        stuck = [(r[0],) for r in rows if r[1] != r[2] and r[0] not in has]
        arrowed = {e for e, ls in profile.items() if any(x[2] for x in ls)}
        if arrowed and run_netconvert:                   # an arrowed edge -> its lane connections (SUMO's OSM import)
            lane_cs = _arrow_connections(con, mode, bus, rows, profile, arrowed, [c[:2] for c in cs],
                                         dict((n, (x, y)) for n, x, y in nodes), out_dir, net_name, _find_netconvert(netconvert_bin), config)
        elif arrowed:
            logger.warning(f"SUMO connections[{mode}]: {len(arrowed)} edges have turn arrows, but run_netconvert=False: their "
                           f"connections are edge-level, netconvert will not apply the arrows")

    def write(nodes, rows, cs, stuck, lanes=None):
        """The plain XML: nodes, edges (id = edge_id, 1:1; real shape; true graph length), connections."""
        with open(nod_path, "w", encoding="utf-8") as f:
            f.write('<?xml version="1.0" encoding="UTF-8"?>\n<nodes>\n')
            for nid, lon, lat in nodes:
                f.write(f'  <node id="{nid}" x="{lon:.7f}" y="{lat:.7f}"/>\n')
            f.write('</nodes>\n')
        n_edges = self_loops = 0
        with open(edg_path, "w", encoding="utf-8") as f:
            f.write('<?xml version="1.0" encoding="UTF-8"?>\n<edges>\n')
            for eid, source, target, highway, name, lanes_, spd, length_m, wkt, *orig in rows:
                if source == target:                        # SUMO rejects from==to (geometry self-loop)
                    self_loops += 1
                    continue
                attrs = [f'id="{eid}"', f'from="{source}"', f'to="{target}"',
                         f'priority="{_HIGHWAY_PRIORITY.get(highway, 1)}"']
                # the caller's lane count (edge_attrs) wins over the profile's lanes
                lanes_here = None if "numLanes" in extra.get(str(eid), {}) else profile.get(eid)
                if lanes_here:
                    attrs.append(f'numLanes="{len(lanes_here)}"')
                elif lanes_ and int(lanes_) > 0 and mode not in _MODE_LANEWIDTH:     # a road's lanes, not a path's
                    attrs.append(f'numLanes="{int(lanes_)}"')
                if spd and float(spd) > 0:
                    attrs.append(f'speed="{float(spd) / 3.6:.3f}"')       # km/h -> m/s
                if length_m and float(length_m) > 0:
                    attrs.append(f'length="{float(length_m):.2f}"')       # true graph length
                if highway:
                    attrs.append(f'type={quoteattr(str(highway))}')
                if mode in _MODE_ALLOW:
                    attrs.append(f'allow="{_MODE_ALLOW[mode]}"')
                src0, tgt0 = orig or (source, target)           # one-way or not: decided on the network as built
                if (tgt0, src0) not in both_ways:          # one-way: its line is the road's middle (SUMO puts lanes right of it)
                    attrs.append('spreadType="center"')
                shape = _linestring_points(wkt)
                if shape:
                    attrs.append(f'shape="{shape}"')
                if name:
                    attrs.append(f'name={quoteattr(str(name))}')
                for k, v in extra.get(str(eid), {}).items():
                    attrs = [a for a in attrs if not a.startswith(f"{k}=")] + [f"{k}={quoteattr(str(v))}"]
                if lanes_here:                              # SUMO numbers lanes from the right: index 0 is the right-most
                    f.write("  <edge " + " ".join(attrs) + ">\n")
                    for i, (use, width, _) in enumerate(reversed(lanes_here)):
                        allow = f' allow="{_LANE_ALLOW[use]}"' if use in _LANE_ALLOW else ""
                        f.write(f'    <lane index="{i}"{allow} width="{float(width):.2f}"/>\n')
                    f.write("  </edge>\n")
                else:
                    f.write("  <edge " + " ".join(attrs) + "/>\n")
                n_edges += 1
            f.write('</edges>\n')
        if use_conns:
            here = {r[0] for r in rows if r[1] != r[2]}
            by_lane = {fe for fe, _ in (lanes or {})}
            with open(con_path, "w", encoding="utf-8") as f:
                f.write('<?xml version="1.0" encoding="UTF-8"?>\n<connections>\n')
                for fe, te, allow, disallow in cs:
                    perm_ = (f' allow="{" ".join(sorted(allow))}"' if allow is not None else "") + \
                            (f' disallow="{" ".join(sorted(disallow))}"' if disallow else "")
                    if lanes and lanes.get((str(fe), str(te))):   # a permission needs lanes, and then every move of its edge too
                        for fl, tl in lanes[(str(fe), str(te))]:
                            f.write(f'  <connection from="{fe}" to="{te}" fromLane="{fl}" toLane="{tl}"{perm_}/>\n')
                    elif fe not in lane_cs:
                        f.write(f'  <connection from="{fe}" to="{te}"/>\n')
                for fe, ls in lane_cs.items():           # the turn arrows' lane connections, onto the edges still there
                    if fe not in here or str(fe) in by_lane:
                        continue
                    ls = [x for x in ls if x[1] in here]
                    for fl, te, tl in ls:
                        f.write(f'  <connection from="{fe}" to="{te}" fromLane="{fl}" toLane="{tl}"/>\n')
                    if not ls:                           # the arrows leave this edge no way on: no connections (lane_check lists it)
                        f.write(f'  <connection from="{fe}"/>\n')
                for (fe,) in stuck:
                    f.write(f'  <connection from="{fe}"/>\n')
                f.write('</connections>\n')
        return n_edges, self_loops

    n_edges, self_loops = write(nodes, rows, cs, stuck)
    logger.info(f"SUMO plain-XML[{mode}]: {len(nodes):,} nodes, {n_edges:,} edges "
                f"(skipped {self_loops} self-loop edge(s))")
    out = {"nod": nod_path, "edg": edg_path, "n_nodes": len(nodes), "n_edges": n_edges}
    if use_conns:
        out["con"], out["n_connections"], out["n_arrowed"] = con_path, len(cs), len(lane_cs)
        logger.info(f"SUMO connections[{mode}]: {len(cs):,} legal successors -> {con_path}")

    if not run_netconvert:
        return out

    # ---- assemble via a standard SUMO netconvert config (.netccfg) ----
    binary = _find_netconvert(netconvert_bin)
    con_arg = con_path if use_conns else None
    if isinstance(config, str):                          # caller supplied their own .netccfg
        cmd = [binary, "-c", config, "--node-files", nod_path, "--edge-files", edg_path,
               "--output-file", net_path]
        if con_arg:
            cmd += ["--connection-files", con_arg]
        out["netccfg"] = config
    else:                                                # default config, or a dict override of it
        opts = dict(DEFAULT_NETCFG)
        if mode == "walking":                            # SUMO pedestrians cross junctions on these
            opts["walkingareas"] = "true"
        if mode in _MODE_LANEWIDTH:
            opts["default.lanewidth"] = str(_MODE_LANEWIDTH[mode])
        if isinstance(config, dict):
            opts.update(config)
        cfg_path = os.path.join(out_dir, f"{net_name}.netccfg")
        _write_netccfg(cfg_path, nod_path, edg_path, con_arg, net_path, opts)
        cmd = [binary, "-c", cfg_path]
        out["netccfg"] = cfg_path
    def run(cmd):
        logger.info("running: " + " ".join(cmd))
        res = subprocess.run(cmd, capture_output=True, text=True)
        if res.returncode != 0:
            raise RuntimeError(f"netconvert failed (exit {res.returncode}):\n{res.stderr[-2000:]}")
    if use_conns and not isinstance(config, str) and str(opts.get("junctions.join")).lower() == "true":
        # netconvert reads the connections before it joins nodes, and drops every connection to an edge that the join swallows
        # (an edge between two joined nodes): a road whose only way on ran through such an edge lost its straight and left turns.
        # So: netconvert names the joins (pass 1), duckOSM merges those nodes and writes each legal path through them as one
        # connection, and netconvert builds the network from that, joining nothing more (pass 2).
        joins_path = os.path.join(out_dir, f"{net_name}.joins.xml")
        run(cmd + ["--junctions.join-output", joins_path])
        groups = _joined_groups(joins_path)
        if groups:
            nodes2, rows2, cs2, stuck2, n_paths = _merge_joined(nodes, rows, cs, stuck, groups, prules)
            out["n_path_restrictions"] = n_paths
            write(nodes2, rows2, cs2, stuck2)
            _write_netccfg(cfg_path, nod_path, edg_path, con_arg, net_path, {**opts, "junctions.join": "false"})
            out["n_joined"] = len(groups)
            logger.info(f"SUMO: {len(groups)} joined junction(s), {len(cs2):,} connections through them, "
                        f"{n_paths} of {len({(r[1], tuple(r[2])) for r in prules})} via-way restriction path(s) applied")
    run(cmd)
    cur = (nodes2, rows2, cs2, stuck2) if out.get("n_joined") else (nodes, rows, cs, stuck)
    perms = {(str(fe), str(te)) for fe, te, allow, disallow in cur[2] if allow is not None or disallow}
    if perms:   # netconvert takes a permission only on a lane-to-lane connection, and once one move of an edge is given by lane it
        # works out no other move of that edge: read the lanes it chose for every move of those edges, write them all back by lane
        import xml.etree.ElementTree as ET
        lanes, froms = {}, {fe for fe, _ in perms}
        for _, el in ET.iterparse(net_path):
            if el.tag == "connection" and el.get("from") in froms and not el.get("from").startswith(":"):
                lanes.setdefault((el.get("from"), el.get("to")), []).append((el.get("fromLane"), el.get("toLane")))
        write(*cur, lanes=lanes)
        run(cmd)
        out["n_permissions"] = len(perms)
        logger.info(f"SUMO: {len(perms)} connection(s) open or closed to some vehicles only")
    out["net"] = net_path
    logger.info(f"wrote SUMO net -> {net_path}")
    return out
