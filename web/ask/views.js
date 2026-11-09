// Table views (docs/design/table_views.md): for each table of a duckOSM file, which kind of map object a row of it is, and the SQL that gives the shape and the card of that object.
// roadstyle's data dock draws the objects and fills the card (its option `kinds`); this file only says what the tables are. No DOM, no fetch and no DuckDB: a host gives the names of its
// tables ("driving.nodes", ...) and passes the result to `dataDock({ kinds })`.
//
//   dataDock({ ..., kinds: viewKinds({ names, hasTags: await hasTags(query), h3: window.h3, query }) });
//
// `h3` is the h3-js module (a host that has it gets the H3 cells on the map and the card of a cell); `query(sql)` answers { columns, rows } (the count of nodes in a cell).
//
// A kind is { label, idColumns, geometryBy(sql, column) }. `geometryBy` gives ONE SELECT for the result's own `sql`: the columns `id` and `geom` (GeoJSON text), then the facts of the card.
// Roads (a result with `edge_id`) are not here: roadstyle lights them on their own shape.

const inner = (sql) => sql.trim().replace(/;\s*$/, "");
const q = (c) => `"${String(c).replace(/"/g, '""')}"`;
const ids = (sql, column) => `IN (SELECT ${q(column)} FROM (${inner(sql)}))`;

// What every table of a duckOSM file is for the map (the test tests/test_ask_views.py fails for a table that is not named here, so a new table cannot be forgotten).
//   kind:<name>   a row is an object of this kind of `viewKinds`
//   road          roadstyle lights the road by its `edge_id`
//   pending:<n>   waits for step n of roadstyle's docs/design/dock_kinds_table_views.md (2: geometryOf, 3: parts with roles, 4: card)
//   none:<why>    no shape, on purpose
const TABLES = {
  // roads
  edges: "road", private_edges: "road", edge_levels: "road", lane_profile: "road", route_edges: "road",
  // nodes
  nodes: "kind:node", virtual_nodes: "kind:node", global_junctions: "kind:node", way_nodes: "kind:node", transfers: "kind:node", signal_controller: "kind:node",
  // ways and routes
  ways: "kind:way", routes: "kind:route",
  // the base map
  buildings: "kind:building", land: "kind:feature", paved: "kind:feature", place_labels: "kind:feature", pois: "kind:feature", public_transport: "kind:feature",
  sites: "kind:feature", streets: "kind:feature", traffic: "kind:node", water_lines: "kind:feature", water_polygons: "kind:feature", ocean: "kind:feature",
  // buildings
  building_faces: "kind:building", building_entrances: "kind:building",
  // the area
  boundary: "kind:boundary", boundary_cells: "kind:cell",
  // a pair or a path of roads: the parts of one row (step 3)
  edge_graph: "kind:turn", turn_restrictions: "kind:restriction", turn_path_restrictions: "kind:path", turn_permission: "kind:permission", edge_id_map: "kind:merged", movement: "kind:movement",
  // H3 cells made by the page, and the card of a cell (steps 2 and 4)
  boundary_h3: "pending:2",
  // the changes of the fixes file
  edits: "pending:2",
  // GMNS
  link: "kind:link", lane: "kind:lane", geometry: "none:the shape of a link, drawn with the link", node: "kind:node", curb_seg: "pending:3",
  // no shape
  visualization_metadata: "none:one row about the whole area", edge_levels_meta: "none:one row about the drawing order", level_edits: "none:the pairs of the level editor",
  config: "none:settings", use_definition: "none:a list of uses", use_group: "none:a list of uses", meso_link: "pending:3", meso_node: "pending:3", micro_link: "pending:3", micro_node: "pending:3",
  elevation_metadata: "none:one row about the height model", admin_boundaries: "pending:2", level_heads: "pending:3", level_caps: "pending:3", level_unmatched: "pending:3",
};
const bare = (n) => n.slice(n.lastIndexOf(".") + 1);

// What the file needs for each table: `coverage(names)` names every table of the list (without `raw.*`) with its status, or "unknown" when this file has no entry.
export const coverage = (names) => Object.fromEntries(names.filter((n) => !n.startsWith("raw.")).map((n) => [n, TABLES[bare(n)] ?? "unknown"]));

