"""
MATSim network export — a built duckOSM routing db → a MATSim ``network.xml`` (``network_v2`` DTD),
the directed node+link substrate MATSim / BEAM / eqasim run agents on. See docs/matsim_export.md.

A MATSim link is directed, and duckOSM ``edges`` are already one row per direction, so each edge maps
straight to one link (a two-way street is already two edges → two links). MATSim is a metric
simulator, so node coordinates are reprojected from EPSG:4326 to a projected CRS (default EPSG:3006,
SWEREF99 TM); ``length`` stays the true graph metres. Capacity reuses GMNS's per-class table so the
two exporters agree.

    from duckosm import to_matsim
    to_matsim("sodermalm_pbf.duckdb", "network.xml.gz", mode="driving", crs="EPSG:3006")
"""
import gzip as _gzip
import logging
from pathlib import Path
from xml.sax.saxutils import quoteattr

from duckosm.gmns import _CAPACITY

logger = logging.getLogger("duckosm")

_MATSIM_MODE = {"driving": "car", "cycling": "bike", "walking": "walk"}
_MODE_ORDER = {"car": 0, "bike": 1, "walk": 2}          # deterministic modes= ordering
_DTD = "http://www.matsim.org/files/dtd/network_v2.dtd"
_FALLBACK_MS = 8.33                                     # ~30 km/h, last-resort freespeed (m/s)


def _norm_hw(hw):
    if not hw:
        return ""
    h = hw.split(";")[0]
    return h[:-5] if h.endswith("_link") else h


def _modes_str(mode_names):
    """duckOSM mode names → sorted MATSim modes string, e.g. {'driving','cycling'} → 'car,bike'."""
    ms = {_MATSIM_MODE.get(m, m) for m in mode_names}
    return ",".join(sorted(ms, key=lambda m: _MODE_ORDER.get(m, 99)))


def _resolve_modes(con, mode):
    """Normalise ``mode`` (single str / 'all' / comma-string / list) to an ordered list of present
    modes; raise if any requested mode has no ``<mode>.edges``."""
    present = [r[0] for r in con.execute(
        "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE table_name='edges' "
        "AND schema_name IN ('driving','cycling','walking')").fetchall()]
    if isinstance(mode, (list, tuple)):
        req = list(mode)
    elif mode == "all":
        req = list(present)
    elif "," in mode:
        req = [m.strip() for m in mode.split(",") if m.strip()]
    else:
        req = [mode]
    missing = [m for m in req if m not in present]
    if missing:
        raise ValueError(f"mode(s) {missing} not present in {sorted(present)} — build them first")
    return [m for m in ("driving", "cycling", "walking") if m in req]    # canonical order, deduped


def _nodes_xy(con, modes, crs):
    """{node_id: (x, y)} over the union of the modes' nodes, reprojected to ``crs``."""
    union = " UNION ".join(f"SELECT node_id, geom FROM {m}.nodes" for m in modes)
    return {nid: (x, y) for nid, x, y in con.execute(
        f"SELECT node_id, ST_X(t), ST_Y(t) FROM (SELECT node_id, "
        f"ST_Transform(geom, 'EPSG:4326', ?, always_xy := true) t FROM ({union}))", [crs]).fetchall()}


def _nodes_ele(con, modes):
    """{node_id: ele} over the modes' nodes when an ``ele`` column is present (added by
    ``duckosm elevation``); ``{}`` otherwise, so a network built without elevation is unchanged."""
    have = con.execute(
        "SELECT count(*) FROM duckdb_columns() WHERE schema_name = ? AND table_name = 'nodes' "
        "AND column_name = 'ele'", [modes[0]]).fetchone()[0]
    if not have:
        return {}
    union = " UNION ".join(f"SELECT node_id, ele FROM {m}.nodes WHERE ele IS NOT NULL" for m in modes)
    return {nid: e for nid, e in con.execute(union).fetchall()}


def _link_row(eid, src, tgt, length_m, freespeed, hw, permlanes, modes_str):
    length = float(length_m) if length_m and length_m > 0 else 1.0
    capacity = _CAPACITY.get(_norm_hw(hw), 800) * permlanes
    return (eid, src, tgt, length, freespeed, capacity, permlanes, modes_str)


def _single_links(con, mode):
    mm = _modes_str([mode])
    rows = con.execute(
        f"SELECT edge_id, source, target, highway, lanes, maxspeed_kmh, length_m, cost_s "
        f"FROM {mode}.edges WHERE source IS NOT NULL AND target IS NOT NULL").fetchall()
    out = []
    for eid, src, tgt, hw, lanes, spd, length_m, cost_s in rows:
        permlanes = max(int(lanes or 1), 1)
        length = float(length_m) if length_m and length_m > 0 else 1.0
        if spd and spd > 0:
            freespeed = float(spd) / 3.6
        elif cost_s and cost_s > 0:
            freespeed = length / float(cost_s)
        else:
            freespeed = _FALLBACK_MS
        out.append(_link_row(eid, src, tgt, length_m, freespeed, hw, permlanes, mm))
    return out


