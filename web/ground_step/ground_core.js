// The logic of the Ground step, with no DOM: which sources cover an area, what a GeoTIFF file is, and a DEM made from the AWS Terrain Tiles.
// The same rules as src/duckosm/dem_sources.py (a test compares the two). Docs: docs/design/ground_step.md.

const inRing = (lon, lat, ring) => {
	let inside = false;
	for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
		const [x1, y1] = ring[j], [x2, y2] = ring[i];
		if ((y1 > lat) !== (y2 > lat) && lon < (x2 - x1) * (lat - y1) / (y2 - y1) + x1) inside = !inside;
	}
	return inside;
};

/** True when the whole box [west, south, east, north] is inside the coverage of the source and touches none of its `exclude` boxes. */
export function covers(source, bbox) {
	const [w, s, e, n] = bbox, cov = source.coverage;
	let ok;
	if (cov.bbox) ok = cov.bbox[0] <= w && e <= cov.bbox[2] && cov.bbox[1] <= s && n <= cov.bbox[3];
	else ok = [w, e].every((x) => [s, n].every((y) => inRing(x, y, cov.polygon)));
	return ok && !(cov.exclude || []).some(([xw, ys, xe, yn]) => w < xe && xw < e && s < yn && ys < n);
}

/** The sources that cover the box, best first: rank, then resolution, then the order of the catalog. */
export function sourcesFor(catalog, bbox, { includeFallback = false, includeAll = false } = {}) {
	return catalog
		.map((s, i) => [s, i])
		.filter(([s]) => covers(s, bbox) && (includeAll || s.suggest !== false) && (includeFallback || !s.fallback))
		.sort((a, b) => a[0].rank - b[0].rank || a[0].resolution_m - b[0].resolution_m || a[1] - b[1])
		.map(([s]) => s);
}

/** The command that makes a file that the build reads: EPSG:4326, 32-bit floats. */
export const gdalwarpCommand = (name) => `gdalwarp -t_srs EPSG:4326 -ot Float32 "${name}" "ground_4326.tif"`;

/** What the build needs to know of a GeoTIFF, from its bytes. `problems` is empty when the build can read the file; `bounds` is
 *  [west, south, east, north] of an EPSG:4326 file. */
export function inspectGeoTiff(bytes) {
	const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
	const bad = (m) => ({ problems: [m], epsg: null, bounds: null });
	if (u8.length < 16) return bad("This is not a GeoTIFF file.");
	const dv = new DataView(u8.buffer, u8.byteOffset, u8.byteLength);
	const le = u8[0] === 0x49 && u8[1] === 0x49;
	if (!le && !(u8[0] === 0x4d && u8[1] === 0x4d)) return bad("This is not a GeoTIFF file.");
	const magic = dv.getUint16(2, le);
	if (magic === 43) return bad("This is a BigTIFF file. Convert it: " + gdalwarpCommand("file.tif"));
	if (magic !== 42) return bad("This is not a GeoTIFF file.");
	const ifd = dv.getUint32(4, le), count = dv.getUint16(ifd, le), tags = {};
	const size = { 1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 6: 1, 7: 1, 8: 2, 9: 4, 10: 8, 11: 4, 12: 8, 16: 8 };
	for (let i = 0; i < count; i++) {
		const at = ifd + 2 + i * 12, tag = dv.getUint16(at, le), type = dv.getUint16(at + 2, le), n = dv.getUint32(at + 4, le);
		const bytesN = (size[type] || 1) * n, base = bytesN <= 4 ? at + 8 : dv.getUint32(at + 8, le);
		if (base + bytesN > u8.length) continue;
		const get = (k) => type === 3 ? dv.getUint16(base + 2 * k, le) : type === 4 ? dv.getUint32(base + 4 * k, le) : type === 12 ? dv.getFloat64(base + 8 * k, le) : null;
		if (type === 2) tags[tag] = new TextDecoder().decode(u8.subarray(base, base + n)).replace(/\0+$/, "");
		else tags[tag] = Array.from({ length: n }, (_, k) => get(k));
	}
	const one = (t) => tags[t]?.[0];
	const width = one(256), height = one(257), bits = one(258), format = one(339) ?? 1, samples = one(277) ?? 1;
	const keys = tags[34735] || [];
	let epsg = null;
	for (let k = 4; k + 3 < keys.length; k += 4) {
		if (keys[k] === 3072 && keys[k + 1] === 0) epsg = keys[k + 3];          // a projected CRS wins
		if (keys[k] === 2048 && keys[k + 1] === 0 && epsg === null) epsg = keys[k + 3];
	}
	const problems = [];
	if (epsg !== 4326) problems.push(epsg ? `The file is in EPSG:${epsg}, not EPSG:4326.` : "The file has no coordinate system (EPSG:4326 is necessary).");
	if (bits !== 32 || format !== 3 || samples !== 1) problems.push("The file must have one band of 32-bit floats.");
	let bounds = null;
	const sc = tags[33550], tp = tags[33922];
	if (epsg === 4326 && sc && tp && width && height) {
		const west = tp[3] - tp[0] * sc[0], north = tp[4] + tp[1] * sc[1];
		bounds = [west, north - height * sc[1], west + width * sc[0], north];
	}
	return { problems, epsg, bits, format, width, height, bounds };
}

