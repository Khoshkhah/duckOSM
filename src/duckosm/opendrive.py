"""
ASAM OpenDRIVE (.xodr) export — a built duckOSM db → roads with a reprojected reference line and
lane-level width offsets, the continuous-geometry format that reaches AV sims (CARLA / esmini) and
commercial micro (PTV Vissim / Aimsun). See docs/opendrive_export.md.

**Phase 1 (this module):** one ``<road>`` per directed edge (``road id = edge_id``, oneway, lanes on
the right), with a metric reference line (piecewise ``<line>``) and ``lanes.lanes`` driving lanes of
default width. ``junction="-1"``, no links — a valid, loadable ``.xodr`` whose geometry & lanes are
real. Routable ``<junction>``s (reusing the meso turn-connector Béziers) are the Phase-2 follow-on.

    from duckosm import to_opendrive
    to_opendrive("sodermalm_pbf.duckdb", "network.xodr", mode="driving", crs="EPSG:3006")
"""
import logging
import math
from pathlib import Path
from xml.sax.saxutils import quoteattr

logger = logging.getLogger("duckosm")

_LANE_W = 3.25          # metres, default lane width when OSM lacks width:lanes
_PROJ4 = {  # exact proj4 for the common defaults (pyproj's to_proj4 lossily approximates these)
    "EPSG:3006": "+proj=tmerc +lat_0=0 +lon_0=15 +k=0.9996 +x_0=500000 +y_0=0 +ellps=GRS80 "
                 "+towgs84=0,0,0,0,0,0,0 +units=m +no_defs",
}


def _geo_ref(crs):
    """proj4 string for the geoReference header (so sims know the projection)."""
    if crs in _PROJ4:
        return _PROJ4[crs]
    try:
        import warnings

        from pyproj import CRS
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return CRS.from_user_input(crs).to_proj4()
    except Exception:
        return f"+init={crs.lower()} +units=m +no_defs"


def _coords(wkt):
    b = wkt[wkt.index("(") + 1:wkt.rindex(")")]
    return [tuple(float(v) for v in p.split()[:2]) for p in b.split(",")]


def _segments(pts):
    """polyline points → [(s, x, y, hdg, seg_length)] + total length (piecewise-linear reference)."""
    segs, s = [], 0.0
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        length = math.hypot(x1 - x0, y1 - y0)
        if length <= 0:
            continue
        segs.append((s, x0, y0, math.atan2(y1 - y0, x1 - x0), length))
        s += length
    return segs, s


def _planview(segs):
    o = ['\t\t<planView>']
    for s, x, y, hdg, length in segs:
        o.append(f'\t\t\t<geometry s="{s:.4f}" x="{x:.4f}" y="{y:.4f}" hdg="{hdg:.6f}" '
                 f'length="{length:.4f}"><line/></geometry>')
    o.append('\t\t</planView>')
    return o


def _lanes(n):
    o = ['\t\t<lanes>', '\t\t\t<laneSection s="0">',
         '\t\t\t\t<center><lane id="0" type="none" level="false"/></center>', '\t\t\t\t<right>']
    for i in range(1, n + 1):
        o += [f'\t\t\t\t\t<lane id="-{i}" type="driving" level="false">',
              f'\t\t\t\t\t\t<width sOffset="0" a="{_LANE_W}" b="0" c="0" d="0"/>', '\t\t\t\t\t</lane>']
    o += ['\t\t\t\t</right>', '\t\t\t</laneSection>', '\t\t</lanes>']
    return o


def _elevation_profile(z0, z1, length):
    """A linear <elevationProfile> (elev(s) = a + b·s) from z0 at the road start to z1 at its end.
    Endpoint ground elevations from edges.z_from/z_to (bare terrain) — Phase-1 straight ramp."""
    b = (z1 - z0) / length if length and length > 0 else 0.0
    return ['\t\t<elevationProfile>',
            f'\t\t\t<elevation s="0" a="{z0:.4f}" b="{b:.6f}" c="0" d="0"/>',
            '\t\t</elevationProfile>']


