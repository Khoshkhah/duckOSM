// Checks of the fixes rules (fixes.js) and of the readers for the 3D preview (fixes_panel.js), with no browser:
//   node web/fixes_editor/fixes_units.mjs
import * as F from "./fixes.js";
import { cssColor, heightOf, wallShown, wallFilter } from "./fixes_panel.js";

const sorted = (k, v) => (v && typeof v === "object" && !Array.isArray(v) ? Object.fromEntries(Object.entries(v).sort()) : v);   // the order of the keys of a map does not matter
const eq = (a, b, m) => { const x = JSON.stringify(a, sorted), y = JSON.stringify(b, sorted); if (x !== y) throw new Error(`${m}: ${x} != ${y}`); };

// the height and the color of a building
const near = (a, b, m) => { if (Math.abs(a - b) > 1e-9) throw new Error(`${m}: ${a} != ${b}`); };
near(heightOf({ height: "15" }), 15, "plain"); near(heightOf({ height: "15.5 m" }), 15.5, "unit m"); near(heightOf({ height: "50 ft" }), 15.24, "feet");
near(heightOf({ height: "50'" }), 15.24, "feet mark"); near(heightOf({ "building:levels": "4" }), 12.8, "levels"); near(heightOf({}), 6, "default");
near(heightOf({ height: "tall", "building:levels": "2" }), 6.4, "a bad height falls back to the floors");
eq([cssColor("Red"), cssColor("#abc"), cssColor("#ABCDEF"), cssColor("x")], ["#ff0000", "#aabbcc", "#abcdef", null], "colors");

// a rule: the mode keys go to `overrides`, the tags go to `extra.tag_edits`
let doc = {};
F.writeRule(doc, "way", 5, { mode: { lanes: 3, oneway: true }, set: { sidewalk: "both" }, delete: ["roof:shape"], note: "why" });
eq(doc.overrides, [{ osm_id: 5, lanes: 3, oneway: true, note: "why" }], "overrides has the mode keys and the note");
eq(doc.extra.tag_edits, [{ type: "way", id: 5, set: { sidewalk: "both" }, delete: ["roof:shape"] }], "tag_edits has the tags");
eq(F.ruleOf(doc, "way", 5), { mode: { oneway: true, lanes: 3 }, set: { sidewalk: "both" }, delete: ["roof:shape"], was: {}, note: "why" }, "ruleOf reads both");
eq(F.describe(doc, "way", 5), "oneway true, lanes 3, sidewalk=both, no roof:shape", "describe");

// a rule with only tags has no `overrides` entry; a rule with nothing is removed, and an empty `extra` goes away
F.writeRule(doc, "way", 5, { mode: {}, set: { sidewalk: "both" }, delete: [], note: "" });
eq(doc.overrides, [], "no mode key: no overrides rule");
F.writeRule(doc, "way", 5, { mode: {}, set: {}, delete: [], note: "" });
eq([F.objects(doc), doc.extra], [[], undefined], "an empty rule is removed with its extra");
eq(F.dump({ dump: () => "x" }, doc), "", "no rule: no file");

// a duckOSM rule with `tags` counts as set tags; saving moves them to tag_edits
doc = { overrides: [{ osm_id: 7, lanes: 2, tags: { surface: "asphalt" }, keep_nodes: [1, 2] }] };
eq(F.ruleOf(doc, "way", 7).set, { surface: "asphalt" }, "overrides tags are set tags");
F.writeRule(doc, "way", 7, { mode: { lanes: 2 }, set: { surface: "asphalt" }, delete: [], note: "" });
eq(doc.overrides, [{ osm_id: 7, keep_nodes: [1, 2], lanes: 2 }], "keep_nodes stays, tags move out");
eq(doc.extra.tag_edits, [{ type: "way", id: 7, set: { surface: "asphalt" } }], "tags are in tag_edits");

// nodes and relations have tag edits only; the objects are listed in file order
F.writeRule(doc, "node", 9, { mode: {}, set: { crossing: "marked" }, delete: [], note: "" });
F.writeRule(doc, "relation", 3, { mode: {}, set: {}, delete: ["building"], note: "" });
eq(F.objects(doc), [{ type: "way", id: 7 }, { type: "node", id: 9 }, { type: "relation", id: 3 }], "objects");
F.deleteObjectRule(doc, "way", 7);
eq(F.objects(doc).map((o) => o.id), [9, 3], "deleteObjectRule");

