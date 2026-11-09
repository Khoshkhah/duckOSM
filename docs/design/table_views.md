# Table views: every table on the map, and one card for every row

Status: **proposed (2026-11-09). Waits for the sign-off of Kaveh. Nothing is built.**
Kaveh asked: almost every table must show its rows on the map, and a click on a row must give a standard info card. A table of H3 cells must draw the cells and give a card for the selected row.
duckOSM must not copy the Ask part of roadstyle. It is much work, so it needs a plan.

## The problem

The data dock of roadstyle draws only roads for a result (`edge_id`). A row of any other table has no shape on the map and no card.
`duckosm ask` fills the gap in its own page (`rust/duckosm-edit/page/ask.js`, about 130 lines): it draws a geometry column, draws H3 cells with h3-js, and builds a card for a row.
duckOSM Studio would have to copy that code. Also, the page knows only the tables that have a geometry column or an H3 column.

## What exists now

| Part | Where | State |
|---|---|---|
| Hook `kinds` of the data dock: `{ idColumns, geometryBy(sql, column) }`. The dock draws the objects, selects a row, zooms, and fills the card | roadstyle (`docs/design/dock_kinds.md`) | built for nodes; buildings follow |
| `objectKinds({ names, hasTags })`: the SQL for node, traffic point and building | duckOSM, `web/ask/schema.js` | built, not used by a host yet |
| Own drawing of a geometry column, of H3 cells, and of a selected row | duckOSM, `page/ask.js` | built, only for this page |

## Decision

1. **roadstyle owns the engine.** It draws the shape of any row, selects it, zooms to it, and shows the standard card. The engine is the hook `kinds`, made general (see "Asked of roadstyle").
2. **duckOSM owns the knowledge of each table, as data.** One file, `web/ask/views.js`, has one entry for each table. Studio and `duckosm ask` import it. It replaces `objectKinds`.
3. **`page/ask.js` loses its drawing code.** It keeps only what is about this page: the server `query`, the table `extra.edits`, and the H3 table of the boundary.
4. **Studio writes no table code.** It moves the version of roadstyle and copies `web/ask/`.

## The entry of a table

```js
export const VIEWS = {
  turn_restrictions: {
    ids: ["restriction_id"],                      // the columns of a result row that name this table's rows (a weak name such as node_id comes last)
    shape: { join: (n) => `...` },                // where the shape comes from: see below
    title: "restriction_type",                    // the card title (else the table and the id)
    facts: ["restriction_type", "via_node"],      // the facts shown first; the other columns follow
    colour: "restriction_type",                   // optional: a column that colours the shape
  },
};
```

`shape` has four forms:

| Form | Meaning | Example tables |
|---|---|---|
| `own: "geom"` | the table has a geometry column. Draw it. | `nodes`, `features.*`, `building_faces`, `boundary`, GMNS `link` |
| `join: SQL` | the row names an object in other tables. The SQL gives its geometry (a line, a point, or several in one collection). | `edge_graph`, `turn_restrictions`, `lane_profile`, `ways`, `routes` |
| `h3: "col"` | the row is an H3 cell, or has a cell. The host draws the hexagon. | `boundary_h3`, `boundary_cells`, `nodes.h3_cell` |
| `none: "reason"` | no shape, on purpose. The card still shows the row, with the reason. | `visualization_metadata`, `config`, `use_definition` |

## The audit of the tables

Checked on a Monaco build with every option that a PBF build has (`building_faces`, `bus_routes`, `boundary_cells`, `multimodal`; 2026-11-09), and on Tartu, Södermalm and Södermalm GMNS (the GMNS, meso and micro tables).
Marked `*`: not built here, so taken from `docs/reference/database.md` or a design doc. `admin_boundaries` needs `ogr2ogr`; `elevation_metadata` needs a height model; `level_heads`, `level_caps` and `level_unmatched` come from `duckosm levels`.
Three facts from the Monaco build change the plan: `<mode>.edge_id_map`, `turn_path_restrictions` and `turn_permission` are **empty** there (0 rows), so the test of step 3 must accept an empty table and check the SQL on a database that has rows;
`main.building_entrances` exists (110 doors, a point in `geom`, with `building_id`); the lists in `via_edges` are `BIGINT[]`, so the SQL of its `join` must unnest them.

