// The OSM data step: where the OpenStreetMap data of the area comes from. A Geofabrik extract that covers the area (the smallest first), or a .osm.pbf
// file on this computer (its path). The step gives the choice; the host downloads or reads. A host in a browser with no file system does not use the file tab.
import { bytes, covering, esc } from "./core.js";

/**
 * initDataStep({
 *   root,       // the element that gets the step
 *   area,       // () gives the area of the Area step ({polygons, ...}) or null
 *   extracts,   // () gives a promise of the Geofabrik extracts (core.js extractsOf)
 *   checkFile,  // (path) gives a promise of the size of the file, or throws an Error that says why it cannot be used
 *   onChange,   // optional: the choice is new
 * })
 * It returns { refresh(), choice() }. choice() is null, { kind: "geofabrik", id, name, url, km2 } or { kind: "file", path, size }.
 */
export function initDataStep({ root, area, extracts, checkFile, onChange = () => {} }) {
	root.classList.add("bs", "bs-data");
	root.innerHTML = `<div class="rs-tabs" role="tablist" aria-label="Where the OSM data comes from">
			<button type="button" class="rs-tab on" role="tab" aria-selected="true" data-tab="geofabrik">Geofabrik</button>
			<button type="button" class="rs-tab" role="tab" aria-selected="false" data-tab="file">File</button></div>
		<section data-p="geofabrik"><p class="small muted" data-ds="note"></p><div class="rs-opts" role="radiogroup" aria-label="The extracts that cover the area" data-ds="list"></div></section>
		<section data-p="file" hidden><label class="rs-cap" for="bs-pbf">Path of a .osm.pbf file on this computer</label>
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
		picked = list[0]?.id ?? "";
		q("note").textContent = list.length ? "The extracts of Geofabrik that cover the area. The first is the smallest." : "No Geofabrik extract covers the whole area. Use a file.";
		q("list").innerHTML = list.slice(0, 6).map((x) => `<label class="rs-opt"><input type="radio" name="bs-extract" value="${esc(x.id)}"${x.id === picked ? " checked" : ""}><span><b>${esc(x.name)}</b><small>${Math.round(x.km2).toLocaleString("en")} km² · ${esc(x.pbf.split("/").slice(-1)[0])}</small></span></label>`).join("");
		onChange();
	}
	q("list").addEventListener("change", (e) => { picked = e.target.value; onChange(); });

	root.querySelectorAll("[data-tab]").forEach((t) => t.addEventListener("click", () => {
		tab = t.dataset.tab;
		root.querySelectorAll("[data-tab]").forEach((x) => { const on = x === t; x.classList.toggle("on", on); x.setAttribute("aria-selected", on); });
		root.querySelectorAll("[data-p]").forEach((p) => { p.hidden = p.dataset.p !== tab; });
		onChange();
	}));

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
	return { refresh, choice };
}
