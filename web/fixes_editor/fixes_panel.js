// The fixes editor of duckOSM ("Edit OSM data", docs/design/fixes_editor.md, the user guide docs/guides/edit-osm-data.md). The rules are data (fixes.js:
// `doc`); this file holds the panel: six tabs: Road, Building and Node (one tag editor for the three, with its own selected object and draft in each tab; the
// fields are in the sections of presets.json), Turn (the turn rules), New road (draw a road) and Changes (the list, undo, and the one Build button). Each tab has
// an "Add to changes" button: nothing is in the rules before it. The map layers: an edited object is amber, the selected one is blue.
// initFixesPanel(root, ctx) adds the tabs and the body to `root` (the host page keeps its own header there) and returns what the host page calls: load(), text(),
// open(), close(), restore(), rebuilt(). The host page gives what it owns in `ctx` (README.md).
import * as F from "./fixes.js";

const AMBER = "#f59e0b", BLUE = "#2563eb";
const TURN_TYPES = { no_left_turn: "No left turn", no_right_turn: "No right turn", no_straight_on: "No straight on", no_u_turn: "No U-turn", no_entry: "No entry", no_exit: "No exit",
  only_left_turn: "Only left turn", only_right_turn: "Only right turn", only_straight_on: "Only straight on", only_u_turn: "Only U-turn" };
const KIND_OF = { road: "way", building: "way", node: "node" };      // what a click on the map picks, and the OSM type of it (a building can be a relation)
const KIND_NAME = { road: "Road", building: "Building", node: "Node", relation: "Relation" };
const EDIT = ["road", "building", "node"], TABS = [...EDIT, "turns", "new", "changes"];
const TAB_NAME = { road: "Road", building: "Building", node: "Node", turns: "Turn", new: "New road", changes: "Changes" };
const PANE = { road: "fix-edit", building: "fix-edit", node: "fix-edit", turns: "fix-turns", new: "fix-new", changes: "fix-changes" };
const HINTS = {
  road: ["Change the tags of a road: lanes, speed, sidewalk, surface and more.", "Click a road on the map, or type its OSM way ID."],
  building: ["Change the tags of a building: floors, height, roof and facade. Give each wall a type, and look at its doors.", "Click a building on the map, or type its OSM ID. The doors show as brown dots at zoom 17 or more."],
  node: ["A node is one point of OSM. Here it is a traffic signal, a crossing, or a junction of the roads. To add a traffic signal, pick the junction (a gray dot), set Kind to Traffic signals and press Add to changes. A change shows in the layer table features.traffic, not in the road edges.", "Click a dot on the map, or type the OSM node ID. The signals and crossings show at zoom 14 or more, and the junctions at zoom 16 or more."],
};
const TURN_OUT = "#16a34a";                  // the roads that leave the junction of the Turn tab
const DOOR_COLOR = "#92400e";                  // the doors of the Building tab: brown dots
const DOOR_KINDS = [["", "(no entrance tag)"], ["yes", "Entrance"], ["main", "Main entrance"], ["service", "Service entrance"], ["home", "Home entrance"], ["emergency", "Emergency exit"], ["exit", "Exit"], ["staircase", "Staircase"], ["garage", "Garage"]];
const DELETED = "#dc2626";                     // an object that the changes delete: red on the map and in the list
const NO_SIGNAL = new Set(["motorway", "motorway_link", "trunk", "trunk_link"]);
const CROSSABLE = new Set(["residential", "living_street", "service", "unclassified", "tertiary", "secondary", "primary"]);
const NODE_COLORS = { signal: "#e11d48", crossing: "#0d9488", other: "#6b7280", junction: "#9ca3af" };
const FACE_COLORS = { front: "#16a34a", back: "#dc2626", side: "#6b7280", party: "#7c3aed", court: "#0891b2" };
// why a wall has its computed type (the column type_reason of main.building_faces): a short note in the row for the walls that the method guessed or took from the data, and a tip for all
const REASONS = {
  entrance: ["entrance", "An entrance of OSM is on this wall."],
  address: ["address", "The wall faces the street of the addr:street tag of the building."],
  street: [null, "The wall faces a street that it can see."],
  corner: [null, "A second front wall: it faces another street that is about as near as the main front street, or it is on the same facade."],
  inferred: ["guess", "No street can be seen from the building (other buildings are in the way). The wall faces the nearest street within 150 m. Check it."],
  opposite: [null, "The wall points away from the front wall."],
  shared: [null, "Two buildings share this wall."],
  other: [null, "A wall of a building that has a front, and no street in front of it."],
  inner: [null, "A wall of a courtyard (an inner ring of a building that is a multipolygon relation)."],
  none: ["no street seen", "No street is near this building. Check the type."],
};
const FACE_NAMES = { front: "Front", back: "Back", side: "Side", party: "Party wall", court: "Courtyard wall" };

// the height and the color of a building from its tags, for the 3D preview of the edited buildings (the same reading as main.building_info in app/duck.js of duckosm-rust)
const NAMED = { white: "#ffffff", black: "#000000", red: "#ff0000", green: "#008000", blue: "#0000ff", yellow: "#ffff00", orange: "#ffa500", brown: "#a52a2a", grey: "#808080", gray: "#808080",
  silver: "#c0c0c0", beige: "#f5f5dc", pink: "#ffc0cb", purple: "#800080", cyan: "#00ffff", tan: "#d2b48c", maroon: "#800000", olive: "#808000", navy: "#000080", teal: "#008080",
  lightgray: "#d3d3d3", lightgrey: "#d3d3d3", darkgray: "#a9a9a9", darkgrey: "#a9a9a9", ivory: "#fffff0", khaki: "#f0e68c", salmon: "#fa8072", gold: "#ffd700", cream: "#fffdd0" };
