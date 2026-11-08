// What roadstyle's Ask and data dock must know about the tables of a duckOSM file (docs/design/ask_schema.md). One module for every page that gives them a duckOSM
// database: the page of `duckosm ask` (rust/duckosm-edit/page/ask.js) and duckOSM Studio (which copies web/ask/ in build.sh). It has no DOM, no fetch and no DuckDB: a page gives
// the names of its tables and a `query(sql)` that answers { columns, rows } (roadstyle's shape), so the page differs only in how it reaches the data.
//
//   const main = mainEdges(names);                                     // names: ["driving.edges", "main.building_faces", ...]
//   dataDock({ query, schema: async () => describeTables(tables, main, query), roadsBy: roadsBy(main), sql: startSql(main), ready });
//   askPanel({ data, dialect: "DuckDB", about: aboutText({ tables: names, classes: await roadClasses(query, names) }), examples: QUESTIONS });

// A note for each table, by the name of the table without its schema (the Tables list of the data dock shows it). A table that is not here has no note.
export const TABLE_NOTES = {
  edges: "One row is one direction of a road (an edge), with its length, speed, lanes, cost and OSM tags.",
  nodes: "The junctions and the ends of the roads.",
  private_edges: "The roads that this network keeps out: private, forbidden or bus only.",
  edge_graph: "The legal turns: one row for each move from an edge to the next edge.",
  turn_restrictions: "The OSM turn restrictions at a node, as pairs of edges.",
  turn_path_restrictions: "The turn restrictions through a via way, as paths of edges.",
  turn_permission: "The turn rules that the graph cannot hold: a rule for some vehicles or with a condition.",
  lane_profile: "Each lane of each edge, from left to right, with its use, width and arrow.",
  edge_id_map: "For each merged edge, the edges that it was made from.",
  ways: "The OSM ways that the network kept, with all their tags and nodes.",
  way_nodes: "The nodes of each way, in order.",
  virtual_nodes: "The virtual nodes (node_id below 0) that split self-loops and parallel roads.",
  global_junctions: "The nodes where all networks cut their roads, so an edge has the same id in each network.",
  boundary: "The area: the boundary and its properties.",
  visualization_metadata: "One row: the center and zoom of the area for a map, its time zone, and the build time.",
  transfers: "The changes of mode (walk to car or bike, and back) at the nodes that the networks share.",
  routes: "The bus lines (OSM route relations): their number, name and operator.",
  route_edges: "The driving edges that each bus line travels, in order.",
  edge_levels: "The drawing order of each edge: a bridge over the road under it, a tunnel under the road over it.",
  edge_levels_meta: "One row: how the drawing order was made (from the tags, solved, or edited) and for which edges.",
  level_edits: "The edits of the drawing order in the level editor: one rule for each row.",
  level_heads: "The head lengths that were set in the level editor, for each road.",
  level_caps: "The line ends that were set in the level editor, for each road.",
  building_faces: "One row is one wall of a building, with its type (front, back, side, party, court) and the type that the build found (computed_type).",
  building_entrances: "The entrances and doors of the buildings.",
  building_info: "One row for each building: floors, height, roof shape, colors and materials, read from its OSM tags.",
  buildings: "The buildings of the base map, with their OSM tags.",
  land: "The land use and the natural areas of the base map (parks, woods, grass, and others).",
  ocean: "The sea.",
  paved: "The squares, the pedestrian areas and the piers.",
  place_labels: "The names of places (districts, suburbs, and others).",
  pois: "The points of interest (shops, restaurants, and others).",
  public_transport: "The bus stops, the stations and the platforms.",
  sites: "The grounds of schools, hospitals, parkings and other sites.",
  streets: "All the ways of the base map (roads, paths, railways) as lines.",
  traffic: "The traffic signals and the crossings: kind is traffic_signals or crossing.",
  water_lines: "The rivers and the other waterways, as lines.",
  water_polygons: "The lakes, the ponds and the other water, as areas.",
  boundary_h3: "The H3 cells that cover the boundary at each resolution 0 to 11: resolution, h3_cell (a hex id), whole_boundary (true for the one cell that contains the whole boundary).",
  edits: "The changes in the fixes file: kind (object, wall, turn, new_road), type, osm_id, what, changed (the time), note.",
};

// The main table of the page: the edges of the driving network, else the first network that has edges, else null. `names` is a list of "schema.table".
export const mainEdges = (names) => names.find((n) => n === "driving.edges") ?? names.find((n) => n.endsWith(".edges")) ?? null;

// dataDock({ roadsBy }): the roads of a result that has a column and no edge_id (the roads whose column has one of the values of the result).
export const roadsBy = (main) => (sql, column) => {
  if (!main) throw new Error("this file has no edges table");
  return `SELECT DISTINCT edge_id FROM ${main} WHERE "${column}" IN (SELECT "${column}" FROM (${sql}))`;
};

// dataDock({ sql }): the first SQL line of the dock.
export const startSql = (main) => main ? `SELECT highway, count(*) AS edges, round(sum(length_m) / 1000, 1) AS km FROM ${main} GROUP BY highway ORDER BY edges DESC` : "SELECT 1";

