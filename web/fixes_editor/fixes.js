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

// ---- two or more records of one object: a file that a person wrote, or two files that were joined. The build applies the records in file order,
// so for the same key the later record wins. The editor reads the join of the records (find*). A write puts one record in the place of the first
// one and removes the others, so an edit cannot go under a later record.
const joinAt = (list, same, one) => {
  const i = list.findIndex(same), out = list.filter((x) => !same(x));
  if (one) out.splice(i < 0 ? out.length : i, 0, one);
  return out;
};

// the overrides of one way, as the build applies them: each key in file order (the later wins), `remove` and `exclude_modes` of every rule, the tags key by key
export function joinOverrides(list) {
  if (!list.length) return null;
  const out = Object.assign({}, ...list), modes = [...new Set(list.flatMap((r) => r.exclude_modes ?? []))], tags = Object.assign({}, ...list.map((r) => r.tags ?? {}));
  if (list.some((r) => r.remove)) out.remove = true;
  if (modes.length) out.exclude_modes = modes;
  if (Object.keys(tags).length) out.tags = tags;
  return out;
}

// the tag edits of one object, as the build applies them: for each record, set the tags, then delete the keys. `was` is the value before the first change
export function joinTagEdits(list) {
  if (!list.length) return null;
  const now = new Map();
  for (const e of list) {
    for (const [k, v] of Object.entries(e.set ?? {})) now.set(k, v);
    for (const k of e.delete ?? []) now.set(k, null);
  }
  const set = Object.fromEntries([...now].filter(([, v]) => v !== null)), del = [...now].filter(([, v]) => v === null).map(([k]) => k);
  const was = Object.assign({}, ...list.map((e) => e.was ?? {}).reverse()), note = list.findLast((e) => e.note)?.note;
  return { type: list[0].type, id: list[0].id, ...(Object.keys(set).length ? { set } : {}), ...(del.length ? { delete: del } : {}), ...(Object.keys(was).length ? { was } : {}), ...(note ? { note } : {}) };
}

export const findRule = (doc, id) => joinOverrides((doc.overrides ?? []).filter((r) => r.osm_id === id));

// the form owns every key except keep_nodes, which only the file can set: it stays
export function upsertRule(doc, rule) {
  const old = findRule(doc, rule.osm_id);
  doc.overrides = joinAt(doc.overrides ?? [], (r) => r.osm_id === rule.osm_id, { ...(old && "keep_nodes" in old ? { keep_nodes: old.keep_nodes } : {}), ...rule });
}

export function deleteRule(doc, id) {
  doc.overrides = (doc.overrides ?? []).filter((r) => r.osm_id !== id);
}

// ---- tag edits (`extra.tag_edits`): set and delete the tags of a way, a node or a relation
export const findTagEdit = (doc, type, id) => joinTagEdits(tagEdits(doc).filter((e) => e.type === type && e.id === id));

// an edit with nothing in it is removed, and an empty `extra` goes away (the file then has only duckOSM keys)
export function upsertTagEdit(doc, edit) {
  const set = edit.set ?? {}, del = edit.delete ?? [];
  const was = edit.was ?? {};
  const clean = { type: edit.type, id: edit.id, ...(Object.keys(set).length ? { set } : {}), ...(del.length ? { delete: del } : {}), ...(Object.keys(was).length ? { was } : {}), ...(edit.note ? { note: edit.note } : {}) };
  const empty = !clean.set && !clean.delete && !clean.was;
  setExtra(doc, "tag_edits", joinAt(tagEdits(doc), (e) => e.type === edit.type && e.id === edit.id, empty ? null : clean));
}
// a list of `extra`: an empty list goes, and an empty `extra` goes too
function setExtra(doc, key, list) {
  if (list.length) (doc.extra ??= {})[key] = list;
  else if (doc.extra) { delete doc.extra[key]; if (!Object.keys(doc.extra).length) delete doc.extra; }
}

