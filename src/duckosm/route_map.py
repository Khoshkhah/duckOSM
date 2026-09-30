"""`duckosm route-map`: an interactive route planner in one self-contained HTML page.

Drop a start and an end on the map, tick the modes, and the route is drawn. There is no server:
the routing graphs are embedded as JSON and Dijkstra runs in the browser, giving the same answers
as the Python API:

* one mode ticked: over ``<mode>.edge_graph`` (edge to edge, legal turns only), exactly ``route()``;
* several modes (walking among them): over ``(node, mode, phase)`` states from ``mm.edges`` and
  ``mm.transfers``, exactly ``route_multimodal(..., allowed_modes=...)``.

Design: docs/design/route_map.md.
"""
import json
import logging
from pathlib import Path

logger = logging.getLogger("duckosm")

MODES = ("driving", "walking", "cycling")
WARN_EDGES = 100_000


def write_route_map(con, out, modes=None, basemap="osm", name="network"):
    """Write the route planner page for the modes of a built db to ``out``; return its path."""
    import geopandas as gpd
    import roadstyle as rs
    from shapely import wkt as _wkt

    from duckosm.viz import BASEMAP_LAYERS, _boundary_geojson

    def has(schema, table):
        return con.execute("SELECT count(*) FROM information_schema.tables "
                           "WHERE table_schema = ? AND table_name = ?", [schema, table]).fetchone()[0] > 0

    present = [m for m in MODES if has(m, "edges") and has(m, "edge_graph")]
    modes = [m for m in (modes or present) if m in present]
    if not modes:
        raise ValueError(f"no mode with edges + edge_graph in the db (present: {present or 'none'})")

    # One map feature per edge_id: the same id in several modes is the same stretch of road.
    union = " UNION ALL ".join(
        f"SELECT edge_id, source, target, COALESCE(name, '') AS name, highway, bridge, tunnel, layer, "
        f"length_m, ST_AsText(geometry) AS wkt FROM {m}.edges" for m in modes)
    rows = con.execute(
        f"SELECT edge_id, any_value(source), any_value(target), any_value(name), any_value(highway), "
        f"any_value(bridge), any_value(tunnel), any_value(layer), any_value(length_m), any_value(wkt) "
        f"FROM ({union}) GROUP BY edge_id ORDER BY edge_id").fetchall()
    if len(rows) > WARN_EDGES:
        logger.warning(f"route-map: {len(rows):,} edges make a heavy page; clip an area first "
                       "(duckosm extract) for a lighter one")
    k_of = {r[0]: k for k, r in enumerate(rows)}               # edge_id -> feature index
    # Node ids can pass 2**53 (content-hashed virtual nodes): the page only sees small indices.
    node_ids = sorted({r[1] for r in rows} | {r[2] for r in rows})
    n_of = {n: i for i, n in enumerate(node_ids)}

    data = {
        "modes": modes,
        "n": len(rows),
        "src": [n_of[r[1]] for r in rows],
        "tgt": [n_of[r[2]] for r in rows],
        "name": [r[3] for r in rows],
        "len": [round(r[8] or 0.0, 1) for r in rows],
        "graphs": {},
        "mm": None,
    }
    for m in modes:                                            # edge-based graph of legal turns
        es = con.execute(f"SELECT edge_id, cost_s, length_m FROM {m}.edges ORDER BY edge_id").fetchall()
        nxt = {}
        for f, t in con.execute(f"SELECT from_edge, to_edge FROM {m}.edge_graph").fetchall():
            if f in k_of and t in k_of:
                nxt.setdefault(k_of[f], []).append(k_of[t])
        data["graphs"][m] = {
            "k": [k_of[e] for e, _, _ in es],
            "cost": [round(c or 0.0, 3) for _, c, _ in es],
            "len": [round(l or 0.0, 2) for _, _, l in es],
            "next": [nxt.get(k_of[e], []) for e, _, _ in es],
        }

    # Across modes: the intermodal graph from `duckosm multimodal`, when it's there.
    # Walk + drive only for now: walk + cycle needs bike stations to mean anything (bike anywhere).
    if {"walking", "driving"} <= set(modes) and has("mm", "edges") and has("mm", "transfers"):
        mi = {m: i for i, m in enumerate(modes) if m in ("walking", "driving")}
        mm_edges = [[mi[md], n_of[s], n_of[t], k_of[e], round(c or 0.0, 3)]
                    for md, s, t, e, c in con.execute(
                        "SELECT mode, source, target, edge_id, cost_s FROM mm.edges").fetchall()
                    if md in mi and e in k_of and s in n_of and t in n_of]
        mm_tr = [[n_of[n], mi[fm], mi[tm], round(c or 0.0, 3)]
                 for n, fm, tm, c in con.execute(
                     "SELECT node_id, from_mode, to_mode, cost_s FROM mm.transfers").fetchall()
                 if fm in mi and tm in mi and n in n_of]
        data["mm"] = {"edges": mm_edges, "transfers": mm_tr, "nodes": len(node_ids)}

    g = gpd.GeoDataFrame(
        {"k": list(range(len(rows))), "edge_id": [str(r[0]) for r in rows], "name": [r[3] for r in rows],
         "highway": [r[4] for r in rows], "bridge": [r[5] for r in rows], "tunnel": [r[6] for r in rows],
         "layer": [r[7] for r in rows]},
        geometry=[_wkt.loads(r[9]) for r in rows], crs="EPSG:4326")
    layers = [basemap] + [b for b in BASEMAP_LAYERS if b != basemap]
    m = rs.render_edges(
        g, palette="mono", basemap=basemap, basemaps=layers, tooltip=["name", "highway"],
        road_popup=False, street_view=False, arrows=False, filter_control=False, name=f"{name}: route planner",
        boundary=_boundary_geojson(con))
    html = m.html.replace("</body>", _panel(data) + "</body>", 1)

    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(html, encoding="utf-8")
    logger.info(f"route-map: {len(rows):,} edges, modes {', '.join(modes)}"
                f"{' + across modes' if data['mm'] else ''} -> {out}")
    return out