/** A GeoTIFF in EPSG:4326 with one band of 32-bit floats, one strip, no compression: what the build reads.
 *  data: Float32Array of width x height, the first row at the north. */
export function writeGeoTiff({ width, height, west, north, dx, dy, data, nodata = -9999 }) {
	const ascii = new TextEncoder().encode(String(nodata) + "\0");
	const geokeys = [1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, 4326];     // GTModelType geographic, PixelIsArea, GeographicType 4326
	const entries = [
		[256, 4, [width]], [257, 4, [height]], [258, 3, [32]], [259, 3, [1]], [262, 3, [1]], [273, 4, ["strip"]], [277, 3, [1]], [278, 4, [height]],
		[279, 4, [data.byteLength]], [339, 3, [3]], [33550, 12, [dx, dy, 0]], [33922, 12, [0, 0, 0, west, north, 0]], [34735, 3, geokeys], [42113, 2, ascii],
	];
	const ifdSize = 2 + entries.length * 12 + 4;
	let extra = 8 + ifdSize;                                                           // where the data that does not fit in an entry starts
	const place = {};
	for (const [tag, type, v] of entries) {
		const len = type === 2 ? v.length : v.length * { 3: 2, 4: 4, 12: 8 }[type];
		if (len > 4) { place[tag] = extra; extra += len + (len % 2); }
	}
	const stripAt = extra + (extra % 4 ? 4 - (extra % 4) : 0);
	const out = new Uint8Array(stripAt + data.byteLength), dv = new DataView(out.buffer);
	out.set([0x49, 0x49, 42, 0]);
	dv.setUint32(4, 8, true);
	dv.setUint16(8, entries.length, true);
	entries.forEach(([tag, type, v], i) => {
		const at = 10 + i * 12, n = v.length;
		dv.setUint16(at, tag, true); dv.setUint16(at + 2, type, true); dv.setUint32(at + 4, n, true);
		const w = (o, k, x) => (type === 3 ? dv.setUint16(o + 2 * k, x, true) : type === 4 ? dv.setUint32(o + 4 * k, x, true) : dv.setFloat64(o + 8 * k, x, true));
		if (type === 2) { if (n > 4) { out.set(v, place[tag]); dv.setUint32(at + 8, place[tag], true); } else out.set(v, at + 8); return; }
		const vals = v.map((x) => (x === "strip" ? stripAt : x)), o = place[tag] ?? at + 8;
		if (place[tag] !== undefined) dv.setUint32(at + 8, o, true);
		vals.forEach((x, k) => w(o, k, x));
	});
	out.set(new Uint8Array(data.buffer, data.byteOffset, data.byteLength), stripAt);
	return out;
}

// ---- the AWS Terrain Tiles (Terrarium): PNG tiles in Web Mercator, height = R * 256 + G + B / 256 - 32768 ----

export const TERRARIUM_URL = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png";
const MAX_LAT = 85.0511;

const tileX = (lon, z) => ((lon + 180) / 360) * 2 ** z;
const tileY = (lat, z) => {
	const r = (Math.max(-MAX_LAT, Math.min(MAX_LAT, lat)) * Math.PI) / 180;
	return ((1 - Math.log(Math.tan(r) + 1 / Math.cos(r)) / Math.PI) / 2) * 2 ** z;
};

