// The logic of the build steps, with no DOM (the steps are in the other files of this folder). The units are tested by `node web/build_steps/steps_units.mjs`.

/** Text for HTML. */
export const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

const rad = (d) => (d * Math.PI) / 180;

/** The polygons of a geometry, as a list of polygons (each a list of rings). */
export const polygons = (g) => (g?.type === "Polygon" ? [g.coordinates] : g?.type === "MultiPolygon" ? g.coordinates : []);

/** A geometry (Polygon or MultiPolygon) from a list of polygons. */
export const geometry = (polys) => (polys.length === 1 ? { type: "Polygon", coordinates: polys[0] } : { type: "MultiPolygon", coordinates: polys });

/** The polygons of any GeoJSON (a geometry, a Feature, a FeatureCollection, a GeometryCollection). */
export function polygonsOf(json) {
  if (!json || typeof json !== "object") return [];
  if (json.type === "FeatureCollection") return (json.features ?? []).flatMap(polygonsOf);
  if (json.type === "Feature") return polygonsOf(json.geometry);
  if (json.type === "GeometryCollection") return (json.geometries ?? []).flatMap(polygonsOf);
  return polygons(json);
}

/** [west, south, east, north] of the outer rings. */
export function bbox(polys) {
  const pts = polys.flatMap((p) => p[0]);
  return [Math.min(...pts.map((p) => p[0])), Math.min(...pts.map((p) => p[1])), Math.max(...pts.map((p) => p[0])), Math.max(...pts.map((p) => p[1]))];
}

/** A box as a Polygon geometry. */
export const box = ([w, s, e, n]) => ({ type: "Polygon", coordinates: [[[w, s], [e, s], [e, n], [w, n], [w, s]]] });

function ringArea(r) {
  let a = 0;
  for (let i = 0; i + 1 < r.length; i++) a += rad(r[i + 1][0] - r[i][0]) * (2 + Math.sin(rad(r[i][1])) + Math.sin(rad(r[i + 1][1])));
  return Math.abs((a * 6378137 * 6378137) / 2);
}

/** The area in square kilometres (a sphere of radius 6378137 m; the holes are taken away). */
export const areaKm2 = (polys) => polys.reduce((s, p) => s + ringArea(p[0]) - p.slice(1).reduce((h, r) => h + ringArea(r), 0), 0) / 1e6;

/** Is the point inside? The even-odd rule over all rings of all polygons. */
export function inside(polys, x, y) {
  let c = false;
  for (const p of polys) for (const r of p)
    for (let i = 0, j = r.length - 1; i < r.length; j = i++)
      if (r[i][1] > y !== r[j][1] > y && x < ((r[j][0] - r[i][0]) * (y - r[i][1])) / (r[j][1] - r[i][1]) + r[i][0]) c = !c;
  return c;
}

/** The extracts of Geofabrik's list (`index-v1.json`, GeoJSON): [{id, name, pbf, bbox, polygon}]. A feature with no polygon or no pbf is left out. */
export function extractsOf(index) {
  return (index?.features ?? []).flatMap((f) => {
    const polygon = polygons(f.geometry), pbf = f.properties?.urls?.pbf;
    return polygon.length && pbf ? [{ id: f.properties.id, name: f.properties.name, parent: f.properties.parent, pbf, bbox: bbox(polygon), polygon }] : [];
  });
}

/** The extracts that contain all points of the outer rings (at most 2000 are tested), the smallest first. The polygons of the list are about
 *  1 km larger than the extracts, so an area at most 1 km outside can be found. */
export function covering(extracts, polys) {
  const pts = polys.flatMap((p) => p[0]);
  const step = Math.ceil(pts.length / 2000);
  const some = pts.filter((_, i) => i % step === 0);
  const [w, s, e, n] = bbox([[some]]);
  return extracts
    .filter((x) => x.bbox[0] <= w && x.bbox[1] <= s && x.bbox[2] >= e && x.bbox[3] >= n)
    .filter((x) => some.every((p) => inside(x.polygon, p[0], p[1])))
    .map((x) => ({ ...x, km2: areaKm2(x.polygon) }))
    .sort((a, b) => a.km2 - b.km2);
}

