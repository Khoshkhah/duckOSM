#!/usr/bin/env python
"""STANDALONE multimodal-route map — one self-contained HTML you can just open in a browser.

Routes across the intermodal `mm.*` graph with `duckosm.route_multimodal` and draws each leg
coloured **by mode** (walking green, driving red, cycling blue). Click any edge to see its info
(mode, street name, highway class, edge_id, length, time).

Two rendering engines:

* ``--engine svg`` (DEFAULT) — a **dependency-free** inline-SVG viewer. No map library, no CDN,
  no basemap tiles: nothing external loads, so it can't freeze and works fully offline. Best when
  folium/Leaflet hangs (restricted network / WSL). No slippy basemap, just the route + a click-for-
  info side panel and a legend.
* ``--engine folium`` — a Leaflet slippy map with an OSM basemap (needs internet for Leaflet + tiles;
  add ``--tiles none`` to drop the basemap). Nicer geographic context when the browser cooperates.

    python scripts/multimodal_route_standalone.py --db data/db/sodermalm_pbf.duckdb
    python scripts/multimodal_route_standalone.py --db data/db/sodermalm_pbf.duckdb \
        --src 336296072 --dst 35120411 --out reports/soder_mm.html
    python scripts/multimodal_route_standalone.py --db ... --engine folium --tiles none

Needs: duckdb (svg engine) / + folium (folium engine). Builds mm.* into the given db if absent —
pass a scratch copy if you don't want to modify the source db.
"""
import argparse
import html
import math
from pathlib import Path

import duckdb

from duckosm import route_multimodal
from duckosm.processors import MultimodalBuilder

# walking green, driving red, cycling blue; vehicular legs drawn a touch wider.
MODE_COLOR = {"walking": "#27ae60", "driving": "#e74c3c", "cycling": "#2b6cb0"}
MODE_WEIGHT = {"walking": 4, "driving": 6, "cycling": 5}
MODE_ORDER = {"driving": 0, "cycling": 0, "walking": 1}          # vehicular first, walking on top


def _wkt_lonlat(wkt):
    """Parse a 2D 'LINESTRING (lon lat, ...)' into [(lon, lat), ...] (native OSM order)."""
    inner = wkt[wkt.index("(") + 1: wkt.rindex(")")]
    return [(float(p.split()[0]), float(p.split()[1])) for p in inner.split(",")]


def _ensure_mm(con, transfer_cost):
    has_mm = con.execute(
        "SELECT COUNT(*) FROM duckdb_tables() WHERE schema_name = 'mm' "
        "AND table_name = 'edges'").fetchone()[0]
    if not has_mm:
        print("building mm.* graph (none present) ...")
        MultimodalBuilder(con, transfer_s=transfer_cost).run()


def _node_coords(con, node_ids):
    """(lon, lat) for each node_id, across every mode's nodes table (dedup by node_id)."""
    modes = [r[0] for r in con.execute(
        "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE table_name = 'nodes' "
        "AND schema_name NOT IN ('information_schema','pg_catalog','main','raw','mm')").fetchall()]
    if not modes or not node_ids:
        return {}
    ids = ", ".join(str(int(n)) for n in node_ids)
    union = " UNION ".join(
        f"SELECT node_id, ST_X(geom) AS lon, ST_Y(geom) AS lat FROM {m}.nodes "
        f"WHERE node_id IN ({ids})" for m in modes)
    return {nid: (lon, lat) for nid, lon, lat in con.execute(union).fetchall()}