def _header(name, xs, ys, crs):
    return [f'\t<header revMajor="1" revMinor="7" name={quoteattr(name)} version="1.00" '
            f'north="{max(ys):.2f}" south="{min(ys):.2f}" east="{max(xs):.2f}" west="{min(xs):.2f}">',
            f'\t\t<geoReference><![CDATA[{_geo_ref(crs)}]]></geoReference>', '\t</header>']


def _tx(col):
    return f"ST_AsText(ST_Transform({col}, 'EPSG:4326', ?, always_xy := true))"


def _build_phase1(con, mode, crs, name):
    # z_from/z_to (from `duckosm elevation`) are optional — emit an <elevationProfile> only when present.
    have_z = con.execute("SELECT count(*) FROM duckdb_columns() WHERE schema_name = ? "
                         "AND table_name = 'edges' AND column_name = 'z_from'", [mode]).fetchone()[0] > 0
    zsel = ", z_from, z_to" if have_z else ""
    rows = con.execute(f"SELECT edge_id, COALESCE(lanes, 1), name, {_tx('geometry')}{zsel} "
                       f"FROM {mode}.edges WHERE geometry IS NOT NULL", [crs]).fetchall()
    xs, ys, o = [], [], []
    for row in rows:
        (eid, lanes, nm, wkt), (z_from, z_to) = row[:4], (row[4:6] if have_z else (None, None))
        pts = _coords(wkt)
        if len(pts) < 2:
            continue
        segs, total = _segments(pts)
        if not segs:
            continue
        xs += [p[0] for p in pts]; ys += [p[1] for p in pts]
        o.append(f'\t<road name={quoteattr(nm or "")} length="{total:.4f}" id="{eid}" junction="-1">')
        o += _planview(segs)
        if z_from is not None and z_to is not None:
            o += _elevation_profile(z_from, z_to, total)     # after planView, before lanes (schema order)
        o += _lanes(max(int(lanes), 1)) + ['\t</road>']
    body = ['<?xml version="1.0" encoding="UTF-8"?>', '<OpenDRIVE>'] + _header(name, xs, ys, crs) + o
    body.append('</OpenDRIVE>\n')
    return "\n".join(body), {"roads": sum(1 for line in o if line.startswith('\t<road '))}