// ---- the type of a wall of a building (`extra.building_faces`): the build writes it in main.building_faces and keeps the computed type
const sameWall = (e, building, a, b) => e.building === building && ((e.from_node === a && e.to_node === b) || (e.from_node === b && e.to_node === a));
// the build keeps the last rule of a wall (all of it: a later rule with no door has no door)
export const findFace = (doc, building, a, b) => faceEdits(doc).findLast((e) => sameWall(e, building, a, b)) ?? null;

// a rule that gives the computed type and no door is not a change: it is removed. `door`: the kind of door of the wall ("" or null: no door)
export function setFace(doc, building, a, b, type, computed, door = "") {
  const one = type !== computed || door ? { building, from_node: a, to_node: b, type, ...(door ? { door } : {}) } : null;
  setExtra(doc, "building_faces", joinAt(faceEdits(doc), (e) => sameWall(e, building, a, b), one));
}
// the walls of a building that have a rule: [{from_node, to_node}], one for each wall, in file order
export function wallsOf(doc, building) {
  const out = [];
  for (const e of faceEdits(doc)) if (e.building === building && !out.some((w) => sameWall(e, building, w.from_node, w.to_node))) out.push({ from_node: e.from_node, to_node: e.to_node });
  return out;
}

// ---- new roads (duckOSM's `new_ways`): a way that OSM does not have. `nodes`: an OSM node id (a junction that the road joins) or [lon, lat]
export const NEW_WAY_MIN = 9_000_000_000_000;               // the lowest id of a new way (duckOSM NEW_WAY_ID_MIN)
export function addNewWay(doc, way) {
  const id = Math.max(NEW_WAY_MIN - 1, ...newWays(doc).map((w) => w.id)) + 1;
  (doc.new_ways ??= []).push({ id, nodes: way.nodes, tags: way.tags, ...(way.note ? { note: way.note } : {}) });
  return id;
}
// the build stops when two new roads have one id: the editor joins them, each key in file order (the later wins)
export function findNewWay(doc, id) {
  const list = newWays(doc).filter((w) => w.id === id);
  return list.length ? Object.assign({}, ...list) : null;
}
export function updateNewWayTags(doc, id, tags) {
  const w = findNewWay(doc, id);
  if (w) doc.new_ways = joinAt(newWays(doc), (x) => x.id === id, { ...w, tags });
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

// a turn has one rule in the form: the later rule of the file
export const findTurn = (doc, from, via, to) => (doc.turn_restrictions ?? []).findLast((t) => same(t, from, via, to)) ?? null;

export function upsertTurn(doc, turn) {
  const old = findTurn(doc, turn.from_way, turn.via_node, turn.to_way);
  doc.turn_restrictions = joinAt(doc.turn_restrictions ?? [], (t) => same(t, turn.from_way, turn.via_node, turn.to_way), { ...old, ...turn });
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

// ---- the records of one picked object in the file (the box "This road is in your changes"). `o`: {type, id} (a way, a node or a relation: its
// `overrides` and its tag edit are one record of the editor), {building, from_node, to_node} (a wall), {from_way, via_node, to_way} (a turn) or
// {new_way} (a new road). count: the number of records; join: one record in the place of all of them; remove: all of them go.
const kindOf = (o) => ("new_way" in o ? "new" : "building" in o ? "wall" : "from_way" in o ? "turn" : "object");
export function count(doc, o) {
  switch (kindOf(o)) {
    case "new": return newWays(doc).filter((w) => w.id === o.new_way).length;
    case "wall": return faceEdits(doc).filter((e) => sameWall(e, o.building, o.from_node, o.to_node)).length;
    case "turn": return (doc.turn_restrictions ?? []).filter((t) => same(t, o.from_way, o.via_node, o.to_way)).length;
    default: return Math.max(o.type === "way" ? (doc.overrides ?? []).filter((r) => r.osm_id === o.id).length : 0, tagEdits(doc).filter((e) => e.type === o.type && e.id === o.id).length);
  }
}
export function join(doc, o) {
  switch (kindOf(o)) {
    case "new": { const w = findNewWay(doc, o.new_way); if (w) doc.new_ways = joinAt(newWays(doc), (x) => x.id === o.new_way, w); return; }
    case "wall": { const f = findFace(doc, o.building, o.from_node, o.to_node); if (f) setExtra(doc, "building_faces", joinAt(faceEdits(doc), (e) => sameWall(e, o.building, o.from_node, o.to_node), f)); return; }
    case "turn": { const t = findTurn(doc, o.from_way, o.via_node, o.to_way); if (t) upsertTurn(doc, t); return; }
    default: {
      const r = o.type === "way" ? findRule(doc, o.id) : null, e = findTagEdit(doc, o.type, o.id);
      if (r) doc.overrides = joinAt(doc.overrides, (x) => x.osm_id === o.id, r);
      if (e) upsertTagEdit(doc, e);
    }
  }
}
export function remove(doc, o) {
  switch (kindOf(o)) {
    case "new": return deleteNewWay(doc, o.new_way);
    case "wall": return setExtra(doc, "building_faces", faceEdits(doc).filter((e) => !sameWall(e, o.building, o.from_node, o.to_node)));
    case "turn": return deleteTurn(doc, o.from_way, o.via_node, o.to_way);
    default: return deleteObjectRule(doc, o.type, o.id);
  }
}
// ---- the time of a change (docs/design/fixes_editor_time_merge.md): the key `changed` on each record, the local date and time with the offset
// from UTC. The build does not read it.
const LISTS = [["overrides"], ["new_ways"], ["turn_restrictions"], ["extra", "tag_edits"], ["extra", "building_faces"]];
const listAt = (doc, [a, b]) => (b ? doc[a]?.[b] : doc[a]) ?? [];
const two = (n) => String(n).padStart(2, "0");
export function localTime(d = new Date()) {
  const o = -d.getTimezoneOffset(), a = Math.abs(o);
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}T${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}${o < 0 ? "-" : "+"}${two(Math.floor(a / 60))}:${two(a % 60)}`;
}
// the time as a number (ms), NaN for no time. js-yaml makes a Date of a time that has no quotes (a file that a person wrote)
export const timeMs = (t) => (t instanceof Date ? t.getTime() : t == null ? NaN : Date.parse(t));
// a record with no `changed`, as a text with its keys sorted: two records with the same text are the same change
const keySorted = (_, v) => (v && typeof v === "object" && !Array.isArray(v) ? Object.fromEntries(Object.entries(v).sort()) : v);
const bare = (r) => { const { changed, ...rest } = r; return JSON.stringify(rest, keySorted); };
// after a change: a record that is new or different from the records of `before` gets the time `now`. A record that the change wrote again with
// the same content gets its old time back
export function stamp(before, doc, now) {
  for (const p of LISTS) {
    const old = new Map(listAt(before, p).map((r) => [bare(r), r.changed]));
    for (const r of listAt(doc, p)) {
      const k = bare(r);
      if (!old.has(k)) r.changed = now;
      else if (old.get(k) != null) r.changed = old.get(k);
    }
  }
}
// a record with no time gets the time `now` (a file that has no times yet); returns the number of records that got it
export function stampMissing(doc, now) {
  let n = 0;
  for (const p of LISTS) for (const r of listAt(doc, p)) if (r.changed == null) { r.changed = now; n++; }
  return n;
}
// the records of one object (`o` as for count) in all the lists
export function recordsOf(doc, o) {
  switch (kindOf(o)) {
    case "new": return newWays(doc).filter((w) => w.id === o.new_way);
    case "wall": return faceEdits(doc).filter((e) => sameWall(e, o.building, o.from_node, o.to_node));
    case "turn": return (doc.turn_restrictions ?? []).filter((t) => same(t, o.from_way, o.via_node, o.to_way));
    default: return [...(o.type === "way" ? (doc.overrides ?? []).filter((r) => r.osm_id === o.id) : []), ...tagEdits(doc).filter((e) => e.type === o.type && e.id === o.id)];
  }
}
// the latest time of the records (ms), NaN when no record has a time
export const latest = (recs) => recs.reduce((m, r) => { const t = timeMs(r.changed); return Number.isNaN(t) ? m : Number.isNaN(m) ? t : Math.max(m, t); }, NaN);

// ---- Add a file: the records of another fixes file go into `doc`
const FILE_KEYS = ["overrides", "new_ways", "turn_restrictions", "extra"], EXTRA_KEYS = ["tag_edits", "building_faces"];
// "" when `doc` can be a fixes file, else what is wrong
export function checkFile(doc) {
  const bad = Object.keys(doc).filter((k) => !FILE_KEYS.includes(k));
  if (bad.length) return `unknown key ${bad.join(", ")}`;
  const ex = doc.extra ?? {};
  if (typeof ex !== "object" || Array.isArray(ex)) return "extra must be a map";
  const badEx = Object.keys(ex).filter((k) => !EXTRA_KEYS.includes(k));
  if (badEx.length) return `unknown key extra.${badEx.join(", extra.")}`;
  for (const p of LISTS) { const v = p[1] ? ex[p[1]] : doc[p[0]]; if (v != null && !Array.isArray(v)) return `${p.join(".")} must be a list`; }
  for (const p of LISTS) if (listAt(doc, p).some((r) => !r || typeof r !== "object" || Array.isArray(r))) return `each item of ${p.join(".")} must be a map`;
  return "";
}
// the object of a record, as a text: the rule and the tag edit of a way are one object
const objKey = ([a, b], r) => (a === "overrides" ? `obj way ${r.osm_id}` : b === "tag_edits" ? `obj ${r.type} ${r.id}` : b === "building_faces"
  ? `wall ${r.building} ${Math.min(r.from_node, r.to_node)} ${Math.max(r.from_node, r.to_node)}` : a === "turn_restrictions" ? `turn ${r.from_way} ${r.via_node} ${r.to_way}` : `new ${r.id}`);
// merge(doc, other): for each object of `other`, its records go to the end of the lists of `doc`. The same object in the two files: the records
// with the later time stay, the others go. The same time, or no time on one side: the two stay, `other` last (the build: it wins key by key).
// A new road of `other` with the ID of another new road of `doc` gets the next free ID, also in the records of `other` that use it. `other` changes.
// Returns { added, replaced, older, both, renumbered }: the number of objects of each case.
export function merge(doc, other) {
  const out = { added: 0, replaced: 0, older: 0, both: 0, renumbered: 0 };
  const mine = new Map(newWays(doc).map((w) => [w.id, JSON.stringify(w.nodes)])), map = new Map();
  let next = Math.max(NEW_WAY_MIN - 1, ...newWays(doc).map((w) => w.id), ...newWays(other).map((w) => w.id));
  for (const w of newWays(other)) if (mine.has(w.id) && mine.get(w.id) !== JSON.stringify(w.nodes) && !map.has(w.id)) map.set(w.id, ++next);
  const to = (id) => map.get(id) ?? id;
  if (map.size) {
    out.renumbered = map.size;
    for (const w of newWays(other)) w.id = to(w.id);
    for (const t of other.turn_restrictions ?? []) { t.from_way = to(t.from_way); t.to_way = to(t.to_way); }
    for (const r of other.overrides ?? []) r.osm_id = to(r.osm_id);
    for (const e of tagEdits(other)) if (e.type === "way") e.id = to(e.id);
  }
  const group = (d) => { const g = new Map(); for (const p of LISTS) for (const r of listAt(d, p)) { const k = objKey(p, r); (g.get(k) ?? g.set(k, []).get(k)).push(r); } return g; };
  const ours = group(doc), theirs = group(other), drop = new Set(), take = new Set();
  for (const [k, recs] of theirs) {
    const have = ours.get(k);
    if (!have) { out.added++; recs.forEach((r) => take.add(r)); continue; }
    const a = latest(have), b = latest(recs);
    if (!Number.isNaN(a) && !Number.isNaN(b) && a !== b) {
      if (b > a) { out.replaced++; have.forEach((r) => drop.add(r)); recs.forEach((r) => take.add(r)); } else out.older++;
    } else { out.both++; recs.forEach((r) => take.add(r)); }
  }
  for (const p of LISTS) {
    const list = [...listAt(doc, p).filter((r) => !drop.has(r)), ...listAt(other, p).filter((r) => take.has(r))];
    if (p[1]) setExtra(doc, p[1], list); else doc[p[0]] = list;
  }
  return out;
}
