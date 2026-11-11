// duckosm studio: the host of the build steps (web/build_steps/, web/ground_step/) on the server of `duckosm studio`. The steps are parts of duckOSM that
// duckOSM Studio (the browser app) loads too; this page gives them the map, the server (the download, the build) and roadstyle's parts.
import { initAreaStep } from "./build_steps/area_step.js";
import { initDataStep } from "./build_steps/data_step.js";
import { initSettingsStep } from "./build_steps/settings_step.js";
import { initBuildStep } from "./build_steps/build_step.js";
import { initResultStep } from "./build_steps/result_step.js";
import { extractsOf } from "./build_steps/core.js";
import { initGroundStep } from "./ground_step/ground_step.js";
import { rsSearch } from "../roadstyle/search.js";
import { rsDrop } from "../roadstyle/drop.js";

const $ = (id) => document.getElementById(id);
const api = async (path, init) => {
	const r = await fetch("api/" + path, init);
	if (!r.ok) throw new Error((await r.text()) || r.statusText);
	return r;
};

async function start() {
	// a plain raster map of OpenStreetMap under the area (its tiles come from the internet; the page works without them)
	const map = new maplibregl.Map({ container: "map", center: [10, 50], zoom: 3, attributionControl: { compact: true }, style: { version: 8,
		sources: { osm: { type: "raster", tiles: ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"], tileSize: 256, maxzoom: 19, attribution: "© OpenStreetMap contributors" } },
		layers: [{ id: "bg", type: "background", paint: { "background-color": "#e8ecef" } }, { id: "osm", type: "raster", source: "osm" }] } });
	map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-left");
	window.map = map;

	const padding = () => matchMedia("(max-width: 700px)").matches
		? { top: 30, left: 30, right: 30, bottom: 30 } : { top: 40, left: 40, bottom: 40, right: $("sb").offsetWidth + 40 };
	const refresh = () => { data.refresh(); build.refresh(); };
	const settings = initSettingsStep({ root: $("step-settings"), onChange: () => build.refresh() });
	const area = initAreaStep({ root: $("step-area"), map, rs: { search: rsSearch, drop: rsDrop }, padding,
		onChange: () => { ground.refresh(); refresh(); } });
	let extracts = null;
	const data = initDataStep({ root: $("step-data"), area: () => area.area(),
		extracts: () => (extracts ??= api("geofabrik").then((r) => r.json()).then(extractsOf).catch((e) => { extracts = null; throw e; })),
		checkFile: async (path) => (await (await api("file", { method: "POST", body: path })).json()).size, onChange: () => build.refresh() });
	const ground = await initGroundStep({ root: $("step-ground"), catalog: "parts/data/dem_sources.json", bbox: () => area.area()?.bbox ?? null,
		status: (t) => { $("step-build").querySelector('[data-bs="status"]').textContent = t; }, onChange: () => build.refresh() });
	const result = initResultStep({ root: $("step-result"), href: "edit/" });
	const build = initBuildStep({ root: $("step-build"),
		ready: () => {
			if (!area.area()) return "Choose the area in step 1.";
			if (!data.choice()) return "Choose the OSM data in step 2.";
			try { settings.yaml(); } catch (e) { return e.message; }
			return "";
		},
		run: async () => {
			result.clear();
			const a = area.area(), dem = await ground.dem();
			if (dem) await api(`dem/${a.name}`, { method: "PUT", body: dem.bytes });
			await api("build", { method: "POST", body: JSON.stringify({ name: a.name, boundary: a.geometry, data: data.choice(), settings: settings.yaml(),
				dem: dem && { product: dem.product, vertical_datum: dem.vertical_datum, license: dem.license, not_reference: dem.not_reference } }) });
		},
		poll: async () => (await api("job")).json(),
		onDone: (job) => { result.show(job); $("h-result").scrollIntoView({ block: "nearest" }); } });
	window.studio = { area, data, settings, ground, build, result, map };           // for the tests
	window.studioReady = true;
}

start().catch((e) => { console.error(e); document.body.insertAdjacentHTML("beforeend", `<p id="edit-error" role="alert">duckosm studio: ${String(e.message).replace(/</g, "&lt;")}</p>`); });