def _panel(data):
    return (_CSS + _HTML + "<script>const RM = " + json.dumps(data, separators=(",", ":"))
            + ";</script>\n<script>" + _JS + "</script>\n")


_CSS = """<style>
#map{right:300px!important} body{--rs-side:300px}
#rm-panel{position:fixed;top:0;right:0;bottom:0;width:300px;box-sizing:border-box;padding:14px 16px;
  overflow-y:auto;background:#fff;border-left:1px solid #ddd;font:14px/1.45 system-ui,sans-serif;color:#222;z-index:5}
#rm-panel h3{margin:0 0 6px;font-size:16px}
#rm-panel .rm-hint{margin:0 0 10px;color:#555;font-size:13px}
#rm-panel fieldset{border:0;padding:0;margin:0 0 8px}
#rm-panel label{margin-right:10px;cursor:pointer}
#rm-panel select{font:inherit;padding:3px 6px}
#rm-result{margin:10px 0;padding:10px 0;border-top:1px solid #eee;border-bottom:1px solid #eee;min-height:20px}
#rm-result .rm-total{font-size:18px;font-weight:600}
#rm-result .rm-leg{display:flex;gap:6px;align-items:center;margin-top:4px}
#rm-result .rm-dot{width:10px;height:10px;border-radius:50%;display:inline-block}
#rm-result ol{margin:6px 0 0;padding-left:20px;color:#444;font-size:13px}
#rm-panel button{padding:5px 12px;border:1px solid #bbb;border-radius:6px;background:#f7f7f7;cursor:pointer}
#rm-panel .rm-note{margin-top:10px;color:#777;font-size:12px}
</style>
"""

_HTML = """<div id="rm-panel">
  <h3>Route planner</h3>
  <p class="rm-hint">Click the map to set the start, then the end. Drag the markers to move them.</p>
  <fieldset><select id="rm-mode"></select></fieldset>
  <fieldset id="rm-weight">
    <label><input type="radio" name="rm-w" value="time" checked> fastest</label>
    <label><input type="radio" name="rm-w" value="length"> shortest</label>
  </fieldset>
  <div id="rm-result">No route yet.</div>
  <button id="rm-clear">Clear</button>
  <p class="rm-note" id="rm-note"></p>
</div>
"""

