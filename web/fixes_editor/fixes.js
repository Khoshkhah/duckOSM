// The OSM fixes of the fixes editor (fixes_panel.js, docs/design/fixes_editor.md) as data. The editor keeps one `doc` (the parsed fixes file: duckOSM's
// `overrides`, `new_ways` and `turn_restrictions`, and the extension keys `extra.tag_edits` and `extra.building_faces`) and writes the YAML from it. It never
// edits the YAML text. `yaml` is js-yaml (the host page loads it). The Rust engine reads the text with rust/duckosm-build/src/config.rs, which rejects unknown keys.

export function parse(yaml, text) {
  const doc = text?.trim() ? yaml.load(text) : {};
  if (doc == null) return {};
  if (typeof doc !== "object" || Array.isArray(doc)) throw new Error("the file must be a map of keys");
  return doc;
}

// "" when the doc has no rule, so that the build runs with no fixes file
export function dump(yaml, doc) {
  const keep = Object.fromEntries(Object.entries(doc).filter(([, v]) => !(Array.isArray(v) && !v.length) && !(v && !Array.isArray(v) && typeof v === "object" && !Object.keys(v).length)));
  return Object.keys(keep).length ? yaml.dump(keep, { lineWidth: -1, noRefs: true }) : "";
}

export const tagEdits = (doc) => doc.extra?.tag_edits ?? [];
export const faceEdits = (doc) => doc.extra?.building_faces ?? [];
export const newWays = (doc) => doc.new_ways ?? [];
export const counts = (doc) => ({ overrides: (doc.overrides ?? []).length, turns: (doc.turn_restrictions ?? []).length, tagEdits: tagEdits(doc).length, faces: faceEdits(doc).length, newWays: newWays(doc).length });

export const findRule = (doc, id) => (doc.overrides ?? []).find((r) => r.osm_id === id) ?? null;

// the form owns every key except keep_nodes, which only the file can set: it stays
export function upsertRule(doc, rule) {
  const list = (doc.overrides ??= []), i = list.findIndex((r) => r.osm_id === rule.osm_id);
  if (i < 0) list.push(rule);
  else list[i] = { ...("keep_nodes" in list[i] ? { keep_nodes: list[i].keep_nodes } : {}), ...rule };
}

export function deleteRule(doc, id) {
  doc.overrides = (doc.overrides ?? []).filter((r) => r.osm_id !== id);
}

// ---- tag edits (`extra.tag_edits`): set and delete the tags of a way, a node or a relation
export const findTagEdit = (doc, type, id) => tagEdits(doc).find((e) => e.type === type && e.id === id) ?? null;

// an edit with nothing in it is removed, and an empty `extra` goes away (the file then has only duckOSM keys)
export function upsertTagEdit(doc, edit) {
  const list = tagEdits(doc).slice(), i = list.findIndex((e) => e.type === edit.type && e.id === edit.id);
  const set = edit.set ?? {}, del = edit.delete ?? [];
  const was = edit.was ?? {};
  const clean = { type: edit.type, id: edit.id, ...(Object.keys(set).length ? { set } : {}), ...(del.length ? { delete: del } : {}), ...(Object.keys(was).length ? { was } : {}), ...(edit.note ? { note: edit.note } : {}) };
  const empty = !clean.set && !clean.delete && !clean.was;
  if (i >= 0) empty ? list.splice(i, 1) : (list[i] = clean);
  else if (!empty) list.push(clean);
  if (list.length) (doc.extra ??= {}).tag_edits = list;
  else if (doc.extra) { delete doc.extra.tag_edits; if (!Object.keys(doc.extra).length) delete doc.extra; }
}

// ---- the type of a wall of a building (`extra.building_faces`): the build writes it in main.building_faces and keeps the computed type
const sameWall = (e, building, a, b) => e.building === building && ((e.from_node === a && e.to_node === b) || (e.from_node === b && e.to_node === a));
export const findFace = (doc, building, a, b) => faceEdits(doc).find((e) => sameWall(e, building, a, b)) ?? null;

// a rule that gives the computed type and no door is not a change: it is removed. `door`: the kind of door of the wall ("" or null: no door)
export function setFace(doc, building, a, b, type, computed, door = "") {
  const list = faceEdits(doc).filter((e) => !sameWall(e, building, a, b));
  if (type !== computed || door) list.push({ building, from_node: a, to_node: b, type, ...(door ? { door } : {}) });
  if (list.length) (doc.extra ??= {}).building_faces = list;
  else if (doc.extra) { delete doc.extra.building_faces; if (!Object.keys(doc.extra).length) delete doc.extra; }
}

// ---- new roads (duckOSM's `new_ways`): a way that OSM does not have. `nodes`: an OSM node id (a junction that the road joins) or [lon, lat]
export const NEW_WAY_MIN = 9_000_000_000_000;               // the lowest id of a new way (duckOSM NEW_WAY_ID_MIN)
export function addNewWay(doc, way) {
  const id = Math.max(NEW_WAY_MIN - 1, ...newWays(doc).map((w) => w.id)) + 1;
  (doc.new_ways ??= []).push({ id, nodes: way.nodes, tags: way.tags, ...(way.note ? { note: way.note } : {}) });
  return id;
}
export function updateNewWayTags(doc, id, tags) {
  const w = newWays(doc).find((x) => x.id === id);
  if (w) w.tags = tags;
}
export function deleteNewWay(doc, id) {
  doc.new_ways = newWays(doc).filter((w) => w.id !== id);
}

