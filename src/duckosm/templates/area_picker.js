// duckosm.AreaPicker (src/duckosm/widget.py): the map and the controls. The ideas come from duckOSM Studio's area
// step: search a place, type an OSM relation id, draw a box, click an H3 cell, or load a GeoJSON file. The widget sets
// `area`; Python runs `duckosm get --dry-run` and sets `plan` (the boundary and the extract), and "Get" downloads and cuts.
import maplibregl from "https://esm.sh/maplibre-gl@4.7.1";
const CSS = "https://esm.sh/maplibre-gl@4.7.1/dist/maplibre-gl.css";
const STYLE = "https://tiles.openfreemap.org/styles/positron";
const h3lib = () => import("https://cdn.jsdelivr.net/npm/h3-js@4.2.1/+esm");

const html = `
<style>
.dk { --dk-fg: var(--jp-ui-font-color1, #222); --dk-mut: var(--jp-ui-font-color2, #555); --dk-bg: var(--jp-layout-color1, #fff);
  --dk-bar: var(--jp-layout-color2, #f6f6f6); --dk-line: var(--jp-border-color1, #bbb); --dk-accent: var(--jp-brand-color1, #2563eb);
  --dk-err: var(--jp-error-color1, #b91c1c);
  font: 13px var(--jp-ui-font-family, system-ui, sans-serif); color: var(--dk-fg); background: var(--dk-bg);
  border: 1px solid var(--dk-line); border-radius: 8px; overflow: hidden; }
.dk-bar, .dk-foot { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; padding: 8px; background: var(--dk-bar); }
.dk button, .dk input, .dk select { font: inherit; color: var(--dk-fg); background: var(--dk-bg); border: 1px solid var(--dk-line);
  border-radius: 6px; padding: 4px 8px; }
.dk button { cursor: pointer; }
.dk-tabs button[aria-selected=true] { background: var(--dk-accent); border-color: var(--dk-accent); color: #fff; }
.dk-pane { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
.dk-pane[hidden] { display: none; }
.dk-list { max-height: 140px; overflow: auto; padding: 0 8px; }
.dk .dk-list button { display: block; width: 100%; text-align: left; border: 0; background: none; padding: 3px 0; }
.dk .dk-list button:hover { color: var(--dk-accent); }
.dk .dk-list button:disabled { color: var(--dk-mut); cursor: default; }
.dk-map { height: 420px; }
.dk .dk-get { background: #16a34a; border-color: #16a34a; color: #fff; padding: 6px 16px; font-weight: 600; }
.dk .dk-get:disabled { background: var(--dk-line); border-color: var(--dk-line); color: var(--dk-mut); cursor: default; }
.dk-info b { font-weight: 600; }
.dk-status { color: var(--dk-mut); font-family: var(--jp-code-font-family, ui-monospace, monospace); font-size: 12px; }
.dk-err { color: var(--dk-err); }
.dk .maplibregl-ctrl button { background-color: #fff; border: 0; border-radius: 0; padding: 0; }
</style>
<div class="dk">
  <div class="dk-bar">
    <span class="dk-tabs" role="tablist">
      <button data-tab="search" aria-selected="true">Search</button><button data-tab="osm">OSM id</button><button
       data-tab="box">Box</button><button data-tab="h3">H3 cell</button><button data-tab="file">File</button>
    </span>
    <span class="dk-pane" data-pane="search"><input type="text" class="dk-q" placeholder="City, region or country" size="28"><button class="dk-go">Search</button></span>
    <span class="dk-pane" data-pane="osm" hidden><input type="number" class="dk-osm" placeholder="OSM relation id"><button class="dk-osm-go">Use</button></span>
    <span class="dk-pane" data-pane="box" hidden>Drag a box on the map, or type it:<input type="text" class="dk-bbox" placeholder="W,S,E,N" size="26"><button class="dk-bbox-go">Use</button></span>
    <span class="dk-pane" data-pane="h3" hidden>Level <select class="dk-h3-level"></select> Click a hexagon on the map, or type it:<input type="text" class="dk-h3" placeholder="H3 cell id" size="17"><button class="dk-h3-go">Use</button></span>
    <span class="dk-pane" data-pane="file" hidden><input type="file" class="dk-file" accept=".geojson,.json,application/geo+json"></span>
  </div>
  <div class="dk-list"></div>
  <div class="dk-map"></div>
  <div class="dk-foot">
    <button class="dk-get" disabled>Get</button>
    <span class="dk-info">Choose an area.</span>
    <span class="dk-status"></span>
  </div>
</div>`;

