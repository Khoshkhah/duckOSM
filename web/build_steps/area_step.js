// The Area steps, as duckOSM Studio has them: card "Choose an area" (a place name, a box that the person draws on the map, a boundary file, or a click on the
// map: "What is here?") and card "The area" (the name, the numbers, the boundary file, Unselect). The steps draw the area on the map and give its
// polygons, its box and its name to the other steps. They need a MapLibre map and roadstyle's parts (`rs`: search, drop, tabs, toggle, kpi).
import { areaKm2, bbox, box, esc, geometry, polygonsOf, slug } from "./core.js";

const NOMINATIM = "https://nominatim.openstreetmap.org";
// the drawing tools of Studio (terra-draw), and the union of the shapes (polyclip-ts): the same versions, loaded when the tab Draw opens the first time
const TERRA = "https://cdn.jsdelivr.net/npm/terra-draw@1.38.0/+esm", TERRA_MAPLIBRE = "https://cdn.jsdelivr.net/npm/terra-draw-maplibre-gl-adapter@1.5.0/+esm", POLYCLIP = "https://cdn.jsdelivr.net/npm/polyclip-ts@0.16.8/+esm";
const icon = (d) => `<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;
const ICONS = {
	search: icon('<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>'), draw: icon('<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/>'),
	file: icon('<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9Z"/><path d="M14 3v6h6"/>'), target: icon('<circle cx="12" cy="12" r="8"/><path d="M12 2v4M12 18v4M2 12h4M18 12h4"/>'),
	download: icon('<path d="M12 4v12M7 11l5 5 5-5M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3"/>'),
	rectangle: icon('<rect x="3.5" y="5.5" width="17" height="13" rx="1"/>'), polygon: icon('<path d="m12 3 8.5 6.2-3.2 10.3H6.7L3.5 9.2Z"/>'), circle: icon('<circle cx="12" cy="12" r="8.5"/>'),
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
 *   reverse,   // optional: (lat, lon) gives Nominatim-shaped features (default: `nominatimAt`)
 *   padding,   // optional: () gives the padding of the map for fitBounds (the panel covers a part of the map)
 *   onChange,  // optional: the area or its name is new
 * })
 * It returns { area(), set(geometry, name, from, { zoom, show }), clear(), zoom(), drawMode() }. area() is null or { name, geometry, polygons, bbox, km2, from }.
 */
export function initAreaStep({ root, detail = root, map, rs, geocode = photonSearch, boundaries = boundariesOf, reverse = nominatimAt, padding = () => 40, onChange = () => {} }) {
	root.classList.add("bs", "bs-area");
	detail.classList.add("bs", "bs-area");
	const unselect = '<button type="button" data-as="unselect" disabled>Unselect</button>';
	root.innerHTML = `<div class="rs-tabs" role="tablist" aria-label="The ways to make an area">
			<button type="button" class="rs-tab on" data-tab="search">${ICONS.search}Search</button>
			<button type="button" class="rs-tab" data-tab="draw">${ICONS.draw}Draw</button>
			<button type="button" class="rs-tab" data-tab="file">${ICONS.file}File</button></div>
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
		<p class="rs-field-msg" role="status" data-as="msg"></p>
		<div class="area-acts">${unselect}</div>`;
	const q = (k) => root.querySelector(`[data-as="${k}"]`);
	const d = (k) => detail.querySelector(`[data-as="${k}"]`);
	const detailHtml = `<input type="text" data-as="name" aria-label="Name of the area" placeholder="Name" autocomplete="off" spellcheck="false" disabled>
		<div class="rs-kpis" data-as="kpis" hidden></div>
		<div class="area-acts"><button type="button" data-as="boundary" disabled>${ICONS.download}Boundary</button>${unselect}</div>`;
	if (detail === root) root.insertAdjacentHTML("beforeend", detailHtml); else detail.innerHTML = detailHtml;
	let current = null, td = null, tdLoading = null, merged = [], selected = null;
	const msg = (t, bad = false) => { q("msg").textContent = t; q("msg").classList.toggle("err", bad); };

	// the area on the map: a fill and a line (made when the style has loaded, and again for a new style)
	const layers = () => {
		if (map.getSource("bs-area")) return;
		map.addSource("bs-area", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
		map.addLayer({ id: "bs-area-fill", type: "fill", source: "bs-area", paint: { "fill-color": "#2563eb", "fill-opacity": 0.14 } });
		map.addLayer({ id: "bs-area-line", type: "line", source: "bs-area", paint: { "line-color": "#2563eb", "line-width": 2.5 } });
	};
	let shape = null;
	const put = () => { layers(); map.getSource("bs-area").setData({ type: "FeatureCollection", features: shape ? [{ type: "Feature", properties: {}, geometry: shape }] : [] }); };
	const draw = (g) => { shape = g; map.loaded() || map.isStyleLoaded() ? put() : map.once("load", put); };
	map.on("styledata", () => { if (shape && !map.getSource("bs-area")) try { put(); } catch { /* the style is not ready: the next styledata tries again */ } });
	const zoom = () => { if (current) { const b = current.bbox; map.fitBounds([[b[0], b[1]], [b[2], b[3]]], { padding: padding(), duration: 0, maxZoom: 17 }); } };

	function set(g, name = "", from = "", { zoom: fit = true, show = true } = {}) {
		const polys = polygonsOf(g);
		if (!polys.length) return msg("This has no polygon.", true);
		const b = bbox(polys), km2 = areaKm2(polys), vertices = polys.reduce((n, p) => n + p.reduce((m, r) => m + r.length, 0), 0);
		current = { name: slug(name) || "area", geometry: geometry(polys), polygons: polys, bbox: b, km2, vertices, from };
		d("name").value = current.name;
		for (const el of [d("name"), d("boundary"), ...detail.querySelectorAll('[data-as="unselect"]'), ...root.querySelectorAll('[data-as="unselect"]')]) el.disabled = false;
		const num = (x, dg = 0) => x.toLocaleString("en", { maximumFractionDigits: dg });
		d("kpis").hidden = false;
		d("kpis").innerHTML = rs.kpi("Area", num(km2, 2), "km²") + rs.kpi("Polygons", num(polys.length), "") + rs.kpi("Vertices", num(vertices), "");
		msg("");
		draw(show ? current.geometry : null);                  // terra-draw shows the shapes that the person draws
		if (fit) zoom();
		onChange();
	}

	function clear() {
		current = null;
		try { td?.clear(); } catch { /* no shape */ }
		d("name").value = ""; d("kpis").hidden = true; d("kpis").innerHTML = "";
		for (const el of [d("name"), d("boundary"), ...detail.querySelectorAll('[data-as="unselect"]'), ...root.querySelectorAll('[data-as="unselect"]')]) el.disabled = true;
		draw(null);
		onChange();
	}

	// the tabs: roadstyle's rs.tabs shows the pane of the chosen tab (the panes beside the tabs); the box stops when another tab opens
	rs.tabs.use(root.querySelector(".rs-tabs"), (id) => { if (id !== "search") here.set(false); tool(id === "draw" ? (td?.getSnapshot().length ? "select" : "rectangle") : null); });

	// search, as in Studio: while the person types (after 300 ms, from 2 letters) the places of Photon are listed, then each gets its boundary from Nominatim.
	// A place with no boundary is listed with no choice. "What is here?" gives the places around a point of the map.
	let found = [], asking = null, typing = 0;
	const list = (items, none = "No place found.") => {
		found = items;
		q("found").innerHTML = items.map((f, i) => {
			const p = f.properties, ok = p.photon || polygonsOf(f.geometry).length > 0, [title, ...rest] = String(p.display_name ?? "").split(", ");
			return `<label class="rs-opt${ok ? "" : " point"}" title="${esc(p.display_name)}"><input type="radio" name="bs-place" value="${i}"${ok ? "" : " disabled"}>` +
				(ok ? `<span><b>${esc(title)}</b><span class="rs-chip">${esc(p.addresstype ?? p.type ?? "")}</span><small>${esc(rest.join(", ") || `${p.osm_type} ${p.osm_id}`)}</small></span></label>`
					: `<span><b>${esc(title)}</b><span class="rs-chip">${esc(p.addresstype ?? p.type ?? "")}, no boundary</span></span></label>`);
		}).join("") || `<p class="small muted">${none}</p>`;
	};
	async function find(text, source) {
		clearTimeout(typing);
		asking?.abort();
		text = (text ?? "").trim();
		if (!source && text.length < 2) { found = []; q("found").innerHTML = ""; return msg(""); }
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
	const hereBtn = Object.assign(document.createElement("button"), { type: "button", title: "What is here? Click the map", innerHTML: ICONS.target });
	hereBtn.setAttribute("aria-label", "What is here? Click the map");
	const here = rs.toggle(hereBtn, (on) => { map.getCanvas().style.cursor = on ? "crosshair" : ""; if (on) msg("Click a point on the map."); else if (q("msg").textContent.startsWith("Click a point")) msg(""); });
	rs.search(q("search"), { placeholder: "City, region or country", label: "Name of a place", end: hereBtn, onSubmit: (t) => find(t), onClear: () => find("") });
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

	d("name").addEventListener("input", () => { if (current) { current.name = slug(d("name").value); onChange(); } });
	for (const el of [...root.querySelectorAll('[data-as="unselect"]'), ...detail.querySelectorAll('[data-as="unselect"]')]) el.addEventListener("click", clear);
	d("boundary").addEventListener("click", () => {                                    // the boundary as a GeoJSON file
		if (!current) return;
		const url = URL.createObjectURL(new Blob([JSON.stringify({ type: "FeatureCollection", features: [{ type: "Feature", properties: { name: current.name }, geometry: current.geometry }] })], { type: "application/geo+json" }));
		Object.assign(document.createElement("a"), { href: url, download: `${current.name}.geojson` }).click();
		setTimeout(() => URL.revokeObjectURL(url), 1000);
	});
	return { area: () => current, set, clear, zoom, drawMode: async () => (await terra()).getMode() };      // drawMode: the tool of terra-draw (for the tests)
}
