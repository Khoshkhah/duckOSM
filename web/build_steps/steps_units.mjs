// Unit tests of the logic of the build steps (core.js): `node web/build_steps/steps_units.mjs`. They need only Node.
import assert from "node:assert/strict";
import { parseSettings, areaKm2, bbox, box, covering, extractsOf, geometry, OPTIONS, polygonsOf, progressText, settingsYaml, slug } from "./core.js";

const monaco = box([7.4, 43.72, 7.45, 43.76]);
const index = { type: "FeatureCollection", features: [
	{ type: "Feature", properties: { id: "europe", name: "Europe", urls: { pbf: "https://x/europe-latest.osm.pbf" } }, geometry: box([-30, 30, 50, 75]) },
	{ type: "Feature", properties: { id: "monaco", name: "Monaco", parent: "europe", urls: { pbf: "https://x/monaco-latest.osm.pbf" } }, geometry: monaco },
	{ type: "Feature", properties: { id: "norway", name: "Norway", urls: { pbf: "https://x/norway-latest.osm.pbf" } }, geometry: box([4, 57, 32, 72]) },
	{ type: "Feature", properties: { id: "nopbf", name: "No file" }, geometry: monaco },
	{ type: "Feature", properties: { id: "point", name: "Point", urls: { pbf: "https://x/p.osm.pbf" } }, geometry: { type: "Point", coordinates: [0, 0] } },
] };

// the extracts of the list: only the features with a polygon and a pbf
const all = extractsOf(index);
assert.deepEqual(all.map((x) => x.id), ["europe", "monaco", "norway"]);
assert.deepEqual(all[1].bbox, [7.4, 43.72, 7.45, 43.76]);

// an area inside Monaco: Monaco first (the smallest), then Europe; Norway is far away
const area = polygonsOf(box([7.41, 43.73, 7.44, 43.75]));
assert.deepEqual(covering(all, area).map((x) => x.id), ["monaco", "europe"]);
assert.ok(covering(all, area)[0].km2 < covering(all, area)[1].km2);
// an area that reaches out of Monaco: only Europe covers it
assert.deepEqual(covering(all, polygonsOf(box([7.3, 43.7, 7.5, 43.8]))).map((x) => x.id), ["europe"]);
// an area in no extract
assert.deepEqual(covering(all, polygonsOf(box([100, 0, 101, 1]))), []);

// the polygons of every GeoJSON shape; a point has none
const poly = box([0, 0, 1, 1]);
for (const j of [poly, { type: "Feature", geometry: poly }, { type: "FeatureCollection", features: [{ type: "Feature", geometry: poly }] }, { type: "GeometryCollection", geometries: [poly] }])
	assert.equal(polygonsOf(j).length, 1);
assert.deepEqual(polygonsOf({ type: "Point", coordinates: [0, 0] }), []);
assert.deepEqual(polygonsOf(null), []);
assert.equal(geometry(polygonsOf(poly)).type, "Polygon");
assert.equal(geometry([poly.coordinates, poly.coordinates]).type, "MultiPolygon");
assert.deepEqual(bbox(polygonsOf(poly)), [0, 0, 1, 1]);

// the area of one degree at the equator: about 12 364 km² on this sphere
assert.ok(Math.abs(areaKm2(polygonsOf(poly)) - 12364) < 30, areaKm2(polygonsOf(poly)));
// a hole is taken away
const holed = { type: "Polygon", coordinates: [poly.coordinates[0], box([0.25, 0.25, 0.75, 0.75]).coordinates[0]] };
assert.ok(Math.abs(areaKm2(polygonsOf(holed)) - 0.75 * areaKm2(polygonsOf(poly))) < 30);

// the name of a folder
assert.equal(slug("Södermalm, Stockholm"), "Sodermalm_Stockholm");
assert.equal(slug("  "), "area");
assert.equal(slug("a/b\\c.."), "a_b_c");
assert.equal(slug("x".repeat(80)).length, 48);

// the settings as the text of a config: the defaults, a change, and no network
const yaml = settingsYaml();
assert.ok(yaml.startsWith("modes: [driving, walking, cycling]\noptions:\n"), yaml);
for (const o of OPTIONS) assert.ok(yaml.includes(o.key), o.key);
assert.ok(yaml.includes("  sea: false\n") && yaml.includes("  build_features: true\n") && yaml.endsWith("clip:\n  keep_largest_component: true\n"), yaml);
const changed = settingsYaml({ modes: ["walking"], build_features: false, sea: true });
assert.ok(changed.startsWith("modes: [walking]\n") && changed.includes("  build_features: false\n") && changed.includes("  sea: overture\n"), changed);
assert.throws(() => settingsYaml({ modes: [] }), /at least one network/);
assert.ok(settingsYaml({ modes: ["driving", "flying"] }).startsWith("modes: [driving]\n"));

// the settings are read back from the text of a config (a saved area): the same values
assert.deepEqual(parseSettings(yaml), { modes: ["driving", "walking", "cycling"], ...Object.fromEntries(OPTIONS.map((o) => [o.key, o.on])) });
assert.deepEqual(parseSettings(changed), { modes: ["walking"], ...Object.fromEntries(OPTIONS.map((o) => [o.key, o.key === "build_features" ? false : o.key === "sea" ? true : o.on])) });
assert.deepEqual(parseSettings("name: x\n"), {});
assert.deepEqual(parseSettings(null), {});

// the line of progress
assert.equal(progressText({ state: "downloading", bytes: 1048576, total: 2097152 }), "Downloading 1.0 MB of 2.0 MB");
assert.equal(progressText({ state: "building", steps: ["[2/9] Loading PBF file"], seconds: 3.2 }), "[2/9] Loading PBF file (3 s)");
assert.equal(progressText({ state: "building", steps: [], seconds: 0 }), "Starting (0 s)");

console.log("build steps: all units passed");