/** The grid of the DEM and the tiles that it needs. The zoom is the highest one (at most `maxZ`) with at most `maxTiles` tiles. The box has 2 % of margin,
 *  so that the nodes at the edge and the 3 pixels that the build adds have heights. dx and dy are the pixel sizes in degrees: dx is the width of a Mercator
 *  pixel, dy is dx times cos(latitude). */
export function terrariumPlan(bbox, { maxTiles = 64, maxZ = 14 } = {}) {
	const pad = (v) => Math.max(0.001, v * 0.02);
	const [w0, s0, e0, n0] = bbox, pw = pad(e0 - w0), ph = pad(n0 - s0);
	const west = Math.max(-180, w0 - pw), east = Math.min(180, e0 + pw), south = Math.max(-MAX_LAT, s0 - ph), north = Math.min(MAX_LAT, n0 + ph);
	let z = maxZ, x0, x1, y0, y1;
	for (; z >= 0; z--) {
		x0 = Math.floor(tileX(west, z)); x1 = Math.floor(tileX(east, z)); y0 = Math.floor(tileY(north, z)); y1 = Math.floor(tileY(south, z));
		if ((x1 - x0 + 1) * (y1 - y0 + 1) <= maxTiles) break;
	}
	z = Math.max(z, 0);
	const dx = 360 / (256 * 2 ** z), dy = dx * Math.cos((((south + north) / 2) * Math.PI) / 180);
	return {
		z, x0, x1, y0, y1, west, north, dx, dy, tiles: (x1 - x0 + 1) * (y1 - y0 + 1),
		width: Math.max(1, Math.ceil((east - west) / dx)), height: Math.max(1, Math.ceil((north - south) / dy)),
		resolution_m: Math.round(dy * 111320),
	};
}

/** The height of a Terrarium pixel. */
export const terrariumHeight = (r, g, b) => r * 256 + g + b / 256 - 32768;

/** The DEM (a GeoTIFF as bytes) of the box from the tiles. `loadTile(z, x, y)` gives the RGBA bytes of a tile of 256 x 256 pixels. */
export async function terrariumGeoTiff(bbox, loadTile, opts = {}) {
	const p = terrariumPlan(bbox, opts), nx = p.x1 - p.x0 + 1, ny = p.y1 - p.y0 + 1, mw = nx * 256, mh = ny * 256;
	const mosaic = new Float32Array(mw * mh);
	const jobs = [];
	for (let ty = 0; ty < ny; ty++) for (let tx = 0; tx < nx; tx++) {
		jobs.push(loadTile(p.z, p.x0 + tx, p.y0 + ty).then((rgba) => {
			for (let r = 0; r < 256; r++) for (let c = 0; c < 256; c++) {
				const i = (r * 256 + c) * 4;
				mosaic[(ty * 256 + r) * mw + tx * 256 + c] = terrariumHeight(rgba[i], rgba[i + 1], rgba[i + 2]);
			}
		}));
	}
	await Promise.all(jobs);
	const data = new Float32Array(p.width * p.height);
	const at = (c, r) => mosaic[Math.max(0, Math.min(mh - 1, r)) * mw + Math.max(0, Math.min(mw - 1, c))];
	for (let r = 0; r < p.height; r++) {
		const lat = p.north - (r + 0.5) * p.dy;
		const py = (tileY(lat, p.z) - p.y0) * 256 - 0.5;                                 // the pixel centres of the mosaic are at whole numbers
		const fy = Math.floor(py), ty = py - fy;
		for (let c = 0; c < p.width; c++) {
			const px = (tileX(p.west + (c + 0.5) * p.dx, p.z) - p.x0) * 256 - 0.5, fx = Math.floor(px), tx = px - fx;
			data[r * p.width + c] = (at(fx, fy) * (1 - tx) + at(fx + 1, fy) * tx) * (1 - ty) + (at(fx, fy + 1) * (1 - tx) + at(fx + 1, fy + 1) * tx) * ty;
		}
	}
	return { bytes: writeGeoTiff({ width: p.width, height: p.height, west: p.west, north: p.north, dx: p.dx, dy: p.dy, data }), plan: p };
}