def _autopick(con, start_mode, end_mode):
    """Far-apart walking-only endpoints, preferring a walk->drive->walk trip."""
    def extreme(order):
        row = con.execute(f"""
            WITH wonly AS (SELECT node_id FROM walking.nodes
                           EXCEPT SELECT node_id FROM driving.nodes)
            SELECT n.node_id FROM walking.nodes n JOIN wonly USING (node_id)
            ORDER BY {order} LIMIT 1""").fetchone()
        return row[0] if row else None
    ext = {"W": extreme("ST_X(n.geom)"), "E": extreme("ST_X(n.geom) DESC"),
           "S": extreme("ST_Y(n.geom)"), "N": extreme("ST_Y(n.geom) DESC")}
    fallback = None
    for a, b in [("W", "E"), ("S", "N"), ("W", "N"), ("E", "S")]:
        src, dst = ext[a], ext[b]
        if src is None or dst is None or src == dst:
            continue
        r = route_multimodal(con, src, dst, start_mode=start_mode, end_mode=end_mode)
        if r is None:
            continue
        modes = [leg["mode"] for leg in r["legs"]]
        fallback = fallback or (src, dst)
        if modes and modes[0] == "walking" and modes[-1] == "walking" and any(
                m != "walking" for m in modes):
            return src, dst
    if fallback is None:
        raise SystemExit("auto-pick found no route — pass --src / --dst")
    return fallback


def _edges_with_geom(route):
    """Flatten the route legs into [(mode, leg_idx, edge_dict, [(lon,lat)...]), ...]."""
    out = []
    for li, leg in enumerate(route["legs"]):
        for p in leg["path"]:
            if p.get("geometry"):
                out.append((leg["mode"], li, p, _wkt_lonlat(p["geometry"])))
    return out


# ================================ SVG engine (no deps, offline) =================================

