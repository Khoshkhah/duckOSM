// The Ground step: suggest the DEM of an area and give it to the build (docs/design/ground_step.md, docs/guides/ground-step.md).
// It shows the sources that cover the area (best first), and for the chosen one the download page and a file field, or, with no file, a
// DEM from the AWS Terrain Tiles that is not a reference. dem() gives the build what it needs: the bytes, the product, the vertical datum and the licence.
import { TERRARIUM_URL, gdalwarpCommand, inspectGeoTiff, sourcesFor, terrariumGeoTiff, terrariumPlan } from "./ground_core.js";

export { sourcesFor, inspectGeoTiff, gdalwarpCommand, terrariumGeoTiff, terrariumPlan } from "./ground_core.js";

export const AWS_NAME = "Ground: AWS Terrain Tiles";
const SHOWN = 4;                                                                    // sources in the list; the others are under "More models"

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const fmt = (b) => b.map((x) => +x.toFixed(4)).join(", ");

/** The default loader of a tile: the PNG of AWS as RGBA bytes. */
export async function fetchTile(z, x, y) {
	const url = TERRARIUM_URL.replace("{z}", z).replace("{x}", x).replace("{y}", y);
	const res = await fetch(url);
	if (!res.ok) throw new Error(`AWS Terrain Tiles gave no tile ${z}/${x}/${y} (HTTP ${res.status}).`);
	const bmp = await createImageBitmap(await res.blob(), { premultiplyAlpha: "none", colorSpaceConversion: "none" });
	const cv = typeof OffscreenCanvas === "function" ? new OffscreenCanvas(256, 256) : Object.assign(document.createElement("canvas"), { width: 256, height: 256 });
	const g = cv.getContext("2d", { willReadFrequently: true });
	g.drawImage(bmp, 0, 0);
	return g.getImageData(0, 0, 256, 256).data;
}

/**
 * initGroundStep({
 *   root,        // the element that gets the step
 *   catalog,     // the list of sources: the array, or the URL of dem_sources.json (its `sources`)
 *   bbox,        // () gives [west, south, east, north] of the area, or null when there is no area yet
 *   status,      // optional: status(text, ms), a short message for the user
 *   onChange,    // optional: (): the choice or the file is new
 *   loadTile,    // optional: (z, x, y) gives the RGBA bytes of a tile (the tests give their own)
 * })
 * It returns { refresh(), dem(), choice() }.
 */
