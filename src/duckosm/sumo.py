"""
SUMO exporter — turn a built duckOSM database into a SUMO network whose edges keep their
duckOSM ``edge_id``.

The DB stores a directed-edge road network (`<mode>.edges` + `<mode>.nodes`). `to_sumo` writes
SUMO *plain-XML* (a `.nod.xml` of nodes and a `.edg.xml` of edges, ids = duckOSM `edge_id`) and
then lets **netconvert** assemble the `.net.xml`. Because netconvert preserves the ``id`` you give
each plain edge, **every SUMO edge id equals the duckOSM ``edge_id``** — so anything keyed on
``edge_id`` (e.g. a per-edge flow/regime table) maps onto the SUMO network by identity, with no
geometry conflation step.

    import duckdb
    from duckosm.sumo import to_sumo
    con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)
    out = to_sumo(con, "sumo/")            # writes sumo/network.{nod,edg}.xml + network.net.xml
    print(out["net"])                      # -> sumo/network.net.xml

netconvert ships with SUMO; install it with the system package or ``pip install duckosm[sumo]``
(the ``eclipse-sumo`` wheel). Pass ``run_netconvert=False`` to write only the plain-XML and run
netconvert yourself.

Notes / current limitations
---------------------------
* Coordinates are geographic (lon/lat); netconvert is run with ``--proj.plain-geo`` so it projects
  them (UTM) and the resulting net is geo-referenced.
* Edge ``shape`` (the real polyline), ``numLanes`` (from ``lanes``), ``speed`` (from
  ``maxspeed_kmh``), ``type`` (the ``highway`` class) and ``name`` are carried over. Where a value
  is missing, netconvert's per-type defaults apply.
* **Turn restrictions are not yet emitted** — netconvert *infers* connections from geometry. The
  ``<mode>.turn_restrictions`` table can be layered in via a ``.con.xml`` later.
"""
import logging
import os
import shutil
import subprocess
from xml.sax.saxutils import escape, quoteattr

logger = logging.getLogger("duckosm")


def _find_netconvert(explicit=None):
    """Locate netconvert: explicit → eclipse-sumo wheel → sumolib → $SUMO_HOME/bin → PATH.

    Importing the ``sumo`` (eclipse-sumo) package also sets ``$SUMO_HOME``, which netconvert needs
    at runtime for its bundled type maps — so the subprocess inherits it.
    """
    if explicit:
        return explicit
    # eclipse-sumo wheel (`pip install duckosm[sumo]`): importing it sets $SUMO_HOME and ships bin/.
    try:
        import sumo
        cand = os.path.join(os.path.dirname(sumo.__file__), "bin", "netconvert")
        if os.path.exists(cand):
            os.environ.setdefault("SUMO_HOME", os.path.dirname(sumo.__file__))
            return cand
    except Exception:
        pass
    try:
        import sumolib  # standard locator when a full SUMO is installed
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


def to_sumo(con, out_dir, mode: str = "driving", net_name: str = "network",
            run_netconvert: bool = True, netconvert_bin: str = None,
            lanes: bool = True, extra_netconvert_args=None):
    """Export the duckOSM network to SUMO, preserving ``edge_id`` as the SUMO edge id.

    Parameters
    ----------
    con : a DuckDB connection to a built duckOSM database (spatial extension is loaded here).
    out_dir : directory for the output files (created if needed).
    mode : the mode schema to read (default ``"driving"``).
    net_name : basename for the outputs (``<net_name>.nod.xml`` / ``.edg.xml`` / ``.net.xml``).
    run_netconvert : if True (default) assemble the ``.net.xml`` with netconvert; if False only
        the plain-XML node/edge files are written.
    netconvert_bin : explicit path to netconvert (else auto-located: sumolib → $SUMO_HOME → PATH).
    lanes : carry the ``lanes`` column over as ``numLanes`` (default True).
    extra_netconvert_args : optional list of extra CLI args forwarded to netconvert.

    Returns a dict with the written paths: ``{"nod", "edg", "net"(if built), "n_nodes", "n_edges"}``.
    The SUMO edge id of every (non-internal) edge equals the duckOSM ``edge_id``.
    """
    try:
        con.execute("INSTALL spatial; LOAD spatial;")
    except Exception:
        pass
    os.makedirs(out_dir, exist_ok=True)
    nod_path = os.path.join(out_dir, f"{net_name}.nod.xml")
    edg_path = os.path.join(out_dir, f"{net_name}.edg.xml")
    net_path = os.path.join(out_dir, f"{net_name}.net.xml")

    # ---- nodes: id + geographic position (lon/lat); netconvert projects them ----
    nodes = con.execute(
        f"SELECT node_id, ST_X(geom) AS lon, ST_Y(geom) AS lat FROM {mode}.nodes "
        f"WHERE geom IS NOT NULL").fetchall()
    with open(nod_path, "w", encoding="utf-8") as f:
        f.write('<?xml version="1.0" encoding="UTF-8"?>\n<nodes>\n')
        for nid, lon, lat in nodes:
            f.write(f'  <node id="{nid}" x="{lon:.7f}" y="{lat:.7f}"/>\n')
        f.write('</nodes>\n')

    # ---- edges: id = duckOSM edge_id; real shape; lanes/speed/type/name where known ----
    rows = con.execute(
        f"SELECT edge_id, source, target, highway, name, lanes, maxspeed_kmh, "
        f"ST_AsText(geometry) AS wkt FROM {mode}.edges").fetchall()
    with open(edg_path, "w", encoding="utf-8") as f:
        f.write('<?xml version="1.0" encoding="UTF-8"?>\n<edges>\n')
        for edge_id, source, target, highway, name, n_lanes, spd_kmh, wkt in rows:
            attrs = [f'id="{edge_id}"', f'from="{source}"', f'to="{target}"']
            if highway:
                attrs.append(f'type={quoteattr(str(highway))}')
            if lanes and n_lanes and int(n_lanes) > 0:
                attrs.append(f'numLanes="{int(n_lanes)}"')
            if spd_kmh and float(spd_kmh) > 0:
                attrs.append(f'speed="{float(spd_kmh) / 3.6:.3f}"')   # km/h -> m/s
            shape = _linestring_points(wkt)
            if shape:
                attrs.append(f'shape="{shape}"')
            if name:
                attrs.append(f'name={quoteattr(str(name))}')
            f.write("  <edge " + " ".join(attrs) + "/>\n")
        f.write('</edges>\n')
    logger.info(f"SUMO plain-XML[{mode}]: {len(nodes):,} nodes -> {nod_path}, "
                f"{len(rows):,} edges -> {edg_path}")

    out = {"nod": nod_path, "edg": edg_path, "n_nodes": len(nodes), "n_edges": len(rows)}
    if not run_netconvert:
        return out

    # ---- assemble the .net.xml; netconvert keeps the edge ids we gave it ----
    binary = _find_netconvert(netconvert_bin)
    cmd = [binary, "--node-files", nod_path, "--edge-files", edg_path,
           "--proj.plain-geo", "true", "--output-file", net_path,
           "--no-turnarounds", "true"]
    if extra_netconvert_args:
        cmd += list(extra_netconvert_args)
    logger.info("running: " + " ".join(cmd))
    res = subprocess.run(cmd, capture_output=True, text=True)
    if res.returncode != 0:
        raise RuntimeError(f"netconvert failed (exit {res.returncode}):\n{res.stderr[-2000:]}")
    out["net"] = net_path
    logger.info(f"wrote SUMO net -> {net_path}")
    return out
