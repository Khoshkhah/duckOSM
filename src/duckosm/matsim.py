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
_DTD = "http://www.matsim.org/files/dtd/network_v2.dtd"
_FALLBACK_MS = 8.33                                     # ~30 km/h, last-resort freespeed (m/s)


def _norm_hw(hw):
    if not hw:
        return ""
    h = hw.split(";")[0]
    return h[:-5] if h.endswith("_link") else h


def to_matsim(source, out_path, mode="driving", crs="EPSG:3006", gzip=True):
    """Write a MATSim ``network.xml`` from a built duckOSM db (schema ``<mode>.edges``/``.nodes``).

    ``crs`` is the projected metric CRS node coordinates are reprojected into (recorded in the network
    attributes). Writes gzip (MATSim convention) when ``gzip`` else plain XML. Returns
    ``{"nodes": n, "links": n}``. Only nodes referenced by an exported link are written.
    """
    import duckdb

    con = duckdb.connect(str(source), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    if con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name=? AND table_name='edges'",
                   [mode]).fetchone()[0] == 0:
        con.close()
        raise ValueError(f"no '{mode}.edges' in {source} — is this a built duckOSM db with mode '{mode}'?")

    matsim_mode = _MATSIM_MODE.get(mode, mode)
    edges = con.execute(
        f"SELECT edge_id, source, target, highway, lanes, maxspeed_kmh, length_m, cost_s "
        f"FROM {mode}.edges WHERE source IS NOT NULL AND target IS NOT NULL").fetchall()
    node_xy = {nid: (x, y) for nid, x, y in con.execute(
        f"SELECT node_id, ST_X(t), ST_Y(t) FROM (SELECT node_id, "
        f"ST_Transform(geom, 'EPSG:4326', ?, always_xy := true) t FROM {mode}.nodes)", [crs]).fetchall()}
    con.close()

    links, used = [], set()
    for eid, src, tgt, hw, lanes, spd, length_m, cost_s in edges:
        if src not in node_xy or tgt not in node_xy:
            continue                                     # link with an unprojectable endpoint — skip
        permlanes = max(int(lanes or 1), 1)
        length = float(length_m) if length_m and length_m > 0 else 1.0
        if spd and spd > 0:
            freespeed = float(spd) / 3.6
        elif cost_s and cost_s > 0:
            freespeed = length / float(cost_s)
        else:
            freespeed = _FALLBACK_MS
        capacity = _CAPACITY.get(_norm_hw(hw), 800) * permlanes
        links.append((eid, src, tgt, length, freespeed, capacity, permlanes))
        used.update((src, tgt))

    out = ['<?xml version="1.0" encoding="UTF-8"?>',
           f'<!DOCTYPE network SYSTEM "{_DTD}">', '<network>',
           '\t<attributes>',
           f'\t\t<attribute name="coordinateReferenceSystem" class="java.lang.String">{crs}</attribute>',
           '\t</attributes>', '\t<nodes>']
    for nid in sorted(used):
        x, y = node_xy[nid]
        out.append(f'\t\t<node id="{nid}" x="{x:.2f}" y="{y:.2f}"/>')
    out.append('\t</nodes>')
    out.append('\t<links capperiod="01:00:00" effectivecellsize="7.5" effectivelanewidth="3.75">')
    for eid, src, tgt, length, freespeed, capacity, permlanes in links:
        out.append(f'\t\t<link id="{eid}" from="{src}" to="{tgt}" length="{length:.2f}" '
                   f'freespeed="{freespeed:.4f}" capacity="{capacity:.1f}" permlanes="{permlanes}" '
                   f'modes={quoteattr(matsim_mode)}/>')
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
    logger.info(f"MATSim[{mode}→{matsim_mode}] {len(used):,} nodes, {len(links):,} links "
                f"(CRS {crs}) -> {out_path}")
    return {"nodes": len(used), "links": len(links)}