export async function initGroundStep({ root, catalog, bbox, status = () => {}, onChange = () => {}, loadTile = fetchTile }) {
	const all = Array.isArray(catalog) ? catalog : (await (await fetch(catalog)).json()).sources;
	const byId = Object.fromEntries(all.map((s) => [s.id, s]));
	const aws = all.find((s) => s.fallback);
	let chosen = null, file = null, info = null, listed = "", shown = null;
	root.id = root.id || "ground-step";
	root.classList.add("gs");
	root.innerHTML = `<p class="gs-area small muted" data-gs="area"></p>
		<div class="rs-cap">Ground for this area</div>
		<div class="rs-opts" role="radiogroup" aria-label="Ground for this area" data-gs="list"></div>
		<details class="gs-more" data-gs="more" hidden><summary>More models</summary><div class="rs-opts" role="radiogroup" data-gs="rest"></div></details>
		<div class="rs-opts" data-gs="aws"></div>
		<div class="gs-detail" data-gs="detail"></div>`;
	const q = (k) => root.querySelector(`[data-gs="${k}"]`);

	const option = (s) => {
		const a = s.access;
		return `<label class="rs-opt"><input type="radio" name="gs-src" value="${esc(s.id)}"><span><b>${esc(s.name)}</b><small>${esc(s.resolution_text)} · ${esc(s.product)} · ${esc(s.license)}${s.commercial_use === false ? " (not for commercial use)" : ""}${a.account !== "none" ? ` · ${a.account === "key" ? "needs a key" : "needs an account"}` : ""}</small></span></label>`;
	};

	function sources() {
		const b = bbox();
		return b ? sourcesFor(all, b) : [];
	}

	function render() {
		const b = bbox(), list = sources(), sig = JSON.stringify(b) + list.map((s) => s.id);
		if (sig === shown) return;                                                  // the same area: nothing to draw again
		shown = sig;
		q("area").textContent = b ? `Area: ${fmt(b)} (west, south, east, north)` : "Choose the area in step 1 first.";
		const key = list.map((s) => s.id).join();
		if (key !== listed) {                                                       // the area changed: a new list, and the best source is the choice
			listed = key;
			chosen = list[0]?.id || null;
			file = info = null;
		}
		q("list").innerHTML = list.slice(0, SHOWN).map(option).join("");
		q("rest").innerHTML = list.slice(SHOWN).map(option).join("");
		q("more").hidden = list.length <= SHOWN;
		q("more").querySelector("summary").textContent = `More models (${Math.max(0, list.length - SHOWN)})`;
		let awsHtml = "";
		if (aws && b) {
			const p = terrariumPlan(b);
			awsHtml = `<label class="rs-opt"><input type="radio" name="gs-src" value="${esc(aws.id)}"><span><b>${esc(AWS_NAME)}</b><small>No file needed. About ${p.resolution_m} m for this area, a surface model (DSM) of mixed sources. It is never a reference ground.</small></span></label>`;
		}
		q("aws").innerHTML = awsHtml;
		for (const r of root.querySelectorAll('input[name="gs-src"]')) r.checked = r.value === chosen;
		detail();
	}

	function detail() {
		const d = q("detail"), s = chosen && byId[chosen];
		if (!s) { d.innerHTML = ""; return; }
		if (s.fallback) {
			d.innerHTML = `<p class="small muted">The build makes a DEM from the AWS Terrain Tiles when you click Build. This DEM has product DSM and the label “${esc(AWS_NAME)}”: no other project takes it as the ground of the area. For a reference ground, choose a model above and select its file.</p>`;
			return;
		}
		const warn = info?.problems?.length ? `<p class="rs-field-msg gs-bad" role="alert">${esc(info.problems.join(" "))} ${info.epsg !== 4326 || info.bits !== 32 ? `Make a new file with this command, then select it:<br><code>${esc(gdalwarpCommand(file.name))}</code>` : ""}</p>` : "";
		const note = info && !info.problems.length && info.bounds && !coversBox(info.bounds, bbox())
			? `<p class="small gs-warn">The file covers ${esc(fmt(info.bounds))}. The area reaches outside it: the nodes there get no height.</p>` : "";
		d.innerHTML = `<p class="small">${s.access.url ? `<a class="rs-link" href="${esc(s.access.url)}" target="_blank" rel="noopener">Download page of ${esc(s.name)}</a>` : ""} <span class="muted">${esc(s.access.text.replace(/`/g, ""))}</span></p>
			${s.crs !== "EPSG:4326" ? `<p class="small muted">The file is in ${esc(s.crs)}. The build needs EPSG:4326 with 32-bit floats: <code>${esc(gdalwarpCommand("file.tif"))}</code></p>` : ""}
			<button class="pick" type="button" data-gs="pick"><span>${file ? esc(file.name) : "Select the file (.tif)"}</span></button><input type="file" accept=".tif,.tiff" hidden data-gs="file">
			${warn}${note}
			<dl class="gs-facts small"><dt>Product</dt><dd>${esc(s.product)}${s.product === "DSM" ? " (surface: not a bare-earth ground)" : ""}</dd><dt>Vertical datum</dt><dd>${esc(s.vertical_datum)}</dd><dt>Licence</dt><dd>${esc(s.license)}</dd></dl>
			${s.commercial_use === false ? `<p class="small gs-warn">This licence does not allow commercial use.</p>` : ""}`;
		const input = d.querySelector('[data-gs="file"]');
		d.querySelector('[data-gs="pick"]').onclick = () => input.click();
		input.onchange = async () => {
			file = input.files[0] || null;
			info = file ? inspectGeoTiff(await file.arrayBuffer()) : null;           // all of the file: GDAL puts the tags at its end
			detail();
			onChange();
		};
	}

	root.addEventListener("change", (e) => {
		if (e.target.name !== "gs-src") return;
		chosen = e.target.value;
		file = info = null;
		detail();
		onChange();
	});

	/** The DEM for the build, or null when there is none to give (a source without a file). It throws an Error with a message for the person when the file cannot be used. */
	async function dem() {
		const s = chosen && byId[chosen], b = bbox();
		if (!s) return null;
		if (s.fallback) {
			if (!b) throw new Error("Choose the area first.");
			status("Reading the AWS Terrain Tiles…");
			const { bytes, plan } = await terrariumGeoTiff(b, loadTile);
			status(`AWS Terrain Tiles: ${plan.tiles} tiles, ${plan.width} × ${plan.height} pixels.`, 4000);
			return { name: AWS_NAME, bytes: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), product: aws.product, vertical_datum: aws.vertical_datum, license: aws.license, not_reference: true };
		}
		if (!file) return null;
		const bytes = await file.arrayBuffer();                                       // a new buffer for each build: the host gives it to the worker
		const head = info || inspectGeoTiff(bytes);
		if (head.problems.length) throw new Error(`${file.name}: ${head.problems.join(" ")} ${gdalwarpCommand(file.name)}`);
		return { name: file.name, bytes, product: s.product, vertical_datum: s.vertical_datum, license: s.license, not_reference: false };
	}

	render();
	return {
		refresh: render,
		dem,
		/** What is chosen: { id, name, file } (the file is null with no file). */
		choice: () => ({ id: chosen, name: chosen && byId[chosen].name, file: file && file.name }),
	};
}

function coversBox(bounds, b) {
	return !b || (bounds[0] <= b[0] && bounds[1] <= b[1] && bounds[2] >= b[2] && bounds[3] >= b[3]);
}
