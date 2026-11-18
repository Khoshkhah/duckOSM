"""
Interactive viewer for a GMNS DuckDB — a single self-contained HTML page (no server, no tiles) that
reads an exported GMNS `.duckdb` and lets you explore it: a **layer toggle** (individual *lanes*
offset by use, vs the *mesoscopic* section + turn-connector network), **hover tooltips** naming every
line (its type, id and attributes), and pan/zoom. Built to make the GMNS output legible — the two
geometric levels (offset lanes vs centerline meso) are separate layers, never confusingly stacked.

    from duckosm.gmns_viewer import write_viewer
    write_viewer("sodermalm_gmns.duckdb", "reports/viewer.html", mode="driving")

Needs only ``duckdb`` (+ the spatial extension) — geometry is drawn client-side on a canvas. The meso
layer appears only if the db has a ``meso_<mode>`` schema (built with ``duckosm gmns --meso``).
"""
import json
import logging
import math
from pathlib import Path

logger = logging.getLogger("duckosm")

_Q = 8192
_USE = {"auto": 0, "bus": 1, "bus,bike": 1, "bike": 2}


def _table(con, schema, table):
    return con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = ? AND table_name = ?",
                       [schema, table]).fetchone()[0] > 0


def _coords(wkt):
    body = wkt[wkt.index("(") + 1:wkt.rindex(")")]
    return [tuple(float(v) for v in p.split()[:2]) for p in body.split(",")]


def build_viewer_payload(con, mode="driving"):
    """Read gmns_<mode> (lane) + meso_<mode> (meso_link/meso_node) and return the JSON payload the
    page renders. Raises ValueError if there's no gmns_<mode> network."""
    g, m = f"gmns_{mode}", f"meso_{mode}"
    if not _table(con, g, "lane"):
        raise ValueError(f"no {g}.lane table — is this a GMNS db with mode '{mode}'?")
    try:
        con.execute("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;")
    except Exception:
        pass

    lanes = con.execute(f"SELECT link_id, lane_num, allowed_uses, turn, ST_AsText(geom) "
                        f"FROM {g}.lane WHERE geom IS NOT NULL").fetchall()
    have_meso = _table(con, m, "meso_link")
    secs = cons = []
    mnodes = []
    if have_meso:
        secs = con.execute(f"SELECT link_id, macro_link_id, lanes, facility_type, ST_AsText(geom) "
                           f"FROM {m}.meso_link WHERE meso_type='normal' AND geom IS NOT NULL").fetchall()
        cons = con.execute(f"SELECT link_id, mvmt_txt_id, lanes, start_ib_lane, end_ib_lane, "
                           f"ST_AsText(geom) FROM {m}.meso_link WHERE meso_type='movement' "
                           f"AND geom IS NOT NULL").fetchall()
        mnodes = con.execute(f"SELECT ST_AsText(geom) FROM {m}.meso_node").fetchall()
    mi = f"micro_{mode}"
    have_micro = _table(con, mi, "micro_link")
    micro = []
    if have_micro:
        micro = con.execute(f"SELECT cell_type, mvmt_txt_id, ST_AsText(geom) FROM {mi}.micro_link "
                            f"WHERE geom IS NOT NULL").fetchall()
    cnt = dict(con.execute(f"SELECT allowed_uses, count(*) FROM {g}.lane GROUP BY 1").fetchall())

    # a lane-rich junction to open on (prefer a bus-lane / multi-lane approach)
    focus = con.execute(f"""SELECT n.x_coord, n.y_coord FROM {g}.node n
        JOIN {g}.link l ON l.to_node_id = n.node_id
        WHERE n.node_id IN (SELECT node_id FROM {g}.movement)
        ORDER BY (l.link_id IN (SELECT link_id FROM {g}.lane WHERE allowed_uses='bus')) DESC,
                 l.lanes DESC, l."length" DESC LIMIT 1""").fetchone() if _table(con, g, "movement") else None

    allc = [p for *_, w in lanes for p in _coords(w)]
    xs = [p[0] for p in allc]; ys = [p[1] for p in allc]
    minx, maxx, miny, maxy = min(xs), max(xs), min(ys), max(ys)
    kx = math.cos(math.radians((miny + maxy) / 2)) or 1.0
    scale = _Q / max((maxx - minx) * kx, (maxy - miny), 1e-9)
    qx = lambda x: round((x - minx) * kx * scale)
    qy = lambda y: round((maxy - y) * scale)
    gw, gh = round((maxx - minx) * kx * scale), round((maxy - miny) * scale)

    def flat(w):
        o = []
        for x, y in _coords(w):
            o += [qx(x), qy(y)]
        return o

    lane_feats = [{"t": "lane", "u": _USE.get(u, 0), "id": str(lid), "n": ln, "tr": tr or "",
                   "g": flat(w)} for lid, ln, u, tr, w in lanes]
    sec_feats = [{"t": "section", "id": sid, "m": str(mid), "la": la, "f": fac or "", "g": flat(w)}
                 for sid, mid, la, fac, w in secs]
    con_feats = [{"t": "conn", "id": cid, "mv": mv or "", "la": la, "s": sib, "e": eib, "g": flat(w)}
                 for cid, mv, la, sib, eib, w in cons]
    mnode_arr = [[qx(x), qy(y)] for x, y in (_coords(w)[0] for (w,) in mnodes)]
    # micro: cells + lane-change drawn from flat coords (no hover); turn connectors hoverable
    mi_cells = [flat(w) for ct, mv, w in micro if ct == "normal"]
    mi_lchg = [flat(w) for ct, mv, w in micro if ct == "lane_change"]
    mi_conns = [{"t": "mconn", "mv": mv or "", "g": flat(w)} for ct, mv, w in micro if ct == "movement"]
    if not focus:
        focus = ((minx + maxx) / 2, (miny + maxy) / 2)
    fx, fy = qx(focus[0]), qy(focus[1])
    half = round(55 * scale / 111320)

    return {
        "grid": {"w": gw, "h": gh}, "focus": [fx - half, fy - half, fx + half, fy + half],
        "lanes": lane_feats, "sections": sec_feats, "conns": con_feats, "mnodes": mnode_arr,
        "micro_cells": mi_cells, "micro_lchg": mi_lchg, "micro_conns": mi_conns,
        "has_meso": bool(sec_feats or con_feats), "has_micro": bool(micro), "mode": mode,
        "stats": {"lanes": sum(cnt.values()), "bus": cnt.get("bus", 0), "bike": cnt.get("bike", 0),
                  "sec": len(sec_feats), "con": len(con_feats),
                  "mcell": len(mi_cells), "mconn": len(mi_conns), "file": ""},
    }


