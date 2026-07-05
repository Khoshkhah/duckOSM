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


def to_opendrive(source, out_path, mode="driving", crs="EPSG:3006"):
    """Write an OpenDRIVE ``.xodr`` from a built duckOSM db (``<mode>.edges``). Node coordinates are
    reprojected to the metric ``crs`` (recorded in ``<geoReference>``). Returns ``{"roads": n}``."""
    import duckdb

    con = duckdb.connect(str(source), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    if con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name=? AND table_name='edges'",
                   [mode]).fetchone()[0] == 0:
        con.close()
        raise ValueError(f"no '{mode}.edges' in {source} — is this a built duckOSM db with mode '{mode}'?")
    rows = con.execute(
        f"SELECT edge_id, COALESCE(lanes, 1), name, "
        f"ST_AsText(ST_Transform(geometry, 'EPSG:4326', ?, always_xy := true)) "
        f"FROM {mode}.edges WHERE geometry IS NOT NULL", [crs]).fetchall()
    con.close()

    roads, xs, ys = [], [], []
    for eid, lanes, name, wkt in rows:
        pts = _coords(wkt)
        if len(pts) < 2:
            continue
        segs, s = [], 0.0
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
            length = math.hypot(x1 - x0, y1 - y0)
            if length <= 0:
                continue
            segs.append((s, x0, y0, math.atan2(y1 - y0, x1 - x0), length))
            s += length
        if not segs:
            continue
        roads.append((eid, max(int(lanes), 1), name or "", segs, s))
        xs += [p[0] for p in pts]; ys += [p[1] for p in pts]

    o = ['<?xml version="1.0" encoding="UTF-8"?>', '<OpenDRIVE>']
    o.append(f'\t<header revMajor="1" revMinor="7" name={quoteattr(Path(source).stem)} version="1.00" '
             f'north="{max(ys):.2f}" south="{min(ys):.2f}" east="{max(xs):.2f}" west="{min(xs):.2f}">')
    o.append(f'\t\t<geoReference><![CDATA[{_geo_ref(crs)}]]></geoReference>')
    o.append('\t</header>')
    for eid, lanes, name, segs, total in roads:
        o.append(f'\t<road name={quoteattr(name)} length="{total:.4f}" id="{eid}" junction="-1">')
        o.append('\t\t<planView>')
        for s, x, y, hdg, length in segs:
            o.append(f'\t\t\t<geometry s="{s:.4f}" x="{x:.4f}" y="{y:.4f}" hdg="{hdg:.6f}" '
                     f'length="{length:.4f}"><line/></geometry>')
        o.append('\t\t</planView>')
        o.append('\t\t<lanes>')
        o.append('\t\t\t<laneSection s="0">')
        o.append('\t\t\t\t<center><lane id="0" type="none" level="false"/></center>')
        o.append('\t\t\t\t<right>')
        for i in range(1, lanes + 1):
            o.append(f'\t\t\t\t\t<lane id="-{i}" type="driving" level="false">')
            o.append(f'\t\t\t\t\t\t<width sOffset="0" a="{_LANE_W}" b="0" c="0" d="0"/>')
            o.append('\t\t\t\t\t</lane>')
        o.append('\t\t\t\t</right>')
        o.append('\t\t\t</laneSection>')
        o.append('\t\t</lanes>')
        o.append('\t</road>')
    o.append('</OpenDRIVE>\n')

    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text("\n".join(o), encoding="utf-8")
    logger.info(f"OpenDRIVE[{mode}]: {len(roads):,} roads (CRS {crs}) -> {out_path}")
    return {"roads": len(roads)}