// ---- the rule of one object: what the person changed in it
// `mode`: the keys of an `overrides` rule that act for each mode (oneway, lanes, layer, exclude_modes, remove, turn lanes); `set` and `delete`: the tags.
// A way's tags from `overrides[].tags` count as set tags. Saving writes all tags to `extra.tag_edits` and takes them out of `overrides`.
export const MODE_KEYS = ["oneway", "lanes", "lanes_forward", "lanes_backward", "layer", "exclude_modes", "remove", "turn_lanes_forward", "turn_lanes_backward"];

export function ruleOf(doc, type, id) {
  const r = type === "way" ? findRule(doc, id) : null, e = findTagEdit(doc, type, id);
  const mode = {};
  for (const k of MODE_KEYS) if (r && r[k] != null) mode[k] = r[k];
  return { mode, set: { ...(r?.tags ?? {}), ...(e?.set ?? {}) }, delete: [...(e?.delete ?? [])], was: { ...(e?.was ?? {}) }, note: e?.note ?? r?.note ?? "" };
}

export const isEmptyRule = (rule) => !Object.keys(rule.mode).length && !Object.keys(rule.set).length && !rule.delete.length;

// `was` keeps what the data had before the change, for the keys that the rule still changes
const pruneWas = (rule) => Object.fromEntries(Object.entries(rule.was ?? {}).filter(([k]) => k in rule.mode || k in rule.set || rule.delete.includes(k)));

export function writeRule(doc, type, id, rule) {
  if (type === "way") {
    const old = findRule(doc, id), mode = { ...rule.mode };
    if (Object.keys(mode).length || (old && "keep_nodes" in old)) upsertRule(doc, { osm_id: id, ...mode, ...(rule.note ? { note: rule.note } : {}) });
    else deleteRule(doc, id);
  }
  upsertTagEdit(doc, { type, id, set: rule.set, delete: rule.delete, was: pruneWas(rule), note: type === "way" ? "" : rule.note });
}

export function deleteObjectRule(doc, type, id) {
  if (type === "way") deleteRule(doc, id);
  upsertTagEdit(doc, { type, id, set: {}, delete: [] });
}

// every object that has a rule: [{type, id}], the ways first, in file order
export function objects(doc) {
  const out = [], seen = new Set(), add = (type, id) => { const k = type + id; if (!seen.has(k)) { seen.add(k); out.push({ type, id }); } };
  for (const r of doc.overrides ?? []) add("way", r.osm_id);
  for (const e of tagEdits(doc)) add(e.type, e.id);
  return out;
}

// a short text of the rule, for the list of changes
// the fields of a change as rows for the list of changes: {k, was, now}; was is null when the data had no value, now is null when the change deletes the field
export function describeBits(doc, type, id) {
  const { mode, set, delete: del, was } = ruleOf(doc, type, id), bits = [];
  const from = (k) => (k in was ? (was[k] === null || was[k] === "" ? "" : String(was[k])) : null);
  for (const [k, v] of Object.entries(mode)) bits.push(k === "exclude_modes" ? { k: "not in", was: null, now: v.join(", ") } : k === "remove" ? { k: "object", was: null, now: "deleted from the data" } : { k, was: from(k), now: String(v) });
  for (const [k, v] of Object.entries(set)) bits.push({ k, was: from(k), now: String(v) });
  for (const k of del) bits.push({ k, was: from(k), now: null });
  return bits;
}
export function describe(doc, type, id) {
  const { mode, set, delete: del, was } = ruleOf(doc, type, id), bits = [];
  const from = (k) => (k in was ? ` (was ${was[k] === null || was[k] === "" ? "none" : was[k]})` : "");
  for (const [k, v] of Object.entries(mode)) bits.push(k === "exclude_modes" ? `not in ${v.join(", ")}` : k === "remove" ? "removed" : `${k} ${v}${from(k)}`);
  for (const [k, v] of Object.entries(set)) bits.push(`${k}=${v}${from(k)}`);
  for (const k of del) bits.push(`no ${k}${from(k)}`);
  return bits.join(", ");
}

// ---- turn rules
const same = (t, from, via, to) => t.from_way === from && t.via_node === via && t.to_way === to;

export const findTurn = (doc, from, via, to) => (doc.turn_restrictions ?? []).find((t) => same(t, from, via, to)) ?? null;

export function upsertTurn(doc, turn) {
  const list = (doc.turn_restrictions ??= []), i = list.findIndex((t) => same(t, turn.from_way, turn.via_node, turn.to_way));
  if (i < 0) list.push(turn);
  else list[i] = { ...list[i], ...turn };
}

export function deleteTurn(doc, from, via, to) {
  doc.turn_restrictions = (doc.turn_restrictions ?? []).filter((t) => !same(t, from, via, to));
}

// "a=b, c=d" <-> {a: "b", c: "d"} (the tags of a rule; the build wants the values as text)
export function tagsFromText(text) {
  const tags = {};
  for (const part of text.split(",")) {
    const eq = part.indexOf("="), k = part.slice(0, eq).trim(), v = part.slice(eq + 1).trim();
    if (eq > 0 && k && v) tags[k] = v;
  }
  return tags;
}
export const tagsToText = (tags) => Object.entries(tags ?? {}).map(([k, v]) => `${k}=${v}`).join(", ");