export function viewKinds({ names = [], hasTags = true, h3 = null, query = null } = {}) {
  const has = (n) => names.includes(n), end = (re) => names.find((n) => re.test(n));
  const tags = (a) => (hasTags ? `, ${a}.tags` : "");
  const kinds = {};

  // a point of the graph (a junction, a virtual node) or a point of the base map with the same OSM id (a traffic signal, a crossing). A node can be in one network only (walking has
  // junctions that driving does not), so the point comes from the first network that has it.
  const mode = (n) => ["driving", "walking", "cycling"].indexOf(n.split(".")[0]);
  const graph = names.filter((n) => /\.nodes$/.test(n) && !n.startsWith("raw.")).sort((a, b) => (mode(a) + 9) % 9 - (mode(b) + 9) % 9);
  const nodes = graph.length ? graph : names.filter((n) => /^gmns_\w+\.node$/.test(n)).slice(0, 1);
  if (nodes.length || has("features.traffic")) {
    kinds.node = {
      label: "Node",
      idColumns: [...(nodes.length ? ["node_id", "source", "target", "via_node"] : []), ...(has("features.traffic") ? ["osm_id"] : [])],
      geometryBy: (sql, column) => {
        if (column === "osm_id") return `SELECT t.osm_id AS id, ST_AsGeoJSON(t.geom) AS geom, t.kind, t.name${tags("t")} FROM features.traffic t WHERE t.osm_type = 'node' AND t.osm_id ${ids(sql, column)}`;
        if (!nodes.length) throw new Error("this file has no nodes table");
        // A node of a way (way_nodes) is mostly not a junction, so it is in no nodes table: when the file keeps raw.nodes, the point comes from its lat and lon.
        const raw = has("raw.nodes"), traffic = has("features.traffic");
        const join = nodes.map((t, i) => ` LEFT JOIN ${t} n${i} ON n${i}.node_id = s.nid`).join("") + (raw ? " LEFT JOIN raw.nodes r ON r.osm_id = s.nid" : "")
          + (traffic ? " LEFT JOIN features.traffic t ON t.osm_type = 'node' AND t.osm_id = s.nid" : "");
        const of = (col) => nodes.map((_, i) => `n${i}.${col}`).join(", ");
        const geom = `COALESCE(${of("geom")}${raw ? ", ST_Point(r.lon, r.lat)" : ""})`;
        const present = [...nodes.map((_, i) => `n${i}.node_id IS NOT NULL`), ...(raw ? ["r.osm_id IS NOT NULL"] : [])].join(" OR ");
        const h3 = nodes[0].endsWith(".nodes") ? `, COALESCE(${of("h3_cell")}) AS h3_cell` : "";
        const facts = traffic ? `, t.kind, t.name${tags("t")}` : "";
        return `SELECT s.nid AS id, ST_AsGeoJSON(${geom}) AS geom${h3}${facts} FROM (SELECT DISTINCT ${q(column)} AS nid FROM (${inner(sql)})) s${join} WHERE ${present}`;
      },
    };
  }

  // a building, from a wall, a door or the building itself
  if (has("features.buildings")) {
    kinds.building = {
      label: "Building",
      idColumns: ["building_id", "osm_id"],
      geometryBy: (sql, column) => `SELECT b.osm_id AS id, ST_AsGeoJSON(b.geom) AS geom, b.osm_type, b.kind, b.name${tags("b")} FROM features.buildings b WHERE b.osm_type IN ('way', 'relation') AND b.osm_id ${ids(sql, column)}`,
    };
  }

  // an OSM way: the edges that were cut from it, as one object
  const edges = names.find((n) => n === "driving.edges") ?? end(/\.edges$/);
  if (edges) {
    kinds.way = {
      label: "OSM way",
      idColumns: ["way_id", "osm_id"],
      geometryBy: (sql, column) => `SELECT e.osm_id AS id, ST_AsGeoJSON(ST_Union_Agg(e.geometry)) AS geom, any_value(e.highway) AS highway, any_value(e.name) AS name, count(*) AS edges FROM ${edges} e WHERE e.osm_id ${ids(sql, column)} GROUP BY e.osm_id`,
    };
  }

  // A turn is the road you come from, the junction and the road you go on to: one object of three parts with the roles from, via and to (roadstyle colours the roles). The key
  // of a row is two columns (`["from_edge", "to_edge"]` is a composite key), so a row of the table is one object. An edge can be in the driving, walking or cycling network,
  // or among the restricted roads, with the same id and the same shape, so the shape comes from any of them.
  // ponytail: two turns with the same from and to edge and different via edges are one object here; add the via edges to the key if that matters.
  const roads = names.filter((n) => /\.(edges|private_edges)$/.test(n) && !n.startsWith("mm.") && !n.startsWith("raw."));
  const roadsSql = !roads.length ? null : `(SELECT DISTINCT ON (edge_id) edge_id, geometry FROM (${roads.map((t) => `SELECT edge_id, geometry FROM ${t}`).join(" UNION ALL ")}) ORDER BY edge_id)`;
  // The SQL of a turn kind. `idColumns` are the two columns of the key; `via` is a column of the result with a list of edges (a path), else the via part is the junction (the end of the
  // road in). The facts of the card come from the table of the kind (`facts.of`, one name for each network), not from the columns that the person selected: only for the pairs of the result.
  const turns = ({ label, idColumns: [from, to], needs, via = null, facts = null, road = roadsSql }) => {
    const cols = facts ? facts.columns : [];
    return {
      label, idColumns: [[from, to]], ...(needs ? { needs } : {}),
      geometryBy: (sql, [fromColumn, toColumn]) => `WITH r AS (SELECT ${q(fromColumn)} AS a, ${q(toColumn)} AS b, ${via ? `any_value(${q(via)})` : "NULL::BIGINT[]"} AS v
          FROM (${inner(sql)}) WHERE ${q(fromColumn)} IS NOT NULL AND ${q(toColumn)} IS NOT NULL GROUP BY 1, 2),
        ${facts ? `fx AS (SELECT r.a, r.b${cols.map((c) => `, any_value(t.${q(c)}) AS ${q(c)}`).join("")} FROM r JOIN (${facts.of.map((t) => `SELECT * FROM ${t}`).join(" UNION ALL BY NAME ")}) t ON t.${q(facts.from)} = r.a AND t.${q(facts.to)} = r.b GROUP BY 1, 2),` : ""}
        parts AS (
          SELECT r.a, r.b, 'from' AS role, f.geometry AS g FROM r JOIN ${road} f ON f.edge_id = r.a
          UNION ALL SELECT r.a, r.b, 'via', ST_EndPoint(f.geometry) FROM r JOIN ${road} f ON f.edge_id = r.a WHERE r.v IS NULL
          UNION ALL SELECT r.a, r.b, 'to', t.geometry FROM r JOIN ${road} t ON t.edge_id = r.b
          UNION ALL SELECT x.a, x.b, 'via', e.geometry FROM (SELECT a, b, unnest(v) AS eid FROM r WHERE v IS NOT NULL) x JOIN ${road} e ON e.edge_id = x.eid)
        SELECT CAST(r.a AS VARCHAR) || '|' || CAST(r.b AS VARCHAR) AS id,
          (SELECT json_group_array(json_object('role', p.role, 'geom', ST_AsGeoJSON(p.g)::JSON)) FROM parts p WHERE p.a = r.a AND p.b = r.b)::VARCHAR AS geom${cols.map((c) => `, fx.${q(c)}`).join("")}
        FROM r${facts ? " LEFT JOIN fx ON fx.a = r.a AND fx.b = r.b" : ""}
        WHERE EXISTS (SELECT 1 FROM parts p WHERE p.a = r.a AND p.b = r.b AND p.role = 'from') AND EXISTS (SELECT 1 FROM parts p WHERE p.a = r.a AND p.b = r.b AND p.role = 'to')`,
    };
  };
  const tablesOf = (suffix) => names.filter((n) => n.endsWith(suffix) && !n.startsWith("raw.") && !n.startsWith("mm."));
  const factsOf = (suffix, from, to, columns) => (tablesOf(suffix).length ? { of: tablesOf(suffix), from, to, columns } : null);
  if (roadsSql) {
    kinds.turn = turns({ label: "Turn", idColumns: ["from_edge", "to_edge"], facts: factsOf(".edge_graph", "from_edge", "to_edge", ["cost", "uses"]) });
    kinds.permission = turns({ label: "Turn rule", idColumns: ["from_edge", "to_edge"], needs: ["allowed"], facts: factsOf(".turn_permission", "from_edge", "to_edge", ["allowed", "vehicles", "except_vehicles", "condition"]) });
    kinds.restriction = turns({ label: "Turn restriction", idColumns: ["from_edge_id", "to_edge_id"], facts: factsOf(".turn_restrictions", "from_edge_id", "to_edge_id", ["restriction_type", "restriction_id", "via_node"]) });
    kinds.path = turns({ label: "Turn path restriction", idColumns: ["from_edge", "to_edge"], needs: ["via_edges"], via: "via_edges", facts: factsOf(".turn_path_restrictions", "from_edge", "to_edge", ["restriction_type", "applies_to", "condition"]) });
    // an edge that was merged from others: the new edge, with the old edge in the card
    const idMap = names.find((n) => n.endsWith(".edge_id_map"));
    if (idMap) kinds.merged = { label: "Merged edge", idColumns: ["new_edge_id"], geometryBy: (sql, column) => `SELECT m.new_edge_id AS id, ST_AsGeoJSON(any_value(e.geometry)) AS geom, count(*) AS old_edges, any_value(m.osm_id) AS osm_id FROM (SELECT DISTINCT * FROM ${idMap} WHERE new_edge_id ${ids(sql, column)}) m JOIN ${roadsSql} e ON e.edge_id = m.new_edge_id GROUP BY m.new_edge_id` };
  }

  // GMNS (the file of `duckosm gmns`): a movement is the link in, the node and the link out
  for (const t of names.filter((n) => /^gmns_\w+\.movement$/.test(n))) {
    const links = t.replace(/\.movement$/, ".link");
    if (names.includes(links)) kinds[`movement_${t.split(".")[0]}`] = turns({ label: "GMNS movement", idColumns: ["ib_link_id", "ob_link_id"], facts: { of: [t], from: "ib_link_id", to: "ob_link_id", columns: ["mvmt_id", "node_id"] }, road: `(SELECT link_id AS edge_id, geom AS geometry FROM ${links})` });
  }

  // a bus line: all the edges that it travels, as one object
  if (has("bus.routes") && has("bus.route_edges") && edges) {
    kinds.route = {
      label: "Bus route",
      idColumns: ["route_id", "osm_id"],
      geometryBy: (sql, column) => `SELECT r.osm_id AS id, ST_AsGeoJSON(ST_Union_Agg(e.geometry)) AS geom, any_value(r.ref) AS ref, any_value(r.name) AS name, any_value(r.operator) AS operator, count(*) AS edges FROM bus.routes r JOIN bus.route_edges re ON re.route_id = r.osm_id JOIN ${edges} e ON e.edge_id = re.edge_id WHERE r.osm_id ${ids(sql, column)} GROUP BY r.osm_id`,
    };
  }

  // the other base-map tables: points of interest, streets, land, water, ...; every one has osm_id, osm_type, kind, name and geom
  const feature = names.filter((n) => n.startsWith("features.") && n !== "features.buildings" && n !== "features.traffic");
  for (const t of feature) {
    kinds[`feature_${bare(t)}`] = {
      label: bare(t).replace(/_/g, " "),
      idColumns: ["osm_id"],
      geometryBy: (sql, column) => `SELECT f.osm_id AS id, ST_AsGeoJSON(f.geom) AS geom, f.osm_type, f.kind, f.name${tags("f")} FROM ${t} f WHERE f.osm_id ${ids(sql, column)}`,
    };
  }

  // the area and its H3 cells (the cells have their own hexagon)
  if (has("main.boundary")) {
    kinds.boundary = { label: "Boundary", idColumns: ["osm_id"], geometryBy: (sql, column) => `SELECT b.osm_id AS id, ST_AsGeoJSON(b.geom) AS geom FROM main.boundary b WHERE b.osm_id ${ids(sql, column)}` };
  }
  if (has("main.boundary_cells")) {
    kinds.cell = { label: "H3 cell", idColumns: ["h3_id"], geometryBy: (sql, column) => `SELECT c.h3_id AS id, ST_AsGeoJSON(c.geometry) AS geom, c.resolution FROM main.boundary_cells c WHERE c.h3_id ${ids(sql, column)}` };
  }

  // GMNS (the file of `duckosm gmns`): links and lanes
  for (const t of names.filter((n) => /^gmns_\w+\.link$/.test(n))) {
    kinds[`link_${t.split(".")[0]}`] = { label: "GMNS link", idColumns: ["link_id"], geometryBy: (sql, column) => `SELECT l.link_id AS id, ST_AsGeoJSON(l.geom) AS geom, l.name, l.from_node_id, l.to_node_id, l.lanes FROM ${t} l WHERE l.link_id ${ids(sql, column)}` };
  }
  for (const t of names.filter((n) => /^gmns_\w+\.lane$/.test(n))) {
    kinds[`lane_${t.split(".")[0]}`] = { label: "GMNS lane", idColumns: ["lane_id"], geometryBy: (sql, column) => `SELECT l.lane_id AS id, ST_AsGeoJSON(l.geom) AS geom, l.link_id, l.lane_num FROM ${t} l WHERE l.lane_id ${ids(sql, column)}` };
  }
  // ---- H3 cells (docs/design/table_views.md). DuckDB here has no H3 functions, so the host gives h3-js and the shape of a cell is made in JavaScript (roadstyle's geometryOf).
  // A cell comes as a hex text (extra.boundary_h3.h3_cell, boundary_cells.h3_id) or as a BIGINT (nodes.h3_cell, resolution 8), so the id is turned into a hex text first.
  if (h3) {
    const cellOf = (id) => {
      const text = String(id);
      try { if (h3.isValidCell(text)) return text; } catch { /* not a hex text */ }
      try { const hex = BigInt(text).toString(16); if (h3.isValidCell(hex)) return hex; } catch { /* not a number */ }
      return null;
    };
    const polygon = (cell) => ({ type: "Polygon", coordinates: [h3.cellToBoundary(cell, true)] });
    // The H3 indexes of all the descendants of a cell are one range of the 64-bit number: the digits below the cell are 0 for the lowest and 6 for the highest (checked on Monaco against cell_to_parent).
    const range = (cell, resolution) => {
      const x = BigInt("0x" + cell), r = Number((x >> 52n) & 15n);
      let lo = (x & ~(15n << 52n)) | (BigInt(resolution) << 52n), hi = lo;
      for (let k = r + 1; k <= resolution; k++) { const shift = BigInt(3 * (15 - k)), mask = 7n << shift; lo &= ~mask; hi = (hi & ~mask) | (6n << shift); }
      return [lo, hi];
    };
    const nodeTable = names.find((n) => n === "driving.nodes") ?? names.find((n) => /\.nodes$/.test(n) && !n.startsWith("raw."));
    const km2 = (v) => (v >= 10 ? v.toFixed(1) : v >= 0.1 ? v.toFixed(2) : v.toFixed(4)) + " km²";
    const info = (cell) => {
      const resolution = h3.getResolution(cell), [lat, lng] = h3.cellToLatLng(cell);
      return [["H3 cell", cell], ["resolution", resolution], ...(resolution > 0 ? [["parent cell", h3.cellToParent(cell, resolution - 1)]] : []),
        ["centre", `${lat.toFixed(5)}, ${lng.toFixed(5)}`], ["area", km2(h3.cellArea(cell, "km2"))], ["edge length", `${Math.round(h3.getHexagonEdgeLengthAvg(resolution, "m"))} m`],
        ...(h3.isPentagon(cell) ? [["shape", "pentagon"]] : [])];
    };
    kinds.h3cell = {
      label: "H3 cell", idColumns: ["h3_cell", "h3_id"],
      geometryOf: (list) => list.flatMap((id) => { const cell = cellOf(id); return cell ? [{ id, geom: polygon(cell) }] : []; }),
      // the card of a cell: what the cell is, the other columns of the clicked row, and how many nodes it holds (the nodes keep their cell at resolution 8, so a larger cell has a count and a smaller one has none)
      card: async (card) => {
        const cell = cellOf(card.id);
        if (!cell) return {};
        const resolution = h3.getResolution(cell), rows = info(cell);
        for (const [k, v] of Object.entries(card.row || {})) if (!/^h3_(cell|id)$/.test(k) && k !== "resolution" && v != null) rows.push([k.replace(/_/g, " "), v]);
        const sections = [];
        if (nodeTable && query) {
          if (resolution > 8) sections.push({ title: "Inside this cell", rows: [["nodes", "not counted: the nodes keep their cell at resolution 8"]] });
          else {
            const [lo, hi] = range(cell, 8);
            try { sections.push({ title: "Inside this cell", rows: [["nodes", Number((await query(`SELECT count(*) FROM ${nodeTable} WHERE h3_cell BETWEEN ${lo} AND ${hi}`)).rows[0][0])]] }); } catch { /* no count */ }
          }
        }
        return { title: `H3 cell ${cell}`, rows, sections };
      },
    };
    delete kinds.cell;      // boundary_cells has its own hexagon in SQL; the cell kind above gives it the card
    // a node has its cell at resolution 8: its card has a button that draws the cell
    if (kinds.node) kinds.node.card = (card) => {
      const raw = card.facts?.h3_cell ?? card.row?.h3_cell, cell = raw == null ? null : cellOf(raw);
      return cell ? { action: { label: "Show H3 cell", note: `H3 cell ${cell}, resolution ${h3.getResolution(cell)}`, run: () => polygon(cell) } } : {};
    };
  }

  // The dock gives a result to the kind that finds the most of its ids, and the first kind of the list wins a tie. A row of turn_restrictions has a via_node (a node) and two edges
  // (a turn): the turn is the better answer, so the kinds of two keys come first.
  const first = (k) => /^(turn|permission|restriction|path|movement)/.test(k);
  return Object.fromEntries([...Object.entries(kinds).filter(([k]) => first(k)), ...Object.entries(kinds).filter(([k]) => !first(k))]);
}