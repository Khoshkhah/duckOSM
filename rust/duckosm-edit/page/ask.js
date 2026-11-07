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

  // what each table of a duckOSM file holds (the agent and the Tables list read it)
  const ABOUT = {
    edges: "One row is one direction of a road (an edge), with its length, speed, lanes, cost and OSM tags.",
    nodes: "The junctions and the ends of the roads.",
    edge_graph: "The legal turns: one row for each move from an edge to the next edge.",
    private_edges: "The roads that this network keeps out: private, forbidden or bus only.",
    turn_restrictions: "The OSM turn restrictions.",
    building_faces: "One row is one wall of a building, with its type (front, back, side, party, court) and the type that the build found (computed_type).",
    building_entrances: "The entrances and doors of the buildings.",
    buildings: "The buildings of the base map, with their OSM tags.",
    edits: "The changes in the fixes file: kind (object, wall, turn, new_road), type, osm_id, what, changed (the time), note.",
  };
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
    const [info] = await Promise.all([api("info").then((r) => r.json()), script("https://cdn.jsdelivr.net/npm/js-yaml@4.1.0/dist/js-yaml.min.js")]);
    const [F, { dataDock }, { askPanel }] = await Promise.all([import("./fixes_editor/fixes.js"), import("./roadstyle/data/data_dock.js"), import("./roadstyle/ask/ask_panel.js")]);

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
    const edgesTable = info.tables.find((t) => t.mode === "driving" && t.table === "edges") ?? info.tables.find((t) => t.table === "edges");
    const main = edgesTable && `${edgesTable.mode}.edges`;

    const schema = async () => {
      const { rows } = await query("SELECT schema_name, table_name, estimated_size, database_name FROM duckdb_tables() WHERE database_name IN (current_database(), 'extra') ORDER BY 1, 2");
      const tables = rows.map(([s, t, n, d]) => ({ name: d === "extra" ? `extra.${t}` : `${s}.${t}`, rows: Number(n), about: ABOUT[t] ?? "" }));
      const first = tables.find((t) => t.name === main);
      if (first) {
        const [s, t] = main.split(".");
        const { rows: cols } = await query(`SELECT column_name, data_type FROM information_schema.columns WHERE table_schema = '${s}' AND table_name = '${t}' AND table_catalog = current_database()`);
        Object.assign(first, { main: true, columns: cols.map(([name, type]) => ({ name, type })) });
      }
      return { tables };
    };

    // ---- the shapes of a result on the map. roadstyle's Ask lights roads only ("I can only show roads"), so the page draws the geometry of the table itself:
    // a result with a geometry column (geom of driving.nodes, features.buildings, building_faces) is drawn as it is: nodes as points, buildings as shapes.
    // The limit of 1000 is for the rows that the TABLE shows, not for the selection: the page runs the same SQL again, for the geometry only, with no limit,
    // so "show all buildings" draws every building. A new table with no geometry clears the shapes (as roadstyle's light follows the table).
    // ponytail: no cap, so a result of millions of shapes (a county of buildings) is slow in the browser; add a cap or tiles if that comes.
    const SHAPES = "ask-shapes", PURPLE = "#7c4dff", EMPTY = { type: "FeatureCollection", features: [] };
    let shapes = EMPTY, drawn = 0, ticket = 0, shapesOn = true;
    // is the table shown on the map now (the switch of the dock is On and the dock is open)? The dock keeps the switch in this browser, so after a reload it can start Off
    // and no event `light` comes: the page asks data.light() each time that it draws, and does not wait for the event only.
    const lit = () => { try { return data.light ? !!data.light() : true; } catch { return true; } };
    const addShapeLayers = () => {
      const map = window.map, polygon = ["in", ["geometry-type"], ["literal", ["Polygon", "MultiPolygon"]]], point = ["in", ["geometry-type"], ["literal", ["Point", "MultiPoint"]]];
      if (map.getSource(SHAPES)) return;
      map.addSource(SHAPES, { type: "geojson", data: shapes });
      const layout = { visibility: shapesOn ? "visible" : "none" };
      map.addLayer({ id: "ask-shapes-fill", type: "fill", source: SHAPES, filter: polygon, layout, paint: { "fill-color": PURPLE, "fill-opacity": 0.4 } });
      map.addLayer({ id: "ask-shapes-line", type: "line", source: SHAPES, filter: ["!", point], layout, paint: { "line-color": PURPLE, "line-width": 2 } });
      map.addLayer({ id: "ask-shapes-pt", type: "circle", source: SHAPES, filter: point, layout, paint: { "circle-color": PURPLE, "circle-radius": 4, "circle-stroke-color": "#fff", "circle-stroke-width": 1.2 } });
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
      if (!col || d.columns.some((c) => c.name === "edge_id")) { if (drawn) showShapes([]); return; }
      const name = '"' + col.name.replace(/"/g, '""') + '"';
      const { rows } = await query(`SELECT ST_AsGeoJSON(${name}) AS g FROM (${d.sql.trim().replace(/;\s*$/, "")}) duckosm_shapes WHERE ${name} IS NOT NULL`);
      if (mine !== ticket) return;                                  // a newer table came while this one loaded
      const features = rows.map(([g]) => { try { return { type: "Feature", properties: {}, geometry: JSON.parse(g) }; } catch { return null; } }).filter((f) => f?.geometry);
      showShapes(features);
      const b = features.length && d.source !== "person" ? bounds(features) : null;             // the camera follows an answer of Ask, not the person's own SQL
      if (b) window.map.fitBounds([[b[0], b[1]], [b[2], b[3]]], { padding: { left: 420, top: 60, right: 60, bottom: 320 }, maxZoom: 18, duration: 800 });
    };

    const ready = loadEdits().catch((e) => console.warn("duckosm ask: the changes did not load:", e.message));
    const data = dataDock({
      query, schema, ready,
      roadsBy: (sql, column) => `SELECT DISTINCT edge_id FROM ${main} WHERE "${column}" IN (SELECT "${column}" FROM (${sql}))`,
      sql: main ? `SELECT highway, count(*) AS edges, round(sum(length_m) / 1000, 1) AS km FROM ${main} GROUP BY highway ORDER BY edges DESC` : "SELECT 1",
    });
    data.addEventListener("light", (e) => showShapes_(e.detail.on));
    data.addEventListener("run", (e) => drawResult(e.detail).catch((err) => console.warn("duckosm ask: the shapes did not draw:", err.message)));
    const ask = askPanel({
      data, dialect: "DuckDB",
      about: "The OSM data of one area, built by duckOSM. Each network is a schema: driving, walking and cycling. In a network, edges has one row for each direction of a road; "
        + "edge_id is its id and osm_id the OSM way. main.building_faces has the walls of the buildings, and the schema features has the buildings and the other base-map layers. "
        + "extra.edits has the changes that the person made in the fixes file (osm_id is the OSM id of the changed object; changed is the time of the change). "
        + "THE PAGE ALSO DRAWS A RESULT THAT HAS A GEOMETRY COLUMN ON THE MAP, as it is: nodes as points, buildings as shapes, walls as lines. So you CAN show nodes, buildings and walls: "
        + "run_query with their geom column (driving.nodes: node_id, geom; features.buildings: osm_id, name, geom; main.building_faces: building_id, face_type, geometry), "
        + "for example SELECT node_id, geom FROM driving.nodes. For \"all\" of them, add no LIMIT and no WHERE: the table shows only the first 1000 rows, and the map shows every row that you select. "
        + "ROADS are lit by the page itself on their own shape when the result has the column edge_id: select edge_id for roads, and not their geometry. "
        + "Never answer that you can only show roads.",
      examples: ["Which objects did I change, and when?", "Show all buildings on the map.", "Show all nodes of the driving network on the map.",
        "How many walls of each type do the buildings have?", "Show the walls that I changed by hand.", "Draw a bar chart of the km of road in each class."],
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
    window.askPage = { data, ask, edits: () => edits, shapes: () => drawn, shapesShown: lit };            // for the tests (tests/test_ask_page.py)
  }
  const go = () => start().catch((e) => { console.error(e); document.body.insertAdjacentHTML("beforeend", `<p id="ask-error" role="alert">duckosm ask: ${esc(e.message)}</p>`); });
  window.rsReady ? go() : document.addEventListener("rs:ready", go, { once: true });
})();