def _render_svg(con, db, route, src, dst, out):
    edges = _edges_with_geom(route)
    if not edges:
        raise SystemExit("route has no drawable geometry")
    all_ll = [pt for _, _, _, pts in edges for pt in pts]
    lons = [p[0] for p in all_ll]
    lats = [p[1] for p in all_ll]
    lat0 = sum(lats) / len(lats)
    k = math.cos(math.radians(lat0))                            # aspect correction for lon

    minx, maxx = min(lons) * k, max(lons) * k
    miny, maxy = min(lats), max(lats)
    W, PAD = 1100.0, 30.0
    span_x = max(maxx - minx, 1e-9)
    s = (W - 2 * PAD) / span_x
    H = (maxy - miny) * s + 2 * PAD

    def proj(lon, lat):
        return (PAD + (lon * k - minx) * s, PAD + (maxy - lat) * s)   # y flipped for SVG

    # draw vehicular under walking
    edges.sort(key=lambda e: MODE_ORDER.get(e[0], 1))
    paths = []
    for mode, li, p, pts in edges:
        col = MODE_COLOR.get(mode, "#999")
        pstr = " ".join(f"{proj(lon, lat)[0]:.1f},{proj(lon, lat)[1]:.1f}" for lon, lat in pts)
        info = {"mode": mode, "leg": li + 1, "name": p.get("name") or "—",
                "highway": p.get("highway") or "—", "edge_id": p.get("edge_id"),
                "length_m": None if p.get("length_m") is None else round(p["length_m"]),
                "time_s": None if p.get("cost_s") is None else round(p["cost_s"])}
        data = " ".join(f'data-{key}="{html.escape(str(val), quote=True)}"'
                        for key, val in info.items())
        paths.append(f'<polyline class="edge" points="{pstr}" stroke="{col}" '
                     f'stroke-width="{MODE_WEIGHT.get(mode, 4)}" {data}/>')

    # transfer points + start/end markers
    coords = _node_coords(con, [t["node_id"] for t in route["transfers"]] + [src, dst])
    markers = []
    for t in route["transfers"]:
        c = coords.get(t["node_id"])
        if c:
            x, y = proj(*c)
            markers.append(
                f'<circle class="xfer" cx="{x:.1f}" cy="{y:.1f}" r="6" '
                f'data-kind="{html.escape(t["kind"])}" '
                f'data-from="{t["from_mode"]}" data-to="{t["to_mode"]}" '
                f'data-node="{t["node_id"]}"/>')
    for nid, cls, lbl in [(src, "start", "A"), (dst, "end", "B")]:
        c = coords.get(nid)
        if c:
            x, y = proj(*c)
            markers.append(f'<circle class="{cls}" cx="{x:.1f}" cy="{y:.1f}" r="9"/>'
                           f'<text class="lbl" x="{x:.1f}" y="{y + 4:.1f}">{lbl}</text>')

    legs_html = " → ".join(
        f'<b style="color:{MODE_COLOR.get(l["mode"], "#999")}">{l["mode"]} {l["time_s"]:.0f}s</b>'
        for l in route["legs"])
    legend = "".join(
        f'<div><span class="sw" style="background:{c}"></span>{m}</div>'
        for m, c in MODE_COLOR.items() if any(e[0] == m for e in edges))

    tmpl = f"""<!doctype html><html><head><meta charset="utf-8">
<title>duckOSM multimodal — {html.escape(db.stem)}</title><style>
  html,body{{margin:0;height:100%;font:13px/1.4 system-ui,sans-serif;background:#eef1f4}}
  #wrap{{position:absolute;inset:0}}
  svg{{width:100%;height:100%}}
  .edge{{fill:none;stroke-linecap:round;stroke-linejoin:round;opacity:.9;cursor:pointer;
         vector-effect:non-scaling-stroke}}
  .edge:hover{{stroke-width:9;opacity:1}}
  .edge.sel{{stroke:#111;opacity:1}}
  .xfer{{fill:#f1c40f;stroke:#222;stroke-width:2;cursor:pointer}}
  .start{{fill:#2ecc71;stroke:#145a32;stroke-width:2}}
  .end{{fill:#e74c3c;stroke:#7b241c;stroke-width:2}}
  .lbl{{font:bold 11px sans-serif;fill:#fff;text-anchor:middle;pointer-events:none}}
  #title{{position:fixed;top:12px;left:12px;background:#fff;padding:8px 12px;border:1px solid #bbb;
          border-radius:6px;box-shadow:0 1px 4px rgba(0,0,0,.25);max-width:60%}}
  #legend{{position:fixed;top:12px;right:12px;background:#fff;padding:8px 12px;border:1px solid #bbb;
           border-radius:6px;box-shadow:0 1px 4px rgba(0,0,0,.25)}}
  #legend .sw{{display:inline-block;width:14px;height:6px;border-radius:3px;margin-right:6px;
               vertical-align:middle}}
  #panel{{position:fixed;bottom:12px;right:12px;width:230px;background:#fff;padding:10px 12px;
          border:1px solid #bbb;border-radius:6px;box-shadow:0 1px 4px rgba(0,0,0,.25)}}
  #panel h4{{margin:0 0 6px}} #panel .row{{display:flex;justify-content:space-between;gap:10px}}
  #panel .k{{color:#666}}
</style></head><body>
<div id="wrap"><svg viewBox="0 0 {W:.0f} {H:.0f}" preserveAspectRatio="xMidYMid meet">
  <rect x="0" y="0" width="{W:.0f}" height="{H:.0f}" fill="#eef1f4"/>
  {''.join(paths)}
  {''.join(markers)}
</svg></div>
<div id="title"><b>duckOSM multimodal</b> — {html.escape(db.stem)}<br>{legs_html}<br>
  total {route['time_s']:.0f}s · {route['length_m']:.0f} m · {len(route['transfers'])} transfer(s)
  <br><i>click any edge for its info</i></div>
<div id="legend">{legend}</div>
<div id="panel"><h4>Click an edge</h4><div class="k">…to see its details here.</div></div>
<script>
  var panel = document.getElementById('panel'), sel = null;
  function row(k,v){{return '<div class="row"><span class="k">'+k+'</span><span>'+v+'</span></div>';}}
  document.querySelectorAll('.edge').forEach(function(el){{
    el.addEventListener('click', function(){{
      if(sel) sel.classList.remove('sel'); el.classList.add('sel'); sel = el;
      var d = el.dataset;
      panel.innerHTML = '<h4 style="color:'+el.getAttribute('stroke')+'">'+d.mode+' · leg '+d.leg+'</h4>'
        + row('name', d.name) + row('highway', d.highway) + row('edge_id', d.edge_id)
        + row('length', d.length_m+' m') + row('time', d.time_s+' s');
    }});
  }});
  document.querySelectorAll('.xfer').forEach(function(el){{
    el.addEventListener('click', function(){{
      var d = el.dataset;
      panel.innerHTML = '<h4>transfer</h4>' + row('', d.from+' → '+d.to+' ('+d.kind+')')
        + row('node', d.node);
    }});
  }});
</script></body></html>"""
    out.write_text(tmpl)
    print(f"\nwrote {out}  (open it directly — no server, no internet needed)")
    return str(out)


