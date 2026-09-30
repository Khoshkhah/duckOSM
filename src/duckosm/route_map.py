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

    # The page opens with a route: markers on the roads nearest 30 % and 70 % along the diagonal.
    x0, y0, x1, y1 = con.execute(f"SELECT min(ST_XMin(geometry)), min(ST_YMin(geometry)), max(ST_XMax(geometry)), "
                                 f"max(ST_YMax(geometry)) FROM {modes[0]}.edges").fetchone()
    near = (f"SELECT ST_X(p), ST_Y(p) FROM (SELECT ST_LineInterpolatePoint(geometry, 0.5) AS p FROM {modes[0]}.edges "
            f"ORDER BY ST_Distance(ST_Centroid(geometry), ST_Point(?, ?)) LIMIT 1)")
    data["start"], data["end"] = (list(con.execute(near, [x0 + (x1 - x0) * f, y0 + (y1 - y0) * f]).fetchone())
                                  for f in (0.3, 0.7))

    g = gpd.GeoDataFrame(
        {"k": list(range(len(rows))), "edge_id": [str(r[0]) for r in rows], "name": [r[3] for r in rows],
         "highway": [r[4] for r in rows], "bridge": [r[5] for r in rows], "tunnel": [r[6] for r in rows],
         "layer": [r[7] for r in rows]},
        geometry=[_wkt.loads(r[9]) for r in rows], crs="EPSG:4326")
    layers = [basemap] + [b for b in BASEMAP_LAYERS if b != basemap]
    m = rs.render_edges(
        g, palette="mono", basemap=basemap, basemaps=layers, tooltip=["name", "highway", "edge_id"],
        filter_control=False, name=f"{name}: route planner",
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
#rm-result .rm-edges{margin-top:8px;font-size:12px} #rm-result .rm-edges summary{cursor:pointer;color:#555}
#rm-result .rm-edges code{font-size:11px;user-select:all}
#rm-panel .rm-clicked{margin:8px 0 0;font-size:12px;color:#555} #rm-panel .rm-clicked code{user-select:all}
#rm-panel .rm-note{margin-top:10px;color:#777;font-size:12px}
</style>
"""

_HTML = """<div id="rm-panel">
  <h3>Route planner</h3>
  <p class="rm-hint">Drag the two markers to set the start and the end. Click a road to copy its
  <code>edge_id</code>.</p>
  <fieldset><select id="rm-mode"></select></fieldset>
  <fieldset id="rm-weight">
    <label><input type="radio" name="rm-w" value="time" checked> fastest</label>
    <label><input type="radio" name="rm-w" value="length"> shortest</label>
  </fieldset>
  <div id="rm-result"></div>
  <p class="rm-clicked" id="rm-clicked"></p>
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

  // ---- snapping a marker to a road: the nearest in the map's data, not among the roads drawn on
  // screen, so it works at any zoom and with the other marker off screen ------------------------
  const CELL = 0.002;                                  // grid cell, degrees (~200 m)
  let GEO = null, GRID = null, KX = 1;
  function buildIndex() {
    const fs = map.getSource("roads")._data.features;  // roadstyle's inline source (route-map never tiles)
    GEO = new Array(D.n); GRID = new Map();
    KX = Math.cos(fs[0].geometry.coordinates[0][1] * Math.PI / 180);
    for (const f of fs) {
      const k = +f.properties.k, c = f.geometry.coordinates; GEO[k] = c;
      for (let i = 1; i < c.length; i++) {
        const [xa, ya] = c[i - 1], [xb, yb] = c[i];
        for (let x = Math.floor(Math.min(xa, xb) / CELL); x <= Math.floor(Math.max(xa, xb) / CELL); x++)
          for (let y = Math.floor(Math.min(ya, yb) / CELL); y <= Math.floor(Math.max(ya, yb) / CELL); y++) {
            const key = x + "," + y; if (!GRID.has(key)) GRID.set(key, new Set()); GRID.get(key).add(k);
          }
      }
    }
  }
  function segM(p, a, b) {                             // metres from p to segment ab (local flat frame)
    const ax = (a[0] - p[0]) * KX, ay = a[1] - p[1], dx = (b[0] - a[0]) * KX, dy = b[1] - a[1], L = dx * dx + dy * dy;
    const t = L ? Math.max(0, Math.min(1, -(ax * dx + ay * dy) / L)) : 0;
    return Math.hypot(ax + t * dx, ay + t * dy) * 111320;
  }
  // The roads of the mode nearest the point (within ~600 m), as two tiers: all of those as close as
  // the nearest (a two-way road's two directions share one line and either may route better), then
  // the others within 30 m of it, tried when the nearest can't route (a one-way road cut by the
  // area's boundary leads nowhere).
  function pick(lngLat, ok) {
    const p = [lngLat.lng, lngLat.lat], cx = Math.floor(p[0] / CELL), cy = Math.floor(p[1] / CELL);
    for (let r = 1; r <= 3; r++) {
      const dk = new Map();
      for (let x = cx - r; x <= cx + r; x++) for (let y = cy - r; y <= cy + r; y++)
        for (const k of GRID.get(x + "," + y) || []) if (!dk.has(k) && ok(k)) {
          const c = GEO[k]; let d = Infinity;
          for (let i = 1; i < c.length; i++) d = Math.min(d, segM(p, c[i - 1], c[i]));
          dk.set(k, d);
        }
      if (!dk.size) continue;
      const near = Math.min(...dk.values());
      if (near > r * CELL * 111320 * KX && r < 3) continue;   // a nearer road may sit in the next ring
      const tier = (lo, hi) => [...dk].filter(([, d]) => d > lo && d <= hi).sort((a, b) => a[1] - b[1]).map(([k]) => k);
      return [tier(-1, near + 1), tier(near + 1, near + 30)];
    }
    return null;
  }

  // The route is its roads, recoloured by leg (rsColor): roadstyle keeps their style and raises
  // them to the top of their level, so a crossing street doesn't cover the route but a bridge
  // above it still does.
  function paint(legs) {
    rsColor(legs ? legs.map((l) => [l.edges.map((k) => fid[k]), COLORS[l.mode]]) : null);
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
    const ks = res.legs.flatMap((l) => l.edges), ps = rsGetProps(ks.map((k) => fid[k]));
    h += `<details class="rm-edges"><summary>${ks.length} edges (edge_id)</summary><ol>` +
         ks.map((k, i) => `<li><code>${ps[i].edge_id}</code> ${D.name[k] || ""}</li>`).join("") + "</ol></details>";
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
      const S = pick(markers.A.getLngLat(), ok), T = pick(markers.B.getLngLat(), ok);
      if (S === null || T === null) { $("rm-result").textContent = "Move the markers onto roads of this mode."; return; }
      const w = document.querySelector('input[name="rm-w"]:checked').value;
      const b = best(S, T, (s, t) => routeOne(modes[0], s, t, w), (r) => w === "length" ? r.len : r.time);
      window.rmLast = {modes, ...b};                     // for tests and scripts
      show(b.res, false);
    } else {
      const ok = inMode("walking");
      const S = pick(markers.A.getLngLat(), ok), T = pick(markers.B.getLngLat(), ok);
      if (S === null || T === null) { $("rm-result").textContent = "Move the markers onto walkable roads."; return; }
      const allowed = new Set(modes.map((m) => D.modes.indexOf(m)));
      const b = best(S, T, (s, t) => routeMulti(D.src[s], D.tgt[t], allowed), (r) => r.time);
      window.rmLast = {modes, ...b, sNode: D.src[b.s], tNode: D.tgt[b.t]};
      show(b.res, true);
    }
  }

  function best(S, T, run, cost) {   // the cheapest route between the nearest roads; if none routes, the next ones
    let b = {s: S[0][0], t: T[0][0], res: null};
    for (const [ss, tt] of [[S[0], T[0]], [S[0].concat(S[1]), T[0].concat(T[1])]]) {
      for (const s of ss) for (const t of tt) {
        const res = run(s, t);
        if (res && (!b.res || cost(res) < cost(b.res))) b = {s, t, res};
      }
      if (b.res) break;
    }
    return b;
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
    buildIndex();
    $("rm-mode").innerHTML = CHOICES.map((c, i) => `<option value="${i}">${c.label}</option>`).join("");
    $("rm-mode").addEventListener("change", update);
    for (const r of document.querySelectorAll('input[name="rm-w"]')) r.addEventListener("change", update);
    // A click on a road (roadstyle's popup): its edge_id is copied and kept in the panel.
    document.addEventListener("rs:select", (e) => {
      const id = !e.detail.overlay && e.detail.properties && e.detail.properties.edge_id;
      if (!id) return;
      $("rm-clicked").innerHTML = `Clicked road: <code>${id}</code> <span id="rm-copied"></span>`;
      if (navigator.clipboard) navigator.clipboard.writeText(String(id)).then(() => { $("rm-copied").textContent = "(copied)"; }, () => {});
    });
    window.rmRoute = function (a, b) { setMarker("A", a); setMarker("B", b); update(); };  // for scripted use
    rmRoute(D.start, D.end);              // opens with a route; drag the markers to change it
  }

  (function wait() {
    try { if (window.map && window.rsQuery && map.isStyleLoaded() && rsQuery(() => true).length) return init(); } catch (e) {}
    setTimeout(wait, 200);
  })();
})();
"""