| Group | Tables | Shape |
|---|---|---|
| Roads | `<mode>.edges`, `private_edges`, `mm.edges` | roadstyle lights them by `edge_id`. No new work. |
| Own geometry, points | `nodes`, `virtual_nodes`, `building_entrances` (doors), `features.{traffic,pois,place_labels,public_transport}`, GMNS `node`, `meso_node`, `micro_node` | `own` |
| Own geometry, lines | `features.{streets,water_lines}`, `building_faces`, GMNS `link`, `lane`, `geometry`, `meso_link`, `micro_link` | `own` |
| Own geometry, areas | `features.{buildings,land,paved,sites,water_polygons}`, `boundary`, `admin_boundaries`* | `own` |
| A pair or a path of roads | `edge_graph` (from, via, to edge), `turn_restrictions` (`from_edge_id`, `via_node`, `to_edge_id`), `turn_path_restrictions` (`from_edge`, `via_edges[]`, `to_edge`), `turn_permission` (`from_edge`, `to_edge`), GMNS `movement` (`ib_link_id`, `node_id`, `ob_link_id`) | `join`: draw the roads in 3 colours (from, via, to) and the node between them |
| One road, more facts | `edge_id_map` (old and new edge), `edge_levels`, `level_*`*, `lane_profile` (the lane of an edge) | `join` to `edges`. `lane_profile` draws the road and puts `lane_num` in the card. A lane offset is a later step. |
| One OSM way or node | `ways` (`osm_id` to the edges of the way), `way_nodes` (`node_id`, `way_id`), `global_junctions`, `mm.transfers` (`node_id`), GMNS `signal_controller`, `curb_seg` | `join` |
| A whole route | `bus.routes` (all its edges), `bus.route_edges` (one edge) | `join` |
| A change of the fixes file | `extra.edits` (`kind`, `type`, `osm_id`) | `join`: by `type`, a node, a way, or a building |
| H3 cells | `extra.boundary_h3` (`h3_cell`), `main.boundary_cells` (`h3_id`, `resolution`, and the hexagon in `geometry`), `nodes.h3_cell` | `h3`. `boundary_cells` has its own hexagon, so it needs no h3-js; it still needs the cell card. |
| No shape | `visualization_metadata` (one row), `edge_levels_meta`, `elevation_metadata`*, GMNS `config`, `use_definition`, `use_group` | `none` |
| Not offered | `raw.*` | Ask hides them already (`askTables`). |

`main.building_entrances` is in the standard Monaco build here, but another design note says it is page-only in a standard build without `options.building_faces`. The test must not assume it.

A name such as `osm_id` or `node_id` is in many tables. The dock takes, for a result, the first view whose `ids` match and whose SQL gives at least one row (the rule of `dock_kinds.md`). The order in `VIEWS` is the order of that test: strong names first.

## H3 cells

Three tables have cells, in two forms: a hex text (`boundary_h3.h3_cell`, `boundary_cells.h3_id`) and a BIGINT at resolution 8 (`nodes.h3_cell`). DuckDB here has no H3 functions, so the host computes the shape with h3-js.

