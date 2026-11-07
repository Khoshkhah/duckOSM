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

    const ready = loadEdits().catch((e) => console.warn("duckosm ask: the changes did not load:", e.message));
    const data = dataDock({
      query, schema, ready,
      roadsBy: (sql, column) => `SELECT DISTINCT edge_id FROM ${main} WHERE "${column}" IN (SELECT "${column}" FROM (${sql}))`,
      sql: main ? `SELECT highway, count(*) AS edges, round(sum(length_m) / 1000, 1) AS km FROM ${main} GROUP BY highway ORDER BY edges DESC` : "SELECT 1",
    });
    const ask = askPanel({
      data, dialect: "DuckDB",
      about: "The OSM data of one area, built by duckOSM. Each network is a schema: driving, walking and cycling. In a network, edges has one row for each direction of a road; "
        + "edge_id is its id and osm_id the OSM way. main.building_faces has the walls of the buildings, and the schema features has the buildings and the other base-map layers. "
        + "extra.edits has the changes that the person made in the fixes file (osm_id is the OSM id of the changed object; changed is the time of the change).",
      examples: ["Which objects did I change, and when?", "Which roads have more than 3 lanes?", "How many walls of each type do the buildings have?",
        "Which walls did I change by hand?", "Draw a bar chart of the km of road in each class."],
    });
    // roads and buildings are clickable: the road card shows beside the table, and a click on a building puts its row in the data dock (no popup:
    // the dock has the road card for roads only, so the building's facts go into the table). roadstyle sends rs:select with the overlay's label.
    window.rsSetInteraction?.("buildings", { clickable: true, popup: false });
    const hasTable = async (name) => (await query(`SELECT 1 FROM duckdb_tables() WHERE schema_name || '.' || table_name = '${name}' AND database_name = current_database()`)).rows.length > 0;
    let walls = null;
    const buildingSql = (id) => `SELECT b.osm_id, b.osm_type, b.name, b.kind,
      ${walls ? `(SELECT count(*) FROM main.building_faces f WHERE f.building_id = b.osm_id) AS walls, (SELECT count(*) FROM main.building_faces f WHERE f.building_id = b.osm_id AND f.edited) AS edited_walls,` : ""}
      (SELECT string_agg(x.what, '; ') FROM extra.edits x WHERE x.osm_id = b.osm_id) AS changes, b.tags
      FROM features.buildings b WHERE b.osm_id = ${id}`.replace(/\s+/g, " ");
    document.addEventListener("rs:select", async (e) => {
      const d = e.detail;
      if (d?.overlay !== "buildings") return;
      const id = Number(d.properties?.osm_id ?? d.id);
      if (!Number.isSafeInteger(id)) return;
      walls ??= await hasTable("main.building_faces");
      data.open();
      data.run(buildingSql(id)).catch((err) => console.warn("duckosm ask: the building did not show:", err.message));
    });
    window.rs.sections.add({ id: "ask", label: "Ask the data", note: "A question about the OSM data and your changes", open: () => ask.open(), icon: "ask", background: true });
    window.rs.sections.add({ id: "sql", label: "Query the tables", note: "One SQL line over a wide table", open: () => data.open(), icon: "ask", background: true });
    await ready;
    ask.open();
    window.askPage = { data, ask, edits: () => edits };            // for the tests (tests/test_ask_page.py)
  }
  const go = () => start().catch((e) => { console.error(e); document.body.insertAdjacentHTML("beforeend", `<p id="ask-error" role="alert">duckosm ask: ${esc(e.message)}</p>`); });
  window.rsReady ? go() : document.addEventListener("rs:ready", go, { once: true });
})();
