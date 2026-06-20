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

    import duckdb
    from duckosm.sumo import to_sumo
    con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)
    out = to_sumo(con, "sumo/")            # sumo/network.{nod,edg,con}.xml + .netccfg + .net.xml

The netconvert options come from a built-in default (``DEFAULT_NETCFG``); pass ``config=`` to
override them — a dict of ``{netconvert-option: value}`` merged onto the default, or a path to your
own ``.netccfg`` to drive netconvert directly. netconvert ships with SUMO (``pip install
duckosm[sumo]``). Coordinates are geographic; the default config projects them (``--proj.plain-geo``).
"""
import logging
import os
import shutil
import subprocess
from xml.sax.saxutils import quoteattr

logger = logging.getLogger("duckosm")

# right-of-way hint for netconvert's junction building (explicit connections still gate movements).
_HIGHWAY_PRIORITY = {
    "motorway": 6, "motorway_link": 6, "trunk": 5, "trunk_link": 5,
    "primary": 4, "primary_link": 4, "secondary": 3, "secondary_link": 3,
    "tertiary": 2, "tertiary_link": 2, "unclassified": 1, "residential": 1,
    "living_street": 1, "service": 0,
}

# Default netconvert options, written into the generated `.netccfg` (standard SUMO config format).
# Tuned to keep the duckOSM topology verbatim: 1:1 edge ids (no merge), explicit turns honoured.
DEFAULT_NETCFG = {
    "proj.plain-geo": "true",                # interpret node/edge coords as lon/lat and project (UTM)
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
    """Write a standard SUMO netconvert config (`.netccfg`) wiring the inputs/output + options."""
    lines = ['<?xml version="1.0" encoding="UTF-8"?>', "<configuration>",
             f'  <node-files value="{nod}"/>', f'  <edge-files value="{edg}"/>']
    if con_xml:
        lines.append(f'  <connection-files value="{con_xml}"/>')
    lines.append(f'  <output-file value="{net}"/>')
    for key, val in opts.items():
        lines.append(f'  <{key} value="{val}"/>')
    lines += ["</configuration>", ""]
    with open(cfg_path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))


def to_sumo(con, out_dir, mode: str = "driving", net_name: str = "network",
            run_netconvert: bool = True, netconvert_bin: str = None,
            connections: bool = True, with_service: bool = False, config=None):
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
    with_service : include ``service`` roads (``service_edges`` + ``edge_graph_with_service``).
    config : netconvert configuration. ``None`` → the built-in :data:`DEFAULT_NETCFG`; a ``dict`` of
        ``{netconvert-option: value}`` is merged onto the default; a ``str`` path to a ``.netccfg``
        is used as-is (inputs/output are still set by this function). The effective config is written
        in SUMO's standard ``.netccfg`` format and run with ``netconvert -c``.

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

    # ---- nodes: geographic position; netconvert projects them ----
    nodes = con.execute(
        f"SELECT node_id, ST_X(geom), ST_Y(geom) FROM {mode}.nodes WHERE geom IS NOT NULL").fetchall()
    with open(nod_path, "w", encoding="utf-8") as f:
        f.write('<?xml version="1.0" encoding="UTF-8"?>\n<nodes>\n')
        for nid, lon, lat in nodes:
            f.write(f'  <node id="{nid}" x="{lon:.7f}" y="{lat:.7f}"/>\n')
        f.write('</nodes>\n')

    # ---- edges: id = edge_id (1:1); real shape; true graph length (meso uses it directly) ----
    cols = "edge_id, source, target, highway, name, lanes, maxspeed_kmh, length_m, geometry"
    esrc = (f"(SELECT {cols} FROM {mode}.edges UNION ALL SELECT {cols} FROM {mode}.service_edges)"
            if with_service else f"{mode}.edges")
    rows = con.execute(
        f"SELECT edge_id, source, target, highway, name, lanes, maxspeed_kmh, length_m, "
        f"ST_AsText(geometry) FROM {esrc}").fetchall()
    n_edges = self_loops = 0
    with open(edg_path, "w", encoding="utf-8") as f:
        f.write('<?xml version="1.0" encoding="UTF-8"?>\n<edges>\n')
        for eid, source, target, highway, name, lanes_, spd, length_m, wkt in rows:
            if source == target:                        # SUMO rejects from==to (geometry self-loop)
                self_loops += 1
                continue
            attrs = [f'id="{eid}"', f'from="{source}"', f'to="{target}"',
                     f'priority="{_HIGHWAY_PRIORITY.get(highway, 1)}"']
            if lanes_ and int(lanes_) > 0:
                attrs.append(f'numLanes="{int(lanes_)}"')
            if spd and float(spd) > 0:
                attrs.append(f'speed="{float(spd) / 3.6:.3f}"')       # km/h -> m/s
            if length_m and float(length_m) > 0:
                attrs.append(f'length="{float(length_m):.2f}"')       # true graph length
            if highway:
                attrs.append(f'type={quoteattr(str(highway))}')
            shape = _linestring_points(wkt)
            if shape:
                attrs.append(f'shape="{shape}"')
            if name:
                attrs.append(f'name={quoteattr(str(name))}')
            f.write("  <edge " + " ".join(attrs) + "/>\n")
            n_edges += 1
        f.write('</edges>\n')
    logger.info(f"SUMO plain-XML[{mode}]: {len(nodes):,} nodes, {n_edges:,} edges "
                f"(skipped {self_loops} self-loop edge(s))")
    out = {"nod": nod_path, "edg": edg_path, "n_nodes": len(nodes), "n_edges": n_edges}

    # ---- connections: the legal successors from edge_graph (turn restrictions already applied) ----
    graph_tbl = f"{mode}.edge_graph_with_service" if with_service else f"{mode}.edge_graph"
    use_conns = connections and _table_exists(con, graph_tbl)
    if connections and not use_conns:
        logger.warning(f"{graph_tbl} absent — letting netconvert INFER connections "
                       f"(turn restrictions NOT enforced)")
    if use_conns:
        cs = con.execute(f"SELECT from_edge, to_edge FROM {graph_tbl}").fetchall()
        with open(con_path, "w", encoding="utf-8") as f:
            f.write('<?xml version="1.0" encoding="UTF-8"?>\n<connections>\n')
            for fe, te in cs:
                f.write(f'  <connection from="{fe}" to="{te}"/>\n')
            f.write('</connections>\n')
        out["con"], out["n_connections"] = con_path, len(cs)
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
        if isinstance(config, dict):
            opts.update(config)
        cfg_path = os.path.join(out_dir, f"{net_name}.netccfg")
        _write_netccfg(cfg_path, nod_path, edg_path, con_arg, net_path, opts)
        cmd = [binary, "-c", cfg_path]
        out["netccfg"] = cfg_path
    logger.info("running: " + " ".join(cmd))
    res = subprocess.run(cmd, capture_output=True, text=True)
    if res.returncode != 0:
        raise RuntimeError(f"netconvert failed (exit {res.returncode}):\n{res.stderr[-2000:]}")
    out["net"] = net_path
    logger.info(f"wrote SUMO net -> {net_path}")
    return out