- **A table of cells** (`boundary_h3`, `boundary_cells`): the dock draws all cells of the result as hexagons, as the page does today. A click on a row selects the hexagon (the purple look), zooms to it, and opens the card.
- **The card of a cell** (today it has only the id): the hex id, the resolution, the parent cell, the centre (lat, lon), the area in km², and, from the database, the number of nodes, edges and buildings inside. **Counts of nodes: checked (2026-11-09).** The H3 indexes of all descendants of a cell are one range of the 64-bit number: the lowest has the digits below the cell set to 0, the highest has them set to 6. `nodes.h3_cell` is stored at resolution 8, so a cell of resolution 8 or lower counts its nodes with `WHERE h3_cell BETWEEN lo AND hi`, with no h3 function in SQL. On Monaco, for the 9 cells of `boundary_cells` (resolution 6 and 8), the range count equals the count by `cell_to_parent` for every cell (1719 nodes in the resolution 6 cell, and 143 + 37 + 233 + 439 + 356 + 303 + 197 + 11 = 1719 in the eight resolution 8 cells). A cell above resolution 8 has no node count, because `nodes` stores only resolution 8: the card says so.
  **Counts of edges and buildings: not by index.** `features.buildings` has no H3 column. These counts need `ST_Intersects` with the hexagon, so they cost more (not timed yet; one query for each click).
- **A table that only has a cell** (`nodes.h3_cell`): the row is a node, so the node is drawn. The cell is a fact of the card. The card gets a "Show cell" action when roadstyle can give one (open question 3).

## Asked of roadstyle

roadstyle's rule: a part that the dock lacks comes first from roadstyle. These are the requests. The text goes to the roadstyle session; duckOSM does not patch roadstyle.

1. **`geometryOf(ids)`**, an async function in a kind, as an alternative to `geometryBy`. It returns `[{ id, geom, ...facts }]`. A host with h3-js can then draw cells that no SQL can build.
2. **A geometry collection per row**, so a pair of roads and the node between them are one object. The kind says a colour for each part (`role`: from, via, to).
3. **A result of many objects**: draw every object of the result (the limit is `KIND_LIMIT`, 20000), and give the card of the clicked row. Kinds for buildings and the click on the map are already planned in `dock_kinds.md`.
4. **A card with sections and one action**: a title, a chip, the facts, then an optional block with a button ("Show cell", "Zoom to road").

## Steps

1. **Finish the audit.** Done 2026-11-09 for Monaco (every PBF option) and for the GMNS tables; the H3 range test passes. Not done: the `*` tables (`admin_boundaries`, `elevation_metadata`, `level_*`), which need `ogr2ogr`, a height model and `duckosm levels`. Their shape is a guess until someone builds them. The per-table list of columns is in the audit table above.
2. **Ask roadstyle** for the four requests. Wait for them. Steps 3 and 4 can start on the SQL parts that need nothing new.
3. **`web/ask/views.js`** and `viewKinds({ names, hasTags })`. A Node test: every table of a Monaco build has an entry (or `none` with a reason), and the SQL of every `join` runs and gives the rows that a second query gives. A new table without an entry fails the test. This stops the drift.
4. **`page/ask.js`**: pass `kinds: viewKinds(...)`, delete the drawing of shapes, the pick layer and the H3 code. `tests/test_ask_page.py` keeps its checks (shapes drawn, a row selected, a card shown).
5. **Studio**: move the roadstyle version, copy `web/ask/`. No table code.

## Hosts (rule 1 and 2 of `AGENTS.md`)

`objectKinds` stays as an alias of `viewKinds` for one version. `CHANGELOG.md` gets a `### Host changes` entry: use `viewKinds`. Each PR ends with the line `Host change: ...`.
Test on a host with the shape of Studio, not only on `duckosm ask` (rule 3).

## Open questions for Kaveh

1. **One kind for each table, or for each id column?** Recommended: each table, as in the entry above. The dock chooses by the id columns of the result.
2. **A turn or a move** (`edge_graph`, `turn_restrictions`): draw the 2 or 3 roads in 3 colours and the node. Recommended. Is this the look you want?
3. **The H3 card counts.** The node count is cheap (an index range). The edge and building counts need a spatial test for each click. Recommended: nodes always; edges and buildings only if a test on a county shows the time is short. Do you agree?
