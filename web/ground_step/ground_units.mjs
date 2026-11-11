// The unit tests of ground_core.js: node web/ground_step/ground_units.mjs (the CI job `test` runs them).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { covers, gdalwarpCommand, inspectGeoTiff, sourcesFor, terrariumGeoTiff, terrariumHeight, terrariumPlan, writeGeoTiff } from "./ground_core.js";

const catalog = JSON.parse(readFileSync(new URL("../../src/duckosm/data/dem_sources.json", import.meta.url))).sources;
const ids = (bbox, o) => sourcesFor(catalog, bbox, o).map((s) => s.id);

// the same cases as tests/test_dem_sources.py
const MONACO = [7.41, 43.73, 7.44, 43.75], TARTU = [26.6, 58.3, 26.85, 58.45], STOCKHOLM = [17.2, 58.6, 19.0, 60.1];
assert.deepEqual(ids(MONACO).slice(0, 3), ["eudtm", "fabdem", "copernicus-glo30"]);
assert.ok(!ids(MONACO).includes("fr-ign-rgealti") && !ids(MONACO).includes("aws-terrain-tiles"));
assert.deepEqual(ids(TARTU).slice(0, 4), ["ee-maaamet-dtm", "eudtm", "fabdem", "copernicus-glo30"]);
assert.deepEqual(ids(STOCKHOLM).slice(0, 4), ["se-lantmateriet-1m", "eudtm", "fabdem", "copernicus-glo30"]);
assert.ok(!ids(STOCKHOLM).includes("srtm"));
assert.deepEqual(ids([-123.2, 49.2, -123.0, 49.3]).slice(0, 3), ["ca-nrcan-hrdem", "fabdem", "copernicus-glo30"]);
assert.deepEqual(ids([10, 85, 11, 86]), ["copernicus-glo30", "copernicus-glo90"]);
assert.ok(!ids([27, 58, 29, 59]).includes("ee-maaamet-dtm"));
assert.equal(ids(MONACO, { includeFallback: true }).at(-1), "aws-terrain-tiles");
assert.ok(ids(MONACO, { includeAll: true }).includes("gebco-etopo") && !ids(MONACO).includes("gebco-etopo"));
assert.ok(covers(catalog[0], [18, 59, 18.1, 59.1]));

// a GeoTIFF that the build reads: written, then inspected
const data = Float32Array.from({ length: 6 }, (_, i) => i * 1.5);
const tif = writeGeoTiff({ width: 3, height: 2, west: 7.4, north: 43.8, dx: 0.001, dy: 0.0008, data });
const info = inspectGeoTiff(tif);
assert.deepEqual(info.problems, []);
assert.equal(info.epsg, 4326); assert.equal(info.width, 3); assert.equal(info.height, 2);
assert.deepEqual(info.bounds.map((x) => +x.toFixed(6)), [7.4, 43.7984, 7.403, 43.8]);
assert.equal(new DataView(tif.buffer).getFloat32(tif.length - 4, true), 7.5);                     // the last pixel
// problems
assert.match(inspectGeoTiff(new Uint8Array(40)).problems[0], /not a GeoTIFF/);
const projected = Uint8Array.from(tif);
{   // change the GeoKey 2048 (4326) to 3006: find the key entry 2048, 0, 1, 4326 and make it 3072 (ProjectedCSType), 0, 1, 3006
	const dv = new DataView(projected.buffer);
	for (let i = 0; i + 8 < projected.length; i += 2) if (dv.getUint16(i, true) === 2048 && dv.getUint16(i + 2, true) === 0 && dv.getUint16(i + 6, true) === 4326) { dv.setUint16(i, 3072, true); dv.setUint16(i + 6, 3006, true); break; }
}
assert.match(inspectGeoTiff(projected).problems[0], /EPSG:3006/);
assert.match(gdalwarpCommand("a b.tif"), /gdalwarp -t_srs EPSG:4326 -ot Float32 "a b.tif"/);

// Terrarium
assert.equal(terrariumHeight(128, 0, 0), 0); assert.equal(terrariumHeight(128, 10, 128), 10.5);
const plan = terrariumPlan(MONACO);
assert.ok(plan.tiles <= 64 && plan.z === 14 && plan.resolution_m > 5 && plan.resolution_m < 15, JSON.stringify(plan));
const big = terrariumPlan(STOCKHOLM);
assert.ok(big.tiles <= 64 && big.z < 11 && big.width * big.height < 6e6, JSON.stringify(big));
// a synthetic world: the height is 1000 m for each degree of longitude east of 7 degrees, as a Terrarium PNG
const tile = (z, x, y) => {
	const rgba = new Uint8ClampedArray(256 * 256 * 4);
	for (let r = 0; r < 256; r++) for (let c = 0; c < 256; c++) {
		const lon = ((x + (c + 0.5) / 256) / 2 ** z) * 360 - 180, v = (lon - 7) * 1000 + 32768, i = (r * 256 + c) * 4;
		rgba[i] = Math.floor(v / 256); rgba[i + 1] = Math.floor(v % 256); rgba[i + 2] = Math.floor((v - Math.floor(v)) * 256); rgba[i + 3] = 255;
	}
	return Promise.resolve(rgba);
};
const { bytes, plan: p2 } = await terrariumGeoTiff(MONACO, tile);
const ti = inspectGeoTiff(bytes);
assert.deepEqual(ti.problems, []);
assert.equal(ti.width, p2.width);
const z = new Float32Array(bytes.buffer.slice(bytes.byteOffset + bytes.length - 4 * p2.width * p2.height));
for (const [c, r] of [[0, 0], [p2.width - 1, 3], [Math.floor(p2.width / 2), p2.height - 1]]) {
	const lon = p2.west + (c + 0.5) * p2.dx;
	assert.ok(Math.abs(z[r * p2.width + c] - (lon - 7) * 1000) < 0.5, `pixel ${c},${r}: ${z[r * p2.width + c]} against ${(lon - 7) * 1000}`);
}
console.log("ground_units: ok");
