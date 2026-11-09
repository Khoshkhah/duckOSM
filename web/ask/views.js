// Table views (docs/design/table_views.md): for each table of a duckOSM file, which kind of map object a row of it is, and the SQL that gives the shape and the card of that object.
// roadstyle's data dock draws the objects and fills the card (its option `kinds`); this file only says what the tables are. No DOM, no fetch and no DuckDB: a host gives the names of its
// tables ("driving.nodes", ...) and passes the result to `dataDock({ kinds })`.
//
//   dataDock({ ..., kinds: viewKinds({ names, hasTags: await hasTags(query) }) });
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
  edge_graph: "pending:3", turn_restrictions: "pending:3", turn_path_restrictions: "pending:3", turn_permission: "pending:3", edge_id_map: "pending:3", movement: "pending:3",
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

export function viewKinds({ names = [], hasTags = true } = {}) {
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
  return kinds;
}