def render_viewer_html(payload, standalone=True, file_label=""):
    """Render the viewer page. ``standalone`` → a full HTML document; else the body-only content an
    Artifact wrapper expects."""
    payload = dict(payload, stats=dict(payload["stats"], file=file_label))
    title = f"GMNS viewer · {file_label}" if file_label else "GMNS viewer"
    content = "<title>" + title + "</title>\n" + _TEMPLATE.replace(
        "/*__P__*/null", json.dumps(payload, separators=(",", ":")))
    if not standalone:
        return content
    return ('<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n'
            '<meta name="viewport" content="width=device-width, initial-scale=1">\n'
            f'<title>{title}</title>\n</head>\n<body>\n{content}\n</body>\n</html>\n')


def write_viewer(gmns_db, out, mode="driving", standalone=True):
    """Build + write the interactive viewer for a GMNS db. Returns the output path."""
    import duckdb

    con = duckdb.connect(str(gmns_db), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;")
    payload = build_viewer_payload(con, mode=mode)
    con.close()
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    Path(out).write_text(render_viewer_html(payload, standalone=standalone,
                                            file_label=Path(gmns_db).name), encoding="utf-8")
    logger.info(f"GMNS viewer [{mode}]: {payload['stats']['lanes']:,} lanes, "
                f"{payload['stats']['con']:,} connectors -> {out}")
    return out


# ---------------------------------------------------------------------------------------------------
# The page (templates/gmns_viewer.html) — CSS + markup + a canvas renderer with a layer toggle and hover tooltips, all inlined.
# `/*__P__*/null` is replaced with the JSON payload. Theme-aware chrome; committed-dark map surface.
# ---------------------------------------------------------------------------------------------------
_TEMPLATE = (Path(__file__).parent / "templates" / "gmns_viewer.html").read_text(encoding="utf-8")