const mb = (b) => (b / 1048576).toFixed(b < 10485760 ? 1 : 0);

function bounds(geom) {
  const b = [180, 90, -180, -90];
  const walk = (c) => (typeof c[0] === "number" ? (b[0] = Math.min(b[0], c[0]), b[1] = Math.min(b[1], c[1]), b[2] = Math.max(b[2], c[0]), b[3] = Math.max(b[3], c[1])) : c.forEach(walk));
  const geoms = geom.type === "FeatureCollection" ? geom.features.map((f) => f.geometry) : [geom.geometry ?? geom];
  geoms.forEach((g) => walk(g.coordinates));
  return [[b[0], b[1]], [b[2], b[3]]];
}

const boxGeom = ([w, s, e, n]) => ({ type: "Polygon", coordinates: [[[w, s], [e, s], [e, n], [w, n], [w, s]]] });

export default {
  render({ model, el }) {
    if (!document.querySelector(`link[href="${CSS}"]`)) {
      document.head.insertAdjacentHTML("beforeend", `<link rel="stylesheet" href="${CSS}">`);
    }
    el.innerHTML = html;
    // the keys typed in the widget are for the widget, not for JupyterLab's shortcuts of the notebook ("m", "o", ...)
    el.setAttribute("data-lm-suppress-shortcuts", "true");
    for (const t of ["keydown", "keypress", "keyup"]) el.addEventListener(t, (e) => e.stopPropagation());
    const $ = (s) => el.querySelector(s);
    // `own`: the change comes from this widget, not from Python (see showArea)
    let own = false;
    const set = (area) => { own = true; model.set("area", area); model.save_changes(); own = false; };
    let tab = "search";

    const map = new maplibregl.Map({ container: $(".dk-map"), style: STYLE, center: [10, 50], zoom: 3, attributionControl: { compact: true } });
    map.addControl(new maplibregl.NavigationControl({ showCompass: false }));
    const empty = { type: "FeatureCollection", features: [] };
    map.on("load", () => {
      for (const [id, color] of [["draft", "#f59e0b"], ["area", "#2563eb"]]) {
        map.addSource(id, { type: "geojson", data: empty });
        map.addLayer({ id: id + "-fill", type: "fill", source: id, paint: { "fill-color": color, "fill-opacity": 0.12 } });
        map.addLayer({ id: id + "-line", type: "line", source: id, paint: { "line-color": color, "line-width": 2 } });
      }
      showPlan();
    });
    const draft = (geom) => map.getSource("draft")?.setData(geom ? { type: "Feature", geometry: geom, properties: {} } : empty);
    // the chosen area at once, before the dry run of Python answers (a slow or lost answer must not hide it)
    const show = (g) => {
      const src = map.getSource("draft");
      if (!src || !g) return;
      src.setData(g.type === "Feature" || g.type === "FeatureCollection" ? g : { type: "Feature", geometry: g, properties: {} });
      map.fitBounds(bounds(g), { padding: 30, maxZoom: 14 });
    };

    // the tabs
    const selectTab = (name) => {
      tab = name;
      el.querySelectorAll(".dk-tabs button").forEach((x) => x.setAttribute("aria-selected", x.dataset.tab === tab));
      el.querySelectorAll(".dk-pane").forEach((p) => (p.hidden = p.dataset.pane !== tab));
      $(".dk-list").innerHTML = "";
      map.getCanvas().style.cursor = tab === "box" || tab === "h3" ? "crosshair" : "";
    };
    for (const b of el.querySelectorAll(".dk-tabs button")) b.onclick = () => selectTab(b.dataset.tab);

    // an area that Python sets (picker.area = {...}): its tab, with its value in the field
    const showArea = () => {
      const a = model.get("area") || {};
      const osmId = a.osm_id ?? a.geojson?.properties?.osm_id;
      if (a.mode === "place") { selectTab("search"); $(".dk-q").value = a.place ?? ""; }
      else if (a.mode === "osm_id" || (a.mode === "boundary" && osmId)) { selectTab("osm"); $(".dk-osm").value = osmId ?? ""; }
      else if (a.mode === "bbox") { selectTab("box"); $(".dk-bbox").value = (a.bbox || []).join(","); }
      else if (a.mode === "h3") { selectTab("h3"); $(".dk-h3").value = a.h3_cell ?? ""; }
      else if (a.mode === "boundary") selectTab("file");
    };
    showArea();
    model.on("change:area", () => { if (!own) showArea(); });

    // search and OSM id: the browser gets the polygon and gives it to `duckosm get --boundary`, so the program sends no
    // request for it. Search: Photon (as duckOSM Studio). Polygon of a relation: polygons.openstreetmap.fr, else Nominatim
    // (Nominatim stops an address that sends many requests, and the browser then only sees "Failed to fetch").
    const info = (text) => { $(".dk-info").classList.remove("dk-err"); $(".dk-info").textContent = text; };
    const getJson = async (url) => { const r = await fetch(url); if (!r.ok) throw new Error(`${new URL(url).host} answered ${r.status}`); return r.json(); };
    const polygon = async (t, id) => {
      const errors = [];
      if (t === "R") {
        try {
          const g = await getJson(`https://polygons.openstreetmap.fr/get_geojson.py?id=${id}&params=0`);
          if (/Polygon$/.test(g?.type ?? "")) return g;
          errors.push("polygons.openstreetmap.fr has no polygon");
        } catch (e) { errors.push(e.message); }
      }
      try {
        const d = await getJson("https://nominatim.openstreetmap.org/lookup?" + new URLSearchParams({ osm_ids: t + id, format: "geojson", polygon_geojson: "1" }));
        const g = d.features?.[0]?.geometry;
        if (/Polygon$/.test(g?.type ?? "")) return g;
        errors.push("Nominatim has no polygon");
      } catch (e) { errors.push(`Nominatim: ${e.message === "Failed to fetch" ? "no answer (too many requests?)" : e.message}`); }
      throw new Error(errors.join("; "));
    };
    const useOsm = async (t, id, name) => {
      const what = { R: "relation", W: "way", N: "node" }[t];
      info(`Getting the boundary of OSM ${what} ${id} ...`);
      try {
        const geometry = await polygon(t, id);
        name = name || `${what}_${id}`;
        show(geometry);
        info(`${name}: finding the extract ...`);
        set({ mode: "boundary", geojson: { type: "Feature", properties: { name, osm_type: what, osm_id: id }, geometry }, name });
      } catch (e) { info(`OSM ${what} ${id}: no boundary (${e.message})`); }
    };
    const search = async () => {
      const q = $(".dk-q").value.trim();
      if (!q) return;
      const list = $(".dk-list");
      list.textContent = "Searching ...";
      try {
        const rows = (await getJson("https://photon.komoot.io/api/?" + new URLSearchParams({ q, limit: "8" }))).features.map((f) => f.properties);
        list.innerHTML = rows.length ? "" : "No match.";
        for (const p of rows) {
          // a relation or a closed way can have a boundary; a node is a point
          const where = [p.city, p.state, p.country].filter((x) => x && x !== p.name).join(", ");
          const b = document.createElement("button");
          b.textContent = `${p.name}${where ? ", " + where : ""} (${p.osm_value})` + (p.osm_type === "N" ? ", a point: no boundary" : "");
          b.disabled = p.osm_type === "N";
          b.onclick = () => { list.innerHTML = ""; useOsm(p.osm_type, p.osm_id, p.name || q); };
          list.append(b);
        }
      } catch (e) { list.textContent = `The search failed: ${e.message}`; }
    };
    $(".dk-go").onclick = search;
    $(".dk-q").addEventListener("keydown", (e) => { if (e.key === "Enter") search(); });

    $(".dk-osm-go").onclick = () => { const id = parseInt($(".dk-osm").value, 10); if (id > 0) useOsm("R", id); };

    // box: drag on the map, or type W,S,E,N
    $(".dk-bbox-go").onclick = () => {
      const v = $(".dk-bbox").value.split(",").map(Number);
      if (v.length === 4 && v.every(Number.isFinite)) { show(boxGeom(v)); set({ mode: "bbox", bbox: v }); }
    };
    let start = null;
    map.on("mousedown", (e) => {
      if (tab !== "box") return;
      e.preventDefault();
      map.dragPan.disable();
      start = e.lngLat;
    });
    map.on("mousemove", (e) => {
      if (start) draft(boxGeom([Math.min(start.lng, e.lngLat.lng), Math.min(start.lat, e.lngLat.lat), Math.max(start.lng, e.lngLat.lng), Math.max(start.lat, e.lngLat.lat)]));
    });
    map.on("mouseup", (e) => {
      if (!start) return;
      const r = (x) => Math.round(x * 1e5) / 1e5;
      const v = [Math.min(start.lng, e.lngLat.lng), Math.min(start.lat, e.lngLat.lat), Math.max(start.lng, e.lngLat.lng), Math.max(start.lat, e.lngLat.lat)].map(r);
      start = null;
      map.dragPan.enable();
      if (v[0] === v[2] || v[1] === v[3]) return draft(null);
      $(".dk-bbox").value = v.join(",");
      set({ mode: "bbox", bbox: v });
    });

    // H3: the levels 5 to 10, as Studio's H3 tab; one cell (`duckosm get --h3-cell`)
    const level = $(".dk-h3-level");
    for (let i = 5; i <= 10; i++) level.add(new Option(String(i), String(i), i === 7, i === 7));
    const useCell = async (cell) => {
      const h3 = await h3lib();
      if (!h3.isValidCell(cell)) return ($(".dk-info").textContent = "This is not an H3 cell id.");
      const ring = h3.cellToBoundary(cell, true);
      show({ type: "Polygon", coordinates: [ring] });
      $(".dk-h3").value = cell;
      set({ mode: "h3", h3_cell: cell });
    };
    map.on("click", async (e) => {
      if (tab !== "h3") return;
      const h3 = await h3lib();
      useCell(h3.latLngToCell(e.lngLat.lat, e.lngLat.lng, Number(level.value)));
    });
    $(".dk-h3-go").onclick = () => useCell($(".dk-h3").value.trim().toLowerCase());

    // a GeoJSON file
    $(".dk-file").onchange = async (e) => {
      const f = e.target.files[0];
      if (!f) return;
      try {
        const geojson = JSON.parse(await f.text());
        show(geojson);
        set({ mode: "boundary", geojson, name: f.name.split(".")[0] });
      } catch (err) { $(".dk-info").textContent = `${f.name}: not a GeoJSON file (${err.message})`; }
    };

    // the dry run: the boundary that the cut takes, and the extract
    const showPlan = () => {
      const plan = model.get("plan") || {};
      const info = $(".dk-info");
      info.classList.toggle("dk-err", !!plan.error);
      if (plan.error) info.textContent = plan.error;
      else if (plan.extract) {
        const x = plan.extract;
        info.innerHTML = `<b>${plan.name}</b>: extract <b>${x.name}</b> (${x.id}), ${x.bytes ? mb(x.bytes) + " MB" : "size unknown"}${x.cached ? ", in the cache" : ", to download"}`;
      } else if (!Object.keys(model.get("area") || {}).length) info.textContent = "Choose an area.";
      else info.textContent = "";
      $(".dk-get").disabled = !plan.extract || model.get("busy");
      $(".dk-get").textContent = model.get("busy") ? "Getting ..." : "Get";
      if (map.getSource("area")) {
        map.getSource("area").setData(plan.boundary || empty);
        if (plan.boundary) { draft(null); map.fitBounds(bounds(plan.boundary), { padding: 30, maxZoom: 14 }); }
      }
    };
    const showStatus = () => { $(".dk-status").textContent = model.get("status"); };
    model.on("change:plan", showPlan);
    model.on("change:busy", showPlan);
    model.on("change:status", showStatus);
    showStatus();
    $(".dk-get").onclick = () => {
      // the answer of the kernel can take a moment: show at once that the click came
      $(".dk-get").disabled = true;
      $(".dk-get").textContent = "Getting ...";
      $(".dk-status").textContent = "starting duckosm get ...";
      model.send({ type: "get" });
    };

    return () => map.remove();
  },
};
