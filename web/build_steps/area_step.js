// The Area steps, as duckOSM Studio has them: card "Choose an area" (a place name, a box that the person draws on the map, a boundary file, or a click on the
// map: "What is here?") and card "The area" (the name, the numbers, the boundary file, Unselect). The steps draw the area on the map and give its
// polygons, its box and its name to the other steps. They need a MapLibre map and roadstyle's parts (`rs`: search, drop, tabs, toggle, kpi).
import { areaKm2, bbox, box, esc, geometry, polygonsOf, slug } from "./core.js";

const NOMINATIM = "https://nominatim.openstreetmap.org";
// the drawing tools of Studio (terra-draw), and the union of the shapes (polyclip-ts): the same versions, loaded when the tab Draw opens the first time
const H3 = "https://cdn.jsdelivr.net/npm/h3-js@4.2.1/+esm", H3_MAX_CELLS = 500, H3_MAX_KM2 = 5000, H3_MAX_GRID = 4000;      // H3: the same library and limits as Studio (a mistake cannot ask for a huge build)
const TERRA = "https://cdn.jsdelivr.net/npm/terra-draw@1.38.0/+esm", TERRA_MAPLIBRE = "https://cdn.jsdelivr.net/npm/terra-draw-maplibre-gl-adapter@1.5.0/+esm", POLYCLIP = "https://cdn.jsdelivr.net/npm/polyclip-ts@0.16.8/+esm";
const icon = (d) => `<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;
const ICONS = {
	search: icon('<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>'), draw: icon('<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/>'),
	file: icon('<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9Z"/><path d="M14 3v6h6"/>'), target: icon('<circle cx="12" cy="12" r="8"/><path d="M12 2v4M12 18v4M2 12h4M18 12h4"/>'),
	download: icon('<path d="M12 4v12M7 11l5 5 5-5M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3"/>'),
	rectangle: icon('<rect x="3.5" y="5.5" width="17" height="13" rx="1"/>'), polygon: icon('<path d="m12 3 8.5 6.2-3.2 10.3H6.7L3.5 9.2Z"/>'), circle: icon('<circle cx="12" cy="12" r="8.5"/>'),
	hex: icon('<path d="M12 2.5 20.2 7.2v9.6L12 21.5l-8.2-4.7V7.2Z"/>'),
	freehand: icon('<path d="M3 15c2-5 5-8 7-5s1 7 4 7 4-6 7-9"/>'), select: icon('<path d="m4 4 7 17 2.5-7.5L21 11Z"/>'), merge: icon('<rect x="3" y="3" width="11" height="11" rx="1"/><rect x="10" y="10" width="11" height="11" rx="1"/>'),
	trash: icon('<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>'), x: icon('<path d="M6 6l12 12M18 6 6 18"/>'),
};

// Nominatim answers one request each second (its rules): the requests go through one queue, and the answers of a URL are kept
let queue = Promise.resolve(), last = 0;
const cache = new Map();
function nominatimGet(path, params) {
	const url = `${NOMINATIM}/${path}?` + new URLSearchParams({ format: "geojson", polygon_geojson: "1", extratags: "1", ...params });
	if (cache.has(url)) return cache.get(url);
	const p = (queue = queue.catch(() => {}).then(async () => {
		await new Promise((ok) => setTimeout(ok, Math.max(0, last + 1000 - Date.now())));
		last = Date.now();
		const r = await fetch(url);
		if (!r.ok) throw new Error(`Nominatim answered ${r.status}`);
		return (await r.json()).features ?? [];
	}));
	cache.set(url, p);
	p.catch(() => cache.delete(url));
	if (cache.size > 20) cache.delete(cache.keys().next().value);
	return p;
}

// a Photon match as a Nominatim feature. A relation or a way can have a boundary: its geometry is its box until `boundariesOf` gets the real one. A node is a point.
function photonFeature(f) {
	const p = f.properties, e = p.extent;      // [west, north, east, south]
	const above = [...new Set([p.city, p.county, p.state, p.country])].filter((x) => x && x !== p.name);
	return { type: "Feature", geometry: e ? geometry([[[[e[0], e[3]], [e[2], e[3]], [e[2], e[1]], [e[0], e[1]], [e[0], e[3]]]]]) : f.geometry,
		properties: { photon: p.osm_type !== "N", osm_type: { R: "relation", W: "way", N: "node" }[p.osm_type], osm_id: p.osm_id, name: p.name, display_name: [p.name, ...above].join(", "),
			addresstype: ["house", "other", "street"].includes(p.type) ? p.osm_value : p.type } };
}

/** The default search, as in Studio: the places of Photon (komoot) for a name. Gives Nominatim-shaped features (see `photonFeature`). */
export async function photonSearch(text, signal) {
	const r = await fetch("https://photon.komoot.io/api/?" + new URLSearchParams({ q: text, limit: "10" }), { signal });
	if (!r.ok) throw new Error(`Photon answered ${r.status}`);
	return ((await r.json()).features ?? []).map(photonFeature);
}

/** The real boundaries of a list of Photon matches: one Nominatim lookup for all its relations and ways. Each match gets its boundary, or stays as it was. */
export async function boundariesOf(feats) {
	const key = (p) => p.osm_type + p.osm_id;
	const ids = feats.filter((f) => f.properties.photon).map((f) => f.properties.osm_type[0].toUpperCase() + f.properties.osm_id);
	if (!ids.length) return feats;
	const byId = new Map((await nominatimGet("lookup", { osm_ids: ids.join(",") })).map((g) => [key(g.properties), g]));
	return feats.map((f) => { const g = f.properties.photon && byId.get(key(f.properties)); return g ? { ...g, properties: { ...g.properties, display_name: f.properties.display_name, addresstype: f.properties.addresstype } } : f; });
}

/** The default "What is here?": the places around a point, at five levels, from Nominatim. */
export async function nominatimAt(lat, lon) {
	const found = [];
	for (const zoom of [14, 10, 8, 5, 3]) {
		for (const f of await nominatimGet("reverse", { lat, lon, zoom })) if (!found.some((g) => g.properties.osm_id === f.properties.osm_id && g.properties.osm_type === f.properties.osm_type)) found.push(f);
	}
	return found;
}

/**
 * initAreaStep({
 *   root,      // the element of the card "Choose an area"
 *   detail,    // the element of the card "The area" (optional: without it, its parts go under `root`)
 *   map,       // the MapLibre map: the step draws the area on it, and the box is drawn on it
 *   rs,        // roadstyle's parts: { search, drop, tabs, toggle, kpi }
 *   geocode,   // optional: (text, signal) gives Nominatim-shaped features (default: `photonSearch`, as in Studio)
 *   boundaries,// optional: (features) gives the same features with their boundaries (default: `boundariesOf`)
 *   buffer,    // optional: (geometry, metres) gives the geometry grown by that distance (a server does it): with it the card "The area" has the field Buffer
 *   reverse,   // optional: (lat, lon) gives Nominatim-shaped features (default: `nominatimAt`)
 *   padding,   // optional: () gives the padding of the map for fitBounds (the panel covers a part of the map)
 *   onChange,  // optional: the area or its name is new
 * })
 * It returns { area(), set(geometry, name, from, { zoom, show }), clear(), zoom(), drawMode() }. area() is null or { name, geometry, polygons, bbox, km2, from }.
 */
export function initAreaStep({ root, detail = root, map, rs, geocode = photonSearch, boundaries = boundariesOf, buffer = null, reverse = nominatimAt, padding = () => 40, onChange = () => {} }) {
	root.classList.add("bs", "bs-area");
	detail.classList.add("bs", "bs-area");
	const unselect = '<button type="button" data-as="unselect" disabled>Unselect</button>';
	root.innerHTML = `<div class="rs-tabs" role="tablist" aria-label="The ways to make an area">
			<button type="button" class="rs-tab on" data-tab="search">${ICONS.search}Search</button>
			<button type="button" class="rs-tab" data-tab="draw">${ICONS.draw}Draw</button>
			<button type="button" class="rs-tab" data-tab="file">${ICONS.file}File</button>
			<button type="button" class="rs-tab" data-tab="h3">${ICONS.hex}H3</button></div>
		<section data-pane="search"><div data-as="search"></div><div class="rs-opts" role="radiogroup" aria-label="The places found" data-as="found"></div></section>
		<section data-pane="draw" hidden>
			<div class="rs-toolrow" role="toolbar" aria-label="Drawing tools">
				<button type="button" data-mode="rectangle" aria-pressed="false" aria-keyshortcuts="R" aria-label="Rectangle" title="Rectangle (R)">${ICONS.rectangle}</button>
				<button type="button" data-mode="polygon" aria-pressed="false" aria-keyshortcuts="P" aria-label="Polygon" title="Polygon (P)">${ICONS.polygon}</button>
				<button type="button" data-mode="circle" aria-pressed="false" aria-keyshortcuts="C" aria-label="Circle" title="Circle (C)">${ICONS.circle}</button>
				<button type="button" data-mode="freehand" aria-pressed="false" aria-keyshortcuts="F" aria-label="Freehand" title="Freehand (F)">${ICONS.freehand}</button>
				<button type="button" data-mode="select" aria-pressed="false" aria-keyshortcuts="S" aria-label="Select, move or change a shape" title="Select, move or change a shape (S)">${ICONS.select}</button>
				<button type="button" data-as="merge" disabled aria-label="Merge the shapes that overlap" title="Merge the shapes that overlap">${ICONS.merge}</button>
				<button type="button" data-as="delete" disabled aria-label="Delete the selected shape" aria-keyshortcuts="Delete" title="Delete the selected shape (Delete)">${ICONS.trash}</button>
				<button type="button" data-as="clear" aria-label="Delete all shapes" title="Delete all shapes">${ICONS.x}</button>
			</div>
			<p class="small muted" data-as="hint" aria-live="polite">Pick a tool, then draw on the map.</p></section>
		<section data-pane="file" hidden><div data-as="drop"></div></section>
		<section data-pane="h3" hidden><select data-as="h3-level" aria-label="The size of a hexagon (H3 level)"></select>
			<div class="bs-h3box"><p data-as="h3-hint" aria-live="polite">Click a hexagon on the map to add it to the area.</p><p data-as="h3-grid-note"></p></div></section>
		<p class="rs-field-msg" role="status" data-as="msg"></p>
		<div class="area-acts">${unselect}</div>`;
	const q = (k) => root.querySelector(`[data-as="${k}"]`);
	const d = (k) => detail.querySelector(`[data-as="${k}"]`);
	const detailHtml = `<input type="text" data-as="name" aria-label="Name of the area" placeholder="Name" autocomplete="off" spellcheck="false" disabled>
		<div data-as="buffer-field"></div>
		<div class="rs-kpis" data-as="kpis" hidden></div>
		<div class="area-acts"><button type="button" data-as="boundary" disabled>${ICONS.download}Boundary</button>${unselect}</div>`;
	if (detail === root) root.insertAdjacentHTML("beforeend", detailHtml); else detail.innerHTML = detailHtml;
	let current = null, h3On = false, td = null, tdLoading = null, merged = [], selected = null;
	const msg = (t, bad = false) => { q("msg").textContent = t; q("msg").classList.toggle("err", bad); };

	// the area on the map: a fill and a line (made when the style has loaded, and again for a new style)
	const layers = () => {
		if (map.getSource("bs-area")) return;
		map.addSource("bs-area", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
		map.addLayer({ id: "bs-area-fill", type: "fill", source: "bs-area", paint: { "fill-color": "#2563eb", "fill-opacity": 0.14 } });
		map.addLayer({ id: "bs-area-line", type: "line", source: "bs-area", paint: { "line-color": "#2563eb", "line-width": 2.5 } });
		// the place under the mouse in the list (as in Studio): purple, on a white casing, with its name along the line; a point is a purple dot
		map.addSource("bs-preview", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
		const notPoint = ["!=", ["geometry-type"], "Point"];
		map.addLayer({ id: "bs-preview-fill", type: "fill", source: "bs-preview", filter: notPoint, paint: { "fill-color": "#7c3aed", "fill-opacity": 0.15 } });
		map.addLayer({ id: "bs-preview-casing", type: "line", source: "bs-preview", filter: notPoint, paint: { "line-color": "#ffffff", "line-width": 6, "line-opacity": 0.9 } });
		map.addLayer({ id: "bs-preview-line", type: "line", source: "bs-preview", filter: notPoint, paint: { "line-color": "#7c3aed", "line-width": 2.5 } });
		map.addLayer({ id: "bs-preview-label", type: "symbol", source: "bs-preview", filter: notPoint, layout: { "symbol-placement": "line", "symbol-spacing": 300, "text-field": ["get", "name"], "text-font": ["Noto Sans Bold"], "text-size": 12, "text-offset": [0, -1] },
			paint: { "text-color": "#6d28d9", "text-halo-color": "#ffffff", "text-halo-width": 2 } });
		map.addLayer({ id: "bs-preview-point", type: "circle", source: "bs-preview", filter: ["==", ["geometry-type"], "Point"], paint: { "circle-radius": 8, "circle-color": "#7c3aed", "circle-stroke-color": "#fff", "circle-stroke-width": 2.5 } });
	};
	let shape = null, previewShape = null;
	const put = () => {
		layers();
		map.getSource("bs-area").setData({ type: "FeatureCollection", features: shape ? [{ type: "Feature", properties: {}, geometry: shape }] : [] });
		map.getSource("bs-preview").setData({ type: "FeatureCollection", features: previewShape ? [{ type: "Feature", properties: { name: previewShape.name }, geometry: previewShape.geometry }] : [] });
	};
	const draw = (g) => { shape = g; if (map.getSource("bs-area") || map.isStyleLoaded()) put(); else map.once("idle", put); };      // "idle": the style is there (a map that loaded before has no more load event)
	map.on("styledata", () => { if (!map.getSource("bs-area") && (shape || cells.size)) try { put(); h3Put(); } catch { /* the style is not ready: the next styledata tries again */ } });
	const zoom = () => { if (current) { const b = current.bbox; map.fitBounds([[b[0], b[1]], [b[2], b[3]]], { padding: padding(), duration: 0, maxZoom: 17 }); } };

	const num = (x, dg = 0) => x.toLocaleString("en", { maximumFractionDigits: dg });
	const kpis = () => {
		const m = bufferM();
		d("kpis").hidden = false;
		d("kpis").innerHTML = rs.kpi("Area", num(current.km2, 2), `km²${m > 0 ? ` · +${num(m)} m` : ""}`) + rs.kpi("Polygons", num(current.polygons.length), "") + rs.kpi("Vertices", num(current.vertices), "");
	};
	const shapeOf = (polys, name, from, base, show) => ({ name: slug(name) || "area", geometry: geometry(polys), polygons: polys, bbox: bbox(polys), km2: areaKm2(polys), from, base, show,
		vertices: polys.reduce((n, p) => n + p.reduce((m, r) => m + r.length, 0), 0) });

	// the buffer (duckOSM's boundary.buffer_m): the area grows by so many metres on all sides, in the UTM zone of its centre. The cut and the build use the grown area; `base` is the shape as chosen.
	const bufferM = () => Math.max(0, Number(d("buffer-field").querySelector("input")?.value) || 0);
	let bufferTimer = 0;
	function rebuffer(delay = 500) {
		clearTimeout(bufferTimer);
		bufferTimer = setTimeout(async () => {
			if (!current) return;
			const m = bufferM(), mine = current;
			let polys = mine.base;
			if (m > 0) {
				try { polys = polygonsOf((await buffer(geometry(mine.base), m)).geometry ?? null); } catch (e) { msg(`The buffer failed: ${e.message}`, true); polys = mine.base; }
				if (current !== mine || !polys.length) return;
			}
			current = { ...shapeOf(polys, mine.name, mine.from, mine.base, mine.show), name: current.name };
			kpis();
			draw(mine.show || m > 0 ? current.geometry : null);                // terra-draw shows the drawn shapes itself, unless the buffer grew them
			onChange();
		}, delay);
	}

	function set(g, name = "", from = "", { zoom: fit = true, show = true } = {}) {
		const polys = polygonsOf(g);
		if (!polys.length) return msg("This has no polygon.", true);
		if (from !== "h3") h3Forget();
		current = shapeOf(polys, name, from, polys, show);
		d("name").value = current.name;
		for (const el of [d("name"), d("boundary"), ...detail.querySelectorAll('[data-as="unselect"]'), ...root.querySelectorAll('[data-as="unselect"]')]) el.disabled = false;
		kpis();
		msg("");
		preview(null);
		draw(show ? current.geometry : null);                  // terra-draw shows the shapes that the person draws
		if (fit) zoom();
		onChange();
		if (bufferM() > 0) rebuffer(0);
	}

	function clear() {
		current = null;
		h3Forget();
		try { td?.clear(); } catch { /* no shape */ }
		d("name").value = ""; d("kpis").hidden = true; d("kpis").innerHTML = "";
		for (const el of [d("name"), d("boundary"), ...detail.querySelectorAll('[data-as="unselect"]'), ...root.querySelectorAll('[data-as="unselect"]')]) el.disabled = true;
		draw(null);
		onChange();
	}

	// the tabs: roadstyle's rs.tabs shows the pane of the chosen tab (the panes beside the tabs); the box stops when another tab opens
	rs.tabs.use(root.querySelector(".rs-tabs"), (id) => {
		if (id !== "search") here.set(false);
		tool(id === "draw" ? (td?.getSnapshot().length ? "select" : "rectangle") : null);
		h3On = id === "h3";
		if (h3On) h3Open().then(h3Grid); else h3Grid();
	});

	// search, as in Studio: while the person types (after 300 ms, from 2 letters) the places of Photon are listed, then each gets its boundary from Nominatim.
	// A place with no boundary is listed with no choice. "What is here?" gives the places around a point of the map.
	let found = [], asking = null, typing = 0;
	const list = (items, none = "No place found.") => {
		found = items;
		q("found").innerHTML = items.map((f, i) => {
			const p = f.properties, ok = p.photon || polygonsOf(f.geometry).length > 0, [title, ...rest] = String(p.display_name ?? "").split(", ");
			return `<label class="rs-opt${ok ? "" : " point"}" data-i="${i}" title="${esc(p.display_name)}"><input type="radio" name="bs-place" value="${i}"${ok ? "" : " disabled"}>` +
				(ok ? `<span><b>${esc(title)}</b><span class="rs-chip">${esc(p.addresstype ?? p.type ?? "")}</span><small>${esc(rest.join(", ") || `${p.osm_type} ${p.osm_id}`)}</small></span></label>`
					: `<span><b>${esc(title)}</b><span class="rs-chip">${esc(p.addresstype ?? p.type ?? "")}, no boundary</span></span></label>`);
		}).join("") || `<p class="small muted">${none}</p>`;
	};
	async function find(text, source) {
		clearTimeout(typing);
		asking?.abort();
		text = (text ?? "").trim();
		if (!source && text.length < 2) { found = []; q("found").innerHTML = ""; return msg(""); }
		if (!source && h3Ids(text)) return h3Search(h3Ids(text));
		const mine = (asking = new AbortController());
		msg("");
		q("found").innerHTML = '<p class="small muted">Searching…</p>';
		try {
			const items = await (source ? source() : geocode(text, mine.signal));
			if (mine.signal.aborted) return;
			list(items);
			if (!source) boundaries(items).then((real) => {                    // the real boundaries come behind the list
				if (found !== items || mine.signal.aborted) return;
				if (q("found").querySelector("input:checked")) real.forEach((f, i) => { items[i] = f; }); else list(real);
			}).catch(() => {});
		} catch (e) {
			if (!mine.signal.aborted) q("found").innerHTML = `<p class="rs-status error"><span>The search failed: ${esc(e.message)}</span></p>`;
		}
	}
	q("found").addEventListener("change", async (e) => {
		const f = found[+e.target.value];
		if (!f) return;
		let g = f.geometry;
		if (f.properties.photon) {                                                // only a box so far: ask for the boundary of this place
			try { g = (await boundaries([f]))[0].geometry; } catch (err) { return msg(`The boundary did not load: ${err.message}`, true); }
		}
		if (!polygonsOf(g).length) return msg("This place has no boundary. Draw a box or use a file.", true);
		set(g, String(f.properties.display_name).split(",")[0], "search");
	});
	// the place under the mouse (or the keyboard) shows its outline on the map, and the map moves to it when it is out of view; after a pick only the picked area shows
	let previewed = null;
	function preview(i) {
		if (i != null && q("found").querySelector("input:checked")) i = null;
		previewed = i;
		const f = i == null ? null : found[i];
		previewShape = f ? { name: String(f.properties.display_name).split(",")[0], geometry: f.geometry } : null;
		draw(shape);
		if (f) {
			const polys = polygonsOf(f.geometry), pts = polys.length ? bbox(polys) : f.geometry?.coordinates ? [...f.geometry.coordinates, ...f.geometry.coordinates] : null;
			if (pts && !(map.getBounds().contains([pts[0], pts[1]]) && map.getBounds().contains([pts[2], pts[3]]))) map.fitBounds([[pts[0], pts[1]], [pts[2], pts[3]]], { padding: padding(), duration: 0, maxZoom: 14 });
		}
	}
	const over = (e) => { const l = e.target.closest("label[data-i]"); if (l) preview(Number(l.dataset.i)); };
	q("found").addEventListener("mouseover", over);
	q("found").addEventListener("focusin", over);
	q("found").addEventListener("mouseleave", () => preview(null));
	// a second click on the picked place takes the area away, as in Studio
	q("found").addEventListener("mousedown", (e) => { const l = e.target.closest("label[data-i]"); if (l) l.dataset.was = l.querySelector("input").checked ? "1" : ""; });
	q("found").addEventListener("click", (e) => { const l = e.target.closest("label[data-i]"); if (l && l.dataset.was === "1" && l.querySelector("input").checked) { l.querySelector("input").checked = false; clear(); } });
	const hereBtn = Object.assign(document.createElement("button"), { type: "button", title: "What is here? Click the map", innerHTML: ICONS.target });
	hereBtn.setAttribute("aria-label", "What is here? Click the map");
	const here = rs.toggle(hereBtn, (on) => { map.getCanvas().style.cursor = on ? "crosshair" : ""; if (on) msg("Click a point on the map."); else if (q("msg").textContent.startsWith("Click a point")) msg(""); });
	rs.search(q("search"), { placeholder: "City, region, country or H3 ID", label: "Name of a place", end: hereBtn, onSubmit: (t) => find(t), onClear: () => find("") });
	q("search").querySelector("input").addEventListener("input", (e) => { clearTimeout(typing); typing = setTimeout(() => find(e.target.value), 300); });
	map.on("click", (e) => {
		if (!here.value()) return;
		here.set(false);
		find("", () => reverse(e.lngLat.lat.toFixed(6), e.lngLat.lng.toFixed(6)));
	});

	// draw: the tools of Studio (terra-draw): rectangle, polygon, circle, freehand, select (move a shape, or change its corners), merge, delete, clear. The shapes that
	// overlap are merged into the area; every shape is a polygon of it.
	const blue = "#2563eb", style = { fillColor: blue, fillOpacity: 0.15, outlineColor: blue, outlineWidth: 2 };
	const hints = { select: "Click a shape to select it.", rectangle: "Drag from corner to corner.", polygon: "Click each corner. Enter closes.", circle: "Drag from the center out.", freehand: "Drag around the area." };
	const buttons = [...root.querySelectorAll("[data-mode]")];
	async function terra() {
		return (tdLoading ??= (async () => {
			const [T, A] = await Promise.all([import(TERRA), import(TERRA_MAPLIBRE)]);
			const validation = (f) => T.ValidateNotSelfIntersecting(f);
			const edit = (coordinates) => ({ feature: { draggable: true, coordinates: { draggable: true, ...coordinates } } });
			const d = new T.TerraDraw({ adapter: new A.TerraDrawMapLibreGLAdapter({ map }), modes: [
				new T.TerraDrawRectangleMode({ drawInteraction: "click-move-or-drag", styles: style }),
				new T.TerraDrawPolygonMode({ validation, styles: { ...style, closingPointColor: blue, closingPointOutlineColor: "#ffffff" } }),
				new T.TerraDrawCircleMode({ drawInteraction: "click-move-or-drag", segments: 64, styles: style }),
				new T.TerraDrawFreehandMode({ drawInteraction: "click-move-or-drag", validation, styles: style }),
				new T.TerraDrawSelectMode({ flags: { rectangle: edit({ resizable: "opposite" }), circle: edit({ resizable: "center-fixed" }), polygon: edit({ midpoints: true, deletable: true }), freehand: edit({ midpoints: true, deletable: true }) },
					styles: { selectedPolygonColor: blue, selectedPolygonFillOpacity: 0.25, selectedPolygonOutlineColor: blue, selectedPolygonOutlineWidth: 3, selectionPointColor: "#ffffff",
						selectionPointOutlineColor: blue, selectionPointWidth: 5, selectionPointOutlineWidth: 2, midPointColor: blue, midPointOutlineColor: "#ffffff", midPointWidth: 3 } })] });
			td = d;
			await new Promise((ok) => { const t = () => (map.isStyleLoaded() ? ok() : setTimeout(t, 100)); t(); });      // terra-draw starts when the style is there
			d.start();
			const shapes = () => d.getSnapshot().filter((f) => f.geometry.type === "Polygon" && f.properties.mode !== "select");
			// the drawn shapes, merged where they overlap, are the area
			const drawn = async () => {
				const list = shapes().map((f) => f.geometry.coordinates);
				q("merge").disabled = true;
				if (!list.length) return current?.from === "drawn" ? clear() : undefined;
				const { union } = await import(POLYCLIP);
				const polys = list.length > 1 ? union(...list) : list;
				merged = polys;
				q("merge").disabled = polys.length >= list.length;                 // no shapes overlap
				set(geometry(polys), current?.from === "drawn" ? d_("name").value : "drawn area", "drawn", { zoom: false, show: false });
			};
			d.on("finish", (id, ctx) => { drawn(); if (ctx.action === "draw") setTimeout(() => { tool("select"); d.selectFeature(id); }); });      // a new shape is selected at once
			d.on("change", (ids, type) => { if (type !== "delete") return; if (ids.includes(selected)) { selected = null; q("delete").disabled = true; } drawn(); });
			d.on("select", (id) => { selected = id; q("delete").disabled = false; });
			d.on("deselect", () => { selected = null; q("delete").disabled = true; });
			q("delete").addEventListener("click", () => { if (selected != null) { d.removeFeatures([selected]); selected = null; q("delete").disabled = true; drawn(); } });
			q("merge").addEventListener("click", () => {                            // the shapes become the polygons of their merge (terra-draw takes at most 9 decimals)
				const old = shapes().map((f) => f.id), round = (c) => c.map((r) => r.map(([x, y]) => [+x.toFixed(9), +y.toFixed(9)]));
				const got = d.addFeatures(merged.map((c) => ({ type: "Feature", geometry: { type: "Polygon", coordinates: round(c) }, properties: { mode: "polygon" } })));
				const bad = got.find((r) => !r.valid);
				if (bad) { d.removeFeatures(got.filter((r) => r.valid).map((r) => r.id)); return msg(`The shapes were not merged: ${bad.reason}`, true); }
				d.removeFeatures(old);
				tool("select");
				d.selectFeature(got[0].id);
			});
			q("clear").addEventListener("click", () => { d.clear(); drawn(); tool("rectangle"); });
			return d;
		})());
	}
	const d_ = d;
	function tool(mode) {
		for (const b of buttons) { b.setAttribute("aria-pressed", b.dataset.mode === mode); b.classList.toggle("on", b.dataset.mode === mode); }
		if (mode) q("hint").textContent = hints[mode];
		if (mode || td) terra().then((t) => t.setMode(mode ?? "static"));
	}
	for (const b of buttons) b.addEventListener("click", () => tool(b.dataset.mode));
	addEventListener("keydown", (e) => {
		if (root.querySelector('[data-pane="draw"]').hidden || e.target.closest("input") || e.ctrlKey || e.metaKey || e.altKey) return;
		const mode = { s: "select", r: "rectangle", p: "polygon", c: "circle", f: "freehand" }[e.key.toLowerCase()];
		if (mode) tool(mode);
	});

	// H3: the area is the union of the hexagons that the person clicks on the map. h3-js loads at the first use of the tab. While the tab shows, a thin gray grid draws every hexagon
	// in view at the chosen level, the hexagon under the pointer is lit, and the picked cells are filled. Too many hexagons for the view: no grid, and the note says what to do.
	const cells = new Set();
	let h3Level = 8, h3Mod = null, hover = null, gridOn = null, estimate = 0, token = 0, autoName = "";
	const lib = () => (h3Mod ??= import(H3).catch((e) => { h3Mod = null; throw e; }));
	const nothing = { type: "FeatureCollection", features: [] };
	const metres = (km) => (km < 1 ? `${Math.round(km * 1000)} m` : `${num(km, 1)} km`);
	const h3 = (k) => root.querySelector(`[data-as="${k}"]`);
	const visible = () => root.offsetParent !== null && h3On;                      // the tab shows, and the card is open
	function h3Layers() {
		if (map.getSource("bs-h3grid")) return;
		for (const id of ["grid", "hover", "cells"]) map.addSource(`bs-h3${id}`, { type: "geojson", data: nothing });
		map.addLayer({ id: "bs-h3-grid", type: "line", source: "bs-h3grid", paint: { "line-color": "#475569", "line-width": 1, "line-opacity": 0.5 } });
		map.addLayer({ id: "bs-h3-fill", type: "fill", source: "bs-h3cells", paint: { "fill-color": "#2563eb", "fill-opacity": 0.22 } });
		map.addLayer({ id: "bs-h3-hover", type: "fill", source: "bs-h3hover", paint: { "fill-color": "#2563eb", "fill-opacity": 0.12 } });
		map.addLayer({ id: "bs-h3-line", type: "line", source: "bs-h3cells", paint: { "line-color": "#2563eb", "line-width": 1.5, "line-opacity": 0.7 } });
	}
	async function h3Put() {
		h3Layers();
		const lb = await lib(), ring = (id) => ({ type: "Feature", properties: {}, geometry: { type: "Polygon", coordinates: [lb.cellToBoundary(id, true)] } });
		map.getSource("bs-h3cells").setData({ type: "FeatureCollection", features: [...cells].map(ring) });
	}
	async function h3Open() {
		let lb;
		try { lb = await lib(); } catch (e) { return msg(`H3 did not load: ${e.message}`, true); }
		h3Layers();
		const sel = h3("h3-level");
		if (sel.options.length) return;
		sel.innerHTML = [5, 6, 7, 8, 9, 10].map((l) => { const km2 = lb.getHexagonAreaAvg(l, "km2"); return `<option value="${l}">Level ${l}: edge ${metres(lb.getHexagonEdgeLengthAvg(l, "km"))}, ${num(km2, km2 < 1 ? 3 : 1)} km²</option>`; }).join("");
		sel.value = h3Level;
	}
	function h3Text(km2, note) {                                                     // what to do, the size of the picks, or why the last click did nothing
		const n = cells.size;
		h3("h3-hint").textContent = note ?? (n ? `${num(n)} cell${n > 1 ? "s" : ""}, ${num(km2, 2)} km². Click a hexagon to add or remove it.` : "Click a hexagon on the map to add it to the area.");
	}
	async function h3Update(lb, note) {
		const ids = [...cells];
		await h3Put();
		if (!ids.length) { if (current?.from === "h3") clear(); return h3Text(0, note); }
		const polys = lb.cellsToMultiPolygon(ids, true);                             // touching cells merge; a ring of cells keeps its hole
		// the name is the H3 ID (the first cell, and "-plus-N" for N more); a name that the person typed stays
		const auto = ids.length === 1 ? ids[0] : `${ids[0]}-plus-${ids.length - 1}`, typed = current?.from === "h3" && d("name").value && d("name").value !== slug(autoName);
		autoName = auto;
		set(geometry(polys), typed ? d("name").value : auto, "h3", { zoom: false, show: false });
		h3Text(areaKm2(polys), note);
	}
	function h3Forget() {                                                          // another way of making the area replaced the cells
		if (!cells.size) return;
		cells.clear();
		map.getSource("bs-h3cells")?.setData(nothing);
		h3Text(0);
	}
	async function h3Grid() {
		const my = ++token, src = map.getSource("bs-h3grid");
		if (!src) return;
		if (!visible()) { src.setData(nothing); map.getSource("bs-h3hover")?.setData(nothing); hover = null; gridOn = null; h3("h3-grid-note").textContent = ""; return; }
		let lb;
		try { lb = await lib(); } catch { return; }
		if (my !== token) return;
		const b = map.getBounds(), pad = lb.getHexagonEdgeLengthAvg(h3Level, "km") / 111 * 1.5;      // a cell shows when its center is out of view but its corner is in
		const w = b.getWest() - pad, e = b.getEast() + pad, south = Math.max(-85, b.getSouth() - pad), north = Math.min(85, b.getNorth() + pad);
		estimate = ((e - w) * 111 * Math.cos(((south + north) / 2) * Math.PI / 180)) * ((north - south) * 111) / lb.getHexagonAreaAvg(h3Level, "km2");
		if (estimate > H3_MAX_GRID) {
			src.setData(nothing);
			gridOn = false;
			h3("h3-grid-note").innerHTML = 'The view has too many hexagons. Zoom in, or choose a bigger hexagon, to pick hexagons. <a href="#" role="button" data-as="h3-zoom">Zoom in</a>';
			return;
		}
		gridOn = true;
		const ids = lb.polygonToCells([[[w, south], [e, south], [e, north], [w, north], [w, south]]], h3Level, true);
		src.setData({ type: "FeatureCollection", features: ids.map((id) => ({ type: "Feature", properties: {}, geometry: { type: "Polygon", coordinates: [lb.cellToBoundary(id, true)] } })) });
		h3("h3-grid-note").textContent = `${num(ids.length)} hexagon${ids.length === 1 ? "" : "s"} in view.`;
	}
	map.on("moveend", h3Grid);
	map.on("mousemove", async (ev) => {                                            // the hexagon under the pointer is lit
		if (!visible()) return;
		let lb;
		try { lb = await lib(); } catch { return; }
		const { lat, lng } = ev.lngLat.wrap(), id = lb.latLngToCell(lat, lng, h3Level);
		if (id === hover) return;
		hover = id;
		map.getSource("bs-h3hover")?.setData({ type: "Feature", properties: {}, geometry: { type: "Polygon", coordinates: [lb.cellToBoundary(id, true)] } });
	});
	map.on("click", async (ev) => {
		if (!visible()) return;
		let lb;
		try { lb = await lib(); } catch (e) { return msg(`H3 did not load: ${e.message}`, true); }
		if (gridOn === false) return h3Text(0, "The view has too many hexagons to pick one. Zoom in, or choose a bigger hexagon.");      // a cell that cannot be seen is not picked
		const { lat, lng } = ev.lngLat.wrap(), id = lb.latLngToCell(lat, lng, h3Level);
		let note;
		if (!cells.delete(id)) {
			if (cells.size >= H3_MAX_CELLS) note = `The area has the most cells: ${num(H3_MAX_CELLS)}. Remove a cell or clear them.`;
			else if ((cells.size + 1) * lb.getHexagonAreaAvg(h3Level, "km2") > H3_MAX_KM2) note = `The area has the most km²: ${num(H3_MAX_KM2)}. Remove a cell, clear them or use a smaller level.`;
			else cells.add(id);
		}
		h3Update(lb, note);
	});
	h3("h3-level").addEventListener("change", async () => {
		const lb = await lib(), had = cells.size;
		h3Level = Number(h3("h3-level").value);
		cells.clear();
		h3Update(lb, had ? "You changed the level. The cells are removed." : undefined);
		hover = null; h3Grid();
	});
	h3("h3-grid-note").addEventListener("click", (e) => {                           // zoom in until about 150 hexagons are in view (a zoom step is 4 times the area)
		if (!e.target.closest('[data-as="h3-zoom"]')) return;
		e.preventDefault();
		map.easeTo({ zoom: Math.min(map.getZoom() + Math.max(1, Math.log(estimate / 150) / Math.log(4)), 18), duration: 600 });
	});
	// H3 cell IDs in the search box (15 hex digits each, separated by spaces or commas) make the area with no network: the cells of one level (5 to 10)
	const h3Ids = (t) => { const x = t.split(/[\s,;]+/).filter(Boolean); return x.length && x.every((i) => /^[0-9a-f]{15}$/i.test(i)) ? x.map((i) => i.toLowerCase()) : null; };
	async function h3Search(ids) {
		const say = (t, bad = true) => { q("found").innerHTML = ""; msg(t, bad); };
		let lb;
		try { lb = await lib(); } catch (e) { return say(`H3 did not load: ${e.message}`); }
		ids = [...new Set(ids)];
		if (!ids.every((i) => lb.isValidCell(i))) return say("This is not an H3 cell ID.");
		const level = lb.getResolution(ids[0]);
		if (ids.some((i) => lb.getResolution(i) !== level)) return say("The cells must have one level.");
		if (level < 5 || level > 10) return say(`The H3 tab works with the levels 5 to 10. This cell has level ${level}.`);
		if (ids.length > H3_MAX_CELLS || ids.length * lb.getHexagonAreaAvg(level, "km2") > H3_MAX_KM2) return say(`The area has the most ${num(H3_MAX_CELLS)} cells and ${num(H3_MAX_KM2)} km².`);
		root.querySelector('[data-tab="h3"]').click();
		await h3Open();
		h3Level = level; h3("h3-level").value = level;
		cells.clear(); for (const i of ids) cells.add(i);
		await h3Update(lb);
		zoom();
		say(`${ids.length === 1 ? "H3 cell" : `${ids.length} H3 cells`}, level ${level}.`, false);
	}

	// the file
	rs.drop(q("drop"), {
		accept: ".geojson,.json", text: "Drop a GeoJSON file here, or <u>choose a file</u>", hint: "A polygon or a multipolygon",
		onFiles: async (files) => {
			try {
				const json = JSON.parse(await files[0].text());
				set(json, json.features?.[0]?.properties?.name ?? files[0].name.replace(/\.[^.]*$/, ""), "file");
			} catch (e) {
				msg(`${files[0].name}: ${e.message}`, true);
			}
		},
		onClear: clear,
	});

	if (buffer && rs.number) {                                                          // the field of roadstyle, as in Studio
		rs.number(d("buffer-field"), { label: "Buffer around the area", unit: "m", min: 0, max: 50000, step: 50, snap: false, value: 0, onChange: () => rebuffer() });
		d("buffer-field").querySelector("input").addEventListener("input", () => rebuffer());      // typing; the - and + buttons call onChange
	}
	d("name").addEventListener("input", () => { if (current) { current.name = slug(d("name").value); onChange(); } });
	for (const el of [...root.querySelectorAll('[data-as="unselect"]'), ...detail.querySelectorAll('[data-as="unselect"]')]) el.addEventListener("click", clear);
	d("boundary").addEventListener("click", () => {                                    // the boundary as a GeoJSON file
		if (!current) return;
		const url = URL.createObjectURL(new Blob([JSON.stringify({ type: "FeatureCollection", features: [{ type: "Feature", properties: { name: current.name }, geometry: current.geometry }] })], { type: "application/geo+json" }));
		Object.assign(document.createElement("a"), { href: url, download: `${current.name}.geojson` }).click();
		setTimeout(() => URL.revokeObjectURL(url), 1000);
	});
	return { area: () => current, rebuffer, set, clear, zoom, h3Cells: () => [...cells], drawMode: async () => (await terra()).getMode() };      // drawMode: the tool of terra-draw (for the tests)
}
