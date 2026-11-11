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
 *   drop,        // optional: roadstyle's rs.drop. With it the file field is a drop zone, and the step shows the ground id (the SHA-256 of the file)
 *   files,       // optional: a host with a folder (duckosm studio): { list(source, bbox), get(source, bbox) } give a promise of
 *                //   { folder, tiles: [{url, file, have, size}], local: [{file, size}] }. Then the step shows the files of the model for the area, downloads them
 *                //   into the folder, and the build uses the files of the folder (dem() gives { files, source }, no bytes)
 * })
 * It returns { refresh(), dem(), choice() }.
 */
export async function initGroundStep({ root, catalog, bbox, status = () => {}, onChange = () => {}, loadTile = fetchTile, drop = null, files = null }) {
	const all = Array.isArray(catalog) ? catalog : (await (await fetch(catalog)).json()).sources;
	const byId = Object.fromEntries(all.map((s) => [s.id, s]));
	const aws = all.find((s) => s.fallback);
	let chosen = null, file = null, info = null, sha = "", listed = "", shown = null, folder = null, busy = false;
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
		return `<label class="rs-opt"><input type="radio" name="gs-src" value="${esc(s.id)}"><span><b>${esc(s.name)}</b><small>${esc(s.resolution_text)} · ${esc(s.product)} · ${esc(s.license)}${s.commercial_use === false ? " (not for commercial use)" : ""}${a.account !== "none" ? ` · ${a.account === "key" ? "needs a key" : "needs an account"}` : ""}</small><small class="gs-row-state" data-row-state="${esc(s.id)}"></small></span><span class="gs-row-do" data-row-do="${esc(s.id)}"></span></label>`;
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
		if (key !== listed) {                                                       // the area changed: a new list, and nothing is chosen: a file gets the product, the datum and the licence of the source that the person names, never of a guess
			listed = key;
			chosen = null;
			file = info = null; sha = "";
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
		const d = q("detail"), s = chosen && byId[chosen], b = bbox();
		if (!s) { d.innerHTML = b ? `<p class="small muted">Choose the source of your DEM file. The product, the vertical datum and the licence of the file are the source's. With no choice, the area has no DEM.</p>` : ""; return; }
		if (s.fallback) {
			d.innerHTML = `<p class="small muted">The build makes a DEM from the AWS Terrain Tiles when you click Build. This DEM has product DSM and the label “${esc(AWS_NAME)}”: no other project takes it as the ground of the area. For a reference ground, choose a model above and select its file.</p>`;
			return;
		}
		const warn = info?.problems?.length ? `<p class="rs-field-msg gs-bad" role="alert">${esc(info.problems.join(" "))} ${info.epsg !== 4326 || info.bits !== 32 ? `Make a new file with this command, then select it:<br><code>${esc(gdalwarpCommand(file.name))}</code>` : ""}</p>` : "";
		const note = info && !info.problems.length && info.bounds && !coversBox(info.bounds, bbox())
			? `<p class="small gs-warn">The file covers ${esc(fmt(info.bounds))}. The area reaches outside it: the nodes there get no height.</p>` : "";
		const field = drop
			? (file ? `<p class="small"><b>${esc(file.name)}</b> <span class="muted">${(file.size / 1048576).toFixed(1)} MB</span> <button type="button" class="rs-link" data-gs="remove">Remove</button></p>` : `<div data-gs="drop"></div>`)
			: `<button class="pick" type="button" data-gs="pick"><span>${file ? esc(file.name) : "Select the file (.tif)"}</span></button><input type="file" accept=".tif,.tiff" hidden data-gs="file">`;
		d.innerHTML = `<p class="small">${s.access.url ? `<a class="rs-link" href="${esc(s.access.url)}" target="_blank" rel="noopener">Download page of ${esc(s.name)}</a>` : ""} <span class="muted">${esc(s.access.text.replace(/`/g, ""))}</span></p>
			${s.crs !== "EPSG:4326" ? `<p class="small muted">The file is in ${esc(s.crs)}. The build needs EPSG:4326 with 32-bit floats: <code>${esc(gdalwarpCommand("file.tif"))}</code></p>` : ""}
			${files ? `<div class="gs-folder" data-gs="folder"></div><p class="rs-cap">Or another file</p>` : ""}
			${field}
			${warn}${note}
			<dl class="gs-facts small"><dt>Product</dt><dd>${esc(s.product)}${s.product === "DSM" ? " (surface: not a bare-earth ground)" : ""}</dd><dt>Vertical datum</dt><dd>${esc(s.vertical_datum)}</dd><dt>Licence</dt><dd>${esc(s.license)}</dd>${drop && file && !info?.problems?.length ? `<dt>Ground id</dt><dd><code data-gs="sha">${esc(sha || "…")}</code></dd>` : ""}</dl>
			${s.commercial_use === false ? `<p class="small gs-warn">This licence does not allow commercial use.</p>` : ""}`;
		const take = async (f) => {
			file = f || null;
			info = file ? inspectGeoTiff(await file.arrayBuffer()) : null;           // all of the file: GDAL puts the tags at its end
			sha = "";
			detail();
			onChange();
			if (file && drop && crypto?.subtle) {                                     // the ground id: the SHA-256 of the bytes, as `sha256sum` gives it
				const mine = file, h = new Uint8Array(await crypto.subtle.digest("SHA-256", await file.arrayBuffer()));
				if (file === mine) { sha = [...h].map((b) => b.toString(16).padStart(2, "0")).join(""); const el = q("detail").querySelector('[data-gs="sha"]'); if (el) el.textContent = sha; }
			} else if (file && drop) { sha = "(needs a secure page: localhost or https)"; const el = q("detail").querySelector('[data-gs="sha"]'); if (el) el.textContent = sha; }
		};
		if (files) loadFolder(s);
		if (drop) {
			const zone = d.querySelector('[data-gs="drop"]');
			if (zone) {
				drop(zone, { accept: ".tif,.tiff", text: "Drop the DEM file here, or <u>choose a file</u>", hint: "GeoTIFF in EPSG:4326, 32-bit floats", onFiles: (fs) => take(fs[0]) });
				zone.querySelector("input")?.setAttribute("data-gs", "file");
			}
			d.querySelector('[data-gs="remove"]')?.addEventListener("click", () => take(null));
		} else {
			const input = d.querySelector('[data-gs="file"]');
			d.querySelector('[data-gs="pick"]').onclick = () => input.click();
			input.onchange = () => take(input.files[0]);
		}
	}

	// the files of the model in the folder of the host: the tiles of the area (download links), what the folder has, and the button that downloads the rest
	const kb = (n) => (n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);
	const usable = (f) => f && (f.tiles.length ? f.tiles.some((t) => t.have) : f.local.length > 0);
	async function loadFolder(s, get = false) {
		const b = bbox(), mine = s.id;
		if (!b) return;
		try {
			busy = get;
			if (get) paintFolder(s);
			const f = await (get ? files.get : files.list)(s.id, b);
			if (chosen !== mine) return;
			folder = f;
		} catch (e) {
			if (chosen === mine) { folder = null; const el = q("detail").querySelector('[data-gs="folder"]'); if (el) el.innerHTML = `<p class="rs-field-msg gs-bad" role="alert">${esc(e.message)}</p>`; }
			return;
		} finally { busy = false; }
		paintFolder(s);
		onChange();
	}
	const ICON = {
		web: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 4h6v6m0-6-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/></svg>',
	};
	// the chosen row, as an extract row of Studio's data card: its state under the name (in the folder, or not), Download (into the folder), then Use or In use,
	// and the web page of the model. The other rows have nothing at the end.
	function paintFolder(s) {
		for (const el of root.querySelectorAll("[data-row-do], [data-row-state]")) el.innerHTML = "";
		const row = root.querySelector(`[data-row-do="${CSS.escape(s.id)}"]`), state = root.querySelector(`[data-row-state="${CSS.escape(s.id)}"]`);
		const box = q("detail").querySelector('[data-gs="folder"]');
		const f = folder;
		const web = s.access?.url ? `<a class="gs-ico" href="${esc(s.access.url)}" target="_blank" rel="noopener" title="Web page of ${esc(s.name)}" aria-label="Web page of ${esc(s.name)}">${ICON.web}</a>` : "";
		if (busy) {
			if (row) row.innerHTML = web;
			if (state) state.innerHTML = `<span class="gs-prog" role="status"><span class="gs-spin" aria-hidden="true"></span>Downloading into the folder…</span>`;
			return;
		}
		if (!f) { if (row) row.innerHTML = web; return; }
		const have = f.tiles.filter((t) => t.have), size = (f.tiles.length ? have : f.local).reduce((n, t) => n + t.size, 0);
		const ok = usable(f), inUse = ok && !file;
		const action = f.tiles.length && have.length < f.tiles.length ? `<button type="button" data-gs-do="get" title="Download the files of ${esc(s.name)} for this area into the folder, for the build">Download</button>`
			: ok ? `<button type="button" data-gs-do="use"${inUse ? " disabled" : ""}>${inUse ? "In use" : "Use"}</button>` : "";
		if (row) row.innerHTML = action + web;
		if (state) state.textContent = f.tiles.length
			? (have.length === f.tiles.length ? `in the folder · ${kb(size)}` : have.length ? `${have.length} of ${f.tiles.length} files in the folder` : `${f.tiles.length} file${f.tiles.length > 1 ? "s" : ""} to download · not in the folder`)
			: f.local.length ? `${f.local.length} file${f.local.length > 1 ? "s" : ""} in the folder · ${kb(size)}` : "no direct download: get the file on the web page, and put it in the folder";
		if (box) box.innerHTML = `<p class="small muted">Folder of ${esc(s.name)}: <code class="bs-path">${esc(f.folder)}</code></p>`
			+ (f.tiles.length > 1 ? `<ul class="small gs-files">${f.tiles.map((t) => `<li>${esc(t.file)} <span class="muted">${t.have ? kb(t.size) : "not downloaded"}</span></li>`).join("")}</ul>` : "")
			+ (ok ? `<p class="small ${inUse ? "gs-ok" : "muted"}">${inUse ? "The build uses the files in the folder: it joins them and cuts them to the area." : "The build uses the file below. Use takes the folder again."}</p>` : "");
	}
	// Download and Use are buttons in a label: a click on them does not choose the row again
	root.addEventListener("click", (e) => {
		const b = e.target.closest("[data-gs-do]");
		if (!b) return;
		e.preventDefault();
		const s = chosen && byId[chosen];
		if (!s) return;
		if (b.dataset.gsDo === "get") loadFolder(s, true);
		else if (b.dataset.gsDo === "use") { file = info = null; sha = ""; detail(); onChange(); }
	});


	root.addEventListener("change", (e) => {
		if (e.target.name !== "gs-src") return;
		chosen = e.target.value;
		file = info = null; folder = null;
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
		if (!file && files && usable(folder)) {                                          // the files of the folder: the host joins and cuts them (no bytes go to the server)
			const names = folder.tiles.length ? folder.tiles.filter((t) => t.have).map((t) => t.file) : folder.local.map((l) => l.file);
			return { name: `${s.name}: ${names.join(", ")}`, source: s.id, files: names, product: s.product, vertical_datum: s.vertical_datum, license: s.license, not_reference: false };
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
		choice: () => ({ id: chosen, name: chosen && byId[chosen].name, file: file ? file.name : files && usable(folder) ? "(the folder)" : null }),
	};
}

function coversBox(bounds, b) {
	return !b || (bounds[0] <= b[0] && bounds[1] <= b[1] && bounds[2] >= b[2] && bounds[3] >= b[3]);
}
