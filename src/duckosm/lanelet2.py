"""
Lanelet2 export — a built GMNS db's per-lane geometry → a **Lanelet2** map (OSM XML), the dominant
open autonomous-driving lane-map format (Autoware / the ``lanelet2`` library). See
docs/exports/lanelet2.md.

This is an HD-map *skeleton in the AD-standard format* — correct lane structure/topology/semantics —
**not** a survey-grade HD map (geometry is OSM centerlines offset by assumed widths). A lanelet is
defined by its **left + right boundaries**, so each GMNS lane (centerline + width) becomes two boundary
ways (centerline ± ½·width) + a ``type=lanelet`` relation. Boundary points are snapped/deduped so
lanelets share nodes → the ``lanelet2`` router infers longitudinal connectivity.

    from duckosm import to_lanelet2
    to_lanelet2("sodermalm_pbf_gmns.duckdb", "sodermalm.lanelet2.osm")
"""
import logging
from pathlib import Path
from xml.sax.saxutils import escape

from duckosm.gmns import _offset_wkt

logger = logging.getLogger("duckosm")

_SUBTYPE = {"auto": "road", "bus": "bus_lane", "bus,bike": "bus_lane", "bike": "bicycle_lane", "walk": "walkway"}


def _coords(wkt):
    if not wkt or not wkt.startswith("LINESTRING"):
        return []
    b = wkt[wkt.index("(") + 1:wkt.rindex(")")]
    return [tuple(float(v) for v in p.split()[:2]) for p in b.split(",")]


def to_lanelet2(gmns_db, out_path, mode="driving", snap_m=0.01):
    """Write a Lanelet2 ``.osm`` from ``gmns_<mode>.lane`` (+ ``link`` for speed). Boundary points are
    deduped on a ``snap_m`` grid so lanelets share nodes. Returns ``{lanelets, ways, nodes}``."""
    import duckdb

    con = duckdb.connect(str(gmns_db), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    g = f"gmns_{mode}"
    if con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name=? AND table_name='lane'",
                   [g]).fetchone()[0] == 0:
        con.close()
        raise ValueError(f"no '{g}.lane' in {gmns_db} — build a GMNS db first (duckosm gmns)")
    lanes = con.execute(
        f"SELECT la.link_id, la.allowed_uses, COALESCE(la.width, 3.25), ST_AsText(la.geom), lk.free_speed "
        f"FROM {g}.lane la LEFT JOIN {g}.link lk ON lk.link_id = la.link_id "
        f"WHERE la.geom IS NOT NULL").fetchall()
    con.close()

    snap = snap_m / 111320.0                          # ~degrees for the point-dedup grid
    counter = [0]
    node_id = {}                                      # (snapped lon, lat) -> osm id
    nodes = []                                        # (id, lon, lat)
    ways = []                                         # (id, [node ids], type)
    rels = []                                         # (id, left_way, right_way, tags)

    def _next():
        counter[0] += 1
        return counter[0]

    def _node(lon, lat):
        key = (round(lon / snap), round(lat / snap))
        nid = node_id.get(key)
        if nid is None:
            nid = node_id[key] = _next()
            nodes.append((nid, lon, lat))
        return nid

    def _way(pts, wtype):
        ns = []
        for x, y in pts:
            nid = _node(x, y)
            if not ns or ns[-1] != nid:               # drop consecutive dupes from snapping
                ns.append(nid)
        if len(ns) < 2:
            return None
        wid = _next()
        ways.append((wid, ns, wtype))
        return wid

    skipped = 0
    for link_id, use, width, wkt, free_speed in lanes:
        left = _coords(_offset_wkt(wkt, width / 2))
        right = _coords(_offset_wkt(wkt, -width / 2))
        if len(left) < 2 or len(right) < 2:
            skipped += 1
            continue
        lw = _way(left, "line_thin")
        rw = _way(right, "line_thin")
        if lw is None or rw is None:
            skipped += 1
            continue
        tags = {"type": "lanelet", "subtype": _SUBTYPE.get(use, "road"), "location": "urban",
                "one_way": "yes", "duckosm:edge_id": str(link_id)}
        if free_speed and free_speed > 0:
            tags["speed_limit"] = f"{float(free_speed):g}"
        rels.append((_next(), lw, rw, tags))

    o = ['<?xml version="1.0" encoding="UTF-8"?>', '<osm version="0.6" generator="duckosm">']
    for nid, lon, lat in nodes:
        o.append(f'  <node id="{nid}" visible="true" version="1" lat="{lat:.8f}" lon="{lon:.8f}"/>')
    for wid, ns, wtype in ways:
        o.append(f'  <way id="{wid}" visible="true" version="1">')
        o += [f'    <nd ref="{n}"/>' for n in ns]
        o.append(f'    <tag k="type" v="{wtype}"/>')
        o.append('  </way>')
    for rid, lw, rw, tags in rels:
        o.append(f'  <relation id="{rid}" visible="true" version="1">')
        o.append(f'    <member type="way" ref="{lw}" role="left"/>')
        o.append(f'    <member type="way" ref="{rw}" role="right"/>')
        o += [f'    <tag k="{escape(k)}" v="{escape(v)}"/>' for k, v in tags.items()]
        o.append('  </relation>')
    o.append('</osm>\n')

    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text("\n".join(o), encoding="utf-8")
    counts = {"lanelets": len(rels), "ways": len(ways), "nodes": len(nodes)}
    logger.info(f"Lanelet2[{mode}]: {counts['lanelets']:,} lanelets, {counts['ways']:,} boundary ways, "
                f"{counts['nodes']:,} nodes" + (f" ({skipped} lanes skipped)" if skipped else "")
                + f" -> {out_path}")
    return counts