// turn rules
F.upsertTurn(doc, { from_way: 1, via_node: 2, to_way: 3, restriction: "no_left_turn" });
F.upsertTurn(doc, { from_way: 1, via_node: 2, to_way: 3, restriction: "no_entry" });
eq(doc.turn_restrictions, [{ from_way: 1, via_node: 2, to_way: 3, restriction: "no_entry" }], "a turn rule is replaced, not repeated");
F.deleteTurn(doc, 1, 2, 3);
eq(F.counts(doc), { overrides: 0, turns: 0, tagEdits: 2, faces: 0, newWays: 0 }, "counts");
// the type of a wall: the nodes can be in either order; the computed type is no change; an empty `extra` goes away
doc = {};
F.setFace(doc, 5, 1, 2, "side", "front");
eq(doc.extra.building_faces, [{ building: 5, from_node: 1, to_node: 2, type: "side" }], "a wall rule");
eq(F.findFace(doc, 5, 2, 1)?.type, "side", "found with the nodes in the other order");
F.setFace(doc, 5, 2, 1, "party", "front");
eq(F.faceEdits(doc).length, 1, "a rule for the same wall is replaced");
F.setFace(doc, 5, 1, 2, "front", "front");
eq([F.faceEdits(doc), doc.extra], [[], undefined], "the computed type removes the rule and the empty extra");
F.setFace(doc, 5, 1, 2, "back", "side"); F.writeRule(doc, "way", 9, { mode: {}, set: { a: "b" }, delete: [], note: "" });
eq(F.counts(doc), { overrides: 0, turns: 0, tagEdits: 1, faces: 1, newWays: 0 }, "counts have the walls");
// `was`: the data before the change, for the keys that the rule still changes; a was-only edit goes when no key is left
doc = {};
F.writeRule(doc, "way", 4, { mode: { lanes: 3 }, set: { surface: "paved" }, delete: [], was: { lanes: "2", surface: null, gone: "x" }, note: "" });
eq(doc.extra.tag_edits, [{ type: "way", id: 4, set: { surface: "paved" }, was: { lanes: "2", surface: null } }], "was keeps the keys that are changed");
eq(F.describe(doc, "way", 4), "lanes 3 (was 2), surface=paved (was none)", "describe shows was");
F.writeRule(doc, "way", 4, { mode: {}, set: {}, delete: [], was: { lanes: "2" }, note: "" });
eq([F.objects(doc), doc.extra], [[], undefined], "no key left: no was either");
// new roads: ids from NEW_WAY_MIN up, the next id after the highest one; a deleted road leaves no empty list in the file
doc = {};
const n1 = F.addNewWay(doc, { nodes: [5, [7.4, 43.7]], tags: { highway: "service" } }), n2 = F.addNewWay(doc, { nodes: [[7.4, 43.7], [7.5, 43.7]], tags: { highway: "path" }, note: "x" });
eq([n1, n2], [F.NEW_WAY_MIN, F.NEW_WAY_MIN + 1], "new ids");
F.deleteNewWay(doc, n1);
eq(F.addNewWay(doc, { nodes: [1, 2], tags: {} }), F.NEW_WAY_MIN + 2, "an id is not used again while a higher one is there");
F.deleteNewWay(doc, n2); F.deleteNewWay(doc, F.NEW_WAY_MIN + 2);
eq([F.newWays(doc), F.dump({ dump: (x) => JSON.stringify(x) }, doc)], [[], ""], "no new road, no file");
// the number of a wall: no wall by default; the wall under the pointer, else the selected wall (a phone has no pointer)
eq([wallShown(null, null), wallShown("1|2", null), wallShown("1|2", "3|4"), wallShown(null, "3|4")], ["", "1|2", "1|2", "3|4"], "the wall with a number");
eq(wallFilter(""), ["==", ["coalesce", ["get", "k"], ""], "-"], "no wall: the filter matches no feature (a wall of another building has no key)");
eq(wallFilter("1|2"), ["==", ["coalesce", ["get", "k"], ""], "1|2"], "one wall");
// two or more records of one object (a file by hand): the editor reads them joined as the build applies them, in file order (the later key wins)
doc = { overrides: [{ osm_id: 5, lanes: 2, tags: { name: "A" }, exclude_modes: ["walking"] }, { osm_id: 6, lanes: 1 }, { osm_id: 5, lanes: 3, oneway: true, tags: { surface: "paved" }, exclude_modes: ["cycling"] }],
  extra: { tag_edits: [{ type: "way", id: 5, set: { name: "B", maxspeed: "30" }, was: { name: "A0" } }, { type: "way", id: 5, set: { maxspeed: "50" }, delete: ["name"], was: { maxspeed: "20" } }] } };