export function cssColor(c) {
  c = String(c ?? "").trim().toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(c)) return c;
  if (/^#[0-9a-f]{3}$/.test(c)) return "#" + [...c.slice(1)].map((x) => x + x).join("");
  return NAMED[c] ?? null;
}
export function heightOf(tags) {
  const h = String(tags.height ?? "").trim(), m = /^(\d+(?:\.\d+)?)\s*(m|meter|meters|ft|feet|')?$/i.exec(h);
  if (m) return Number(m[1]) * (/^(ft|feet|')$/i.test(m[2] ?? "") ? 0.3048 : 1);
  const lv = /^\s*(\d+)/.exec(tags["building:levels"] ?? "");
  return lv ? Number(lv[1]) * 3.2 : 6;                         // 3.2 m for a floor; 6 m when the data has neither
}

// the tabs and the body of the panel; `note`: more text of the host page for the Changes tab (ctx.changesNote)
const MARKUP = (note) => String.raw`
<div class="fix-tabs" role="tablist" aria-label="What to fix">
  <button role="tab" id="fix-tab-road" data-tab="road" aria-controls="fix-edit" aria-selected="true" type="button">Road<i class="dot" hidden title="Not added to the changes yet"></i></button>
  <button role="tab" id="fix-tab-building" data-tab="building" aria-controls="fix-edit" aria-selected="false" type="button">Building<i class="dot" hidden title="Not added to the changes yet"></i></button>
  <button role="tab" id="fix-tab-node" data-tab="node" aria-controls="fix-edit" aria-selected="false" type="button">Node<i class="dot" hidden title="Not added to the changes yet"></i></button>
  <button role="tab" id="fix-tab-turns" data-tab="turns" aria-controls="fix-turns" aria-selected="false" type="button">Turn<i class="dot" hidden title="Not added to the changes yet"></i></button>
  <button role="tab" id="fix-tab-new" data-tab="new" aria-controls="fix-new" aria-selected="false" type="button">New road<i class="dot" hidden title="Not added to the changes yet"></i></button>
  <button role="tab" id="fix-tab-changes" data-tab="changes" aria-controls="fix-changes" aria-selected="false" type="button">Changes <span id="fix-n" class="badge"></span></button>
</div>
<div class="fix-body">
  <div id="fix-edit" class="fix-pane" role="tabpanel">
    <p id="fix-intro" class="why"></p>
    <section class="fsec"><h3>Pick</h3>
      <p id="fix-pickhint" class="why"></p>
      <p id="fix-doors-legend" class="small" hidden><span class="legend" style="margin-left:0"><i style="background:#92400e;border-radius:50%"></i>Door (an entrance node)</span><br><span id="fix-doors-info" class="muted"></span></p>
      <p id="fix-nodes-legend" class="small" hidden><span class="legend" style="margin-left:0"><i style="background:#e11d48"></i>Traffic signal</span><span class="legend"><i style="background:#0d9488"></i>Crossing</span><span class="legend"><i style="background:#6b7280"></i>Other</span><span class="legend"><i style="background:#9ca3af"></i>Junction</span><br><span id="fix-nodes-info" class="muted"></span></p>
      <div class="row"><input id="fix-id" type="text" inputmode="numeric" placeholder="OSM ID" aria-label="OSM ID" style="flex:1"><button id="fix-load" class="btn" type="button">Load</button></div>
      <label id="fix-walls-row" class="small walls-all" hidden><input type="checkbox" id="fix-walls-all">Show the walls of all buildings in view (zoom 16 or more)</label>
    </section>
    <section class="fsec" id="fix-objsec"><div class="fsec-hd"><h3>Selected</h3><button id="fix-unselect" class="btn sm" type="button" hidden>Unselect</button></div><div id="fix-obj" class="fix-obj"></div></section>
    <div id="fix-inchg" class="fix-inchg" role="group" aria-label="In your changes" hidden></div>
    <div id="fix-form"></div>
    <div class="fix-bar" id="fix-actions" hidden>
      <div class="barline"><span id="fix-state" class="small" role="status"></span><button type="button" class="link" data-review>Review changes →</button></div>
      <div class="row"><button id="fix-save" class="btn primary" type="button">Add to changes</button><button id="fix-revert" class="btn" type="button"><span class="long">Reset form</span><span class="short">Reset</span></button><button id="fix-del" class="btn" type="button"><span class="long">Remove from changes</span><span class="short">Remove</span></button></div>
    </div>
  </div>
  <div id="fix-turns" class="fix-pane" role="tabpanel" hidden>
    <p class="why">A turn rule says that a driver cannot make a turn (or must make it) at a junction. The rules are for the driving network.</p>
    <section class="fsec"><h3>Pick the road</h3>
      <p class="why">Click the road that the turn starts on, or type its way ID.</p>
      <div class="row"><input id="fix-from" type="text" inputmode="numeric" placeholder="From way ID" aria-label="From way ID" style="flex:1"><button id="fix-turn-find" class="btn" type="button">Find turns</button><button id="fix-turn-clear" class="btn" type="button" hidden>Unselect</button></div>
      <p id="fix-turn-legend" class="small" hidden><span class="legend" style="margin-left:0"><i style="background:#2563eb"></i>The road the turn starts on</span><span class="legend"><i style="background:#16a34a"></i>Roads that leave the junction</span><span class="legend"><i style="border:3px solid #2563eb;border-radius:50%;background:rgba(37,99,235,.25)"></i>The junction</span><br><span class="muted">Point at a road in the list to outline it on the map.</span></p>
      <div id="fix-via-row" hidden><p class="small" style="margin:10px 0 4px"><b>Junction at the end of the road.</b> Pick one to see the roads that leave it. The map shows the junction that you pick.</p><select id="fix-via" aria-label="Junction" style="width:100%"></select></div>
    </section>
    <div id="fix-turn-inchg" class="fix-inchg" role="group" aria-label="In your changes" hidden></div>
    <section class="fsec" id="fix-turnsec" hidden><h3>Rule for each road that leaves the junction</h3><div id="fix-out"></div></section>
    <div class="fix-bar" id="fix-turn-bar" hidden>
      <p id="fix-turn-note" class="small" role="status"></p>
      <div class="row"><button id="fix-turn-add" class="btn primary" type="button">Add to changes</button></div>
      <p class="small"><button type="button" class="link" data-review>Review the changes →</button></p>
    </div>
  </div>
  <div id="fix-new" class="fix-pane" role="tabpanel" hidden>
    <p class="why">Draw a road that OSM does not have. A click near a junction joins the road to it. The road is in the networks after you build again.</p>
    <div class="row" style="margin-top:0"><button id="fix-draw" class="btn primary" type="button">Draw a new road</button></div>
    <div id="fix-drawbox" hidden>
      <div id="fix-new-inchg" class="fix-inchg" role="group" aria-label="In your changes" hidden></div>
      <section class="fsec"><h3>Points</h3>
        <p class="why" id="fix-draw-help">Click the points of the road on the map. <b>Enter</b> finishes, <b>Esc</b> cancels.</p><p class="small" id="fix-draw-n"></p>
        <div class="row"><button id="fix-draw-undo" class="btn" type="button">Undo point</button><button id="fix-draw-cancel" class="btn" type="button">Cancel</button></div>
      </section>
      <section class="fsec"><h3>The road</h3><div id="fix-newform"></div></section>
      <div class="fix-bar">
        <div class="row"><button id="fix-draw-done" class="btn primary" type="button">Add to changes</button><button id="fix-new-del" class="btn" type="button" hidden>Remove from changes</button></div>
        <p class="small"><button type="button" class="link" data-review>Review the changes →</button></p>
      </div>
    </div>
  </div>
  <div id="fix-changes" class="fix-pane" role="tabpanel" hidden>
    <div id="fix-pending" class="fix-pending" hidden></div>
    <div class="row" style="margin-top:0"><button id="fix-undo" class="btn" type="button">Undo</button><button id="fix-redo" class="btn" type="button">Redo</button><button id="fix-download" class="btn" type="button">Download</button><button id="fix-clear" class="btn" type="button">Clear all</button></div>
    <div id="fix-list" class="fix-list"></div>
    <p class="why">A change shows in amber on the map. Click a change to see it and its data. Download saves the rules as a fixes file: <code>duckosm build --fixes</code> reads it. This browser keeps the changes of each area.${note}</p>
    <div class="fix-bar">
      <button id="fix-apply" class="btn primary" type="button">Build again<span id="fix-apply-n"></span></button>
      <p id="fix-apply-why" class="why" role="status"></p>
    </div>
  </div>
</div>`;

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;");

// the walls of the picked building have no number on the map, only the wall under the pointer (on the map or in the list of walls) and, with no
// pointer on a wall (a phone), the selected wall. The map filter of the number (fix-faces-label) and of the lit wall (fix-faces-hover, -glow).
export function wallShown(hover, selected) { return hover ?? selected ?? ""; }
export const wallFilter = (key) => ["==", ["coalesce", ["get", "k"], ""], key || "-"];

export async function initFixesPanel(root, ctx) {
  const { duck, map, status, yaml } = ctx;
  root.insertAdjacentHTML("beforeend", MARKUP(ctx.changesNote ? " " + ctx.changesNote : ""));
  const presets = await fetch(new URL("presets.json", import.meta.url)).then((r) => r.json());
  let doc = {}, hist = [], future = [];
  let sel = null, data = null, draft = {}, touched = new Set(), kind = "road", token = 0;   // the state of the tab `kind` (Road, Building or Node)
  let ask = false;                             // the picked object has records in the changes, and the box waits for Edit it or Delete it (the form waits)
  let current = "road";                        // the tab that shows
  const stash = {};                            // the state of the other two edit tabs: { sel, data, draft, touched, walls, ask }
  let walls = [];                              // the walls of the selected building (main.building_faces), with their lines
  let wallSel = null;                          // the selected wall of that building: "from_node|to_node"
  let wallHover = null;                        // the wall under the pointer, on the map or in the list of walls (the same key)
  let doors = [];                              // the doors of the selected building (main.building_entrances)
  const kinds = new Map();                     // "way123" -> "road" | "building": what an edited way is (the list of changes)
  const names = new Map();

  // ---- the data
  const snap = () => structuredClone(doc);
  function commit(change) { hist.push(snap()); if (hist.length > 100) hist.shift(); future = []; change(); persist(); sync(); }
  const rows = async (sql) => (await duck.query(sql)).toArray().map((r) => r.toJSON());
  const whole = (v) => { const n = Number(v); return Number.isSafeInteger(n) && n > 0 ? n : null; };
  const built = (mode, table) => duck.tables.some((t) => t.mode === mode && t.table === table);
  const modes = () => ["driving", "walking", "cycling"].filter((m) => built(m, "edges"));
  const hasTable = (schema, table) => built(schema, table);
  const tagMap = (r) => { const k = r?.k ? String(r.k).split("\x01") : [], v = r?.v ? String(r.v).split("\x01") : []; return Object.fromEntries(k.map((key, i) => [key, v[i] ?? ""])); };
  const TAGS = "array_to_string(map_keys(tags), chr(1)) AS k, array_to_string(map_values(tags), chr(1)) AS v";
  const safe = async (sql) => { try { return await rows(sql); } catch (e) { console.warn("fixes panel: a query failed:", e.message.split("\n")[0], "|", sql.slice(0, 90)); return []; } };

  // the data of one object: its tags (all of them when the build has raw.*), and what the networks have of it
  async function loadData(type, id, k) {
    const out = { type, id, kind: k, tags: {}, mode: {}, facts: [], found: false, lonlat: null };
    if (type === "node") {
      // the roads of the driving network at the node: the number of different neighbour nodes (a plain road has 2, a T junction 3) and the classes
      out.arms = 0; out.classes = [];
      if (built("driving", "edges")) {
        const nb = (await safe(`SELECT count(DISTINCT CASE WHEN source = ${id} THEN target ELSE source END)::INTEGER AS arms, string_agg(DISTINCT highway, ',') AS cls FROM driving.edges WHERE source = ${id} OR target = ${id}`))[0];
        out.arms = nb?.arms ?? 0; out.classes = nb?.cls ? String(nb.cls).split(",") : [];
      }
      const r = (await safe(`SELECT ST_X(geom) AS lon, ST_Y(geom) AS lat, ${TAGS} FROM features.traffic WHERE osm_id = ${id}`))[0];
      if (r) { out.found = true; out.tags = tagMap(r); out.lonlat = [r.lon, r.lat]; out.facts.push(`Node ${id}, at ${r.lon.toFixed(5)}, ${r.lat.toFixed(5)}`); return out; }
      for (const m of modes()) {                      // a node of the roads (a junction) with no traffic tag in the data
        const q = (await safe(`SELECT ST_X(geom) AS lon, ST_Y(geom) AS lat FROM ${m}.nodes WHERE node_id = ${id} AND node_id IN (SELECT node_id FROM ${m}.way_nodes)`))[0];
        if (!q) continue;
        const w = (await safe(`SELECT count(DISTINCT way_id)::INTEGER AS n FROM ${m}.way_nodes WHERE node_id = ${id}`))[0];
        out.found = out.junction = true; out.lonlat = [q.lon, q.lat];
        out.facts.push(`Node ${id}, at ${q.lon.toFixed(5)}, ${q.lat.toFixed(5)}`, `A node of the roads${w ? `: ${plural(w.n, "road")} meet${w.n === 1 ? "s" : ""} here` : ""}. It has no traffic tag in the data.`);
        break;
      }
      return out;
    }
    if (k === "building") {
      const r = (await safe(`SELECT osm_type, ${TAGS} FROM features.buildings WHERE osm_id = ${id} LIMIT 1`))[0];
      if (r) { out.found = true; out.tags = tagMap(r); out.type = r.osm_type === "relation" ? "relation" : "way"; out.facts.push(`Building ${out.type} ${id}`); }
      return out;
    }
    let raw = null;
    for (const m of modes()) { raw = (await safe(`SELECT ${TAGS} FROM ${m}.ways WHERE osm_id = ${id}`))[0]; if (raw) break; }      // <mode>.ways has all the tags of the ways that the mode kept
    if (raw) { out.found = true; out.tags = tagMap(raw); }
    for (const m of modes()) {
      const walk = m === "walking" ? ", any_value(walk_type) AS wt" : "";
      const e = (await safe(`SELECT any_value(name) AS name, any_value(highway) AS highway, any_value(lanes)::INTEGER AS lanes, bool_or(oneway) AS oneway, any_value(layer) AS layer,
        round(sum(length_m))::INTEGER AS m, count(*)::INTEGER AS n${walk} FROM ${m}.edges WHERE osm_id = ${id}`))[0];
      if (!e || !e.n) continue;
      out.found = true;
      if (!raw && !Object.keys(out.tags).length) out.tags = Object.fromEntries(Object.entries({ name: e.name, highway: e.highway, lanes: e.lanes, layer: e.layer }).filter(([, v]) => v != null).map(([k2, v]) => [k2, String(v)]));
      if (out.mode.oneway == null) Object.assign(out.mode, { oneway: String(!!e.oneway), lanes: e.lanes != null ? String(e.lanes) : "", layer: e.layer != null ? String(e.layer) : "" });
      out.facts.push(`${m}: ${e.n} edge${e.n === 1 ? "" : "s"}, ${e.m} m${e.wt ? `, walk type ${e.wt}` : ""}`);
    }
    return out;
  }

  // ---- the panel: what is selected
  // why a traffic signal or a crossing cannot go on this node ("" when it can): the node must be on a road of the driving network, a signal needs a
  // junction of at least 3 roads, a crossing needs a crossable road class, a motorway or a trunk road has neither, and a node has one kind
  function nodeRule(kind, d) {
    if (!d?.arms) return "This node is not on a road of the driving network.";
    if (d.classes.some((c) => NO_SIGNAL.has(c))) return "A motorway or a trunk road meets here: no signal and no crossing.";
    const cur = d.tags?.highway;
    if (kind === "stop" || kind === "give_way") return d.arms < 3 ? "A stop sign or a give-way sign needs a junction of at least 3 roads." : "";
    if (kind === "bus_stop") return "";
    if (kind === "traffic_signals") {
      if (cur === "crossing") return "This node is a crossing already.";
      if (d.arms < 3) return "A traffic signal needs a junction of at least 3 roads.";
    } else if (kind === "crossing") {
      if (cur === "traffic_signals") return "This node is a traffic signal already.";
      if (!d.classes.some((c) => CROSSABLE.has(c))) return "A crossing needs a road of the class residential, service, tertiary, secondary or primary here.";
    }
    return "";
  }
  let turnSel = null;                                // the road of the Turn list that is picked (its row has the mark and the map outlines it)
  const presetsOf = () => presets[sel?.kind ?? kind] ?? [];
  const plural = (n, w) => `${n} ${w}${n === 1 ? "" : "s"}`;
  const isRuleKey = (f) => !!f.rule;
  function initialDraft() {
    const rule = F.ruleOf(doc, sel.type, sel.id), d = {};
    for (const f of presetsOf()) {
      if (isRuleKey(f)) d[f.key] = rule.mode[f.key] != null ? String(rule.mode[f.key]) : data.mode[f.key] ?? data.tags[f.key.replace(/_/g, ":")] ?? "";
      else d[f.key] = f.key in rule.set ? String(rule.set[f.key]) : rule.delete.includes(f.key) ? "" : data.tags[f.key] ?? "";
    }
    for (const k of new Set([...Object.keys(data.tags), ...Object.keys(rule.set)])) if (!(k in d)) d[k] = k in rule.set ? String(rule.set[k]) : rule.delete.includes(k) ? "" : data.tags[k];
    for (const k of rule.delete) if (!(k in d)) d[k] = "";
    d.exclude_modes = rule.mode.exclude_modes ?? [];
    d.remove = !!rule.mode.remove;
    d.note = rule.note ?? "";
    return d;
  }

  const editedKey = (rule, f) => (isRuleKey(f) ? f.key in rule.mode : f.key in rule.set || rule.delete.includes(f.key));

  // ---- guards: one choice closes a field, or narrows the list of a field, for another (presets.json, `_guards`)
  const guardRules = presets._guards ?? {};
  const guardKeys = new Set(Object.values(guardRules).filter(Array.isArray).flat().flatMap((r) => Object.keys(r.if).map((k) => k.replace(/!$/, ""))));
  function guardsOf(k, values) {
    const off = new Map(), allow = new Map();
    for (const r of guardRules[k] ?? []) {
      const on = Object.entries(r.if).every(([key, vals]) => { const not = key.endsWith("!"), v = String(values[not ? key.slice(0, -1) : key] ?? ""); return not ? !vals.includes(v) : vals.includes(v); });
      if (!on) continue;
      for (const key of r.disable ?? []) if (!off.has(key)) off.set(key, r.reason);
      for (const [key, vals] of Object.entries(r.allow ?? {})) allow.set(key, { vals: allow.has(key) ? allow.get(key).vals.filter((x) => vals.includes(x)) : vals, reason: r.reason });
    }
    return { off, allow };
  }
  // the reason under a field that a guard closed or narrowed
  const guardNote = (f, g) => (g?.off.has(f.key) ? g.off.get(f.key) : g?.allow.has(f.key) && f.type === "choice" ? g.allow.get(f.key).reason : "");

  function control(f, value, g = null) {
    const id = `fixf-${f.key.replace(/[^a-z0-9]/gi, "_")}`;
    let el;
    if (f.type === "choice") {
      const allowed = g?.allow.get(f.key)?.vals;
      const opts = [["", "(not set)"], ...f.options.filter((o) => !allowed || allowed.includes(o[0]) || o[0] === value)];
      if (value !== "" && !opts.some((o) => o[0] === value)) opts.push([value, value]);
      el = `<select id="${id}" data-key="${esc(f.key)}">${opts.map((o) => `<option value="${esc(o[0])}"${o[0] === value ? " selected" : ""}>${esc(o[1])}</option>`).join("")}</select>`;
    } else if (f.type === "color") {
      const hex = /^#[0-9a-f]{6}$/i.test(value) ? value : "#999999";
      el = `<div class="colorrow"><input id="${id}" type="text" data-key="${esc(f.key)}" value="${esc(value)}" placeholder="#rrggbb or a name"><input type="color" data-for="${id}" value="${hex}" aria-label="${esc(f.label)}, pick a color"></div>`;
    } else {
      el = `<input id="${id}" type="text" data-key="${esc(f.key)}" value="${esc(value)}"${f.type === "int" ? ' inputmode="numeric"' : ""}${f.placeholder ? ` placeholder="${esc(f.placeholder)}"` : ""}>`;
    }
    return g?.off.has(f.key) ? el.replace(/<(select|input)/g, '<$1 disabled') : el;
  }

  // the number of fields that are not added to the changes yet, for each edit tab (the tab of the state in the variables, and the others in the stash)
  const dirty = (k) => (k === kind ? touched.size : stash[k]?.touched?.size ?? 0);
  function paintBar() {
    if (!sel || !data) return paintDots();
    const rule = F.ruleOf(doc, sel.type, sel.id), inChanges = !F.isEmptyRule(rule);
    $("fix-save").textContent = inChanges ? "Update change" : "Add to changes";
    $("fix-save").disabled = $("fix-revert").disabled = touched.size === 0;
    $("fix-del").disabled = !inChanges;
    $("fix-state").textContent = touched.size ? `${plural(touched.size, "change")} not added yet.` : inChanges ? "This object is in the changes." : "No change yet. Edit a field.";
    paintDots();
  }

  // the section Delete: a road or a building way is removed from the data (the rule `remove` of duckOSM: it is in no network and no layer after the next build), a building that is a
  // relation loses its `building` tag, and a signal or a crossing loses its tags. The delete is part of the draft until Add to changes.
  function deleteSection(rule) {
    if (!sel || !data) return "";
    const what = sel.kind === "road" ? "road" : sel.kind === "building" ? "building" : data.tags.highway === "traffic_signals" ? "traffic signal" : data.tags.highway === "crossing" ? "crossing" : null;
    if (!what) return "";
    const gone = deletedNow();
    const effect = what === "road" || what === "building" ? "It is in no network and no layer after the next build." : "It has no traffic tag after the next build.";
    return `<section class="fsec danger"><h3>Delete</h3>` + (gone && !touched.size
      ? `<p class="small" style="margin:0"><b>This ${what} is deleted in the changes.</b> ${effect} To take it back, use Remove from changes.</p>`
      : gone
      ? `<p class="small"><b>This ${what} will be deleted from the data.</b> ${effect} Press Add to changes to keep this.</p><button type="button" class="btn" id="fix-delete" data-undo="1">Undo the delete</button>`
      : `<p class="why" style="margin:0 0 8px">Delete this ${what} from the data. ${sel.kind === "node" ? "The node stays on its road, without its traffic tag." : "It is a change like the others: the data has it until you build again."}</p><button type="button" class="btn" id="fix-delete">Delete this ${what}</button>`) + `</section>`;
  }
  // whether the draft deletes the object: `remove` (a way), no `building` tag (a relation), no traffic tag (a node)
  function deletedNow() {
    if (!sel || !data) return false;
    if (sel.kind === "node") return !!data.tags.highway && draft.highway === "";
    if (sel.kind === "building" && sel.type === "relation") return !!data.tags.building && draft.building === "";
    return !!draft.remove;
  }
  function toggleDelete() {
    const undo = !!$("fix-delete")?.dataset.undo;
    if (sel.kind === "node") {
      for (const k of ["highway", "crossing"]) if (k in draft && (data.tags[k] || k === "highway")) { draft[k] = undo ? initialDraft()[k] : ""; touched.add(k); }
    } else if (sel.kind === "building" && sel.type === "relation") { draft.building = undo ? initialDraft().building : ""; touched.add("building"); }
    else { draft.remove = !undo; touched.add("remove"); }
    renderForm();
  }
  // ---- the box "This road is in your changes" (docs/design/fixes_editor.md, "The box of a picked object"). When a picked object has records in
  // the changes, the box shows over the form, and the form waits: Edit it opens the form with the values of the records (two or more records are
  // joined into one, in file order: for the same key the later wins, as in the build), Delete it removes all of them (Undo brings them back).
  const said = (v) => (/^-?\d+(\.\d+)?$|^(true|false)$/.test(v) ? v : `"${v}"`);
  const ruleText = (type, id) => F.describeBits(doc, type, id).map(({ k, now }) => (k === "object" ? "deleted" : k === "not in" ? `not in ${now}` : now === null ? `no ${k}` : `${k} ${said(now)}`));
  function askBox(el, what, bits, n, many = false) {
    el.hidden = false;
    el.innerHTML = (n > 1 ? `<p><b>This ${what} has ${n} rules in your changes.</b> Edit joins them into one: for the same key, the later rule wins, as in the build.</p><p class="small">${esc(bits || "(no change in it)")}</p>`
      : `<p><b>${many ? `These ${what}s are` : `This ${what} is`} in your changes:</b> ${esc(bits || "(no change in it)")}.</p>`)
      + `<div class="row"><button type="button" class="btn primary" data-ask="edit">Edit it</button><button type="button" class="btn" data-ask="delete">Delete it</button></div>`;
  }
  // the records of the picked object: the object's own (overrides and tag edit), and for a building the rules of its walls; null when it has none
  const wallRecs = () => (sel.kind === "building" ? F.wallsOf(doc, sel.id).map((w) => ({ building: sel.id, ...w })) : []);
  function editRecords() {
    const own = { type: sel.type, id: sel.id }, bits = F.count(doc, own) ? ruleText(sel.type, sel.id) : [];
    let n = F.count(doc, own);
    for (const w of wallRecs()) {
      const f = F.findFace(doc, sel.id, w.from_node, w.to_node), at = walls.find((x) => (x.from_node === w.from_node && x.to_node === w.to_node) || (x.from_node === w.to_node && x.to_node === w.from_node));
      n = Math.max(n, F.count(doc, w));
      bits.push(`wall ${at ? `#${at.face_index + 1}` : `${w.from_node}-${w.to_node}`} ${(FACE_NAMES[f.type] ?? f.type).toLowerCase()}${f.door ? `, door ${f.door}` : ""}`);
    }
    return n ? { n, text: bits.join(", ") } : null;
  }
  function paintAsk() {
    const r = ask && sel && data ? editRecords() : null;
    if (!r) ask = false;
    $("fix-edit").classList.toggle("asking", !!r);
    if (r) askBox($("fix-inchg"), sel.kind, r.text, r.n); else $("fix-inchg").hidden = true;
  }
  function answer(how) {
    if (!sel || !data) return;
    const all = [{ type: sel.type, id: sel.id }, ...wallRecs()], what = `${sel.kind} ${sel.id}`;
    if (how === "edit") { if (all.some((o) => F.count(doc, o) > 1)) commit(() => all.forEach((o) => F.join(doc, o))); }
    else { commit(() => all.forEach((o) => F.remove(doc, o))); status(`Deleted from the changes: ${what}. Undo brings it back.`); }
    ask = false; touched = new Set(); draft = initialDraft(); renderForm(); drawFaces(); drawSelected(false);
  }
  $("fix-inchg").addEventListener("click", (e) => { const b = e.target.closest("[data-ask]"); if (b) answer(b.dataset.ask); });

  function renderForm() {
    const box = $("fix-form");
    paintAsk();
    if (!sel || !data) { box.innerHTML = ""; $("fix-actions").hidden = true; return paintDots(); }
    const rule = F.ruleOf(doc, sel.type, sel.id), fields = presetsOf(), g = guardsOf(sel.kind, draft);
    const row = (f) => `<label for="fixf-${f.key.replace(/[^a-z0-9]/gi, "_")}" class="small${editedKey(rule, f) || touched.has(f.key) ? " edited" : ""}"${editedKey(rule, f) && f.key in rule.was ? ` title="Was: ${esc(rule.was[f.key] ?? "none")}"` : ""}>${esc(f.label)}${editedKey(rule, f) ? ` <button type="button" class="link" data-revert="${esc(f.key)}" title="Take this change back">↺</button>` : ""}</label>${control(f, draft[f.key] ?? "", g)}${guardNote(f, g) ? `<p class="why gwhy">${esc(guardNote(f, g))}</p>` : ""}`;
    const section = (title, inner) => `<section class="fsec"><h3>${title}</h3>${inner}</section>`;
    let html = [...new Set(fields.map((f) => f.group ?? "Fields"))].map((g) => section(esc(g), `<div class="grid2 fixgrid">${fields.filter((f) => (f.group ?? "Fields") === g).map(row).join("")}</div>`)).join("");
    if (sel.kind === "road") {
      html += section("Modes", `<div class="checks" role="group" aria-label="Exclude from"><span class="small">Not in</span>${["driving", "walking", "cycling"].map((m) => `<label><input type="checkbox" name="fix-ex" value="${m}"${draft.exclude_modes.includes(m) ? " checked" : ""}>${m[0].toUpperCase() + m.slice(1)}</label>`).join("")}</div>`);
    }
    if (sel.kind === "building" && walls.length) {
      html += section(`Walls <span class="muted">(${walls.length})</span>`, `<div class="wallbar"><button type="button" class="btn sm" data-wall-nav="-1">‹ Previous</button><button type="button" class="btn sm" data-wall-nav="1">Next ›</button><button type="button" class="btn sm" data-wall-nav="0" id="fix-wall-off">Unselect</button></div>
        <p class="small" id="fix-wallpos" style="margin:6px 0"></p>
        <p class="small" style="margin:0 0 6px">${Object.entries(FACE_COLORS).map(([t, c]) => `<span class="legend" style="margin-left:0;margin-right:8px"><i style="background:${c}"></i>${FACE_NAMES[t]}</span>`).join("")}</p>
        <div class="walls">${walls.map((w) => { const t = wallType(w), ruled = !!F.findFace(doc, sel.id, w.from_node, w.to_node) || touched.has(`@wall:${wallKey(w)}`) || touched.has(`@wdoor:${wallKey(w)}`);
          return `<div class="wallrow${ruled ? " edited" : ""}${wallKey(w) === wallSel ? " sel" : ""}" tabindex="0" data-key="${wallKey(w)}" data-a="${w.from_node}" data-b="${w.to_node}" data-computed="${w.computed_type}"><span class="k"><i class="sw" style="background:${FACE_COLORS[t] ?? FACE_COLORS.side}"></i>#${w.face_index + 1}</span><span class="small"${REASONS[w.type_reason] ? ` title="${esc(REASONS[w.type_reason][1])}"` : ""}>${Number(w.length_m).toFixed(1)} m${w.street_name ? ` · ${esc(w.street_name)}` : ""}${REASONS[w.type_reason]?.[0] ? ` · <i>${REASONS[w.type_reason][0]}</i>` : ""}</span>
          <select data-wall aria-label="Type of wall ${w.face_index + 1}">${Object.entries(FACE_NAMES).map(([k, v]) => `<option value="${k}"${k === t ? " selected" : ""}>${v}${k === w.computed_type ? " (computed)" : ""}</option>`).join("")}</select>
          <span class="small dlabel">Door</span><select data-walldoor aria-label="Door of wall ${w.face_index + 1}">${[["", "No door"], ...DOOR_KINDS.filter(([v]) => v)].map(([v, n]) => `<option value="${esc(v)}"${v === wallDoor(w) ? " selected" : ""}>${esc(n)}</option>`).join("")}</select></div>`; }).join("")}</div>
        <p class="why" style="margin:6px 0 0">Click a wall in the list or on the map to select it. A wall type and a door wait for Add to changes, like a field. A door belongs to the wall: it has no place on it.</p>`);
    } else if (sel.kind === "building" && !hasFaces()) {
      html += section("Walls", '<p class="why" style="margin:0">Build with the layers on to see the walls of a building and to give them a type.</p>');
    }
    if (sel.kind === "building" && sel.type === "way" && hasDoors()) {
      const addr = (d) => [d.street, d.housenumber].filter(Boolean).join(" ");
      html += section(`Doors <span class="muted">(${doors.length})</span>`, doors.length ? `<div class="doors">${doors.map((d, i) => {
        const orig = d.kind === "door" ? "" : d.kind, cur = doorKind(d), opts = DOOR_KINDS.some(([v]) => v === cur) ? DOOR_KINDS : [...DOOR_KINDS, [cur, cur]];
        return `<div class="doorrow${cur !== orig ? " edited" : ""}" data-id="${d.osm_id}" data-was="${esc(orig)}" tabindex="0"><span class="k">#${i + 1}</span><span class="small">${esc(addr(d) || d.name || `node ${d.osm_id}`)}${d.access ? ` · access ${esc(d.access)}` : ""}${d.level ? ` · level ${esc(d.level)}` : ""}${d.wheelchair ? ` · wheelchair ${esc(d.wheelchair)}` : ""}</span>
          <select data-door aria-label="Kind of door ${i + 1}">${opts.map(([v, n]) => `<option value="${esc(v)}"${v === cur ? " selected" : ""}>${esc(n)}</option>`).join("")}</select></div>`; }).join("")}</div>
        <p class="why" style="margin:6px 0 0">Click a door to zoom to it. A door kind waits for Add to changes, like a field. A new door needs a new node, which the fixes file cannot add yet.</p>`
        : '<p class="why" style="margin:0">No door is mapped on this building in OSM.</p>');
    }
    const preset = new Set(fields.map((f) => f.key));
    const other = Object.keys(draft).filter((k) => !preset.has(k) && k[0] !== "@" && !["exclude_modes", "remove", "note"].includes(k));
    html += section("Other tags", `<div class="other">${other.map((k) => `<div class="otherrow${k in rule.set || rule.delete.includes(k) ? " edited" : ""}"><span class="k" title="${esc(k)}">${esc(k)}</span><input type="text" data-key="${esc(k)}" value="${esc(draft[k])}" aria-label="${esc(k)}"><button type="button" class="btn sm" data-clear="${esc(k)}" title="Delete this tag" aria-label="Delete ${esc(k)}">×</button></div>`).join("") || '<span class="small muted">None.</span>'}
      <div class="otherrow add"><input type="text" id="fix-addkey" placeholder="key" aria-label="New tag, key"><input type="text" id="fix-addval" placeholder="value" aria-label="New tag, value"><button type="button" class="btn sm" id="fix-add">Add</button></div></div>
      <div class="grid2 fixgrid" style="margin-top:8px"><label for="fix-note" class="small">Note</label><input type="text" id="fix-note" value="${esc(draft.note)}" placeholder="Why"></div>`);
    html += deleteSection(rule);
    box.innerHTML = html;
    if (sel.kind === "node") {                       // the Kind field offers only what the node can have; the reason is under it
      const hw = $("fixf-highway"), no = [["traffic_signals", "Traffic signals"], ["crossing", "Crossing"], ["stop", "Stop"], ["give_way", "Give way"], ["bus_stop", "Bus stop"]].map(([k, n]) => [k, n, nodeRule(k, data)]).filter(([, , w]) => w);
      for (const [k] of no) { const o = hw?.querySelector(`option[value="${k}"]`); if (o && hw.value !== k) o.disabled = true; }
      if (hw && no.length) hw.closest(".fixgrid").insertAdjacentHTML("afterend", no.map(([, n, w]) => `<p class="why quickwhy" style="margin:4px 0 0">${n}: ${esc(w)}</p>`).join(""));
    }
    $("fix-actions").hidden = false;
    paintBar(); paintWalls();
  }

  // take the picked object out of the tab: the form, the draft and the blue on the map go (a field that is not added is dropped)
  function unselect() {
    if (!sel) return;
    const dropped = touched.size;
    ++token; sel = null; data = null; draft = {}; touched = new Set(); walls = []; wallSel = null; wallHover = null; doors = []; ask = false;
    $("fix-id").value = "";
    renderObject(); drawSelected(false); drawFaces(); clearMapSelection();
    status(dropped ? `Unselected. ${plural(dropped, "field")} that you did not add ${dropped === 1 ? "was" : "were"} dropped.` : "Unselected.");
  }

  function renderObject() {
    const box = $("fix-obj");
    $("fix-unselect").hidden = !sel;
    if (!sel) { box.innerHTML = `<p class="why" style="margin:0">Nothing is picked in this tab.</p>`; renderForm(); return; }
    if (!data) { box.innerHTML = '<p class="why" style="margin:0">Loading…</p>'; return; }
    const name = data.tags.name ? ` “${esc(data.tags.name)}”` : "";
    const head = `${KIND_NAME[sel.kind === "building" && sel.type === "relation" ? "relation" : sel.kind]} ${sel.type} ${sel.id}${name}`;
    const tagCount = Object.keys(data.tags).length;
    box.innerHTML = `<div class="objhead">${head}</div>${data.found ? "" : '<p class="why">Not found in this area. A rule for this ID still works if the ID is right.</p>'}${data.facts.map((f) => `<div class="small muted">${esc(f)}</div>`).join("")}
      <details class="alltags"><summary>All tags in the data (${tagCount})</summary><table>${Object.entries(data.tags).map(([k, v]) => `<tr><td>${esc(k)}</td><td>${esc(v)}</td></tr>`).join("") || '<tr><td colspan="2" class="muted">No tags (build with the layers on to see them all).</td></tr>'}</table></details>`;
  }

  async function select(k, type, id, { zoom = false, go = true } = {}) {
    if (!whole(id)) return;
    if (go) setTab(k);                                // saves the state of the tab that shows, and shows the tab of the object
    const again = sel?.kind === k && sel.id === id;   // the same object again (a build, Load): the box keeps the person's answer
    kind = k;
    const my = ++token;
    sel = { kind: k, type: type ?? KIND_OF[k], id }; data = null; draft = {}; touched = new Set(); wallSel = null; wallHover = null; doors = [];
    $("fix-id").value = id;
    renderObject();
    const d = await loadData(sel.type, id, k);
    if (my !== token) return;
    sel.type = d.type; data = d; draft = initialDraft();
    walls = k === "building" ? await loadWalls(id, d.type) : [];
    doors = k === "building" && d.type === "way" ? await loadDoors(id) : [];
    if (my !== token) return;
    if (!again) ask = !!editRecords();
    renderObject(); renderForm(); drawFaces();
    await drawSelected(zoom);
  }

  // the walls of a building: main.building_faces has them after a build with the layers (docs/osm-editing.md, M4)
  const hasFaces = () => built("main", "building_faces");
  // the doors of a building: main.building_entrances has them after a build with the layers (the nodes with `entrance` or `door` on its outline)
  const hasDoors = () => built("main", "building_entrances");
  async function loadDoors(id) {
    if (!hasDoors()) return [];
    return (await safe(`SELECT osm_id, kind, door, access, housenumber, street, level, wheelchair, name, ST_X(geom) AS lon, ST_Y(geom) AS lat FROM main.building_entrances WHERE building_id = ${id} ORDER BY osm_id`))
      .map((r) => ({ ...r, osm_id: Number(r.osm_id) }));
  }
  // the kind of a door now: the rule of the node on top of the data (a door with only a `door` tag has no entrance tag: "")
  const doorKindNow = (d) => { const r = F.ruleOf(doc, "node", d.osm_id); return "entrance" in r.set ? String(r.set.entrance) : r.delete.includes("entrance") ? "" : d.kind === "door" ? "" : d.kind; };
  // a wall type and a door kind wait in the draft like a field, under the keys "@wall:from|to" and "@door:id" (a tag key never starts with "@"), until Add to changes
  const doorKind = (d) => draft[`@door:${d.osm_id}`] ?? doorKindNow(d);
  function applyDoorKind(id, v, orig) {
    const rule = structuredClone(F.ruleOf(doc, "node", id));
    if (!("entrance" in rule.was) && !("entrance" in rule.set) && !rule.delete.includes("entrance")) rule.was.entrance = orig === "" ? null : orig;
    delete rule.set.entrance; rule.delete = rule.delete.filter((x) => x !== "entrance");
    if (v === orig) delete rule.was.entrance; else if (v === "") rule.delete.push("entrance"); else rule.set.entrance = v;
    F.writeRule(doc, "node", id, rule);
  }
  async function loadWalls(id, type = "way") {
    if (!hasFaces()) return [];
    // the id of a way and the id of a relation are two number spaces: a table of an older build has no `building_type` (all its rows are ways)
    const typed = (await safe("SELECT 1 FROM duckdb_columns() WHERE table_name = 'building_faces' AND column_name = 'building_type'")).length
      ? ` AND building_type = '${type === "relation" ? "relation" : "way"}'` : (type === "relation" ? " AND false" : "");
    return (await safe(`SELECT * EXCLUDE (geometry), ST_AsGeoJSON(geometry) AS g FROM main.building_faces WHERE building_id = ${id}${typed} ORDER BY face_index`))
      .map((r) => ({ ...r, from_node: Number(r.from_node), to_node: Number(r.to_node), face_index: Number(r.face_index) }));
  }
  const wallKey = (w) => `${w.from_node}|${w.to_node}`;
  const wallLine = (w) => (w._line ??= JSON.parse(w.g).coordinates);
  // the selected wall: its row has a mark, the bar tells which wall it is, and the map outlines it (the layer fix-faces-sel)
  function paintWalls() {
    for (const row of $("fix-form").querySelectorAll(".wallrow")) row.classList.toggle("sel", row.dataset.key === wallSel);
    const pos = $("fix-wallpos"), i = walls.findIndex((w) => wallKey(w) === wallSel);
    if (pos) {
      const w = walls[i];
      pos.textContent = w ? `Wall ${i + 1} of ${walls.length} · ${Number(w.length_m).toFixed(1)} m${w.street_name ? ` · faces ${w.street_name}` : ""} · ${FACE_NAMES[wallType(w)]}` : "No wall is selected.";
    }
    if ($("fix-wall-off")) $("fix-wall-off").disabled = i < 0;
  }
  // scroll a row of a list into view in the panel, above the bar that sticks to its bottom (scrollIntoView does not know the bar and leaves the row under it)
  function reveal(row) {
    const body = row?.closest(".fix-body");
    if (!body) return;
    const b = body.getBoundingClientRect(), r = row.getBoundingClientRect();
    const bars = [...body.querySelectorAll(".fix-bar")].filter((x) => x.offsetParent).map((x) => x.getBoundingClientRect().top);
    const bottom = Math.min(b.bottom, ...bars) - 8;
    if (r.top < b.top + 8) body.scrollTop += r.top - b.top - 8;
    else if (r.bottom > bottom) body.scrollTop += r.bottom - bottom;
  }
  // the wall under the pointer: the map lights it and shows its number, and its row in the list has the class "hover"
  function paintHover() {
    const k = wallShown(wallHover, wallSel);
    if (map.getLayer("fix-faces-label")) {
      for (const id of ["fix-faces-glow", "fix-faces-hover"]) map.setFilter(id, wallFilter(k));
      map.setFilter("fix-faces-label", ["all", ["has", "n"], wallFilter(k)]);
    }
    for (const row of $("fix-form").querySelectorAll(".wallrow")) row.classList.toggle("hover", row.dataset.key === wallHover);
  }
  function hoverWall(key) { if (key !== wallHover) { wallHover = key; paintHover(); } }
  function selectWall(key, { zoom = false } = {}) {
    wallSel = key; paintWalls(); drawFaces();
    reveal($("fix-form").querySelector(".wallrow.sel"));
    const w = walls.find((x) => wallKey(x) === key);
    if (zoom && w) zoomTo(wallLine(w), 20);
  }
  function stepWall(dir) {
    if (!walls.length) return;
    if (dir === 0) { wallSel = null; paintWalls(); drawFaces(); return; }
    const i = walls.findIndex((w) => wallKey(w) === wallSel);
    selectWall(wallKey(walls[i < 0 ? (dir > 0 ? 0 : walls.length - 1) : (i + dir + walls.length) % walls.length]), { zoom: true });
  }
  const wallTypeNow = (w) => F.findFace(doc, sel.id, w.from_node, w.to_node)?.type ?? w.face_type;
  // the door of a wall: the rule in the changes if there is one (no `door` in it: no door), else the table; the draft first
  const wallDoorNow = (w) => { const r = F.findFace(doc, sel.id, w.from_node, w.to_node); return r ? r.door ?? "" : w.door ?? ""; };
  const wallDoor = (w) => draft[`@wdoor:${wallKey(w)}`] ?? wallDoorNow(w);
  const wallType = (w) => draft[`@wall:${wallKey(w)}`] ?? wallTypeNow(w);


  // ---- edits in the form
  function onInput(e) {
    const el = e.target;
    if (el.dataset?.door !== undefined) {                                        // the kind of a door: a node edit at once
      const row = el.closest(".doorrow"), k = `@door:${row.dataset.id}`, d = doors.find((x) => x.osm_id === Number(row.dataset.id));
      if (el.value === doorKindNow(d)) { touched.delete(k); delete draft[k]; } else { touched.add(k); draft[k] = el.value; }
      row.classList.toggle("edited", el.value !== row.dataset.was);
      return paintBar();
    }
    if (el.dataset?.walldoor !== undefined) {                                    // the door of a wall: a draft until Add to changes
      const row = el.closest(".wallrow"), k = `@wdoor:${row.dataset.key}`, w = walls.find((x) => wallKey(x) === row.dataset.key);
      if (el.value === wallDoorNow(w)) { touched.delete(k); delete draft[k]; } else { touched.add(k); draft[k] = el.value; }
      row.classList.toggle("edited", touched.has(k) || touched.has(`@wall:${row.dataset.key}`) || !!F.findFace(doc, sel.id, w.from_node, w.to_node));
      return paintBar();
    }
    if (el.dataset?.wall !== undefined) {                                        // the type of a wall: a rule at once (a Save button is for tags)
      const row = el.closest(".wallrow"), k = `@wall:${row.dataset.key}`, w = walls.find((x) => wallKey(x) === row.dataset.key);
      if (el.value === wallTypeNow(w)) { touched.delete(k); delete draft[k]; } else { touched.add(k); draft[k] = el.value; }
      const sw = row.querySelector(".sw"); if (sw) sw.style.background = FACE_COLORS[el.value] ?? FACE_COLORS.side;
      row.classList.toggle("edited", touched.has(k) || touched.has(`@wdoor:${row.dataset.key}`) || !!F.findFace(doc, sel.id, w.from_node, w.to_node));
      return paintBar();
    }
    if (el.name === "fix-ex") { draft.exclude_modes = [...document.querySelectorAll("input[name=fix-ex]:checked")].map((c) => c.value); touched.add("exclude_modes"); return paintBar(); }
    if (el.id === "fix-note") { draft.note = el.value; touched.add("note"); return paintBar(); }
    if (el.dataset.for) { const t = $(el.dataset.for); t.value = el.value; draft[t.dataset.key] = el.value; touched.add(t.dataset.key); document.querySelector(`label[for="${t.id}"]`)?.classList.add("edited"); return paintBar(); }
    const k = el.dataset?.key;
    if (!k) return;
    draft[k] = el.value; touched.add(k);
    el.closest(".otherrow")?.classList.add("edited");
    if (el.id) document.querySelector(`label[for="${el.id}"]`)?.classList.add("edited");
    paintBar();
    if (e.type === "change" && guardKeys.has(k)) { const id = el.id; renderForm(); document.getElementById(id)?.focus(); }      // a choice that closes or narrows another field
  }

  function save() {
    if (!sel || !data) return status("Pick an object first.");
    if (sel.kind === "node" && touched.has("highway") && ["traffic_signals", "crossing"].includes(draft.highway) && draft.highway !== data.tags.highway) {
      const why = nodeRule(draft.highway, data);
      if (why) return status(why, 7000);
    }
    const gd = guardsOf(sel.kind, draft);
    for (const k of touched) {
      const f = presetsOf().find((x) => x.key === k), v = String(draft[k] ?? "");
      if (!f || v === "") continue;
      if (gd.off.has(k) && v !== (data.tags[k] ?? "")) return status(`${f.label}: ${gd.off.get(k)}`, 7000);
      const allow = gd.allow.get(k);
      if (allow && !allow.vals.includes(v) && v !== (data.tags[k] ?? "")) return status(`${f.label}: ${allow.reason}`, 7000);
    }
    const rule = structuredClone(F.ruleOf(doc, sel.type, sel.id)), fields = presetsOf(), ruleFields = new Map(fields.filter(isRuleKey).map((f) => [f.key, f]));
    const dataMode = data.mode ?? {};
    const specials = [...touched].filter((k) => k[0] === "@");                        // wall types and door kinds
    for (const k of touched) {
      if (k[0] === "@") continue;
      if (k === "note") { rule.note = (draft.note ?? "").trim(); continue; }
      if (k === "exclude_modes") { draft.exclude_modes.length ? (rule.mode.exclude_modes = draft.exclude_modes) : delete rule.mode.exclude_modes; continue; }
      if (k === "remove") { draft.remove ? (rule.mode.remove = true) : delete rule.mode.remove; continue; }
      const v = String(draft[k] ?? "").trim();
      // what the data had before the first change of this key: kept in the rule (`was`), to show "was to now"
      if (!(k in rule.was) && !(k in rule.mode) && !(k in rule.set) && !rule.delete.includes(k)) {
        const before = ruleFields.has(k) ? dataMode[k] : data.tags[k];
        rule.was[k] = before === undefined || before === "" ? null : before;
      }
      if (ruleFields.has(k)) {
        const f = ruleFields.get(k);
        if (f.type === "int" && v !== "" && !/^-?\d+$/.test(v)) return status(`${f.label} is a whole number.`);
        const conv = v === "" ? null : f.key === "oneway" ? v === "true" : f.type === "int" ? Number(v) : v;
        if (conv === null) delete rule.mode[k];
        else if (String(conv) === (dataMode[k] ?? "") && !(k in rule.mode)) continue;            // the same as the data: no change
        else rule.mode[k] = conv;
        continue;
      }
      const f = fields.find((x) => x.key === k);
      if (f?.type === "int" && v !== "" && !/^-?\d+$/.test(v)) return status(`${f.label} is a whole number.`);
      if (v === "") {
        delete rule.set[k];
        if (k in data.tags && !rule.delete.includes(k)) rule.delete.push(k);
      } else if (v === data.tags[k] && !(k in rule.set)) { /* the same as the data: no change */ }
      else { rule.set[k] = v; rule.delete = rule.delete.filter((x) => x !== k); }
    }
    commit(() => {
      F.writeRule(doc, sel.type, sel.id, rule);
      for (const key of new Set(specials.filter((k) => k[1] === "w").map((k) => k.slice(k.indexOf(":") + 1)))) {       // a wall: its type and its door in one rule
        const [a, b] = key.split("|").map(Number), w = walls.find((x) => wallKey(x) === key);
        F.setFace(doc, sel.id, a, b, draft[`@wall:${key}`] ?? wallTypeNow(w), w.computed_type, draft[`@wdoor:${key}`] ?? wallDoorNow(w));
      }
      for (const k of specials) {
        if (k.startsWith("@door:")) { const id = Number(k.slice(6)), d = doors.find((x) => x.osm_id === id); applyDoorKind(id, draft[k], d.kind === "door" ? "" : d.kind); }
      }
    });
    status(F.isEmptyRule(rule) && !specials.length ? "No change to add." : `Added to the changes: ${sel.type} ${sel.id}.`);
    touched = new Set();
    draft = initialDraft(); renderForm(); drawSelected(false);
  }

  function revertKey(k) {
    const rule = structuredClone(F.ruleOf(doc, sel.type, sel.id));
    delete rule.mode[k]; delete rule.set[k]; rule.delete = rule.delete.filter((x) => x !== k);
    commit(() => F.writeRule(doc, sel.type, sel.id, rule));
    touched.delete(k); draft = initialDraft(); renderForm();
    status("Taken back. The data has the original value after the next build.");
  }

  // ---- the tabs: Road, Building and Node share one editor, and each has its own object, form and draft (the stash holds the two that do not show)
  function switchKind(k) {
    stash[kind] = { sel, data, draft, touched, walls, wallSel, doors, ask };
    const st = stash[k] ?? { sel: null, data: null, draft: {}, touched: new Set(), walls: [], wallSel: null, doors: [], ask: false };
    kind = k; ({ sel, data, draft, touched, walls, wallSel = null, doors = [], ask = false } = st); ++token;
    $("fix-id").value = sel?.id ?? "";
    if (sel && !data) select(k, sel.type, sel.id, { go: false });       // an object that waited for its data (a build came between)
  }
  // nothing stays selected on the map from the tab that was left: roadstyle's road selection (the glow and the popup) and the hover marks go
  function clearMapSelection() {
    window.rsDeselect?.();
    document.querySelectorAll(".maplibregl-popup:not(.rs-tip)").forEach((p) => p.remove());
    leaveNode();
    turnSel = null; paintTurnSel();
  }
  function setTab(name, { zoom = false } = {}) {
    if (name !== current) clearMapSelection();
    if (EDIT.includes(name) && name !== kind) switchKind(name);
    current = name;
    for (const t of TABS) $(`fix-tab-${t}`).setAttribute("aria-selected", String(t === name));
    for (const id of new Set(Object.values(PANE))) $(id).hidden = id !== PANE[name];
    if (EDIT.includes(name)) {
      $("fix-intro").textContent = HINTS[name][0]; $("fix-pickhint").textContent = HINTS[name][1]; $("fix-walls-row").hidden = name !== "building"; $("fix-nodes-legend").hidden = name !== "node"; $("fix-doors-legend").hidden = name !== "building";
      renderObject(); renderForm();
    }
    if (name === "changes") renderChanges();
    if (name === "turns" && !$("fix-turnsec").hidden) outgoing();
    if (draw) map.getCanvas().style.cursor = name === "new" ? "crosshair" : "";
    paintDots();
    if (open()) { drawSelected(zoom, { turn: name === "turns" && !$("fix-turnsec").hidden ? { via: $("fix-via").value } : null }); drawFaces(); drawNodes(); drawTurnRoads(); drawDoors(); }
  }
  // an amber dot on the tab that has edits that are not in the changes yet
  function paintDots() {
    const on = (t, v) => { $(`fix-tab-${t}`).querySelector(".dot").hidden = !v; };
    for (const k of EDIT) on(k, dirty(k) > 0);
    on("turns", turnDiffs().length > 0);
    on("new", !!draw && draw.pts.length > 0);
  }
  function pendingList() {
    const out = [];
    for (const k of EDIT) if (dirty(k)) out.push({ tab: k, text: `${TAB_NAME[k]} tab: ${plural(dirty(k), "field")} not added.` });
    const t = turnDiffs().length;
    if (t) out.push({ tab: "turns", text: `Turn tab: ${plural(t, "rule")} not added.` });
    if (draw?.pts.length) out.push({ tab: "new", text: `New road tab: a road with ${plural(draw.pts.length, "point")} is not added.` });
    return out;
  }
  const total = () => { const { turns, faces, newWays } = F.counts(doc); return F.objects(doc).length + turns + faces + newWays; };
  // the one Build button: it waits while a tab has edits that are not in the changes
  function renderPending() {
    const list = pendingList(), n = total();
    $("fix-pending").hidden = !list.length;
    $("fix-pending").innerHTML = list.length ? `<b>Not added to the changes yet</b>${list.map((x) => `<div class="small">${esc(x.text)} <button type="button" class="link" data-go="${x.tab}">Open ${esc(TAB_NAME[x.tab])} →</button></div>`).join("")}` : "";
    $("fix-apply").disabled = !n || list.length > 0;
    $("fix-apply-why").textContent = !n ? "There are no changes to build." : list.length ? "Add the edits above to the changes, or reset them, first." : "";
  }

  // ---- the list of changes
  async function classify() {
    const ids = F.objects(doc).filter((o) => o.type === "way").map((o) => o.id).filter((id) => !kinds.has("way" + id));
    if (!ids.length) return;
    for (const id of ids) kinds.set("way" + id, "road");
    for (const r of await safe(`SELECT osm_id AS id FROM features.buildings WHERE osm_id IN (${ids.join(",")})`)) kinds.set("way" + r.id, "building");
    for (const m of modes()) for (const r of await safe(`SELECT osm_id AS id, any_value(name) AS n FROM ${m}.edges WHERE osm_id IN (${ids.join(",")}) GROUP BY osm_id`)) if (r.n) names.set("way" + r.id, r.n);
  }

  async function renderChanges() {
    await classify();
    const objs = F.objects(doc), turns = doc.turn_restrictions ?? [], faces = F.faceEdits(doc), roads = F.newWays(doc);
    const item = (attrs, label, inner, cls = "", n = 1) => `<div class="chg${cls}" ${attrs} tabindex="0" role="button"><div class="chgtop"><b>${label}${n > 1 ? ` <span class="nrules" title="This object has ${n} records in the file. Pick it, then Edit it joins them.">${n} rules</span>` : ""}</b><button type="button" class="btn sm" data-x aria-label="Delete this change">×</button></div>${inner}</div>`;
    // one item for each wall, turn and new road: the record that the build keeps (the last), and the number of records
    const uniq = (list, key) => { const m = new Map(); for (const x of list) { const k = key(x); m.set(k, { last: x, n: (m.get(k)?.n ?? 0) + 1 }); } return [...m.values()]; };
    const bits = (list) => list.length ? `<ul class="bits">${list.map(({ k, was, now }) => `<li><span class="k">${esc(k)}</span><span>${was === null ? "" : `<span class="was">${was === "" ? "none" : esc(was)}</span> → `}${now === null ? "(deleted)" : esc(now)}</span></li>`).join("")}</ul>` : '<div class="small">(no change in it)</div>';
    const note = (t) => `<div class="small">${esc(t)}</div>`;
    const group = (title, items) => (items.length ? `<div class="fix-group">${title} (${items.length})</div>${items.join("")}` : "");
    const by = { road: [], building: [], node: [], relation: [] };
    for (const o of objs) {
      const k = o.type === "way" ? kinds.get("way" + o.id) ?? "road" : o.type, nm = names.get(o.type + o.id);
      by[k]?.push(item(`data-type="${o.type}" data-id="${o.id}" data-kind="${k === "relation" ? "building" : k}"`, `${KIND_NAME[k] ?? k} ${o.id}${nm ? ` · ${esc(nm)}` : ""}`, bits(F.describeBits(doc, o.type, o.id)), F.ruleOf(doc, o.type, o.id).mode.remove ? " del" : "", F.count(doc, o)));
    }
    const newRoads = uniq(roads, (w) => w.id).map(({ last, n }) => { const w = F.findNewWay(doc, last.id); return item(`data-newway="${w.id}"`, `New road · ${w.id}`, note(`${w.tags?.highway ?? "road"}${w.tags?.name ? ` “${w.tags.name}”` : ""}, ${w.nodes.length} points${w.nodes.some(Number.isInteger) ? ", joins a junction" : ""}`), "", n); });
    const walls = uniq(faces, (f) => `${f.building}|${Math.min(f.from_node, f.to_node)}|${Math.max(f.from_node, f.to_node)}`).map(({ last: f, n }) => item(`data-face="${f.building}|${f.from_node}|${f.to_node}"`, `Wall · building ${f.building}`, note(`${FACE_NAMES[f.type] ?? f.type}${f.door ? `, door: ${f.door}` : ""}, between nodes ${f.from_node} and ${f.to_node}`), "", n));
    const turnItems = uniq(turns, (t) => `${t.from_way}|${t.via_node}|${t.to_way}`).map(({ last: t, n }) => item(`data-turn="${t.from_way}|${t.via_node}|${t.to_way}"`, `Turn · ${esc(TURN_TYPES[t.restriction] ?? t.restriction)}`, note(`way ${t.from_way}, at junction ${t.via_node}, to way ${t.to_way}`), "", n));
    const html = [group("Roads", by.road), group("Buildings", by.building), group("Walls", walls), group("Nodes", by.node), group("Other objects", by.relation), group("Turn rules", turnItems), group("New roads", newRoads)].join("");
    $("fix-list").innerHTML = html || '<p class="why" style="margin:0">No changes yet. Pick an object on the map, change a field and press <b>Add to changes</b>.</p>';
    $("fix-undo").disabled = !hist.length; $("fix-redo").disabled = !future.length;
    $("fix-clear").disabled = !total();
    renderPending();
  }

  $("fix-list").addEventListener("click", async (e) => {
    const c = e.target.closest(".chg");
    if (!c) return;
    if (e.target.closest("[data-x]")) {
      if (c.dataset.newway) { commit(() => F.deleteNewWay(doc, Number(c.dataset.newway))); return; }
      if (c.dataset.face) { const [bd, a, b] = c.dataset.face.split("|").map(Number); commit(() => F.remove(doc, { building: bd, from_node: a, to_node: b })); if (sel?.id === bd) { renderForm(); drawFaces(); } return; }
      if (c.dataset.turn) { const [a, b, d] = c.dataset.turn.split("|").map(Number); commit(() => F.deleteTurn(doc, a, b, d)); }
      else commit(() => F.deleteObjectRule(doc, c.dataset.type, Number(c.dataset.id)));
      if (sel && !c.dataset.turn && sel.type === c.dataset.type && sel.id === Number(c.dataset.id) && data) { draft = initialDraft(); renderForm(); }
      return;
    }
    if (c.dataset.newway) { setTab("new"); startEdit(Number(c.dataset.newway)); drawSelected(true); return; }
    if (c.dataset.face) { select("building", "way", Number(c.dataset.face.split("|")[0]), { zoom: true }); return; }
    if (c.dataset.turn) { const [a, b] = c.dataset.turn.split("|").map(Number); setTab("turns"); $("fix-from").value = a; findTurns(a, b, { zoom: true }); return; }
    select(c.dataset.kind, c.dataset.type, Number(c.dataset.id), { zoom: true });
  });
  $("fix-undo").onclick = () => { if (hist.length) { future.push(snap()); doc = hist.pop(); persist(); sync(); if (sel && data) { draft = initialDraft(); renderForm(); } } };
  $("fix-redo").onclick = () => { if (future.length) { hist.push(snap()); doc = future.pop(); persist(); sync(); if (sel && data) { draft = initialDraft(); renderForm(); } } };
  $("fix-clear").onclick = () => { commit(() => { doc = {}; }); if (sel && data) { draft = initialDraft(); renderForm(); } };
  $("fix-download").onclick = () => {
    const a = Object.assign(document.createElement("a"), { download: "osm_fixes.yaml", href: URL.createObjectURL(new Blob([text() ?? ""], { type: "text/yaml" })) });
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href));
  };

  // ---- the turns: the junctions at the end of a road, and a rule for each road that leaves one
  // the way that a turn goes, from the direction of the road at the end of the start road and the direction of the road that leaves: straight on within 30 degrees
  // of the line, a U-turn when it turns back (more than 150 degrees), else left or right. Only the rules for that way are offered (`no_left_turn` and `only_left_turn`
  // for a left turn); a rule that is in the changes already is offered too, whatever its way.
  const WAY_NAMES = { straight_on: "straight on", u_turn: "U-turn", right_turn: "right turn", left_turn: "left turn" };
  const turnWay = (a, b) => {
    if (!a || !b || [...Object.values(a), ...Object.values(b)].some((v) => v == null)) return null;
    const bearing = (p) => Math.atan2((p.x2 - p.x1) * Math.cos((p.y1 * Math.PI) / 180), p.y2 - p.y1) * 180 / Math.PI;
    const t = ((bearing(b) - bearing(a) + 540) % 360) - 180;                 // from -180 to 180: a positive angle is a turn to the right
    return Math.abs(t) <= 30 ? "straight_on" : Math.abs(t) > 150 ? "u_turn" : t > 0 ? "right_turn" : "left_turn";
  };
  const turnSelect = (cur, way) => {
    const keys = way ? [`no_${way}`, `only_${way}`] : Object.keys(TURN_TYPES).filter((k) => !/entry|exit/.test(k));
    if (cur && !keys.includes(cur)) keys.push(cur);
    return `<select aria-label="Turn rule"><option value="">Allowed</option>${keys.map((k) => `<option value="${k}"${cur === k ? " selected" : ""}>${TURN_TYPES[k]}</option>`).join("")}</select>`;
  };
  let turnFrom = null;
  const turnOk = new Set();                          // the junctions ("from|via") of the picked road where the person answered the box
  async function findTurns(from, via = null, { zoom = false } = {}) {
    if (from !== turnFrom) { turnFrom = from; turnOk.clear(); }
    $("fix-from").value = from;
    // the junctions at the ends of the edges of the road, the one with the most roads that leave it first
    const js = await safe(`SELECT t.n::VARCHAR AS n, count(DISTINCT o.osm_id)::INTEGER AS c FROM (SELECT DISTINCT target AS n FROM driving.edges WHERE osm_id = ${from}) t
      LEFT JOIN driving.edges o ON o.source = t.n AND o.osm_id != ${from} GROUP BY t.n ORDER BY c DESC, t.n`);
    const nodes = js.map((r) => String(r.n));
    $("fix-via-row").hidden = $("fix-turnsec").hidden = $("fix-turn-bar").hidden = !nodes.length;
    $("fix-turn-clear").hidden = false; $("fix-turn-legend").hidden = !nodes.length;
    // a dropdown list: one option for each junction
    $("fix-via").innerHTML = js.map((r) => `<option value="${r.n}">Junction ${r.n}: ${r.c ? `${plural(r.c, "road")} ${r.c === 1 ? "leaves" : "leave"} it` : "no road leaves it"}</option>`).join("");
    $("fix-via").value = nodes.includes(String(via)) ? String(via) : nodes[0] ?? "";       // the junction that the tab picks first: the one at the end of the road with the most roads that leave it
    $("fix-out").innerHTML = nodes.length ? "" : `<p class="why">Way ${from} is not in the driving network.</p>`;
    if (nodes.length) await outgoing();
    drawSelected(zoom, { turn: nodes.length ? { via: $("fix-via").value } : null });
    if (!nodes.length) { turnOut = []; drawTurnRoads(); }
  }
  function paintTurnSel() {
    for (const row of $("fix-out").querySelectorAll(".turn")) row.classList.toggle("sel", Number(row.dataset.to) === turnSel);
    if (map.getLayer("fix-turn-hl")) map.setFilter("fix-turn-hl", ["==", ["get", "id"], turnSel ?? -1]);
  }
  async function outgoing() {
    const from = whole($("fix-from").value), via = Number($("fix-via").value);
    if (!from || !Number.isSafeInteger(via)) return;
    const roads = await safe(`SELECT osm_id, any_value(name) AS name, any_value(highway) AS highway, any_value(ST_X(ST_PointN(geometry, 1))) AS x1, any_value(ST_Y(ST_PointN(geometry, 1))) AS y1,
      any_value(ST_X(ST_PointN(geometry, 2))) AS x2, any_value(ST_Y(ST_PointN(geometry, 2))) AS y2 FROM driving.edges WHERE source = ${via} AND osm_id != ${from} GROUP BY osm_id ORDER BY osm_id LIMIT 30`);
    const inEdge = (await safe(`SELECT ST_X(ST_PointN(geometry, (ST_NPoints(geometry) - 1)::INTEGER)) AS x1, ST_Y(ST_PointN(geometry, (ST_NPoints(geometry) - 1)::INTEGER)) AS y1, ST_X(ST_PointN(geometry, ST_NPoints(geometry)::INTEGER)) AS x2,
      ST_Y(ST_PointN(geometry, ST_NPoints(geometry)::INTEGER)) AS y2 FROM driving.edges WHERE osm_id = ${from} AND target = ${via} LIMIT 1`))[0];
    turnOut = roads.map((r) => Number(r.osm_id)); drawTurnRoads();
    const arrive = await safe(`SELECT osm_id, any_value(name) AS name, any_value(highway) AS highway, bool_or(oneway) AS oneway FROM driving.edges WHERE target = ${via} AND osm_id != ${from} GROUP BY osm_id ORDER BY osm_id LIMIT 12`);
    const arrivals = arrive.length ? `<p class="small" style="margin:12px 0 4px"><b>Roads that arrive at this junction.</b> Start from one of them to give a rule for the turn onto a road that leaves.</p>${arrive.map((r) => `<div class="small"><button type="button" class="link" data-from="${r.osm_id}">${esc(r.name || "no name")}</button> (${esc(r.highway)}, way ${r.osm_id}${r.oneway ? ", one-way" : ""})</div>`).join("")}` : "";
    const none = '<p class="why">No other road leaves this junction from this road. A road leaves it only when it is in the driving network and its direction goes away from the junction.</p>';
    $("fix-out").innerHTML = (roads.length ? "" : none) + arrivals;
    if (roads.length) $("fix-out").insertAdjacentHTML("afterbegin", roads.map((r) => `<div class="turn" data-to="${r.osm_id}"><span class="small"><i class="sw" style="background:#16a34a"></i>To <b>${esc(r.name || "no name")}</b> (${esc(r.highway)}, way ${r.osm_id})${turnWay(inEdge, r) ? ` · <i>${WAY_NAMES[turnWay(inEdge, r)]}</i>` : ""}</span>${turnSelect(F.findTurn(doc, from, via, Number(r.osm_id))?.restriction, turnWay(inEdge, r))}</div>`).join(""));
    for (const row of $("fix-out").querySelectorAll(".turn")) { row.tabIndex = 0; row.querySelector("select").onchange = paintTurnBar; }
    if (!roads.some((r) => Number(r.osm_id) === turnSel)) turnSel = null;
    turnNames = new Map(roads.map((r) => [Number(r.osm_id), r.name]));
    paintTurnSel();
    paintTurnBar();
    paintTurnAsk();
  }
  // the box over the list when the road has turn rules at this junction in the changes: the list waits for Edit it or Delete it
  let turnNames = new Map();
  const turnsHere = () => {
    const from = whole($("fix-from").value), via = Number($("fix-via").value);
    return from && Number.isSafeInteger(via) ? [...new Set((doc.turn_restrictions ?? []).filter((t) => t.from_way === from && t.via_node === via).map((t) => t.to_way))].map((to) => ({ from_way: from, via_node: via, to_way: to })) : [];
  };
  function paintTurnAsk() {
    const list = turnsHere(), waits = list.length > 0 && !turnOk.has(`${$("fix-from").value}|${$("fix-via").value}`) && !$("fix-turnsec").hidden;
    $("fix-turns").classList.toggle("asking", waits);
    if (!waits) { $("fix-turn-inchg").hidden = true; return; }
    const bits = list.map((t) => { const r = F.findTurn(doc, t.from_way, t.via_node, t.to_way).restriction, nm = turnNames.get(t.to_way); return `${(TURN_TYPES[r] ?? r).toLowerCase()} to ${nm ? `“${nm}” (way ${t.to_way})` : `way ${t.to_way}`}`; });
    askBox($("fix-turn-inchg"), "turn", bits.join(", "), Math.max(...list.map((t) => F.count(doc, t))), list.length > 1);
  }
  $("fix-turn-inchg").addEventListener("click", (e) => {
    const b = e.target.closest("[data-ask]"), list = turnsHere();
    if (!b) return;
    if (b.dataset.ask === "edit") { if (list.some((t) => F.count(doc, t) > 1)) commit(() => list.forEach((t) => F.join(doc, t))); }
    else { commit(() => list.forEach((t) => F.remove(doc, t))); status(`Deleted from the changes: ${plural(list.length, "turn rule")} of way ${list[0].from_way}. Undo brings ${list.length === 1 ? "it" : "them"} back.`); }
    turnOk.add(`${$("fix-from").value}|${$("fix-via").value}`);
    outgoing();
  });
  // the rules that the person chose in the list and did not add yet
  function turnDiffs() {
    const from = whole($("fix-from").value), via = Number($("fix-via").value), out = [];
    if (!from || !Number.isSafeInteger(via)) return out;
    for (const row of $("fix-out").querySelectorAll(".turn")) {
      const to = Number(row.dataset.to), want = row.querySelector("select").value, have = F.findTurn(doc, from, via, to)?.restriction ?? "";
      if (want !== have) out.push({ from, via, to, want });
    }
    return out;
  }
  function paintTurnBar() {
    const n = turnDiffs().length;
    $("fix-turn-add").disabled = !n;
    $("fix-turn-note").textContent = n ? `${plural(n, "rule")} not added yet.` : "";
    paintDots();
  }
  $("fix-turn-add").onclick = () => {
    const d = turnDiffs();
    if (!d.length) return;
    commit(() => { for (const x of d) x.want ? F.upsertTurn(doc, { from_way: x.from, via_node: x.via, to_way: x.to, restriction: x.want }) : F.deleteTurn(doc, x.from, x.via, x.to); });
    status(`Added to the changes: ${plural(d.length, "turn rule")}.`);
  };
  function clearTurns() {
    $("fix-from").value = ""; $("fix-via").innerHTML = ""; turnSel = null; $("fix-out").innerHTML = ""; turnFrom = null; turnOk.clear(); paintTurnAsk();
    $("fix-via-row").hidden = $("fix-turnsec").hidden = $("fix-turn-bar").hidden = $("fix-turn-clear").hidden = $("fix-turn-legend").hidden = true;
    turnOut = []; drawTurnRoads(); paintTurnBar(); drawSelected(false);
  }
  $("fix-turn-clear").onclick = clearTurns;
  $("fix-out").addEventListener("click", (e) => { const b = e.target.closest("[data-from]"); if (b) findTurns(Number(b.dataset.from), Number($("fix-via").value), { zoom: true }); });
  $("fix-turn-find").onclick = () => { const id = whole($("fix-from").value); id ? findTurns(id, null, { zoom: true }) : status("Type a way ID (a whole number)."); };
  $("fix-via").onchange = () => { turnSel = null; outgoing(); drawSelected(false, { turn: { via: $("fix-via").value } }); };

  // ---- the map: edited objects are amber, the selected object is blue
  const EMPTY = { type: "FeatureCollection", features: [] };
  function ensureLayers() {
    if (!map.getSource("fix-nodes")) {                // the nodes of the Node tab (the lowest of the panel's layers)
      map.addSource("fix-nodes", { type: "geojson", data: EMPTY });
      map.addLayer({ id: "fix-nodes-pt", type: "circle", source: "fix-nodes", paint: { "circle-radius": ["interpolate", ["linear"], ["zoom"], 14, ["case", ["==", ["get", "k"], "junction"], 3, 4], 19, ["case", ["==", ["get", "k"], "junction"], 6, 8]],
        "circle-color": ["match", ["get", "k"], "signal", NODE_COLORS.signal, "crossing", NODE_COLORS.crossing, "junction", NODE_COLORS.junction, NODE_COLORS.other], "circle-stroke-color": "#ffffff", "circle-stroke-width": 2 } });
      map.addLayer({ id: "fix-nodes-hover", type: "circle", source: "fix-nodes", filter: ["==", ["get", "id"], -1], paint: { "circle-radius": ["interpolate", ["linear"], ["zoom"], 14, 9, 19, 14],
        "circle-color": "rgba(255, 255, 255, 0.55)", "circle-stroke-color": "#111827", "circle-stroke-width": 2.5 } });
      map.on("mousemove", "fix-nodes-pt", hoverNode);
      map.on("mouseleave", "fix-nodes-pt", leaveNode);
    }
    if (!map.getSource("fix-doors")) {                // the doors of the Building tab
      map.addSource("fix-doors", { type: "geojson", data: EMPTY });
      map.addLayer({ id: "fix-doors-pt", type: "circle", source: "fix-doors", paint: { "circle-radius": ["interpolate", ["linear"], ["zoom"], 17, ["case", ["==", ["get", "k"], "main"], 4.5, 3.5], 20, ["case", ["==", ["get", "k"], "main"], 8, 6]],
        "circle-color": DOOR_COLOR, "circle-stroke-color": "#ffffff", "circle-stroke-width": 1.5 } });
      map.on("mousemove", "fix-doors-pt", hoverDoor);
      map.on("mouseleave", "fix-doors-pt", leaveDoor);
    }
    if (!map.getSource("fix-turn")) {                 // the roads that leave the junction of the Turn tab; the one under the pointer in the list has a dark outline
      map.addSource("fix-turn", { type: "geojson", data: EMPTY });
      map.addLayer({ id: "fix-turn-hl", type: "line", source: "fix-turn", filter: ["==", ["get", "id"], -1], layout: { "line-cap": "round", "line-join": "round" }, paint: { "line-color": "#111827", "line-width": 12, "line-opacity": 0.85 } });
      map.addLayer({ id: "fix-turn-line", type: "line", source: "fix-turn", layout: { "line-cap": "round", "line-join": "round" }, paint: { "line-color": TURN_OUT, "line-width": 6, "line-opacity": 0.95 } });
    }
    for (const [src, color0, w, fa] of [["fix-edits", AMBER, 5, 0.28], ["fix-sel", BLUE, 7, 0.2]]) {
      const color = src === "fix-edits" ? ["case", ["==", ["get", "d"], 1], DELETED, AMBER] : color0;          // a deleted object is red
      if (map.getSource(src)) continue;
      map.addSource(src, { type: "geojson", data: EMPTY });
      map.addLayer({ id: `${src}-fill`, type: "fill", source: src, filter: ["==", ["geometry-type"], "Polygon"], paint: { "fill-color": color, "fill-opacity": fa } });
      map.addLayer({ id: `${src}-glow`, type: "line", source: src, filter: ["!=", ["geometry-type"], "Point"], layout: { "line-cap": "round", "line-join": "round" }, paint: { "line-color": color, "line-width": w * 2 + 8, "line-opacity": 0.3, "line-blur": 2 } });
      map.addLayer({ id: `${src}-line`, type: "line", source: src, filter: ["!=", ["geometry-type"], "Point"], layout: { "line-cap": "round", "line-join": "round" }, paint: { "line-color": color, "line-width": w, "line-opacity": 0.95 } });
      map.addLayer({ id: `${src}-pt`, type: "circle", source: src, filter: ["==", ["geometry-type"], "Point"], paint: { "circle-radius": src === "fix-sel" ? ["case", ["==", ["get", "j"], 1], 17, w + 3] : w + 3, "circle-color": color, "circle-opacity": 0.35, "circle-stroke-color": color, "circle-stroke-width": src === "fix-sel" ? ["case", ["==", ["get", "j"], 1], 5, 3] : 3 } });
    }
    if (!map.getSource("fix-draw")) {                 // the new road that is being drawn
      map.addSource("fix-draw", { type: "geojson", data: EMPTY });
      map.addLayer({ id: "fix-draw-line", type: "line", source: "fix-draw", filter: ["==", ["geometry-type"], "LineString"], paint: { "line-color": BLUE, "line-width": 4, "line-dasharray": [2, 1.5] } });
      map.addLayer({ id: "fix-draw-pt", type: "circle", source: "fix-draw", filter: ["==", ["geometry-type"], "Point"], paint: { "circle-radius": ["case", ["==", ["get", "snap"], 1], 8, 6], "circle-color": ["case", ["==", ["get", "snap"], 1], "#16a34a", "#ffffff"], "circle-stroke-color": BLUE, "circle-stroke-width": 3 } });
    }
    if (!map.getSource("fix-faces")) {                // the walls of buildings, one color for each type; a wall with a rule has an amber glow
      map.addSource("fix-faces", { type: "geojson", data: EMPTY });
      map.addLayer({ id: "fix-faces-edit", type: "line", source: "fix-faces", filter: ["==", ["get", "e"], 1], paint: { "line-color": AMBER, "line-width": 10, "line-opacity": 0.55 } });
      map.addLayer({ id: "fix-faces-sel", type: "line", source: "fix-faces", filter: ["==", ["get", "s"], 1], layout: { "line-cap": "round" }, paint: { "line-color": "#111827", "line-width": 14, "line-opacity": 0.9 } });
      const faceColor = ["match", ["get", "t"], "front", FACE_COLORS.front, "back", FACE_COLORS.back, "party", FACE_COLORS.party, FACE_COLORS.side];
      map.addLayer({ id: "fix-faces-glow", type: "line", source: "fix-faces", filter: wallFilter(""), layout: { "line-cap": "round" }, paint: { "line-color": faceColor, "line-width": 22, "line-opacity": 0.35, "line-blur": 4 } });
      map.addLayer({ id: "fix-faces-line", type: "line", source: "fix-faces", layout: { "line-cap": "round" },
        paint: { "line-color": faceColor, "line-width": 4.5 } });
      map.addLayer({ id: "fix-faces-hover", type: "line", source: "fix-faces", filter: wallFilter(""), layout: { "line-cap": "round" }, paint: { "line-color": faceColor, "line-width": 9 } });
      map.addLayer({ id: "fix-faces-door", type: "circle", source: "fix-faces", filter: ["==", ["get", "door"], 1], paint: { "circle-radius": 6.5, "circle-color": "#0ea5e9", "circle-stroke-color": "#ffffff", "circle-stroke-width": 2 } });   // a wall with a door: a dot in the middle of it
      map.addLayer({ id: "fix-faces-label", type: "symbol", source: "fix-faces", filter: ["all", ["has", "n"], wallFilter("")],      // the number of one wall (wallShown), as in the list
        layout: { "text-field": ["concat", "#", ["get", "n"]], "text-font": ["Noto Sans Bold"], "text-size": 13, "text-allow-overlap": true, "text-ignore-placement": true },
        paint: { "text-color": "#111827", "text-halo-color": "#ffffff", "text-halo-width": 2.5 } });
      map.on("mousemove", "fix-faces-line", (e) => { const k = e.features[0]?.properties.k; if (k) { map.getCanvas().style.cursor = "pointer"; hoverWall(k); } });
      map.on("mouseleave", "fix-faces-line", () => { map.getCanvas().style.cursor = ""; hoverWall(null); });
    }
    if (!map.getSource("fix-3d")) {                  // the edited buildings, as extrusions with their height and facade color (tilt the map to see them)
      map.addSource("fix-3d", { type: "geojson", data: EMPTY });
      map.addLayer({ id: "fix-3d", type: "fill-extrusion", source: "fix-3d", paint: { "fill-extrusion-height": ["get", "h"], "fill-extrusion-color": ["get", "c"], "fill-extrusion-opacity": 0.85 } }, "fix-edits-fill");
    }
  }

  // the lines, polygons and points of some objects: [{type, id, kind}] -> Feature[]
  async function shapes(list) {
    const out = [], ways = list.filter((o) => o.type === "way" || o.type === "relation");
    const ids = (t) => list.filter((o) => o.type === t).map((o) => o.id).filter((n) => Number.isSafeInteger(n) && n > 0);
    const geo = (r) => { try { return { type: "Feature", properties: { id: Number(r.id) }, geometry: JSON.parse(r.g) }; } catch { return null; } };   // a BigInt in a property stops the map's source: nothing is drawn
    const wayIds = ids("way"), relIds = ids("relation");
    if (wayIds.length) {
      for (const m of modes()) for (const r of await safe(`SELECT osm_id AS id, ST_AsGeoJSON(geometry) AS g FROM ${m}.edges WHERE osm_id IN (${wayIds.join(",")})`)) out.push(geo(r));
      for (const r of await safe(`SELECT osm_id AS id, ST_AsGeoJSON(geom) AS g FROM features.buildings WHERE osm_type = 'way' AND osm_id IN (${wayIds.join(",")})`)) out.push(geo(r));
    }
    if (relIds.length) for (const r of await safe(`SELECT osm_id AS id, ST_AsGeoJSON(geom) AS g FROM features.buildings WHERE osm_type = 'relation' AND osm_id IN (${relIds.join(",")})`)) out.push(geo(r));
    const nodeIds = ids("node");
    if (nodeIds.length) {
      const got = new Set();
      for (const r of await safe(`SELECT osm_id AS id, ST_AsGeoJSON(geom) AS g FROM features.traffic WHERE osm_id IN (${nodeIds.join(",")})`)) { out.push(geo(r)); got.add(Number(r.id)); }
      const missing = nodeIds.filter((n) => !got.has(n));                  // a junction of the roads that has no traffic tag
      if (missing.length) for (const m of modes()) for (const r of await safe(`SELECT node_id AS id, ST_AsGeoJSON(geom) AS g FROM ${m}.nodes WHERE node_id IN (${missing.join(",")})`)) { out.push(geo(r)); got.add(Number(r.id)); }
      const doorsMissing = missing.filter((n) => !got.has(n));
      if (doorsMissing.length && hasDoors()) for (const r of await safe(`SELECT osm_id AS id, ST_AsGeoJSON(geom) AS g FROM main.building_entrances WHERE osm_id IN (${doorsMissing.join(",")})`)) out.push(geo(r));
    }
    return out.filter(Boolean);
  }

  const drawnCount = { edits: 0, sel: 0, heights: [], faces: 0, draw: 0, nodes: 0, turn: 0, doors: 0 };      // what the panel put in the map's sources (the tests read it)
  // the edited buildings with their height and color: the tags of the data with the rule on top, so a change shows before the next build
  async function extrusions() {
    await classify();
    const ids = F.objects(doc).filter((o) => o.type === "way" && kinds.get("way" + o.id) === "building").map((o) => o.id);
    if (!ids.length) return [];
    const out = [];
    for (const r of await safe(`SELECT osm_id AS id, ST_AsGeoJSON(geom) AS g, ${TAGS} FROM features.buildings WHERE osm_type = 'way' AND osm_id IN (${ids.join(",")})`)) {
      const rule = F.ruleOf(doc, "way", Number(r.id)), tags = { ...tagMap(r), ...rule.set };
      for (const k of rule.delete) delete tags[k];
      try { out.push({ type: "Feature", properties: { h: Math.round(heightOf(tags) * 10) / 10, c: cssColor(tags["building:colour"]) ?? cssColor(tags["roof:colour"]) ?? AMBER }, geometry: JSON.parse(r.g) }); } catch { /* no geometry */ }
    }
    return out;
  }

  // the walls on the map: those of the selected building (with the types of the rules on top), and with the check box all walls in view (the types of the data)
  // the line of a new road: the nodes that are OSM ids come from the nodes tables (a junction that the road joins), the others are [lon, lat]
  async function newWayLine(w) {
    const ids = w.nodes.filter(Number.isInteger), at = new Map();
    if (ids.length) for (const m of modes()) for (const r of await safe(`SELECT node_id AS id, ST_X(geom) AS x, ST_Y(geom) AS y FROM ${m}.nodes WHERE node_id IN (${ids.join(",")})`)) at.set(Number(r.id), [r.x, r.y]);
    return w.nodes.map((n) => (Array.isArray(n) ? n : at.get(n))).filter(Boolean);
  }
  function zoomTo(coords, maxZoom = 18) {
    if (!coords.length) return;
    const b = new maplibregl.LngLatBounds();
    coords.forEach((c) => b.extend(c));
    map.fitBounds(b, { padding: { top: 80, bottom: 80, left: innerWidth > 700 ? 440 : 40, right: 90 }, maxZoom, duration: 600 });
  }

  // hover on a node dot: a ring around it, the pointer cursor and a tooltip (roadstyle's tooltip look, `rs-tip`) with its kind, its OSM id and its name
  let nodeTip = null, hoverNodeId = null;
  function hoverNode(e) {
    const f = e.features?.[0];
    if (!f || !open() || current !== "node") return;
    map.getCanvas().style.cursor = "pointer";
    const { id, k, kind, name } = f.properties;
    if (hoverNodeId !== id) { hoverNodeId = id; map.setFilter("fix-nodes-hover", ["==", ["get", "id"], id]); }
    nodeTip ??= new maplibregl.Popup({ closeButton: false, closeOnClick: false, className: "rs-tip fix-tip", offset: 12, maxWidth: "240px" });
    nodeTip.setLngLat(f.geometry.coordinates).setHTML(`<b>${esc(k === "signal" ? "Traffic signal" : k === "crossing" ? "Crossing" : k === "junction" ? "Junction (no traffic tag)" : kind || "Node")}</b><br>node ${id}${name ? `<br>${esc(name)}` : ""}<br><span class="muted">${k === "junction" ? "Click to add a signal or a crossing here" : "Click to pick it"}</span>`).addTo(map);
  }
  function leaveNode() {
    hoverNodeId = null;
    if (map.getLayer("fix-nodes-hover")) map.setFilter("fix-nodes-hover", ["==", ["get", "id"], -1]);
    map.getCanvas().style.cursor = "";
    nodeTip?.remove();
  }
  // hover on a door dot: the pointer cursor and a tooltip with the kind, the address and the access
  let doorTip = null;
  function hoverDoor(e) {
    const f = e.features?.[0];
    if (!f || !open() || current !== "building") return;
    map.getCanvas().style.cursor = "pointer";
    const { id, k, a, acc } = f.properties;
    doorTip ??= new maplibregl.Popup({ closeButton: false, closeOnClick: false, className: "rs-tip fix-tip", offset: 10, maxWidth: "240px" });
    doorTip.setLngLat(f.geometry.coordinates).setHTML(`<b>Door · ${esc(k === "door" ? "no entrance tag" : k)}</b><br>node ${id}${a ? `<br>${esc(a)}` : ""}${acc ? `<br>access ${esc(acc)}` : ""}`).addTo(map);
  }
  function leaveDoor() { map.getCanvas().style.cursor = ""; doorTip?.remove(); }
  // the doors in view (zoom 17 or more) while the Building tab shows
  let doorToken = 0;
  async function drawDoors() {
    const my = ++doorToken;
    let feats = [], info = "";
    if (open() && current === "building" && hasDoors()) {
      ensureLayers();
      if (map.getZoom() < 17) info = "Zoom in to 17 to see the doors.";
      else {
        const b = map.getBounds();
        feats = (await safe(`SELECT osm_id AS id, kind AS k, building_id AS b, access AS acc, trim(concat_ws(' ', street, housenumber)) AS a, ST_X(geom) AS x, ST_Y(geom) AS y FROM main.building_entrances
          WHERE ST_X(geom) BETWEEN ${b.getWest()} AND ${b.getEast()} AND ST_Y(geom) BETWEEN ${b.getSouth()} AND ${b.getNorth()} LIMIT 5000`))
          .map((r) => ({ type: "Feature", properties: { id: Number(r.id), k: r.k, b: r.b == null ? 0 : Number(r.b), a: r.a ?? "", acc: r.acc ?? "" }, geometry: { type: "Point", coordinates: [r.x, r.y] } }));
        info = `In view: ${plural(feats.length, "door")}.`;
      }
    }
    if (my !== doorToken) return;
    if (!feats.length) leaveDoor();
    map.getSource("fix-doors")?.setData({ type: "FeatureCollection", features: feats });
    if ($("fix-doors-info")) $("fix-doors-info").textContent = info;
    drawnCount.doors = feats.length;
  }
  // the nodes that a click in the Node tab can pick (the traffic signals and the crossings of features.traffic) in view, as dots with a color for each kind
  let nodeToken = 0;
  async function drawNodes() {
    const my = ++nodeToken;
    let feats = [], info = "";
    if (open() && current === "node") {
      ensureLayers();
      if (map.getZoom() < 14) info = "Zoom in to see the dots (zoom 14 or more).";
      else {
        const b = map.getBounds();
        feats = (await safe(`SELECT osm_id AS id, kind, name, CASE WHEN kind LIKE '%signal%' THEN 'signal' WHEN kind LIKE '%crossing%' THEN 'crossing' ELSE 'other' END AS k, ST_X(geom) AS x, ST_Y(geom) AS y
          FROM features.traffic WHERE ST_X(geom) BETWEEN ${b.getWest()} AND ${b.getEast()} AND ST_Y(geom) BETWEEN ${b.getSouth()} AND ${b.getNorth()} LIMIT 5000`))
          .map((r) => ({ type: "Feature", properties: { id: Number(r.id), k: r.k, kind: r.kind ?? "", name: r.name ?? "" }, geometry: { type: "Point", coordinates: [r.x, r.y] } }));
        if (map.getZoom() >= 16 && modes().length) {                  // the junctions of the roads (OSM nodes where two ways meet) that have no traffic tag: a place for a new signal
          const m = modes()[0];
          feats.push(...(await safe(`SELECT n.node_id AS id, ST_X(n.geom) AS x, ST_Y(n.geom) AS y FROM ${m}.nodes n WHERE ST_X(n.geom) BETWEEN ${b.getWest()} AND ${b.getEast()} AND ST_Y(n.geom) BETWEEN ${b.getSouth()} AND ${b.getNorth()}
            AND n.node_id IN (SELECT nid FROM (SELECT source AS nid, target AS o FROM ${m}.edges UNION ALL SELECT target, source FROM ${m}.edges) GROUP BY nid HAVING count(DISTINCT o) >= 3)
            AND n.node_id IN (SELECT node_id FROM ${m}.way_nodes) AND n.node_id NOT IN (SELECT osm_id FROM features.traffic) LIMIT 5000`))
            .map((r) => ({ type: "Feature", properties: { id: Number(r.id), k: "junction", kind: "", name: "" }, geometry: { type: "Point", coordinates: [r.x, r.y] } })));
        }
        const n = (k) => feats.filter((f) => f.properties.k === k).length;
        info = `In view: ${plural(n("signal"), "traffic signal")}, ${plural(n("crossing"), "crossing")}${n("other") ? `, ${n("other")} other` : ""}${n("junction") ? `, ${plural(n("junction"), "junction")}` : map.getZoom() < 16 ? ". Zoom in to 16 to see the junctions" : ""}.`;
      }
    }
    if (my !== nodeToken) return;
    if (!feats.length) leaveNode();
    map.getSource("fix-nodes")?.setData({ type: "FeatureCollection", features: feats });
    $("fix-nodes-info").textContent = info;
    drawnCount.nodes = feats.length;
  }

  let faceToken = 0;
  async function drawFaces() {
    const my = ++faceToken;
    if (!open()) { map.getSource("fix-faces")?.setData({ type: "FeatureCollection", features: [] }); drawnCount.faces = 0; return; }       // closed: no walls on the map
    ensureLayers();
    let feats = [];
    const asFeature = (g, t, e, s = 0, k = null) => { try { return { type: "Feature", properties: { t, e: e ? 1 : 0, s: s ? 1 : 0, ...(k ? { k } : {}) }, geometry: JSON.parse(g) }; } catch { return null; } };
    const mid = (g) => { const c = JSON.parse(g).coordinates; return [(c[0][0] + c[1][0]) / 2, (c[0][1] + c[1][1]) / 2]; };
    const numberDot = (g, n, k) => { try { return { type: "Feature", properties: { n, k }, geometry: { type: "Point", coordinates: mid(g) } }; } catch { return null; } };
    const doorDot = (g) => { try { const c = JSON.parse(g).coordinates; return { type: "Feature", properties: { door: 1 }, geometry: { type: "Point", coordinates: [(c[0][0] + c[1][0]) / 2, (c[0][1] + c[1][1]) / 2] } }; } catch { return null; } };
    if (hasFaces()) {
      if (current === "building" && $("fix-walls-all").checked && map.getZoom() >= 16) {
        const b = map.getBounds();
        const all = (await safe(`SELECT * EXCLUDE (geometry), ST_AsGeoJSON(geometry) AS g FROM main.building_faces WHERE ST_Intersects(geometry, ST_MakeEnvelope(${b.getWest()}, ${b.getSouth()}, ${b.getEast()}, ${b.getNorth()})) LIMIT 30000`))
        ;
        const mine = (r) => sel?.kind === "building" && Number(r.building_id) === sel.id, key = (r) => `${Number(r.from_node)}|${Number(r.to_node)}`;
        feats = all.map((r) => asFeature(r.g, r.face_type, r.edited, mine(r) && key(r) === wallSel, mine(r) ? key(r) : null));
        feats.push(...all.filter(mine).map((r) => numberDot(r.g, r.face_index + 1, key(r))));
        feats.push(...all.filter((r) => r.door).map((r) => doorDot(r.g)));
      } else if (current === "building" && sel?.kind === "building" && walls.length) {
        feats = walls.map((w) => asFeature(w.g, wallType(w), !!F.findFace(doc, sel.id, w.from_node, w.to_node) || w.edited, wallKey(w) === wallSel, wallKey(w)));
        feats.push(...walls.filter((w) => wallDoorNow(w)).map((w) => doorDot(w.g)), ...walls.map((w) => numberDot(w.g, w.face_index + 1, wallKey(w))));
      }
    }
    if (my !== faceToken) return;
    feats = feats.filter(Boolean);
    map.getSource("fix-faces")?.setData({ type: "FeatureCollection", features: feats });
    drawnCount.faces = feats.length;
    paintHover();
  }
  $("fix-walls-all").onchange = drawFaces;
  map.on("moveend", () => { if (open() && current === "building" && $("fix-walls-all").checked) drawFaces(); if (open() && current === "node") drawNodes(); if (open() && current === "building") drawDoors(); });

  let mapToken = 0;
  async function drawEdits() {
    if (!open() && !total()) return;                    // no layers before the panel opens or a change exists: they would go under the page's own layers
    const my = ++mapToken;
    if (!map.isStyleLoaded?.() && !map.getSource("fix-edits")) await new Promise((ok) => map.once("idle", ok));
    ensureLayers();
    const list = F.objects(doc), feats = await shapes(list);
    const gone = new Set(list.filter((o) => F.ruleOf(doc, o.type, o.id).mode.remove).map((o) => o.id));
    for (const f of feats) f.properties.d = gone.has(f.properties.id) ? 1 : 0;
    for (const w of F.newWays(doc)) { const line = await newWayLine(w); if (line.length > 1) feats.push({ type: "Feature", properties: { id: w.id, road: 1 }, geometry: { type: "LineString", coordinates: line } }); }
    const via = [...new Set((doc.turn_restrictions ?? []).map((t) => t.via_node))].filter((n) => Number.isSafeInteger(n));
    if (via.length) for (const r of await safe(`SELECT node_id AS id, ST_AsGeoJSON(geom) AS g FROM driving.nodes WHERE node_id IN (${via.join(",")})`)) { try { feats.push({ type: "Feature", properties: { turn: 1 }, geometry: JSON.parse(r.g) }); } catch { /* no geometry */ } }
    const solids = await extrusions();
    if (my === mapToken) {
      map.getSource("fix-edits")?.setData({ type: "FeatureCollection", features: feats }); drawnCount.edits = feats.length;
      map.getSource("fix-3d")?.setData({ type: "FeatureCollection", features: solids }); drawnCount.heights = solids.map((f) => f.properties.h);
    }
  }

  // the roads that leave the junction (the Turn tab), in green; a hover on a row of the list outlines its road
  let turnOut = [], turnToken = 0;
  async function drawTurnRoads() {
    const my = ++turnToken;
    let feats = [];
    if (open() && current === "turns" && turnOut.length) {
      ensureLayers();
      feats = (await safe(`SELECT osm_id AS id, ST_AsGeoJSON(geometry) AS g FROM driving.edges WHERE osm_id IN (${turnOut.join(",")})`))
        .map((r) => { try { return { type: "Feature", properties: { id: Number(r.id) }, geometry: JSON.parse(r.g) }; } catch { return null; } }).filter(Boolean);
    }
    if (my !== turnToken) return;
    map.getSource("fix-turn")?.setData({ type: "FeatureCollection", features: feats });
    drawnCount.turn = feats.length;
  }
  // a click (or Enter) on a junction row picks the junction; on a row of a turn road it picks the road (the same as a wall of a building)
  const pickTurn = (row) => { if (!row) return; turnSel = turnSel === Number(row.dataset.to) ? null : Number(row.dataset.to); paintTurnSel(); if (turnSel != null) reveal(row); };
  $("fix-out").addEventListener("click", (e) => { if (!e.target.closest("select, button")) pickTurn(e.target.closest(".turn")); });
  $("fix-out").addEventListener("keydown", (e) => { if (e.target.classList?.contains("turn") && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); pickTurn(e.target); } });
  $("fix-out").addEventListener("mouseover", (e) => { const r = e.target.closest(".turn"); if (r && map.getLayer("fix-turn-hl")) map.setFilter("fix-turn-hl", ["==", ["get", "id"], Number(r.dataset.to)]); });
  $("fix-out").addEventListener("mouseout", (e) => { if (!e.relatedTarget?.closest?.(".turn") && map.getLayer("fix-turn-hl")) map.setFilter("fix-turn-hl", ["==", ["get", "id"], turnSel ?? -1]); });

  let selToken = 0;
  async function drawSelected(zoom, { turn = null } = {}) {
    const my = ++selToken;
    ensureLayers();
    let feats = [];
    if (!open()) {                                    // the panel is closed: nothing is selected on the map
      map.getSource("fix-sel")?.setData({ type: "FeatureCollection", features: [] }); drawnCount.sel = 0;
      return;
    }
    if (turn) {
      const from = whole($("fix-from").value), feat = (g, props) => { try { return { type: "Feature", properties: props, geometry: JSON.parse(g) }; } catch { return null; } };
      const road = from ? await safe(`SELECT ST_AsGeoJSON(geometry) AS g FROM driving.edges WHERE osm_id = ${from}`) : [];
      const junction = await safe(`SELECT ST_AsGeoJSON(geom) AS g FROM driving.nodes WHERE node_id = ${Number(turn.via)}`);
      feats = [...road.map((r) => feat(r.g, {})), ...junction.map((r) => feat(r.g, { j: 1 }))].filter(Boolean);
    } else if (sel && EDIT.includes(current)) feats = await shapes([{ type: sel.type, id: sel.id }]);
    else if (current === "new" && editNew != null) {
      const w = F.findNewWay(doc, editNew), line = w ? await newWayLine(w) : [];
      if (line.length > 1) feats = [{ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: line } }];
    }
    if (my !== selToken) return;
    map.getSource("fix-sel")?.setData({ type: "FeatureCollection", features: feats }); drawnCount.sel = feats.length;
    if (zoom && feats.length) {
      const b = new maplibregl.LngLatBounds();
      const add = (c) => (typeof c[0] === "number" ? b.extend(c) : c.forEach(add));
      for (const f of feats) add(f.geometry.coordinates);
      map.fitBounds(b, { padding: { top: 80, bottom: 80, left: innerWidth > 700 ? 440 : 40, right: 90 }, maxZoom: 19, duration: 600 });
    }
  }

  // ---- a new road: click points on the map (a click near a junction joins it), set the kind, finish; the rule is in `new_ways`
  let draw = null, newDraft = {}, editNew = null;          // editNew: the id of a new road of the changes that is open for edit (its points stay)
  const newFields = presets.new_road ?? [];
  function renderNewForm() {
    const g = guardsOf("new_road", newDraft);
    $("fix-newform").innerHTML = `<div class="grid2 fixgrid">${newFields.map((f) => `<label for="fixn-${f.key}" class="small">${esc(f.label)}</label>${control({ ...f }, newDraft[f.key] ?? "", g).replace(`id="fixf-${f.key.replace(/[^a-z0-9]/gi, "_")}"`, `id="fixn-${f.key}"`)}${guardNote(f, g) ? `<p class="why gwhy">${esc(guardNote(f, g))}</p>` : ""}`).join("")}</div>`;
  }
  $("fix-newform").addEventListener("change", (e) => {                 // a choice that closes or narrows another field
    for (const f of newFields) { const el = document.getElementById(`fixn-${f.key}`); if (el) newDraft[f.key] = el.value; }
    const id = e.target.id;
    renderNewForm(); document.getElementById(id)?.focus();
  });
  const drawn = () => { paintDots(); $("fix-draw-n").textContent = draw ? `${draw.pts.length} point${draw.pts.length === 1 ? "" : "s"}${draw.pts.some((p) => p.id != null) ? ", joins a junction" : ""}` : ""; };
  async function drawPreview() {
    ensureLayers();
    const pts = draw?.pts ?? [], feats = pts.map((p) => ({ type: "Feature", properties: { snap: p.id != null ? 1 : 0 }, geometry: { type: "Point", coordinates: [p.lng, p.lat] } }));
    if (pts.length > 1) feats.push({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: pts.map((p) => [p.lng, p.lat]) } });
    map.getSource("fix-draw")?.setData({ type: "FeatureCollection", features: feats });
    drawnCount.draw = pts.length;
  }
  function startDraw() {
    draw = { pts: [] }; newDraft = { highway: "residential" };
    $("fix-drawbox").hidden = false; $("fix-draw").hidden = true;
    renderNewForm(); paintNewMode(); drawn(); drawPreview();
    map.getCanvas().style.cursor = current === "new" ? "crosshair" : "";
  }
  function stopDraw() {
    draw = null; editNew = null; $("fix-drawbox").hidden = true; $("fix-draw").hidden = false;
    $("fix-new").classList.remove("asking"); $("fix-new-inchg").hidden = true;
    paintNewMode(); map.getCanvas().style.cursor = ""; drawn(); drawPreview(); drawSelected(false);
  }
  // the New road tab has two modes: draw (points and form) and edit (the form of a road that is in the changes)
  function paintNewMode() {
    const edit = editNew != null;
    $("fix-draw-undo").hidden = edit;
    $("fix-new-del").hidden = !edit;
    $("fix-draw-done").textContent = edit ? "Update change" : "Add to changes";
    $("fix-draw-help").innerHTML = edit ? "The points of this road stay. To change them, remove the road and draw it again." : "Click the points of the road on the map. <b>Enter</b> finishes, <b>Esc</b> cancels.";
  }
  function startEdit(id, { asking = true } = {}) {
    const w = F.findNewWay(doc, id);
    if (!w) return;
    if (draw) stopDraw();
    editNew = id; newDraft = { ...w.tags };
    $("fix-drawbox").hidden = false; $("fix-draw").hidden = true;
    renderNewForm(); paintNewMode();
    $("fix-draw-n").textContent = `${plural(w.nodes.length, "point")}${w.nodes.some(Number.isInteger) ? ", joins a junction" : ""}`;
    // a new road of the changes: the box first, and the form waits
    $("fix-new").classList.toggle("asking", asking);
    if (asking) askBox($("fix-new-inchg"), "new road", `${w.tags?.highway ?? "road"}${w.tags?.name ? ` "${w.tags.name}"` : ""}, ${plural(w.nodes.length, "point")}`, F.count(doc, { new_way: id }));
    else $("fix-new-inchg").hidden = true;
  }
  $("fix-new-inchg").addEventListener("click", (e) => {
    const b = e.target.closest("[data-ask]"), id = editNew;
    if (!b || id == null) return;
    if (b.dataset.ask === "edit") { if (F.count(doc, { new_way: id }) > 1) commit(() => F.join(doc, { new_way: id })); startEdit(id, { asking: false }); return; }
    commit(() => F.remove(doc, { new_way: id })); stopDraw(); status(`Deleted from the changes: new road ${id}. Undo brings it back.`);
  });
  $("fix-new-del").onclick = () => { const id = editNew; commit(() => F.deleteNewWay(doc, id)); stopDraw(); status(`Removed from the changes: new road ${id}.`); };
  async function addPoint(e) {
    const { lng, lat } = e.lngLat, d = px(e, 14);
    let best = null;
    for (const m of modes()) for (const r of await safe(`SELECT node_id AS id, ST_X(geom) AS x, ST_Y(geom) AS y FROM ${m}.nodes WHERE node_id IN (SELECT node_id FROM ${m}.way_nodes) AND ST_X(geom) BETWEEN ${lng - d} AND ${lng + d} AND ST_Y(geom) BETWEEN ${lat - d} AND ${lat + d}`)) {   // only OSM nodes: the nodes table also has synthetic ones
      const dist = (r.x - lng) ** 2 + (r.y - lat) ** 2;
      if (!best || dist < best.dist) best = { id: Number(r.id), lng: r.x, lat: r.y, dist };
    }
    draw?.pts.push(best ? { id: best.id, lng: best.lng, lat: best.lat } : { id: null, lng, lat });
    drawn(); drawPreview();
  }
  function finishDraw() {
    if (editNew == null && (!draw || draw.pts.length < 2)) return status("Click at least two points on the map.");
    const w = editNew != null ? F.findNewWay(doc, editNew) : null, tags = {};
    for (const f of newFields) {
      const v = (document.getElementById(`fixn-${f.key}`)?.value ?? "").trim();
      if (f.type === "int" && v !== "" && !/^\d+$/.test(v)) return status(`${f.label} is a whole number.`);
      if (v) tags[f.key] = v;
    }
    if (!tags.highway) return status("Pick the kind of road.");
    if (w) {                                                          // the tags that the form has no field for stay as they are
      const keep = Object.fromEntries(Object.entries(w.tags ?? {}).filter(([k]) => !newFields.some((f) => f.key === k)));
      commit(() => F.updateNewWayTags(doc, w.id, { ...keep, ...tags }));
      stopDraw();
      return status(`Updated the change: new road ${w.id}.`);
    }
    const nodes = draw.pts.map((q) => q.id ?? [Math.round(q.lng * 1e7) / 1e7, Math.round(q.lat * 1e7) / 1e7]);
    let id;
    commit(() => { id = F.addNewWay(doc, { nodes, tags }); });
    stopDraw();
    status(`Added to the changes: new road ${id}. Build again to have it in the tables.`);
  }
  $("fix-draw").onclick = startDraw;
  $("fix-draw-done").onclick = finishDraw;
  $("fix-draw-undo").onclick = () => { draw?.pts.pop(); drawn(); drawPreview(); };
  $("fix-draw-cancel").onclick = stopDraw;
  document.addEventListener("keydown", (e) => { if (!draw || !open() || current !== "new") return; if (e.key === "Escape") stopDraw(); else if (e.key === "Enter" && !/^(INPUT|SELECT|TEXTAREA)$/.test(document.activeElement?.tagName)) finishDraw(); });

  // ---- what a click on the map picks
  const open = () => document.body.classList.contains("editing-fixes");
  const px = (e, n) => { const a = map.unproject([e.point.x - n, e.point.y]), c = map.unproject([e.point.x + n, e.point.y]); return Math.abs(c.lng - a.lng) / 2; };
  // the distance in pixels from a point to a line of the map
  function pixelsToLine(pt, coords) {
    const q = coords.map((c) => map.project(c));
    let best = Infinity;
    for (let i = 1; i < q.length; i++) {
      const ax = q[i - 1].x, ay = q[i - 1].y, dx = q[i].x - ax, dy = q[i].y - ay, len = dx * dx + dy * dy;
      const t = len ? Math.max(0, Math.min(1, ((pt.x - ax) * dx + (pt.y - ay) * dy) / len)) : 0;
      best = Math.min(best, Math.hypot(pt.x - (ax + t * dx), pt.y - (ay + t * dy)));
    }
    return best;
  }
  // a click within 10 px of a wall: a wall of the picked building, or (with "Show the walls of all buildings") of another building, which gets picked too
  async function wallAt(e) {
    let best = null;
    for (const w of walls) { const d = pixelsToLine(e.point, wallLine(w)); if (d <= 10 && (!best || d < best.d)) best = { d, key: wallKey(w) }; }
    if (best) { selectWall(best.key); return true; }
    if (!hasFaces() || !$("fix-walls-all").checked || map.getZoom() < 16) return false;
    const { lng, lat } = e.lngLat, dd = px(e, 10);
    const r = (await safe(`SELECT building_id AS b, from_node AS f, to_node AS n FROM main.building_faces WHERE ST_Distance(geometry, ST_Point(${lng}, ${lat})) < ${dd} ORDER BY ST_Distance(geometry, ST_Point(${lng}, ${lat})) LIMIT 1`))[0];
    if (!r) return false;
    const key = `${Number(r.f)}|${Number(r.n)}`;
    if (!(sel?.kind === "building" && sel.id === Number(r.b))) await select("building", "way", Number(r.b));
    selectWall(key);
    return true;
  }
  async function onMapClick(e) {
    if (open() && draw && current === "new") return addPoint(e);
    if (!open() || (current !== "building" && current !== "node")) return;        // a road is picked by the dashboard's click (rs:select), a turn too
    const { lng, lat } = e.lngLat;
    if (current === "building") {
      const door = map.queryRenderedFeatures(e.point, { layers: ["fix-doors-pt"] })[0];
      if (door) {
        const bid = door.properties.b;
        if (bid && !(sel?.kind === "building" && sel.id === bid)) select("building", "way", bid);
        else if (bid) status(`Door ${door.properties.id}: change its kind in the Doors section.`);
        else status(`Door ${door.properties.id} is not on a building outline.`);
        return;
      }
      if (await wallAt(e)) return;
      const d = px(e, 6);
      const r = (await safe(`SELECT osm_id, osm_type FROM features.buildings WHERE ST_Intersects(geom, ST_Buffer(ST_Point(${lng}, ${lat}), ${d})) ORDER BY ST_Distance(geom, ST_Point(${lng}, ${lat})) LIMIT 1`))[0];
      if (r) select("building", r.osm_type === "relation" ? "relation" : "way", Number(r.osm_id));
      else status("No building here.");
    } else {
      const d = px(e, 14);
      const r = (await safe(`SELECT osm_id FROM features.traffic WHERE ST_X(geom) BETWEEN ${lng - d} AND ${lng + d} AND ST_Y(geom) BETWEEN ${lat - d} AND ${lat + d}
        ORDER BY (ST_X(geom) - ${lng}) ^ 2 + (ST_Y(geom) - ${lat}) ^ 2 LIMIT 1`))[0];
      if (r) select("node", "node", Number(r.osm_id));
      else {
        let best = null;
        for (const m of modes()) for (const q of await safe(`SELECT node_id AS id, ST_X(geom) AS x, ST_Y(geom) AS y FROM ${m}.nodes WHERE node_id IN (SELECT node_id FROM ${m}.way_nodes) AND ST_X(geom) BETWEEN ${lng - d} AND ${lng + d} AND ST_Y(geom) BETWEEN ${lat - d} AND ${lat + d}`)) {
          const dist = (q.x - lng) ** 2 + (q.y - lat) ** 2;
          if (!best || dist < best.dist) best = { id: Number(q.id), dist };
        }
        if (best) select("node", "node", best.id);
        else status("No node here. Click a dot, or a junction of the roads.");
      }
    }
    setTimeout(() => { window.rsDeselect?.(); document.querySelectorAll(".maplibregl-popup:not(.rs-tip)").forEach((p) => p.remove()); });
  }
  map.on("click", onMapClick);
  // roadstyle picks the nearest road for a click within a few pixels of it, also when the click is on a building. The Road and Turn tabs ignore such a click
  // (a building is picked in the Building tab). The point of the press comes first: the dashboard's click handler runs before the page's own.
  let pressed = null;
  for (const ev of ["mousedown", "touchstart"]) map.on(ev, (e) => { pressed = { p: e.lngLat, t: performance.now() }; });
  async function onBuilding(p) {
    if (!p || performance.now() - p.t > 2000 || !built("features", "buildings")) return false;       // a press that is older is not this select
    return (await safe(`SELECT 1 FROM features.buildings WHERE ST_Intersects(geom, ST_Point(${p.p.lng}, ${p.p.lat})) LIMIT 1`)).length > 0;
  }
  document.addEventListener("rs:select", async (e) => {                       // a road: the dashboard's click on the roads (roadstyle), with no popup
    if (!open()) return;
    setTimeout(() => document.querySelectorAll(".maplibregl-popup:not(.rs-tip)").forEach((p) => p.remove()));
    if (draw && current === "new") { setTimeout(() => window.rsDeselect?.()); return; }              // a click while drawing adds a point (onMapClick)
    const turns = current === "turns";
    if (!turns && current !== "road") return;                            // a building or a node is picked by its own click
    const id = Number(ctx.roadIdOfRow(e.detail?.id));
    if (!Number.isSafeInteger(id) || id <= 0) return;
    if (await onBuilding(pressed)) { window.rsDeselect?.(); return; }
    if (turns) { $("fix-from").value = id; findTurns(id); }
    else select("road", "way", id);
  });

  // ---- wiring
  $("fix-form").addEventListener("keydown", (e) => { const row = e.target.closest?.(".wallrow"); if (row && e.target === row && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); selectWall(row.dataset.key, { zoom: true }); } });
  $("fix-form").addEventListener("mouseover", (e) => { const row = e.target.closest?.(".wallrow"); if (row) hoverWall(row.dataset.key); });
  $("fix-form").addEventListener("mouseout", (e) => { const row = e.target.closest?.(".wallrow"); if (row && !row.contains(e.relatedTarget)) hoverWall(null); });
  $("fix-form").addEventListener("input", onInput);
  $("fix-form").addEventListener("change", onInput);
  $("fix-form").addEventListener("click", (e) => {
    const nav = e.target.closest("[data-wall-nav]");
    if (nav) return stepWall(Number(nav.dataset.wallNav));
    const drow = e.target.closest(".doorrow");
    if (drow && !e.target.closest("select")) { const d = doors.find((x) => x.osm_id === Number(drow.dataset.id)); if (d) zoomTo([[d.lon, d.lat]], 20); return; }
    const row = e.target.closest(".wallrow");
    if (row && !e.target.closest("select")) return selectWall(row.dataset.key, { zoom: true });
    const t = e.target.closest("button");
    if (!t) return;
    if (t.dataset.revert) return revertKey(t.dataset.revert);
    if (t.dataset.clear) { draft[t.dataset.clear] = ""; touched.add(t.dataset.clear); t.closest(".otherrow").querySelector("input").value = ""; t.closest(".otherrow").classList.add("edited"); return paintBar(); }
    if (t.id === "fix-add") {
      const k = $("fix-addkey").value.trim(), v = $("fix-addval").value.trim();
      if (!k || !v) return status("Type a key and a value.");
      draft[k] = v; touched.add(k); renderForm();
    }
  });
  $("fix-save").onclick = save;
  $("fix-unselect").onclick = unselect;
  $("fix-form").addEventListener("click", (e) => { if (e.target.closest("#fix-delete")) toggleDelete(); });
  $("fix-revert").onclick = () => { touched = new Set(); draft = initialDraft(); renderForm(); };
  for (const b of document.querySelectorAll("#fix-panel [data-review]")) b.onclick = () => setTab("changes");
  $("fix-pending").addEventListener("click", (e) => { const b = e.target.closest("[data-go]"); if (b) setTab(b.dataset.go, { zoom: true }); });
  $("fix-del").onclick = () => {
    if (!sel) return;
    commit(() => F.deleteObjectRule(doc, sel.type, sel.id));
    draft = initialDraft(); renderForm();
    status(`Removed from the changes: ${sel.type} ${sel.id}. The data has the original values after the next build.`);
  };
  $("fix-load").onclick = () => { const id = whole($("fix-id").value); id ? select(kind, KIND_OF[kind], id, { zoom: true }) : status("Type an OSM ID (a whole number)."); };
  $("fix-walls-row").onchange = drawFaces;
  $("fix-id").onkeydown = (e) => { if (e.key === "Enter") $("fix-load").click(); };
  for (const t of TABS) $(`fix-tab-${t}`).onclick = () => setTab(t, { zoom: true });          // the tab shows its object on the map

  // ---- the numbers on the page
  function sync() {
    const { turns, faces, newWays } = F.counts(doc), n = F.objects(doc).length + turns + faces + newWays;
    $("fix-n").textContent = n ? String(n) : "";
    $("fix-apply-n").textContent = n ? ` (${plural(n, "change")})` : "";
    paintBar();
    if (current === "turns" && !$("fix-turnsec").hidden) outgoing();
    const s = (x) => (x === 1 ? "" : "s");
    ctx.onChange(n ? `${n} change${s(n)}: ${F.objects(doc).length} object${s(F.objects(doc).length)}, ${turns} turn rule${s(turns)}, ${faces} wall${s(faces)}, ${newWays} new road${s(newWays)}.` : "No fixes.", n);
    if (!$("fix-changes").hidden) renderChanges();
    drawEdits();
    drawFaces();
  }

  // ---- what the page uses
  function text() { return F.dump(yaml, doc) || null; }
  // the changes of an area stay in this browser (one text for each area name), so a reload of the page does not lose them
  const storeKey = () => "duckosm.fixes:" + (ctx.areaKey?.() || "area");
  function persist() { try { const t = text(); t ? localStorage.setItem(storeKey(), t) : localStorage.removeItem(storeKey()); } catch { /* no storage */ } }
  function restore() {
    if (total()) return 0;
    try { const t = localStorage.getItem(storeKey()); if (!t) return 0; doc = F.parse(yaml, t); hist = []; future = []; sync(); return total(); } catch { return 0; }
  }
  function load(txt) {
    try { doc = F.parse(yaml, txt); hist = []; future = []; persist(); sync(); if (sel && data) { draft = initialDraft(); renderForm(); } }
    catch (e) { status("The fixes file is not valid: " + e.message.split("\n")[0], 8000); }
  }
  function openPanel() {
    clearMapSelection();                             // what was selected before (roadstyle's road selection and its popup) is not selected in this panel
    document.body.classList.add("editing-fixes"); ensureLayers();
    const back = restore();
    if (back) status(`${plural(back, "change")} from your last visit ${back === 1 ? "is" : "are"} back.`, 6000);
    setTab(current); drawEdits();
  }
  async function openFrom() { await duck.loadSpatial(); openPanel(); }
  // after a build: the data is new, so each tab loads its object again (the changes and the picked objects stay)
  const rebuilt = () => {
    kinds.clear(); names.clear(); walls = []; wallSel = null;
    for (const k of EDIT) if (k !== kind && stash[k]?.sel) stash[k] = { sel: stash[k].sel, data: null, draft: {}, touched: new Set(), walls: [], wallSel: null, ask: stash[k].ask };
    if (sel) select(kind, sel.type, sel.id, { go: false }); else { renderObject(); drawSelected(false); }
    drawEdits(); drawFaces(); drawNodes(); drawDoors();
  };
  // leaving the panel unselects everything on the map: the blue objects, roadstyle's road selection, the node dots, the turn roads and the walls (the amber changes stay)
  function close() {
    if (draw) stopDraw();
    document.body.classList.remove("editing-fixes");
    clearMapSelection(); drawSelected(false); drawFaces(); drawNodes(); drawTurnRoads(); drawDoors();
  }

  $("fix-apply").onclick = () => ctx.onBuild?.();          // the host page builds again (it reads text())
  setTab("road"); sync();
  return { text, load, restore, open: openFrom, close, isOpen: open, rebuilt, select, tab: setTab, drawn: () => ({ ...drawnCount }), count: () => F.objects(doc).length + (doc.turn_restrictions ?? []).length + F.faceEdits(doc).length + F.newWays(doc).length };
}
