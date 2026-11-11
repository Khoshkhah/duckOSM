// The OSM data step: where the OpenStreetMap data of the area comes from. A Geofabrik extract that covers the area (the smallest first), or a .osm.pbf
// file on this computer (its path). The step gives the choice; the host downloads or reads. A host in a browser with no file system does not use the file tab.
import { bytes, covering, esc, geometry } from "./core.js";

/**
 * initDataStep({
 *   root,       // the element that gets the step
 *   rs,         // roadstyle's parts: { tabs }
 *   map,        // optional: the MapLibre map. With it an extract shows on the map (orange, dashed): under the mouse, and after a click (the map moves out to it)
 *   padding,    // optional: () gives the padding of the map for fitBounds
 *   zoomToArea, // optional: () moves the map back to the area (a second click on the shown extract)
 *   area,       // () gives the area of the Area step ({polygons, ...}) or null
 *   extracts,   // () gives a promise of the Geofabrik extracts (core.js extractsOf)
 *   checkFile,  // (path) gives a promise of the size of the file, or throws an Error that says why it cannot be used
 *   onChange,   // optional: the choice is new
 * })
 * It returns { refresh(), choice() }. choice() is null, { kind: "geofabrik", id, name, url, km2 } or { kind: "file", path, size }.
 */
export function initDataStep({ root, rs, map = null, padding = () => 40, zoomToArea = () => {}, area, extracts, checkFile, onChange = () => {} }) {
	root.classList.add("bs", "bs-data");
	root.innerHTML = `<div class="rs-tabs" role="tablist" aria-label="Where the OSM data comes from">
			<button type="button" class="rs-tab on" data-tab="geofabrik">Geofabrik</button>
			<button type="button" class="rs-tab" data-tab="file">File</button></div>
		<section data-pane="geofabrik"><p class="small muted" data-ds="note"></p><div class="rs-opts" role="radiogroup" aria-label="The extracts that cover the area" data-ds="list"></div></section>
		<section data-pane="file" hidden><label class="rs-cap" for="bs-pbf">Path of a .osm.pbf file on this computer</label>
			<input id="bs-pbf" type="text" data-ds="path" autocomplete="off" spellcheck="false" placeholder="/data/area.osm.pbf">
			<p class="rs-field-msg" role="status" data-ds="msg"></p></section>`;
	const q = (k) => root.querySelector(`[data-ds="${k}"]`);
	let tab = "geofabrik", list = [], picked = "", file = null, key = "", loading = null;

	const choice = () => {
		if (tab === "file") return file;
		const x = list.find((e) => e.id === picked);
		return x ? { kind: "geofabrik", id: x.id, name: x.name, url: x.pbf, km2: x.km2 } : null;
	};

	async function refresh() {
		const a = area();
		const k = a ? JSON.stringify(a.bbox) : "";
		if (k === key) return;
		key = k;
		list = [];
		q("list").innerHTML = "";
		if (!a) { q("note").textContent = "Choose the area first."; return onChange(); }
		q("note").textContent = "Looking for the extracts that cover the area…";
		const mine = (loading = Symbol());
		try {
			const all = await extracts();
			if (mine !== loading) return;
			list = covering(all, a.polygons);
		} catch (e) {
			if (mine === loading) { key = ""; q("note").textContent = `The list of the extracts did not load: ${e.message}. Use a file.`; }
			return onChange();
		}
		picked = "";
		q("note").textContent = list.length ? "The extracts of Geofabrik that cover the area, the smallest first. Choose one." : "No Geofabrik extract covers the whole area. Use a file.";
		q("list").innerHTML = list.slice(0, 6).map((x) => `<label class="rs-opt"><input type="radio" name="bs-extract" value="${esc(x.id)}"><span><b>${esc(x.name)}</b><small>${Math.round(x.km2).toLocaleString("en")} km² · ${esc(x.pbf.split("/").slice(-1)[0])}</small></span></label>`).join("");
		onChange();
	}
	q("list").addEventListener("change", (e) => { picked = e.target.value; onChange(); });

	rs.tabs.use(root.querySelector(".rs-tabs"), (id) => { tab = id; onChange(); });      // roadstyle shows the pane of the chosen tab

	let timer = 0;
	q("path").addEventListener("input", () => {
		clearTimeout(timer);
		file = null;
		q("msg").textContent = ""; q("msg").classList.remove("err");
		onChange();
		const path = q("path").value.trim();
		if (!path) return;
		timer = setTimeout(async () => {
			try {
				const size = await checkFile(path);
				if (q("path").value.trim() !== path) return;
				file = { kind: "file", path, size };
				q("msg").textContent = `Found: ${bytes(size)}.`;
			} catch (e) {
				q("msg").textContent = e.message; q("msg").classList.add("err");
			}
			onChange();
		}, 250);
	});
	// the extract on the map, as in Studio: the outline of the extract under the mouse; a click keeps it and moves the map out to it; a second click on it hides it and moves the map back to the area
	let shown = null, hovered = null;
	const outline = (x) => (x ? { type: "Feature", properties: { name: `${x.name} (Geofabrik)` }, geometry: geometry(x.polygon) } : null);
	function paint() {
		const x = list.find((e) => e.id === (hovered ?? shown));
		if (!map) return;
		const put = () => {
			if (!map.getSource("bs-extract")) {
				map.addSource("bs-extract", { type: "geojson", data: { type: "FeatureCollection", features: [] } });
				map.addLayer({ id: "bs-extract-casing", type: "line", source: "bs-extract", paint: { "line-color": "#ffffff", "line-width": 6, "line-opacity": 0.9 } });
				map.addLayer({ id: "bs-extract-line", type: "line", source: "bs-extract", paint: { "line-color": "#ea580c", "line-width": 3, "line-dasharray": [2, 1.5] } });
				map.addLayer({ id: "bs-extract-label", type: "symbol", source: "bs-extract", layout: { "symbol-placement": "line", "symbol-spacing": 300, "text-field": ["get", "name"], "text-font": ["Noto Sans Bold"], "text-size": 12, "text-offset": [0, -1] },
					paint: { "text-color": "#c2410c", "text-halo-color": "#ffffff", "text-halo-width": 2 } });
			}
			map.getSource("bs-extract").setData({ type: "FeatureCollection", features: x ? [outline(x)] : [] });
		};
		if (map.getSource("bs-extract") || map.isStyleLoaded()) put(); else map.once("idle", put);
	}
	const fit = (x) => { const b = x.bbox; map.fitBounds([[b[0], b[1]], [b[2], b[3]]], { padding: padding(), maxZoom: 15, duration: 600 }); };
	// under the mouse the map moves to the extract when it is not all in view (an extract is mostly bigger than the view), or too small to see (the rule of the search list)
	const fitIfHidden = (x) => {
		const b = x.bbox, view = map.getBounds();
		const small = b[2] - b[0] < (view.getEast() - view.getWest()) / 4 && b[3] - b[1] < (view.getNorth() - view.getSouth()) / 4;
		if (!(view.contains([b[0], b[1]]) && view.contains([b[2], b[3]])) || small) fit(x);
	};
	if (map) {
		map.on("styledata", () => { if ((shown || hovered) && !map.getSource("bs-extract")) try { paint(); } catch { /* the next styledata tries again */ } });
		const over = (e) => { const r = e.target.closest("label")?.querySelector('input[name="bs-extract"]'); if (r && hovered !== r.value) { hovered = r.value; paint(); const x = list.find((e) => e.id === hovered); if (x) fitIfHidden(x); } };
		q("list").addEventListener("mouseover", over);
		q("list").addEventListener("focusin", over);
		q("list").addEventListener("mouseleave", () => { hovered = null; paint(); });
		q("list").addEventListener("click", (e) => {
			const r = e.target.closest('input[name="bs-extract"]');
			if (!r) return;
			if (shown === r.value) { r.checked = false; shown = null; picked = ""; paint(); zoomToArea(); return onChange(); }       // a second click: the outline goes, the map goes back to the area
			shown = r.value; paint(); fit(list.find((x) => x.id === shown));
		});
	}
	return { refresh, choice };
}