// dataDock({ schema }): the tables of the page, each with its note, and the columns of the main table (the agent reads them first).
// `tables` is a list of { name: "schema.table", rows }; `query` answers { columns, rows }.
export async function describeTables(tables, main, query) {
  const out = tables.map((t) => ({ ...t, about: TABLE_NOTES[t.name.slice(t.name.lastIndexOf(".") + 1)] ?? "" }));
  const first = out.find((t) => t.name === main);
  if (first) {
    const [s, t] = main.split(".");
    const { rows } = await query(`SELECT column_name, data_type FROM information_schema.columns WHERE table_schema = '${s}' AND table_name = '${t}' AND table_catalog = current_database()`);
    Object.assign(first, { main: true, columns: rows.map(([name, type]) => ({ name, type })) });
  }
  return { tables: out };
}

// The road classes of each network with their number of edges, as one text for aboutText: the agent must not guess the values of `highway` (asked to colour the roads by
// class, it used classes that do not exist, found no roads, and had no step left to look them up). A network that gives an error is left out.
export async function roadClasses(query, names) {
  const parts = await Promise.all(names.filter((n) => n.endsWith(".edges")).map(async (n) => {
    try {
      const { rows } = await query(`SELECT highway, count(*) AS n FROM ${n} GROUP BY highway ORDER BY n DESC`);
      return `${n}: ${rows.map(([h, c]) => `${h ?? "NULL"} (${c})`).join(", ")}`;
    } catch { return ""; }
  }));
  return parts.filter(Boolean).join("; ");
}

// askPanel({ about }): the text that tells the agent what the data is. It names a table only when `tables` has it. `classes` is the text of roadClasses.
// `shapes: true` is for a page that draws the geometry column of a result on the map (nodes as points, buildings as shapes, walls as lines, H3 cells as hexagons).
export function aboutText({ tables = [], classes = "", shapes = false } = {}) {
  const has = (re) => tables.some((n) => re.test(n));
  const t = [
    "The OSM data of one area, built by duckOSM. Each network is a schema: driving, walking and cycling. In a network, edges has one row for each direction of a road; "
    + "edge_id is its id and osm_id the OSM way. ROADS are lit by the page itself on their own shape when the result has the column edge_id: select edge_id for roads, and not their geometry. ",
  ];
  if (has(/\.edge_graph$/)) t.push("edge_graph has the legal turns (from_edge, to_edge, via_edge). ");
  if (has(/^driving\.turn_restrictions$/)) t.push("driving.turn_restrictions has the OSM turn restrictions (restriction_type, via_node, from_edge_id, to_edge_id). ");
  if (has(/^main\.building_faces$/)) t.push("main.building_faces has the walls of the buildings (building_id, face_type, computed_type, edited, length_m, distance_m, street_name, geometry). ");
  if (has(/^features\./)) {
    t.push("The schema features has the base-map layers: buildings, traffic (kind is traffic_signals or crossing), sites, public_transport, pois, streets, water_polygons and others; each has osm_id, kind, name, tags and geom. "
      + "tags is a MAP: write tags['building:levels'], and cast it (try_cast(tags['height'] AS DOUBLE)). ");
  }
  if (has(/^extra\.edits$/)) {
    t.push("extra.edits has the changes that the person made in the fixes file (osm_id is the OSM id of the changed object; changed is the time of the change). ");
  }
  if (shapes) {
    t.push("THE PAGE ALSO DRAWS A RESULT THAT HAS A GEOMETRY COLUMN ON THE MAP, as it is: nodes as points, buildings as shapes, walls as lines. So you CAN show nodes, buildings and walls: "
      + "run_query with their geom column (driving.nodes: node_id, geom; features.buildings: osm_id, name, geom; main.building_faces: building_id, face_type, geometry), "
      + "for example SELECT node_id, geom FROM driving.nodes. For \"all\" of them, add no LIMIT and no WHERE: the table shows only the first 1000 rows, and the map shows every row that you select. ");
  }
  if (has(/\.nodes$/) || has(/^extra\.boundary_h3$/)) {
    t.push("H3: the nodes tables (driving.nodes and the others) have h3_cell, a BIGINT H3 index at resolution 8. ");
  }
  if (has(/^extra\.boundary_h3$/)) {
    t.push("extra.boundary_h3 (resolution, h3_cell, whole_boundary) has the H3 cells that cover the boundary at each resolution 0 to 11, and whole_boundary is true for the one cell that contains the whole boundary: "
      + "the SMALLEST cell that contains the whole boundary is the row with the largest resolution where whole_boundary is true. "
      + (shapes ? "THE PAGE DRAWS A RESULT THAT HAS A COLUMN h3_cell (and no geometry column) AS HEXAGONS, so you CAN show H3 cells: SELECT h3_cell FROM extra.boundary_h3 WHERE whole_boundary ORDER BY resolution DESC LIMIT 1, or SELECT DISTINCT h3_cell FROM driving.nodes. " : ""));
  }
  if (classes) t.push(`THE VALUES OF highway (the road classes) with the number of edges, for each network: ${classes}. Use exactly these values in a WHERE, and never guess a class. `);
  t.push("To colour roads by class, call color_roads once, with one group for each class that you colour (for example primary red, secondary orange, tertiary yellow, the others grey), each group with its sql: SELECT edge_id FROM driving.edges WHERE highway = 'primary'. ");
  if (shapes) {
    t.push("COLOURS: a shape is coloured by its column face_type (front green, back red, side grey, party wall purple, court cyan), so a result with face_type needs no more. "
      + "For any other colour select a column named color with a CSS colour, for example CASE highway WHEN 'primary' THEN '#dc2626' ELSE '#6b7280' END AS color. ");
  }
  if (shapes) t.push("Never answer that you can only show roads.");
  return t.join("");
}
