// duckosm ask (docs/design/ask_page.md): the host of roadstyle's data dock (one SQL line over a wide table under the map) and Ask (a question in words,
// answered with SQL) on roadstyle's standard dashboard of a duckOSM database. The page is roadstyle's (the server made it); this script gives the two parts
// what only the host has: `query` (the server runs each statement on the database, read only), `schema`, and the changes of the fixes file as the table
// `extra.edits`. Roads and buildings are clickable. The two parts are roadstyle's own modules, which the server gives at /roadstyle/.
(() => {
  const $ = (id) => document.getElementById(id);
  const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;");
  const api = async (path, init) => {
    const r = await fetch("api/" + path, init);
    if (!r.ok) throw new Error((await r.text()) || r.statusText);
    return r;
  };
  const script = (src) => new Promise((ok, no) => document.head.append(Object.assign(document.createElement("script"), { src, crossOrigin: "anonymous", onload: ok, onerror: () => no(new Error("cannot load " + src)) })));

  const BIG = /^(U?BIGINT|U?HUGEINT)$/;
  // a value that the data dock can show: a 64-bit integer as a BigInt (it came as a text), a list or a struct as JSON
  const cell = (v, type) => (v == null ? null : BIG.test(type) ? BigInt(v) : typeof v === "object" ? JSON.stringify(v) : v);

  // the records of the fixes file as the rows of one table: one row for each object, wall, turn and new road, with the time of its latest record
  function editRows(F, doc) {
    const out = [], at = (recs) => { const t = F.latest(recs); return Number.isNaN(t) ? null : new Date(t).toISOString(); };
    for (const o of F.objects(doc)) out.push(["object", o.type, o.id, F.describe(doc, o.type, o.id), at(F.recordsOf(doc, o)), F.ruleOf(doc, o.type, o.id).note || null]);
    const walls = new Map();
    for (const e of F.faceEdits(doc)) walls.set(`${e.building}|${Math.min(e.from_node, e.to_node)}|${Math.max(e.from_node, e.to_node)}`, e);
    for (const e of walls.values()) out.push(["wall", "building", e.building, `${e.type}${e.door ? `, door ${e.door}` : ""}, nodes ${e.from_node} and ${e.to_node}`, at(F.recordsOf(doc, e)), e.note || null]);
    const turns = new Map();
    for (const t of doc.turn_restrictions ?? []) turns.set(`${t.from_way}|${t.via_node}|${t.to_way}`, t);
    for (const t of turns.values()) out.push(["turn", "way", t.from_way, `${t.restriction}: way ${t.from_way} to way ${t.to_way} at node ${t.via_node}`, at(F.recordsOf(doc, t)), t.note || null]);
    for (const id of new Set(F.newWays(doc).map((w) => w.id))) {
      const w = F.findNewWay(doc, id);
      out.push(["new_road", "way", id, `${w.tags?.highway ?? "road"}${w.tags?.name ? ` ${w.tags.name}` : ""}, ${w.nodes.length} points`, at(F.recordsOf(doc, { new_way: id })), w.note || null]);
    }
    return out;
  }

  async function start() {
    // h3-js: the H3 cells of the boundary and their shapes. DuckDB here has no H3 functions, so the page computes them; without it (no network) the page has no H3 table.
    const h3Loaded = script("https://cdn.jsdelivr.net/npm/h3-js@4.1.0/dist/h3-js.umd.js").then(() => !!window.h3, () => false);
    const [info] = await Promise.all([api("info").then((r) => r.json()), script("https://cdn.jsdelivr.net/npm/js-yaml@4.1.0/dist/js-yaml.min.js")]);
    // QUESTIONS and S: the questions to ask (web/ask/questions.js) and what Ask knows about the tables (web/ask/schema.js, docs/design/ask_schema.md): one file each for this page and
    // for duckOSM Studio (docs/guides/ask-questions.md). roadstyle's Ask shows its
    // general groups first (any data), then these groups, which are about the data of duckOSM.
    const [F, { dataDock }, { askPanel }, { QUESTIONS }, S, V] = await Promise.all([import("./fixes_editor/fixes.js"), import("./roadstyle/data/data_dock.js"), import("./roadstyle/ask/ask_panel.js"), import("./ask/questions.js"), import("./ask/schema.js"), import("./ask/views.js")]);

    const query = async (sql, { signal } = {}) => {
      const { columns, rows } = await (await api("sql", { method: "POST", body: sql, signal })).json();
      return { columns, rows: rows.map((r) => r.map((v, i) => cell(v, columns[i].type))) };
    };
    // the changes of the fixes file: the table extra.edits (an empty table when there is no file, so that a question about the changes has an answer)
    let edits = 0;
    const loadEdits = async () => {
      let doc = {};
      if (info.exists) { try { doc = F.parse(window.jsyaml, await (await api("fixes")).text()); } catch (e) { console.warn("duckosm ask: the fixes file is not valid:", e.message); } }
      const rows = editRows(F, doc);
      const columns = [["kind", "VARCHAR"], ["type", "VARCHAR"], ["osm_id", "BIGINT"], ["what", "VARCHAR"], ["changed", "TIMESTAMPTZ"], ["note", "VARCHAR"]].map(([name, type]) => ({ name, type }));
      await api("table", { method: "POST", body: JSON.stringify({ name: "edits", columns, rows }) });
      edits = rows.length;
    };
    const names = info.tables.map((t) => `${t.mode}.${t.table}`);
    const main = S.mainEdges(names);

    const schema = async () => {
      const { rows } = await query("SELECT schema_name, table_name, estimated_size, database_name FROM duckdb_tables() WHERE database_name IN (current_database(), 'extra') ORDER BY 1, 2");
      return S.describeTables(rows.map(([s, t, n, d]) => ({ name: d === "extra" ? `extra.${t}` : `${s}.${t}`, rows: Number(n) })), main, query);
    };

    // ---- the shapes of a result on the map. roadstyle's Ask lights roads only ("I can only show roads"), so the page draws the geometry of the table itself:
    // a result with a geometry column (geom of driving.nodes, features.buildings, building_faces) is drawn as it is: nodes as points, buildings as shapes.
    // The limit of 1000 is for the rows that the TABLE shows, not for the selection: the page runs the same SQL again, for the geometry only, with no limit,
    // so "show all buildings" draws every building. A new table with no geometry clears the shapes (as roadstyle's light follows the table).
    // ponytail: no cap, so a result of millions of shapes (a county of buildings) is slow in the browser; add a cap or tiles if that comes.
    const SHAPES = "ask-shapes", PURPLE = "#7c4dff", EMPTY = { type: "FeatureCollection", features: [] };
    // the colour of a shape: the column `color` of the result (a CSS colour, so Ask can colour by any rule in SQL), else its `face_type` in the colours of the editor's
    // walls (web/fixes_editor/fixes_panel.js, FACE_COLORS), else roadstyle's purple
    const FACE = { front: "#16a34a", back: "#dc2626", side: "#6b7280", party: "#7c3aed", court: "#0891b2" };
    const COLOUR = ["coalesce", ["get", "color"], ["match", ["get", "face_type"], ...Object.entries(FACE).flat(), PURPLE]];
    let shapes = EMPTY, drawn = 0, ticket = 0, shapesOn = true;
    // the opacity of the shapes of a result (the tool set_shapes of Ask): null is the default look (the fill 0.4, the outline and the points 1)
    let shapeOpacity = null;
    const applyOpacity = () => {
      const map = window.map, o = shapeOpacity;
      if (map.getLayer("ask-shapes-fill")) map.setPaintProperty("ask-shapes-fill", "fill-opacity", o ?? 0.4);
      if (map.getLayer("ask-shapes-line")) map.setPaintProperty("ask-shapes-line", "line-opacity", o ?? 1);
      if (map.getLayer("ask-shapes-pt")) { map.setPaintProperty("ask-shapes-pt", "circle-opacity", o ?? 1); map.setPaintProperty("ask-shapes-pt", "circle-stroke-opacity", o ?? 1); }
      // the objects that roadstyle's data dock draws for a kind (H3 cells, nodes, turns) have the same switch
      for (const [id, props] of [["rs-dock-kinds-fill", ["fill-opacity"]], ["rs-dock-kinds-line", ["line-opacity"]], ["rs-dock-kinds-dot", ["circle-opacity", "circle-stroke-opacity"]]]) if (map.getLayer(id)) for (const prop of props) map.setPaintProperty(id, prop, o ?? 1);
    };
    // is the table shown on the map now (the switch of the dock is On and the dock is open)? The dock keeps the switch in this browser, so after a reload it can start Off
    // and no event `light` comes: the page asks data.light() each time that it draws, and does not wait for the event only.
    const lit = () => { try { return data.light ? !!data.light() : true; } catch { return true; } };
    const addShapeLayers = () => {
      const map = window.map, polygon = ["in", ["geometry-type"], ["literal", ["Polygon", "MultiPolygon"]]], point = ["in", ["geometry-type"], ["literal", ["Point", "MultiPoint"]]];
      if (map.getSource(SHAPES)) return;
      map.addSource(SHAPES, { type: "geojson", data: shapes });
      const layout = { visibility: shapesOn ? "visible" : "none" };
      map.addLayer({ id: "ask-shapes-fill", type: "fill", source: SHAPES, filter: polygon, layout, paint: { "fill-color": COLOUR, "fill-opacity": 0.4 } });
      map.addLayer({ id: "ask-shapes-line", type: "line", source: SHAPES, filter: ["!", point], layout, paint: { "line-color": COLOUR, "line-width": 3 } });
      map.addLayer({ id: "ask-shapes-pt", type: "circle", source: SHAPES, filter: point, layout, paint: { "circle-color": COLOUR, "circle-radius": 4, "circle-stroke-color": "#fff", "circle-stroke-width": 1.2 } });
      applyOpacity();                                                  // new layers (a new base map, the first result) keep the opacity that Ask set
    };
    // the map switch of the data dock (On or Off, roadstyle): the shapes show with the table's roads. Off, or a closed dock: the map has its own colours only.
    const showShapes_ = (on) => { shapesOn = on; for (const id of ["ask-shapes-fill", "ask-shapes-line", "ask-shapes-pt"]) if (window.map.getLayer(id)) window.map.setLayoutProperty(id, "visibility", on ? "visible" : "none"); };
    const showShapes = (features) => {
      shapes = { type: "FeatureCollection", features }; drawn = features.length; shapesOn = lit();
      addShapeLayers(); window.map.getSource(SHAPES).setData(shapes); showShapes_(shapesOn);
    };
    window.map.on("styledata", () => { if (drawn && !window.map.getSource(SHAPES)) showShapes(shapes.features); });          // a new base map drops the layers
    const bounds = (features) => {
      let b = null;
      const walk = (c) => { if (typeof c[0] === "number") { b = b ? [Math.min(b[0], c[0]), Math.min(b[1], c[1]), Math.max(b[2], c[0]), Math.max(b[3], c[1])] : [c[0], c[1], c[0], c[1]]; } else c.forEach(walk); };
      features.forEach((f) => walk(f.geometry.coordinates));
      return b;
    };
    const drawResult = async (d) => {
      // A result with the id of a road (edge_id) is lit by roadstyle on the road's own shape: the page draws no line over it (the geometry column of an edges table
      // is the centre line of the road, and a thin line over the road shows the road twice). The shapes are for what roadstyle cannot light.
      const col = d.columns.find((c) => /^GEOMETRY$/i.test(c.type)), mine = ++ticket;
      const q = (n) => '"' + n.replace(/"/g, '""') + '"';
      if (!col || d.columns.some((c) => c.name === "edge_id")) { if (drawn) showShapes([]); return; }
      const name = q(col.name);
      const kept = ["face_type", "color"].filter((n) => d.columns.some((c) => c.name === n));         // the columns that colour a shape
      const { rows } = await query(`SELECT ST_AsGeoJSON(${name}) AS g${kept.map((n) => `, CAST(${q(n)} AS VARCHAR) AS ${q(n)}`).join("")} FROM (${d.sql.trim().replace(/;\s*$/, "")}) duckosm_shapes WHERE ${name} IS NOT NULL`);
      if (mine !== ticket || owned) return;                         // a newer table came while this one loaded, or a kind owns the result
      const features = rows.map(([g, ...vals]) => {
        const properties = {};
        kept.forEach((n, i) => { if (vals[i] != null && !(n === "color" && !CSS.supports("color", vals[i]))) properties[n] = vals[i]; });        // a colour that CSS does not know is left out
        try { return { type: "Feature", properties, geometry: JSON.parse(g) }; } catch { return null; }
      }).filter((f) => f?.geometry);
      showShapes(features);
      const b = features.length && d.source !== "person" ? bounds(features) : null;             // the camera follows an answer of Ask, not the person's own SQL
      if (b) window.map.fitBounds([[b[0], b[1]], [b[2], b[3]]], { padding: { left: 420, top: 60, right: 60, bottom: 320 }, maxZoom: 18, duration: 800 });
    };

    // the H3 cells of the boundary, for each resolution 0 to 11: the cells of the boundary's vertices, and from resolution 5 the cells inside it. `whole_boundary` is true for
    // the one cell that holds all the vertices (a cell is convex, so it holds the whole boundary): the smallest such cell has the largest resolution with the flag.
    const loadH3 = async () => {
      if (!(await h3Loaded)) return;
      const { rows: g } = await query("SELECT ST_AsGeoJSON(geom) FROM main.boundary");
      const polys = g.flatMap(([x]) => { const j = JSON.parse(x); return j.type === "Polygon" ? [j.coordinates] : j.type === "MultiPolygon" ? j.coordinates : []; });
      if (!polys.length) return;
      const out = [];
      for (let r = 0; r <= 11; r++) {
        const vertex = new Set(), cells = new Set();
        for (const poly of polys) for (const ring of poly) for (const [lng, lat] of ring) vertex.add(window.h3.latLngToCell(lat, lng, r));
        vertex.forEach((c) => cells.add(c));
        if (r >= 5) for (const poly of polys) for (const c of window.h3.polygonToCells(poly, r, true)) cells.add(c);
        for (const c of cells) out.push([r, c, vertex.size === 1]);
      }
      const columns = [["resolution", "BIGINT"], ["h3_cell", "VARCHAR"], ["whole_boundary", "BOOLEAN"]].map(([name, type]) => ({ name, type }));
      await api("table", { method: "POST", body: JSON.stringify({ name: "boundary_h3", columns, rows: out }) });
    };
    const ready = Promise.all([loadEdits(), loadH3()]).catch((e) => console.warn("duckosm ask: a table of the page did not load:", e.message));
    // the kinds of object that a row can be (docs/design/table_views.md, web/ask/views.js): roadstyle draws the object of a row, selects it and fills the card.
    // Until roadstyle can colour a kind and draw a wall (docs/design/dock_kinds_table_views.md, steps 2 to 4), a result that has a geometry column keeps the page's own
    // drawing below (the colour column, the walls by face_type). So a kind answers only a result with ids and no geometry column (a via_node, a way_id, a route_id).
    const hasGeom = () => (data.result?.()?.columns || []).some((c) => /^GEOMETRY$/i.test(c.type));          // the dock sets its result before it asks a kind
    const NONE = "SELECT NULL::BIGINT AS id, NULL::VARCHAR AS geom WHERE false";
    const kinds = V.viewKinds({ names: info.tables.map((t) => `${t.mode}.${t.table}`), hasTags: await S.hasTags(query), h3: (await h3Loaded) ? window.h3 : null, query });
    for (const k of Object.values(kinds)) {
      const by = k.geometryBy, of = k.geometryOf;
      if (by) k.geometryBy = (sql, column) => (hasGeom() ? NONE : by(sql, column));
      if (of) k.geometryOf = (list, opts) => (hasGeom() ? [] : of(list, opts));
    }
    const data = dataDock({
      query, schema, ready,
      roadsBy: S.roadsBy(main), sql: S.startSql(main),
      kinds,
    });
    // a kind owns the result: the page draws nothing for it, and the object of a kind has its card (roadstyle's event `kind`; a new result resets it)
    let owned = false;
    data.addEventListener("run", () => { owned = false; });
    data.addEventListener("kind", (e) => { owned = !!e.detail.name; if (owned) { ticket++; if (drawn) showShapes([]); clearPick(); window.map.once("idle", applyOpacity); } });
    data.addEventListener("kinderror", (e) => console.warn(`duckosm ask: the kind ${e.detail.kind} failed:`, e.detail.message));
    // ---- a row of the table: its item on the map, and its facts in the card (roadstyle's data dock: the event `row`, data.card(...) and the event `cardclose`).
    // A road row (the result has edge_id) is selected by roadstyle itself, with its own card. A row that has a geometry column (a node, a building, a wall) is drawn here in
    // the select look (roadstyle's solid purple), the map zooms to it, and the card has all its values except the geometry. A new selection of roadstyle, a new result and the
    // close button of the card send `cardclose`: the drawing goes then. The map switch (On or Off) is for the table only: the selected item stays on the map, also Off.
    const PICK = "ask-pick";
    let picked = 0;
    const addPickLayers = () => {
      const map = window.map, polygon = ["in", ["geometry-type"], ["literal", ["Polygon", "MultiPolygon"]]], point = ["in", ["geometry-type"], ["literal", ["Point", "MultiPoint"]]];
      if (map.getSource(PICK)) return;
      map.addSource(PICK, { type: "geojson", data: EMPTY });
      const layout = { visibility: "visible" };                      // the selected item has no switch: it is in the card, and it stays on the map
      map.addLayer({ id: "ask-pick-fill", type: "fill", source: PICK, filter: polygon, layout, paint: { "fill-color": PURPLE, "fill-opacity": 0.85 } });
      map.addLayer({ id: "ask-pick-line", type: "line", source: PICK, filter: ["!", point], layout, paint: { "line-color": PURPLE, "line-width": 6 } });
      map.addLayer({ id: "ask-pick-pt", type: "circle", source: PICK, filter: point, layout, paint: { "circle-color": PURPLE, "circle-radius": 9, "circle-stroke-color": "#fff", "circle-stroke-width": 2.5 } });
    };
    const showPick = (features) => {
      addPickLayers(); picked = features.length;
      window.map.getSource(PICK).setData({ type: "FeatureCollection", features });
    };
    window.map.on("styledata", () => { if (picked && !window.map.getSource(PICK)) showPick(pickedFeatures); });          // a new base map drops the layers
    let pickedFeatures = [], rowTicket = 0, lastSql = "";
    const clearPick = () => { rowTicket++; pickedFeatures = []; if (picked) showPick([]); };
    data.addEventListener("light", (e) => showShapes_(e.detail.on));                  // the switch is for the table (the shapes of the result), not for the selected item
    data.addEventListener("cardclose", clearPick);
    data.addEventListener("run", (e) => { lastSql = e.detail.sql; drawResult(e.detail).catch((err) => console.warn("duckosm ask: the shapes did not draw:", err.message)); });
    data.addEventListener("row", async (e) => {
      if (owned) return;                                            // the object of a kind: roadstyle selects it and fills the card
      const { columns, row } = e.detail, mine = ++rowTicket;
      const g = columns.findIndex((c) => /^GEOMETRY$/i.test(c.type));
      const named = columns.findIndex((c, i) => c.name === "name" && row[i] != null), idCol = columns.findIndex((c) => /(^|_)id$/.test(c.name));
      const table = (lastSql.match(/\bFROM\s+([\w."]+)/i) || [])[1]?.replace(/"/g, "") ?? "row";
      const card = (title, skip, id) => data.card({ title, chip: table, rows: columns.map((c, i) => [c.name.replace(/_/g, " "), i === skip ? null : row[i]]).filter(([, v]) => v != null), id });
      if (columns.some((c) => c.name === "edge_id") || g < 0 || row[g] == null) { clearPick(); return; }      // a road: roadstyle selects it; no geometry: nothing to draw
      const wkt = String(row[g]).replace(/'/g, "''");
      const { rows } = await query(`SELECT ST_AsGeoJSON(ST_GeomFromText('${wkt}')) AS g`);
      if (mine !== rowTicket) return;                                 // another row was clicked while this one loaded
      let geometry; try { geometry = JSON.parse(rows[0][0]); } catch { return; }
      pickedFeatures = [{ type: "Feature", properties: {}, geometry }];
      showPick(pickedFeatures);
      const b = bounds(pickedFeatures);
      if (b) window.map.fitBounds([[b[0], b[1]], [b[2], b[3]]], { padding: { left: 420, top: 60, right: 60, bottom: 320 }, maxZoom: 19, duration: 600 });
      card(named >= 0 ? String(row[named]) : `${table} ${idCol >= 0 ? row[idCol] : e.detail.index + 1}`, g, idCol >= 0 ? String(row[idCol]) : undefined);
    });
    // what Ask is told about the tables (web/ask/schema.js): the tables of the file, the page's own tables (extra.edits always, extra.boundary_h3 with h3-js) and the real road classes
    const aboutText = S.aboutText({ tables: [...names, "extra.edits", ...((await h3Loaded) ? ["extra.boundary_h3"] : [])], classes: await S.roadClasses(query, names), shapes: true, kinds: true, hasTags: await S.hasTags(query) });
    // the page's own tool for Ask (roadstyle's `tools` option): set_map changes roadstyle's layers only, and the shapes of a result (H3 cells, buildings, walls, nodes) are
    // the page's own layer (Kaveh: "increase transparency of h3 cells on map", and Ask answered that it could not)
    const shapeTools = [{
      name: "set_shapes",
      description: "Change how transparent the shapes are that the page draws for a result (H3 cells, buildings, walls, nodes). opacity: 0 is invisible, 1 is solid; by default the fill of a shape is 0.4 and its outline and points are 1. reset: true brings the default look back. It does not change the layers of the map (use set_map for those).",
      parameters: { opacity: { type: "number", description: "0 to 1" }, reset: { type: "boolean", description: "true: the default look" } },
      required: [],
      step: () => "Changing the shapes",
      run: async (a) => {
        if (a.reset) shapeOpacity = null;
        else if (a.opacity != null) { const o = Number(a.opacity); if (!(o >= 0 && o <= 1)) throw new Error("opacity must be a number from 0 to 1"); shapeOpacity = o; }
        else throw new Error("give opacity (0 to 1) or reset: true");
        applyOpacity();
        return { done: shapeOpacity == null ? "The shapes have the default look." : `The shapes have opacity ${shapeOpacity}.`, shapes_now: drawn };
      },
    }];
    const ask = askPanel({
      data, dialect: "DuckDB", maxSteps: 8, tools: shapeTools,
      about: aboutText,
      examples: QUESTIONS,
    });
    // roads and buildings are clickable: roadstyle selects the clicked item, and the data dock's card shows its facts (a road, or an item of a base-map layer
    // such as a building). No popup anywhere: rs.holdPopup holds the popups of roads and of items, here for the whole page (the dock holds them only while it is open).
    // A click must not run a query: a new table with no geometry would clear the shapes of the map.
    window.rsSetInteraction?.("buildings", { clickable: true, popup: false });
    window.rs.holdPopup?.("ask-page", true);
    window.rs.sections.add({ id: "ask", label: "Ask the data", note: "A question about the OSM data and your changes", open: () => ask.open(), icon: "ask", background: true });
    window.rs.sections.add({ id: "sql", label: "Query the tables", note: "One SQL line over a wide table", open: () => data.open(), icon: "ask", background: true });
    await ready;
    ask.open();
    window.askPage = { data, ask, about: aboutText, shapeTools, opacity: () => shapeOpacity, questions: QUESTIONS, edits: () => edits, shapes: () => drawn, shapeProps: () => shapes.features.map((f) => f.properties), picked: () => picked, shapesShown: lit };            // for the tests (tests/test_ask_page.py)
  }
  const go = () => start().catch((e) => { console.error(e); document.body.insertAdjacentHTML("beforeend", `<p id="ask-error" role="alert">duckosm ask: ${esc(e.message)}</p>`); });
  window.rsReady ? go() : document.addEventListener("rs:ready", go, { once: true });
})();