eq(F.count(doc, { type: "way", id: 5 }), 2, "two records of way 5");
eq(F.findRule(doc, 5), { osm_id: 5, lanes: 3, oneway: true, tags: { name: "A", surface: "paved" }, exclude_modes: ["walking", "cycling"] }, "the overrides joined: the later key wins, the modes and the tags of both");
eq(F.findTagEdit(doc, "way", 5), { type: "way", id: 5, set: { maxspeed: "50" }, delete: ["name"], was: { name: "A0", maxspeed: "20" } }, "the tag edits joined: a later delete wins over an earlier set");
F.join(doc, { type: "way", id: 5 });
eq([F.count(doc, { type: "way", id: 5 }), doc.overrides.map((r) => r.osm_id)], [1, [5, 6]], "join: one record, in the place of the first");
eq(doc.extra.tag_edits, [{ type: "way", id: 5, set: { maxspeed: "50" }, delete: ["name"], was: { name: "A0", maxspeed: "20" } }], "join: one tag edit");
// a save with duplicates in the file writes one record: the edit cannot go under a later record
doc = { extra: { tag_edits: [{ type: "node", id: 9, set: { highway: "crossing" } }, { type: "node", id: 9, set: { highway: "stop" } }] } };
F.writeRule(doc, "node", 9, { mode: {}, set: { highway: "traffic_signals" }, delete: [], note: "" });
eq(doc.extra.tag_edits, [{ type: "node", id: 9, set: { highway: "traffic_signals" } }], "a write replaces all the records of the object");
// remove: all the records go
doc = { overrides: [{ osm_id: 5, lanes: 2 }, { osm_id: 5, remove: true }], extra: { tag_edits: [{ type: "way", id: 5, set: { a: "1" } }, { type: "way", id: 5, set: { b: "2" } }] } };
eq(F.findRule(doc, 5).remove, true, "remove of any record");
F.remove(doc, { type: "way", id: 5 });
eq([doc.overrides, doc.extra], [[], undefined], "remove: every record of the way goes");
// walls: the build keeps the last rule of a wall (all of it); join keeps that one, remove takes all
doc = { extra: { building_faces: [{ building: 5, from_node: 1, to_node: 2, type: "back", door: "main" }, { building: 5, from_node: 3, to_node: 4, type: "side" }, { building: 5, from_node: 2, to_node: 1, type: "party" }] } };
eq([F.count(doc, { building: 5, from_node: 1, to_node: 2 }), F.findFace(doc, 5, 1, 2)], [2, { building: 5, from_node: 2, to_node: 1, type: "party" }], "the later wall rule wins, with no door");
eq(F.wallsOf(doc, 5), [{ from_node: 1, to_node: 2 }, { from_node: 3, to_node: 4 }], "the walls of a building that have a rule");
F.join(doc, { building: 5, from_node: 1, to_node: 2 });
eq(F.faceEdits(doc), [{ building: 5, from_node: 2, to_node: 1, type: "party" }, { building: 5, from_node: 3, to_node: 4, type: "side" }], "join: one wall rule in the place of the first");
F.remove(doc, { building: 5, from_node: 1, to_node: 2 }); F.remove(doc, { building: 5, from_node: 3, to_node: 4 });
eq(doc.extra, undefined, "remove: no wall rule, no extra");
// turns and new roads
doc = { turn_restrictions: [{ from_way: 1, via_node: 2, to_way: 3, restriction: "no_left_turn" }, { from_way: 1, via_node: 2, to_way: 3, restriction: "no_u_turn" }] };
eq([F.count(doc, { from_way: 1, via_node: 2, to_way: 3 }), F.findTurn(doc, 1, 2, 3).restriction], [2, "no_u_turn"], "the later turn rule");
F.join(doc, { from_way: 1, via_node: 2, to_way: 3 });
eq(doc.turn_restrictions, [{ from_way: 1, via_node: 2, to_way: 3, restriction: "no_u_turn" }], "join: one turn rule");
const nw = F.NEW_WAY_MIN;
doc = { new_ways: [{ id: nw, nodes: [1, 2], tags: { highway: "path" } }, { id: nw, nodes: [1, 3], tags: { highway: "service" }, note: "x" }] };
eq([F.count(doc, { new_way: nw }), F.findNewWay(doc, nw)], [2, { id: nw, nodes: [1, 3], tags: { highway: "service" }, note: "x" }], "the new roads joined");
F.updateNewWayTags(doc, nw, { highway: "track" });
eq(doc.new_ways, [{ id: nw, nodes: [1, 3], tags: { highway: "track" }, note: "x" }], "a tag change writes one new road");
F.remove(doc, { new_way: nw });
eq(F.newWays(doc), [], "remove: the new road goes");
console.log("fixes_units.mjs: ok");