_JS = r"""
(function () {
  const COLORS = {walking: "#16a34a", driving: "#dc2626", cycling: "#2563eb"};
  const LABEL = {walking: "Walk", driving: "Drive", cycling: "Cycle"};
  const D = RM, hub = D.modes.indexOf("walking");
  let fid = null, markers = {A: null, B: null}, picked = {A: null, B: null};

  // ---- small binary heap -----------------------------------------------------------------------
  function Heap() { this.a = []; }
  Heap.prototype.push = function (d, x) {
    const a = this.a; a.push([d, x]); let i = a.length - 1;
    while (i > 0) { const p = (i - 1) >> 1; if (a[p][0] <= a[i][0]) break; [a[p], a[i]] = [a[i], a[p]]; i = p; }
  };
  Heap.prototype.pop = function () {
    const a = this.a, top = a[0], last = a.pop();
    if (a.length) { a[0] = last; let i = 0;
      for (;;) { const l = 2 * i + 1, r = l + 1; let m = i;
        if (l < a.length && a[l][0] < a[m][0]) m = l; if (r < a.length && a[r][0] < a[m][0]) m = r;
        if (m === i) break; [a[m], a[i]] = [a[i], a[m]]; i = m; } }
    return top;
  };

  // ---- one mode: edge-based Dijkstra over edge_graph, as route() -----------------------------------
  const G = {};
  for (const m of D.modes) { const g = D.graphs[m]; g.pos = new Map(g.k.map((k, i) => [k, i])); G[m] = g; }
  function routeOne(mode, s, t, weight) {
    const g = G[mode], w = weight === "length" ? g.len : g.cost;
    const dist = new Map([[s, 0]]), prev = new Map(), h = new Heap(); h.push(0, s);
    while (h.a.length) {
      const [d, u] = h.pop(); if (d > dist.get(u)) continue; if (u === t) break;
      const i = g.pos.get(u), wu = w[i];
      for (const v of g.next[i]) { const nd = d + wu;
        if (nd < (dist.has(v) ? dist.get(v) : Infinity)) { dist.set(v, nd); prev.set(v, u); h.push(nd, v); } }
    }
    if (!dist.has(t)) return null;
    const path = [t]; while (path[path.length - 1] !== s) path.push(prev.get(path[path.length - 1]));
    path.reverse();
    let time = 0, len = 0; for (const k of path) { const i = g.pos.get(k); time += g.cost[i]; len += g.len[i]; }
    return {time, len, legs: [{mode, edges: path, time, len}], transfers: 0};
  }

  // ---- across modes: (node, mode, phase) Dijkstra, as route_multimodal() -------------------------
  let MM = null;
  if (D.mm) {
    const N = D.mm.nodes, adj = new Map(), tadj = new Map();
    for (const [m, s, t, k, c] of D.mm.edges) { const key = m * N + s; if (!adj.has(key)) adj.set(key, []); adj.get(key).push([t, k, c]); }
    for (const [n, fm, tm, c] of D.mm.transfers) { const key = fm * N + n; if (!tadj.has(key)) tadj.set(key, []); tadj.get(key).push([tm, c]); }
    MM = {N, adj, tadj};
  }
  function routeMulti(sNode, tNode, allowed) {
    const {N, adj, tadj} = MM, key = (n, m, p) => (n * 8 + m) * 3 + p;
    const start = key(sNode, hub, 0), dist = new Map([[start, 0]]), prev = new Map(), h = new Heap();
    h.push(0, [sNode, hub, 0]); let goal = null;
    while (h.a.length) {
      const [d, [n, m, p]] = h.pop(), st = key(n, m, p); if (d > dist.get(st)) continue;
      if (n === tNode && m === hub) { goal = st; break; }
      for (const [t, k, c] of adj.get(m * N + n) || []) {
        const ns = key(t, m, p), nd = d + c;
        if (nd < (dist.has(ns) ? dist.get(ns) : Infinity)) { dist.set(ns, nd); prev.set(ns, [st, {k, m, c}]); h.push(nd, [t, m, p]); }
      }
      for (const [tm, c] of tadj.get(m * N + n) || []) {
        if (!allowed.has(tm)) continue;
        let np = p;
        if (m === hub && tm !== hub) { if (p !== 0) continue; np = 1; }
        else if (m !== hub && tm === hub) { if (p !== 1) continue; np = 2; }
        const ns = key(n, tm, np), nd = d + c;
        if (nd < (dist.has(ns) ? dist.get(ns) : Infinity)) { dist.set(ns, nd); prev.set(ns, [st, {tr: true, c}]); h.push(nd, [n, tm, np]); }
      }
    }
    if (goal === null) return null;
    const steps = []; let s = goal; while (s !== start) { const [ps, step] = prev.get(s); steps.push(step); s = ps; }
    steps.reverse();
    const legs = []; let transfers = 0, trTime = 0;
    for (const st of steps) {
      if (st.tr) { transfers++; trTime += st.c; continue; }
      const mode = D.modes[st.m], last = legs[legs.length - 1];
      if (!last || last.mode !== mode) legs.push({mode, edges: [], time: 0, len: 0});
      const leg = legs[legs.length - 1]; leg.edges.push(st.k); leg.time += st.c; leg.len += D.len[st.k];
    }
    let time = trTime, len = 0; for (const l of legs) { time += l.time; len += l.len; }
    return {time, len, legs, transfers};
  }

  // ---- the page ------------------------------------------------------------------------------------
  const $ = (id) => document.getElementById(id);
  // One mode, or walk + drive (with the mm tables): what route() / route_multimodal() support today.
  const CHOICES = D.modes.map((m) => ({label: LABEL[m], modes: [m]}));
  if (MM && D.modes.includes("driving")) CHOICES.push({label: "Walk + drive", modes: ["walking", "driving"]});
  function ticked() { return CHOICES[+$("rm-mode").value].modes; }
  function fmtTime(s) { s = Math.round(s); return s < 60 ? s + " s" : Math.floor(s / 60) + " min " + (s % 60) + " s"; }
  function fmtLen(m) { return m < 1000 ? Math.round(m) + " m" : (m / 1000).toFixed(1) + " km"; }
  function inMode(mode) { return (k) => G[mode].pos.has(k); }

  function segDist(p, a, b) {                          // pixel distance from p to segment ab
    const dx = b.x - a.x, dy = b.y - a.y, L = dx * dx + dy * dy;
    const t = L ? Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / L)) : 0;
    return Math.hypot(p.x - a.x - t * dx, p.y - a.y - t * dy);
  }
  function pick(lngLat, ok) {                        // the nearest road of the mode, within 80 px
    const p = map.project(lngLat);
    for (const r of [10, 30, 80]) {
      const fs = map.queryRenderedFeatures([[p.x - r, p.y - r], [p.x + r, p.y + r]])
        .filter((f) => f.properties && f.properties.k !== undefined && ok(+f.properties.k));
      if (!fs.length) continue;
      let best = null, bestD = Infinity;
      for (const f of fs) {
        const g = f.geometry, lines = g.type === "LineString" ? [g.coordinates] : g.coordinates;
        for (const line of lines) for (let i = 1; i < line.length; i++) {
          const d = segDist(p, map.project(line[i - 1]), map.project(line[i]));
          if (d < bestD) { bestD = d; best = +f.properties.k; }
        }
      }
      return best;
    }
    return null;
  }

  // The route as its own line, drawn at the top of its level: over the roads it crosses at an
  // intersection, but still under a bridge above it (tunnel / ground / bridge are roadstyle's three
  // bands, on its "roads" source: same geometry, no second copy of it).
  const BANDS = [["<", "roads-casing"], ["==", "roads-bridge-casing"], [">", "roads-highlight"]];
  function paint(legs) {
    BANDS.forEach((_, b) => ["", "-casing"].forEach((c) => { if (map.getLayer(`rm-route${c}-${b}`)) map.removeLayer(`rm-route${c}-${b}`); }));
    const seen = new Set(), color = ["match", ["id"]];
    for (const l of legs || []) {
      const ids = l.edges.map((k) => fid[k]).filter((i) => !seen.has(i) && seen.add(i));
      if (ids.length) color.push(ids, COLORS[l.mode]);
    }
    if (!seen.size) return;
    const w = (a, b) => ["interpolate", ["linear"], ["zoom"], 12, a, 18, b];
    BANDS.forEach(([op, before], b) => {
      const line = {type: "line", source: "roads", layout: {"line-cap": "round", "line-join": "round"},
                    filter: ["all", ["in", ["id"], ["literal", [...seen]]], [op, ["coalesce", ["get", "lvl"], 0], 0]]};
      const at = map.getLayer(before) ? before : undefined;
      map.addLayer({id: `rm-route-casing-${b}`, ...line, paint: {"line-color": "#fff", "line-width": w(6, 14)}}, at);
      map.addLayer({id: `rm-route-${b}`, ...line, paint: {"line-color": [...color, "#000"], "line-width": w(3.5, 9)}}, at);
    });
  }

  function show(res, multi) {
    const out = $("rm-result");
    if (!res) { out.textContent = "No route between these points with these modes."; paint(null); return; }
    let h = `<div class="rm-total">${fmtTime(res.time)} · ${fmtLen(res.len)}</div>`;
    if (multi) {
      for (const l of res.legs) h += `<div class="rm-leg"><span class="rm-dot" style="background:${COLORS[l.mode]}"></span>${LABEL[l.mode]}: ${fmtTime(l.time)}, ${fmtLen(l.len)}</div>`;
      if (res.transfers) h += `<div class="rm-leg">${res.transfers} change${res.transfers > 1 ? "s" : ""} of mode</div>`;
    }
    const names = []; for (const l of res.legs) for (const k of l.edges) { const n = D.name[k]; if (n && n !== names[names.length - 1]) names.push(n); }
    if (names.length) h += "<ol>" + names.slice(0, 15).map((n) => `<li>${n}</li>`).join("") + (names.length > 15 ? "<li>…</li>" : "") + "</ol>";
    out.innerHTML = h;
    paint(res.legs);
  }

  function update() {
    const modes = ticked(), multi = modes.length > 1;
    $("rm-weight").style.opacity = multi ? 0.4 : 1;
    $("rm-note").textContent = multi ? "Walk + drive: the car can be taken at any junction (60 s per change). " +
      "Routes go junction to junction and don't apply turn restrictions (as route_multimodal)." : "";
    if (!markers.A || !markers.B) return;
    if (!multi) {
      const ok = inMode(modes[0]);
      const s = pick(markers.A.getLngLat(), ok), t = pick(markers.B.getLngLat(), ok);
      if (s === null || t === null) { $("rm-result").textContent = "Move the markers onto roads of this mode."; return; }
      const w = document.querySelector('input[name="rm-w"]:checked').value;
      const res = routeOne(modes[0], s, t, w);
      window.rmLast = {modes, s, t, res};                // for tests and scripts
      show(res, false);
    } else {
      const ok = inMode("walking");
      const s = pick(markers.A.getLngLat(), ok), t = pick(markers.B.getLngLat(), ok);
      if (s === null || t === null) { $("rm-result").textContent = "Move the markers onto walkable roads."; return; }
      const res = routeMulti(D.src[s], D.tgt[t], new Set(modes.map((m) => D.modes.indexOf(m))));
      window.rmLast = {modes, s, t, sNode: D.src[s], tNode: D.tgt[t], res};
      show(res, true);
    }
  }

  function setMarker(which, lngLat) {
    if (!markers[which]) {
      markers[which] = new maplibregl.Marker({color: which === "A" ? "#111827" : "#6b7280", draggable: true})
        .setLngLat(lngLat).addTo(map);
      markers[which].on("dragend", update);
    } else markers[which].setLngLat(lngLat);
  }

  function init() {
    const ids = rsQuery(() => true), props = rsGetProps(ids);
    fid = new Array(D.n); for (let i = 0; i < ids.length; i++) fid[props[i].k] = ids[i];
    $("rm-mode").innerHTML = CHOICES.map((c, i) => `<option value="${i}">${c.label}</option>`).join("");
    $("rm-mode").addEventListener("change", update);
    for (const r of document.querySelectorAll('input[name="rm-w"]')) r.addEventListener("change", update);
    $("rm-clear").addEventListener("click", () => {
      for (const w of ["A", "B"]) if (markers[w]) { markers[w].remove(); markers[w] = null; }
      paint(null); $("rm-result").textContent = "No route yet.";
    });
    map.on("click", (e) => {
      if (!markers.A || markers.B) { if (markers.B) { markers.B.remove(); markers.B = null; } setMarker("A", e.lngLat); $("rm-result").textContent = "Now click the end."; paint(null); }
      else { setMarker("B", e.lngLat); update(); }
    });
    window.rmRoute = function (a, b) { setMarker("A", a); setMarker("B", b); update(); };  // for scripted use
    update();
  }

  (function wait() {
    try { if (window.map && window.rsQuery && map.isStyleLoaded() && rsQuery(() => true).length) return init(); } catch (e) {}
    setTimeout(wait, 200);
  })();
})();
"""
