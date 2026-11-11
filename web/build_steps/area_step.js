// The Area steps, as duckOSM Studio has them: card "Choose an area" (a place name, a box that the person draws on the map, a boundary file, or a click on the
// map: "What is here?") and card "The area" (the name, the numbers, the boundary file, Unselect). The steps draw the area on the map and give its
// polygons, its box and its name to the other steps. They need a MapLibre map and roadstyle's parts (`rs`: search, drop, tabs, toggle, kpi).
import { areaKm2, bbox, box, esc, geometry, polygonsOf, slug } from "./core.js";

const NOMINATIM = "https://nominatim.openstreetmap.org";
const icon = (d) => `<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;
const ICONS = {
	search: icon('<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>'), draw: icon('<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/>'),
	file: icon('<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9Z"/><path d="M14 3v6h6"/>'), target: icon('<circle cx="12" cy="12" r="8"/><path d="M12 2v4M12 18v4M2 12h4M18 12h4"/>'),
	download: icon('<path d="M12 4v12M7 11l5 5 5-5M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3"/>'),
};

/** The default search: Nominatim (a place with its outline). Gives [{name, geometry}] of the places that have a polygon. */
export async function nominatim(text) {
	const r = await fetch(`${NOMINATIM}/search?format=geojson&polygon_geojson=1&limit=8&q=${encodeURIComponent(text)}`);
	if (!r.ok) throw new Error(`The search gave HTTP ${r.status}.`);
	return places((await r.json()).features);
}

/** The default "What is here?": the places around a point, from the largest outline that Nominatim has for each zoom. */
export async function nominatimAt(lat, lon) {
	const found = [];
	for (const zoom of [14, 10, 8, 5, 3]) {
		const r = await fetch(`${NOMINATIM}/reverse?format=geojson&polygon_geojson=1&zoom=${zoom}&lat=${lat}&lon=${lon}`);
		if (!r.ok) throw new Error(`The search gave HTTP ${r.status}.`);
		for (const p of places((await r.json()).features)) if (!found.some((f) => f.name === p.name)) found.push(p);
	}
	return found;
}

const places = (features) => (features ?? []).filter((f) => polygonsOf(f.geometry).length).map((f) => ({ name: f.properties?.display_name ?? "", geometry: f.geometry }));

/**
 * initAreaStep({
 *   root,      // the element of the card "Choose an area"
 *   detail,    // the element of the card "The area" (optional: without it, its parts go under `root`)
 *   map,       // the MapLibre map: the step draws the area on it, and the box is drawn on it
 *   rs,        // roadstyle's parts: { search, drop, tabs, toggle, kpi }
 *   geocode,   // optional: (text) gives [{name, geometry}] (default: Nominatim)
 *   reverse,   // optional: (lat, lon) gives [{name, geometry}] (default: Nominatim)
 *   padding,   // optional: () gives the padding of the map for fitBounds (the panel covers a part of the map)
 *   onChange,  // optional: the area or its name is new
 * })
 * It returns { area(), set(geometry, name, from), clear(), zoom() }. area() is null or { name, geometry, polygons, bbox, km2, from }.
 */
export function initAreaStep({ root, detail = root, map, rs, geocode = nominatim, reverse = nominatimAt, padding = () => 40, onChange = () => {} }) {
	root.classList.add("bs", "bs-area");
	detail.classList.add("bs", "bs-area");
	const unselect = '<button type="button" data-as="unselect" disabled>Unselect</button>';
	root.innerHTML = `<div class="rs-tabs" role="tablist" aria-label="The ways to make an area">
			<button type="button" class="rs-tab on" data-tab="search">${ICONS.search}Search</button>
			<button type="button" class="rs-tab" data-tab="draw">${ICONS.draw}Draw</button>
			<button type="button" class="rs-tab" data-tab="file">${ICONS.file}File</button></div>
		<section data-pane="search"><div data-as="search"></div><div class="rs-opts" role="radiogroup" aria-label="The places found" data-as="found"></div></section>
		<section data-pane="draw" hidden><button type="button" class="pick" data-as="draw" aria-pressed="false">${ICONS.draw}<span>Draw a box on the map</span></button>
			<p class="small muted" data-as="hint">Press the button, then drag on the map from one corner to the other.</p></section>
		<section data-pane="file" hidden><div data-as="drop"></div></section>
		<p class="rs-field-msg" role="status" data-as="msg"></p>
		<div class="area-acts">${unselect}</div>`;
	const q = (k) => root.querySelector(`[data-as="${k}"]`);
	const d = (k) => detail.querySelector(`[data-as="${k}"]`);
	const detailHtml = `<input type="text" data-as="name" aria-label="Name of the area" placeholder="Name" autocomplete="off" spellcheck="false" disabled>
		<div class="rs-kpis" data-as="kpis" hidden></div>
		<div class="area-acts"><button type="button" data-as="boundary" disabled>${ICONS.download}Boundary</button>${unselect}</div>`;
	if (detail === root) root.insertAdjacentHTML("beforeend", detailHtml); else detail.innerHTML = detailHtml;
	let current = null, drawing = false;
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

	function set(g, name = "", from = "") {
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
		draw(current.geometry);
		zoom();
		onChange();
	}

	function clear() {
		current = null;
		d("name").value = ""; d("kpis").hidden = true; d("kpis").innerHTML = "";
		for (const el of [d("name"), d("boundary"), ...detail.querySelectorAll('[data-as="unselect"]'), ...root.querySelectorAll('[data-as="unselect"]')]) el.disabled = true;
		draw(null);
		onChange();
	}

	// the tabs: roadstyle's rs.tabs shows the pane of the chosen tab (the panes beside the tabs); the box stops when another tab opens
	rs.tabs.use(root.querySelector(".rs-tabs"), (id) => { if (id !== "draw") stopDrawing(); if (id !== "search") here.set(false); });

	// search: the places that have a polygon; a pick makes the area. "What is here?" gives the places around a point of the map.
	let found = [];
	const list = (items) => {
		found = items;
		q("found").innerHTML = items.map((p, i) => `<label class="rs-opt"><input type="radio" name="bs-place" value="${i}"><span><b>${esc(p.name.split(",")[0])}</b><small>${esc(p.name.split(",").slice(1, 3).join(","))}</small></span></label>`).join("");
	};
	const find = async (text, source) => {
		text = text.trim();
		list([]);
		if (!text && !source) return msg("");
		msg("Searching…");
		try {
			const items = await (source ?? geocode)(text);
			list(items);
			msg(items.length ? "" : "No place with an outline was found. Draw a box or use a file.");
		} catch (e) {
			msg(`The search failed: ${e.message}`, true);
		}
	};
	q("found").addEventListener("change", (e) => { const p = found[+e.target.value]; if (p) set(p.geometry, p.name.split(",")[0], "search"); });
	const hereBtn = Object.assign(document.createElement("button"), { type: "button", title: "What is here? Click the map", innerHTML: ICONS.target });
	hereBtn.setAttribute("aria-label", "What is here? Click the map");
	const here = rs.toggle(hereBtn, (on) => { map.getCanvas().style.cursor = on ? "crosshair" : ""; if (on) msg("Click a point on the map."); else if (q("msg").textContent.startsWith("Click a point")) msg(""); });
	rs.search(q("search"), { placeholder: "City, region or country", label: "Name of a place", end: hereBtn, onSubmit: (t) => find(t), onClear: () => list([]) });
	map.on("click", (e) => {
		if (!here.value()) return;
		here.set(false);
		find("", () => reverse(e.lngLat.lat.toFixed(6), e.lngLat.lng.toFixed(6)));
	});

	// the box: the button turns the drawing on; a drag on the map gives the corners (mouse and touch)
	const canvas = map.getCanvas(), button = q("draw");
	let a = null;
	const at = (e) => { const r = canvas.getBoundingClientRect(); return map.unproject([e.clientX - r.left, e.clientY - r.top]); };
	const rect = (p, o) => box([Math.min(p.lng, o.lng), Math.min(p.lat, o.lat), Math.max(p.lng, o.lng), Math.max(p.lat, o.lat)]);
	function stopDrawing() {
		drawing = false; a = null;
		button.setAttribute("aria-pressed", "false"); button.querySelector("span").textContent = "Draw a box on the map";
		canvas.style.cursor = ""; canvas.style.touchAction = ""; map.dragPan.enable(); map.touchZoomRotate?.enable();
	}
	button.addEventListener("click", () => {
		if (drawing) return stopDrawing();
		drawing = true;
		button.setAttribute("aria-pressed", "true"); button.querySelector("span").textContent = "Drag on the map…";
		canvas.style.cursor = "crosshair"; canvas.style.touchAction = "none"; map.dragPan.disable(); map.touchZoomRotate?.disable();
	});
	canvas.addEventListener("pointerdown", (e) => { if (drawing) { a = at(e); canvas.setPointerCapture(e.pointerId); } });
	canvas.addEventListener("pointermove", (e) => { if (drawing && a) draw(rect(a, at(e))); });
	canvas.addEventListener("pointerup", (e) => {
		if (!drawing || !a) return;
		const [w, s, ea, n] = bbox([rect(a, at(e)).coordinates]);
		stopDrawing();
		if (Math.abs(ea - w) < 1e-5 || Math.abs(n - s) < 1e-5) { draw(current?.geometry ?? null); return msg("The box is too small. Drag on the map from one corner to the other.", true); }
		set(box([w, s, ea, n]), "box", "box");
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
	return { area: () => current, set, clear, zoom };
}