def _multi_links(con, modes):
    """Merge the modes' edges by (mode-stable) edge_id → one link each, ``modes=`` the union of modes;
    car attributes (lanes / maxspeed / highway) taken from the driving row where present."""
    from collections import defaultdict

    union = " UNION ALL ".join(
        f"SELECT '{m}' AS mode, edge_id, source, target, highway, lanes, maxspeed_kmh, length_m, cost_s "
        f"FROM {m}.edges WHERE source IS NOT NULL AND target IS NOT NULL" for m in modes)
    groups = defaultdict(list)
    for r in con.execute(f"SELECT * FROM ({union})").fetchall():
        groups[r[1]].append(r)                           # key on edge_id (stable across modes)
    out = []
    for eid, rs in groups.items():
        mm = _modes_str({r[0] for r in rs})
        src, tgt = rs[0][2], rs[0][3]
        length = next((float(r[7]) for r in rs if r[7] and r[7] > 0), 1.0)
        drow = next((r for r in rs if r[0] == "driving"), None)   # car attributes come from driving
        hw = drow[4] if drow else next((r[4] for r in rs if r[4]), None)
        permlanes = max(int((drow[5] if drow else None) or 1), 1)
        spd = drow[6] if drow else None
        if spd and spd > 0:
            freespeed = float(spd) / 3.6
        else:
            costs = [float(r[8]) for r in rs if r[8] and r[8] > 0]
            freespeed = length / min(costs) if costs else _FALLBACK_MS   # fastest traversal
        out.append(_link_row(eid, src, tgt, length, freespeed, hw, permlanes, mm))
    return out


def to_matsim(source, out_path, mode="driving", crs=None, gzip=True):
    """Write a MATSim ``network.xml`` from a built duckOSM db (schema ``<mode>.edges``/``.nodes``).

    ``mode`` selects the network: a single mode name (``"driving"``) → a single-mode network;
    ``"all"``, a comma-string, or a list → a **multimodal** network where each link carries its
    allowed ``modes`` (``car,bike,walk``), merged by the mode-stable ``edge_id``. ``crs`` is the
    projected metric CRS node coordinates are reprojected into (recorded in the network attributes);
    ``None`` picks the UTM zone of the data's centre.
    Writes gzip (MATSim convention) when ``gzip`` else plain XML. Returns ``{"nodes": n, "links": n}``.
    Only nodes referenced by an exported link are written.
    """
    import duckdb

    con = duckdb.connect(str(source), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    modes = _resolve_modes(con, mode)
    if crs is None:
        from duckosm.utils import data_utm_crs
        crs = data_utm_crs(con, f"{modes[0]}.nodes")
    node_xy = _nodes_xy(con, modes, crs)
    node_ele = _nodes_ele(con, modes)                    # {} unless `duckosm elevation` was run
    raw = _single_links(con, modes[0]) if len(modes) == 1 else _multi_links(con, modes)
    con.close()

    links, used = [], set()
    for link in raw:
        src, tgt = link[1], link[2]
        if src not in node_xy or tgt not in node_xy:
            continue                                     # link with an unprojectable endpoint — skip
        links.append(link)
        used.update((src, tgt))

    out = ['<?xml version="1.0" encoding="UTF-8"?>',
           f'<!DOCTYPE network SYSTEM "{_DTD}">', '<network>',
           '\t<attributes>',
           f'\t\t<attribute name="coordinateReferenceSystem" class="java.lang.String">{crs}</attribute>',
           '\t</attributes>', '\t<nodes>']
    for nid in sorted(used):
        x, y = node_xy[nid]
        z = node_ele.get(nid)
        zattr = f' z="{z:.2f}"' if z is not None else ""     # network_v2 allows an optional node z
        out.append(f'\t\t<node id="{nid}" x="{x:.2f}" y="{y:.2f}"{zattr}/>')
    out.append('\t</nodes>')
    out.append('\t<links capperiod="01:00:00" effectivecellsize="7.5" effectivelanewidth="3.75">')
    for eid, src, tgt, length, freespeed, capacity, permlanes, modes_str in links:
        out.append(f'\t\t<link id="{eid}" from="{src}" to="{tgt}" length="{length:.2f}" '
                   f'freespeed="{freespeed:.4f}" capacity="{capacity:.1f}" permlanes="{permlanes}" '
                   f'modes={quoteattr(modes_str)}/>')
    out.append('\t</links>')
    out.append('</network>\n')
    xml = "\n".join(out)

    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    if gzip:
        with _gzip.open(out_path, "wt", encoding="utf-8") as fh:
            fh.write(xml)
    else:
        out_path.write_text(xml, encoding="utf-8")
    logger.info(f"MATSim[{'+'.join(modes)}] {len(used):,} nodes, {len(links):,} links "
                f"(CRS {crs}) -> {out_path}")
    return {"nodes": len(used), "links": len(links), "crs": crs}