def _build_junctions(con, g, crs, name):
    from collections import defaultdict

    links = con.execute(f"SELECT link_id, from_node_id, to_node_id, COALESCE(lanes, 1), name, "
                        f"{_tx('geom')} FROM {g}.link WHERE geom IS NOT NULL", [crs]).fetchall()
    movements = con.execute(
        f"SELECT node_id, ib_link_id, ob_link_id, start_ib_lane, end_ib_lane, {_tx('ST_GeomFromText(geometry)')} "
        f"FROM {g}.movement WHERE geometry IS NOT NULL AND ib_link_id IS NOT NULL "
        f"AND ob_link_id IS NOT NULL", [crs]).fetchall()

    # a node is a junction if any inbound link has ≥2 outbound choices, or ≥3 inbound links meet
    ib_obs = defaultdict(lambda: defaultdict(set))
    for node, ib, ob, sib, eib, wkt in movements:
        ib_obs[node][ib].add(ob)
    jnodes = {n for n, ibs in ib_obs.items() if len(ibs) >= 3 or any(len(o) >= 2 for o in ibs.values())}

    # direct road-to-road links at non-junction (through) nodes
    succ, pred = {}, {}
    for node, ib, ob, sib, eib, wkt in movements:
        if node not in jnodes:
            succ.setdefault(ib, ob)          # ib ends at `node`, continues to ob
            pred.setdefault(ob, ib)          # ob starts at `node`, comes from ib

    xs, ys = [], []
    road_o, conn_o = [], []
    conns_by_node = defaultdict(list)
    # main roads (from links), with junction/road end-links
    for lid, fn, tn, lanes, nm, wkt in links:
        pts = _coords(wkt)
        if len(pts) < 2:
            continue
        segs, total = _segments(pts)
        if not segs:
            continue
        xs += [p[0] for p in pts]; ys += [p[1] for p in pts]
        link = []
        if fn in jnodes:
            link.append(f'\t\t\t<predecessor elementType="junction" elementId="{fn}"/>')
        elif lid in pred:
            link.append(f'\t\t\t<predecessor elementType="road" elementId="{pred[lid]}" contactPoint="end"/>')
        if tn in jnodes:
            link.append(f'\t\t\t<successor elementType="junction" elementId="{tn}"/>')
        elif lid in succ:
            link.append(f'\t\t\t<successor elementType="road" elementId="{succ[lid]}" contactPoint="start"/>')
        road_o.append(f'\t<road name={quoteattr(nm or "")} length="{total:.4f}" id="{lid}" junction="-1">')
        if link:
            road_o += ['\t\t<link>'] + link + ['\t\t</link>']
        road_o += _planview(segs) + _lanes(max(int(lanes), 1)) + ['\t</road>']
    # connecting roads (one per movement at a junction node) + junction connections
    ci = 0
    for node, ib, ob, sib, eib, wkt in movements:
        if node not in jnodes or not wkt:
            continue
        pts = _coords(wkt)
        if len(pts) < 2:
            continue
        segs, total = _segments(pts)
        if not segs:
            continue
        cid = f"c{ci}"; ci += 1
        xs += [p[0] for p in pts]; ys += [p[1] for p in pts]
        conn_o.append(f'\t<road name="" length="{total:.4f}" id="{cid}" junction="{node}">')
        conn_o += ['\t\t<link>',
                   f'\t\t\t<predecessor elementType="road" elementId="{ib}" contactPoint="end"/>',
                   f'\t\t\t<successor elementType="road" elementId="{ob}" contactPoint="start"/>',
                   '\t\t</link>']
        conn_o += _planview(segs) + _lanes(1) + ['\t</road>']
        conns_by_node[node].append((cid, ib))
    junc_o = []
    for node, cs in conns_by_node.items():
        junc_o.append(f'\t<junction id="{node}" name="">')
        for k, (cid, ib) in enumerate(cs):
            junc_o += [f'\t\t<connection id="{k}" incomingRoad="{ib}" connectingRoad="{cid}" contactPoint="start">',
                       '\t\t\t<laneLink from="-1" to="-1"/>', '\t\t</connection>']
        junc_o.append('\t</junction>')

    body = (['<?xml version="1.0" encoding="UTF-8"?>', '<OpenDRIVE>'] + _header(name, xs, ys, crs)
            + road_o + conn_o + junc_o + ['</OpenDRIVE>\n'])
    counts = {"roads": sum(1 for line in road_o if line.startswith('\t<road ')),
              "connecting_roads": ci, "junctions": len(conns_by_node)}
    return "\n".join(body), counts


def to_opendrive(source, out_path, mode="driving", crs="EPSG:3006", junctions=False):
    """Write an OpenDRIVE ``.xodr`` from a built duckOSM db, reprojected to the metric ``crs``.

    ``junctions=False`` (Phase 1): read ``<mode>.edges`` → roads + lanes, no junctions.
    ``junctions=True`` (Phase 2): read a **GMNS db** (``gmns_<mode>.link`` + ``.movement``) → roads
    that link through ``<junction>`` elements whose connecting roads carry the turn geometry.
    Returns counts."""
    import duckdb

    con = duckdb.connect(str(source), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    if junctions:
        g = f"gmns_{mode}"
        if con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name=? AND table_name='movement'",
                       [g]).fetchone()[0] == 0:
            con.close()
            raise ValueError(f"--junctions needs a GMNS db (no '{g}.movement' in {source}) — build with duckosm gmns")
        xml, counts = _build_junctions(con, g, crs, Path(source).stem)
    else:
        if con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name=? AND table_name='edges'",
                       [mode]).fetchone()[0] == 0:
            con.close()
            raise ValueError(f"no '{mode}.edges' in {source} — is this a built duckOSM db with mode '{mode}'?")
        xml, counts = _build_phase1(con, mode, crs, Path(source).stem)
    con.close()

    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(xml, encoding="utf-8")
    extra = (f", {counts['connecting_roads']:,} connecting roads, {counts['junctions']:,} junctions"
             if junctions else "")
    logger.info(f"OpenDRIVE[{mode}]: {counts['roads']:,} roads{extra} (CRS {crs}) -> {out_path}")
    return counts