# ================================ folium engine (Leaflet basemap) ===============================

def _render_folium(con, db, route, src, dst, out, tiles):
    import folium

    edges = _edges_with_geom(route)
    if not edges:
        raise SystemExit("route has no drawable geometry")
    all_pts = [[lat, lon] for _, _, _, pts in edges for (lon, lat) in pts]
    lats = [c[0] for c in all_pts]
    lons = [c[1] for c in all_pts]
    center = [sum(lats) / len(lats), sum(lons) / len(lons)]

    m = folium.Map(location=center, zoom_start=14,
                   tiles=(None if str(tiles).lower() == "none" else tiles),
                   prefer_canvas=True, control_scale=True)
    groups = {}
    for mode in sorted({e[0] for e in edges}, key=lambda x: MODE_ORDER.get(x, 1)):
        groups[mode] = folium.FeatureGroup(name=f"{mode} ({MODE_COLOR.get(mode, '#999')})",
                                           show=True).add_to(m)
    for mode, li, p, pts in edges:
        col = MODE_COLOR.get(mode, "#999")
        rows = [f"<b style='color:{col}'>{mode}</b> · leg {li + 1}"]
        if p.get("name"):
            rows.append(f"<b>{p['name']}</b>")
        rows += [f"highway: {p.get('highway') or ''}", f"edge_id: {p.get('edge_id')}"]
        if p.get("length_m") is not None:
            rows.append(f"length: {p['length_m']:.0f} m")
        if p.get("cost_s") is not None:
            rows.append(f"time: {p['cost_s']:.0f} s")
        folium.PolyLine([[lat, lon] for lon, lat in pts], color=col,
                        weight=MODE_WEIGHT.get(mode, 4) + 2, opacity=0.9,
                        tooltip=f"{mode} · {p.get('highway') or ''}",
                        popup=folium.Popup("<br>".join(rows), max_width=280)).add_to(groups[mode])

    coords = _node_coords(con, [t["node_id"] for t in route["transfers"]] + [src, dst])
    tg = folium.FeatureGroup(name="transfers", show=True).add_to(m)
    for t in route["transfers"]:
        c = coords.get(t["node_id"])
        if c:
            folium.CircleMarker([c[1], c[0]], radius=7, color="#222", weight=2, fill=True,
                                fill_color="#f1c40f", fill_opacity=1.0,
                                popup=f"{t['from_mode']} → {t['to_mode']} ({t['kind']})").add_to(tg)
    if coords.get(src):
        folium.Marker([coords[src][1], coords[src][0]], tooltip=f"start {src}",
                      icon=folium.Icon(color="green", icon="play")).add_to(m)
    if coords.get(dst):
        folium.Marker([coords[dst][1], coords[dst][0]], tooltip=f"end {dst}",
                      icon=folium.Icon(color="red", icon="stop")).add_to(m)

    title = (f"<b>duckOSM multimodal</b> — {db.stem}<br>"
             + " → ".join(f"<span style='color:{MODE_COLOR.get(l['mode'], '#999')}'>"
                          f"{l['mode']} {l['time_s']:.0f}s</span>" for l in route["legs"])
             + f"<br>total {route['time_s']:.0f}s · {len(route['transfers'])} transfer(s)"
             + "<br><i>click any segment for its info</i>")
    m.get_root().html.add_child(folium.Element(
        "<div style='position:fixed;top:12px;left:60px;z-index:9999;background:white;"
        "padding:8px 12px;border:1px solid #999;border-radius:6px;font:13px sans-serif;"
        f"box-shadow:0 1px 4px rgba(0,0,0,.3);pointer-events:none'>{title}</div>"))
    folium.map.LayerControl(collapsed=False).add_to(m)
    m.fit_bounds([[min(lats), min(lons)], [max(lats), max(lons)]])
    m.save(str(out))
    print(f"\nwrote {out}  (open it directly in a browser — needs internet for Leaflet/tiles)")
    return str(out)