/** A folder name for an area: letters, digits, `_`, `-`; the rest becomes `_`. */
export const slug = (text) => String(text ?? "").normalize("NFKD").replace(/[̀-ͯ]/g, "").replace(/[^A-Za-z0-9_-]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 48) || "area";

/** A size in bytes as text. */
export const bytes = (n) => (n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(0)} KB` : n < 1073741824 ? `${(n / 1048576).toFixed(1)} MB` : `${(n / 1073741824).toFixed(2)} GB`);

/** The settings of the build step: the key of the config, its default, its name and its note. `at` is the section of the key (the top level when empty). */
export const OPTIONS = [
  { key: "build_features", at: "options", on: true, name: "Base-map layers", note: "Water, land use, buildings, railways and more (features.*)." },
  { key: "merge_segments", at: "options", on: true, name: "Join chains of edges", note: "One edge for a road that runs through simple junctions (writes edge_id_map)." },
  { key: "infer_separate_sidewalks", at: "options", on: true, name: "Infer separate sidewalks", note: "A road with a sidewalk line on both sides gets sidewalk=separate." },
  { key: "cycling_dismount", at: "options", on: true, name: "Dismount edges", note: "Cycling uses the footways that are walked." },
  { key: "bus_routes", at: "options", on: true, name: "Bus routes", note: "Match the OSM bus routes to the driving edges." },
  { key: "levels", at: "options", on: true, name: "Drawing order", note: "visualization.edge_levels: which road is on top at a bridge or a tunnel." },
  { key: "sea", at: "options", on: false, name: "Sea from Overture Maps", note: "Needs the internet. Off: the build stays offline." },
  { key: "keep_largest_component", at: "clip", on: true, name: "Keep the largest connected part", note: "Roads that do not connect to the main network are removed." },
];
export const MODES = ["driving", "walking", "cycling"];

/** The settings as the text of a config of `duckosm build` (the keys of docs/reference/configuration.md). `values`: {modes: [...], <key>: bool}; a key
 *  that is missing takes its default from OPTIONS. The place keys (name, output_path, source, boundary, elevation) are not here: the host adds them. */
export function settingsYaml(values = {}) {
  const modes = (values.modes ?? MODES).filter((m) => MODES.includes(m));
  if (!modes.length) throw new Error("choose at least one network");
  const sections = { options: [], clip: [] };
  for (const o of OPTIONS) {
    const v = values[o.key] ?? o.on;
    // `sea: overture` is the default of the build; `sea: false` is no sea
    sections[o.at].push(o.key === "sea" ? `  sea: ${v ? "overture" : "false"}` : `  ${o.key}: ${v}`);
  }
  // `fixes`: the path of a fixes file on this computer (the key osm_overrides of the config)
  const fixes = values.fixes ? `osm_overrides: ${JSON.stringify(String(values.fixes))}\n` : "";
  return `${fixes}modes: [${modes.join(", ")}]\noptions:\n${sections.options.join("\n")}\nclip:\n${sections.clip.join("\n")}\n`;
}

/** The text of one line of progress, for the state of the build job (`{state, steps, bytes, total, seconds}`). */
export function progressText(job) {
  const last = job.steps?.[job.steps.length - 1] ?? "";
  if (job.state === "downloading") return `Downloading ${bytes(job.bytes)}${job.total ? ` of ${bytes(job.total)}` : ""}`;
  if (job.state === "building") return `${last || "Starting"} (${Math.round(job.seconds)} s)`;
  return last;
}

/** The settings from the text of a config that `settingsYaml` wrote (the other keys are not read): { modes, <key>: bool }. A key that is missing keeps its default. */
export function parseSettings(text) {
	const values = {}, modes = /^modes:\s*\[([^\]]*)\]/m.exec(text ?? "");
	const fixes = /^osm_overrides:\s*"((?:[^"\\]|\\.)*)"/m.exec(text ?? "");
	if (fixes) values.fixes = JSON.parse(`"${fixes[1]}"`);
	if (modes) values.modes = modes[1].split(",").map((m) => m.trim()).filter((m) => MODES.includes(m));
	for (const o of OPTIONS) {
		const m = new RegExp(`^\\s+${o.key}:\\s*(\\S+)`, "m").exec(text ?? "");
		if (m) values[o.key] = o.key === "sea" ? m[1] !== "false" : m[1] === "true";
	}
	return values;
}
