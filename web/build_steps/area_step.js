// The Area step: the area of the build, from a place name, a box that the person draws on the map, or a boundary file (GeoJSON). It draws the area on the map
// and gives its polygons, its box and its name to the other steps. Needs a MapLibre map and roadstyle's parts rs.search and rs.drop.
import { areaKm2, bbox, box, esc, geometry, polygonsOf, slug } from "./core.js";

/** The default search: Nominatim (a place with its outline). Gives [{name, geometry}] of the places that have a polygon. */
export async function nominatim(text) {
	const r = await fetch(`https://nominatim.openstreetmap.org/search?format=geojson&polygon_geojson=1&limit=8&q=${encodeURIComponent(text)}`);
	if (!r.ok) throw new Error(`The search gave HTTP ${r.status}.`);
	return ((await r.json()).features ?? []).filter((f) => polygonsOf(f.geometry).length).map((f) => ({ name: f.properties?.display_name ?? text, geometry: f.geometry }));
}

/**
 * initAreaStep({
 *   root,      // the element that gets the step
 *   map,       // the MapLibre map: the step draws the area on it, and the box is drawn on it
 *   rs,        // roadstyle's parts: { search, drop }
 *   geocode,   // optional: (text) gives [{name, geometry}] (default: Nominatim)
 *   padding,   // optional: () gives the padding of the map for fitBounds (the panel covers a part of the map)
 *   onChange,  // optional: the area or its name is new
 * })
 * It returns { area(), set(geometry, name, from), clear() }. area() is null or { name, geometry, polygons, bbox, km2, from }.
 */
export function initAreaStep({ root, map, rs, geocode = nominatim, padding = () => 40, onChange = () => {} }) {
	root.classList.add("bs", "bs-area");
	root.innerHTML = `<div class="rs-tabs" role="tablist" aria-label="The ways to make an area">
			<button type="button" class="rs-tab on" role="tab" aria-selected="true" data-tab="search">Search</button>
			<button type="button" class="rs-tab" role="tab" aria-selected="false" data-tab="draw">Draw</button>
			<button type="button" class="rs-tab" role="tab" aria-selected="false" data-tab="file">File</button></div>
		<section data-p="search"><div data-as="search"></div><div class="rs-opts" role="radiogroup" aria-label="The places found" data-as="found"></div></section>
		<section data-p="draw" hidden><button type="button" class="pick" data-as="draw" aria-pressed="false">Draw a box on the map</button>
			<p class="small muted" data-as="hint">Press the button, then drag on the map from one corner to the other.</p></section>
		<section data-p="file" hidden><div data-as="drop"></div></section>
		<p class="rs-field-msg" role="status" data-as="msg"></p>
		<div data-as="area" hidden>
			<label class="rs-cap" for="bs-name">Name of the area</label><input id="bs-name" type="text" data-as="name" autocomplete="off" spellcheck="false">
			<p class="small muted" data-as="summary"></p><button type="button" class="rs-link" data-as="clear">Unselect the area</button></div>`;
	const q = (k) => root.querySelector(`[data-as="${k}"]`);
	let current = null, drawing = false;
	const msg = (t, bad = false) => { q("msg").textContent = t; q("msg").classList.toggle("err", bad); };

	// the area on the map: a fill and a line (made when the style has loaded)
	const layers = () => {
		if (map.getSource("bs-area")) return;
		map.addSource("bs-area", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
		map.addLayer({ id: "bs-area-fill", type: "fill", source: "bs-area", paint: { "fill-color": "#0e7c8e", "fill-opacity": 0.14 } });
		map.addLayer({ id: "bs-area-line", type: "line", source: "bs-area", paint: { "line-color": "#0e7c8e", "line-width": 2.5 } });
	};
	const draw = (g) => {
		const put = () => { layers(); map.getSource("bs-area").setData({ type: "FeatureCollection", features: g ? [{ type: "Feature", properties: {}, geometry: g }] : [] }); };
		map.loaded() || map.isStyleLoaded() ? put() : map.once("load", put);
	};

	function set(g, name = "", from = "") {
		const polys = polygonsOf(g);
		if (!polys.length) return msg("This has no polygon.", true);
		const b = bbox(polys), km2 = areaKm2(polys);
		current = { name: slug(name) || "area", geometry: geometry(polys), polygons: polys, bbox: b, km2, from };
		q("name").value = current.name;
		q("area").hidden = false;
		q("summary").innerHTML = `${esc(km2 < 10 ? km2.toFixed(2) : Math.round(km2).toLocaleString("en"))} km², box ${b.map((x) => +x.toFixed(4)).join(", ")} (west, south, east, north).${km2 > 2000 ? " A large area: the build needs much memory." : ""}`;
		msg("");
		draw(current.geometry);
		map.fitBounds([[b[0], b[1]], [b[2], b[3]]], { padding: padding(), duration: 0, maxZoom: 17 });
		onChange();
	}

	function clear() {
		current = null;
		q("area").hidden = true;
		draw(null);
		onChange();
	}

	// the tabs
	root.querySelectorAll("[data-tab]").forEach((t) => t.addEventListener("click", () => {
		root.querySelectorAll("[data-tab]").forEach((x) => { const on = x === t; x.classList.toggle("on", on); x.setAttribute("aria-selected", on); });
		root.querySelectorAll("[data-p]").forEach((p) => { p.hidden = p.dataset.p !== t.dataset.tab; });
		if (t.dataset.tab !== "draw") stopDrawing();
	}));

	// search: the places that have a polygon; a pick makes the area
	let places = [];
	const find = async (text) => {
		text = text.trim();
		q("found").innerHTML = "";
		if (!text) return msg("");
		msg("Searching…");
		try {
			places = await geocode(text);
		} catch (e) {
			return msg(`The search failed: ${e.message}`, true);
		}
		msg(places.length ? "" : "No place with an outline was found. Draw a box or use a file.");
		q("found").innerHTML = places.map((p, i) => `<label class="rs-opt"><input type="radio" name="bs-place" value="${i}"><span><b>${esc(p.name.split(",")[0])}</b><small>${esc(p.name.split(",").slice(1, 3).join(","))}</small></span></label>`).join("");
	};
	q("found").addEventListener("change", (e) => { const p = places[+e.target.value]; if (p) set(p.geometry, p.name.split(",")[0], "search"); });
	rs.search(q("search"), { placeholder: "City, region or country", label: "Name of a place", onSubmit: find });

	// the box: the button turns the drawing on; a drag on the map gives the corners (mouse and touch)
	const canvas = map.getCanvas(), button = q("draw");
	let a = null;
	const at = (e) => { const r = canvas.getBoundingClientRect(); return map.unproject([e.clientX - r.left, e.clientY - r.top]); };
	const rect = (p, o) => box([Math.min(p.lng, o.lng), Math.min(p.lat, o.lat), Math.max(p.lng, o.lng), Math.max(p.lat, o.lat)]);
	function stopDrawing() {
		drawing = false; a = null;
		button.setAttribute("aria-pressed", "false"); button.textContent = "Draw a box on the map";
		canvas.style.cursor = ""; canvas.style.touchAction = ""; map.dragPan.enable(); map.touchZoomRotate?.enable();
	}
	button.addEventListener("click", () => {
		if (drawing) return stopDrawing();
		drawing = true;
		button.setAttribute("aria-pressed", "true"); button.textContent = "Drag on the map…";
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

	q("name").addEventListener("input", () => { if (current) { current.name = slug(q("name").value); onChange(); } });
	q("clear").addEventListener("click", clear);
	return { area: () => current, set, clear };
}