# ================================ driver ========================================================

def render_standalone(db, src=None, dst=None, out=None, transfer_cost=60.0,
                      start_mode="walking", end_mode="walking", engine="svg",
                      tiles="OpenStreetMap"):
    """Route across db's mm.* graph and write a single self-contained HTML. Returns its path.

    engine : "svg" (dependency-free, offline, no basemap) | "folium" (Leaflet basemap, needs net).
    """
    db = Path(db)
    out = Path(out) if out else Path("reports") / f"{db.stem}_multimodal_route.html"
    out.parent.mkdir(parents=True, exist_ok=True)

    con = duckdb.connect(str(db))                                # read-write: may build mm.*
    con.execute("INSTALL spatial; LOAD spatial;")
    _ensure_mm(con, transfer_cost)

    if src is None or dst is None:
        src, dst = _autopick(con, start_mode, end_mode)
        print(f"auto-picked demo trip: {src} -> {dst}")

    route = route_multimodal(con, src, dst, start_mode=start_mode, end_mode=end_mode)
    if route is None:
        raise SystemExit(f"no multimodal route between {src} and {dst}")
    legs = " → ".join(f"{leg['mode']}({leg['time_s']:.0f}s)" for leg in route["legs"])
    print(f"route {src} → {dst}: {legs}")
    print(f"  total {route['time_s']:.0f}s over {route['length_m']:.0f} m, "
          f"{len(route['transfers'])} transfer(s)")

    if engine == "svg":
        return _render_svg(con, db, route, src, dst, out)
    return _render_folium(con, db, route, src, dst, out, tiles)


def main(argv=None):
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--db", default="data/db/sodermalm_pbf.duckdb", help="duckOSM .duckdb file")
    ap.add_argument("--src", type=int, default=None, help="start OSM junction node_id")
    ap.add_argument("--dst", type=int, default=None, help="destination OSM junction node_id")
    ap.add_argument("--out", default=None,
                    help="output .html (default: reports/<db-stem>_multimodal_route.html)")
    ap.add_argument("--engine", choices=["svg", "folium"], default="svg",
                    help="svg: dependency-free offline viewer (default) | folium: Leaflet basemap")
    ap.add_argument("--transfer-cost", type=float, default=60.0,
                    help="flat transfer penalty in seconds (used only if mm.* must be built)")
    ap.add_argument("--start-mode", default="walking")
    ap.add_argument("--end-mode", default="walking")
    ap.add_argument("--tiles", default="OpenStreetMap",
                    help="folium engine only: basemap tiles, or 'none' for no basemap")
    a = ap.parse_args(argv)
    render_standalone(a.db, src=a.src, dst=a.dst, out=a.out, transfer_cost=a.transfer_cost,
                      start_mode=a.start_mode, end_mode=a.end_mode, engine=a.engine, tiles=a.tiles)


if __name__ == "__main__":
    main()
