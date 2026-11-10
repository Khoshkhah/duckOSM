# Changelog

## Unreleased

### Host changes

For a host that runs the fixes editor (duckOSM Studio, `duckosm edit`). Each entry says what the host must do.

- **The editor reads the map's road table itself** (duckOSM #43). `roadIdOfRow` and `roadRows` are now optional. Without them the editor reads `window.RS_SIDE.props`
  (then `rsGetProps` and `rsQuery`) and builds the index of the roads by OSM way. A row of the table is an object, or an array when the host gives the new option
  `roadKeys` (the column names). `roadRows: false` keeps the editor's own line layers. *Host: nothing is needed. Studio can drop its own `roadIdOfRow` and `roadRows` and give `roadKeys`.*
- **The editor files are a library crate, `duckosm-editor`** (duckOSM #45). It has the four files of `web/fixes_editor/` (`FILES`, `file(name)`). *Host: nothing is needed. A Rust host can add the crate
  at the same revision as `duckosm-core`, serve `FILES` from one folder, and remove its step that fetches and copies the folder.*
- **`askTables` names an entry `mode.table` when it has both** (`web/ask/schema.js`). *Host: nothing is needed; a host that passes its entries as they are (Studio, whose `name` is a file name) moves its duckOSM pin.*

- **A host can give the data dock `kinds` as a function and read it again** (roadstyle `f3d1efc5`, PR 134 and 136; `viewKinds` needs no change). A host whose tables change after the start (Studio: a build with other layers) gives `kinds: () => viewKinds({ names, hasTags, h3, query })`
  and calls `await data.refreshKinds()` after the change. The dock then draws the rows of the new tables. *Host: nothing is needed. `duckosm ask` adds no table while it runs, so its page still gives the object. Studio replaced its proxy of kinds with the function.*

### Added

- **`aboutText({ kinds: true })` tells Ask that results of ids are drawn** (`web/ask/schema.js`). A page whose data dock has the kinds of `web/ask/views.js` passes `kinds: true`, and the agent then knows that it can show junctions, turns, ways, buildings, bus routes and H3 cells by their ids. `duckosm ask` does this. *Host: duckOSM Studio passes `kinds: true` to `aboutText`.*
- **The kinds work on the tables of the Rust engine** (`web/ask/views.js`, found on duckOSM Studio). Their `nodes` have no `h3_cell` column, so the node kind failed with a binder error and drew no node (`turn_restrictions`, `nodes`, `way_nodes`). The column is optional now. The card of a turn took `cost` and `uses` from any network that has the same pair of edges, so it could differ from the row: it has facts of its own only when one table gives them (`turn_restrictions`, `turn_path_restrictions`); for `edge_graph` and `turn_permission` the section Row has them. *Host: nothing is needed.*
- **Docs: every table on the map** (`docs/guides/query.md`, section "Every table on the map"; `docs/reference/cli.md`, `ask`). What a row of each table is on the map, the card sections, the H3 cell card and Reset.
- **The turn tables work on a file without the `uses` column** (`web/ask/views.js`). A file built before `edge_graph.uses` gave a SQL error in the turn kind, so `edge_graph` showed nothing on the map and had no card. The card now has the columns that the table has, and the objects keep the order of the result, so the first rows of a big table (`walking.edge_graph`) are the first objects that the dock draws. *Host: nothing is needed.*
- **The card of every kind ends with the clicked row** (`web/ask/views.js`, section "Row"): all the columns of the row as the table shows them (a turn restriction: `restriction id`, `from edge id`, ...). The facts of the object come first. *Host: nothing is needed.*
- **H3 cells on the map with a card** (`viewKinds({ names, hasTags, h3, query })`, roadstyle step 4). A table of cells (`extra.boundary_h3`, `main.boundary_cells`, `SELECT DISTINCT h3_cell FROM driving.nodes`) is drawn as hexagons by the data dock, and a click on a row gives the card of the cell: id, resolution, parent, centre, area, edge length, the other columns of the row, and the number of nodes inside (an index range, no H3 function in SQL). The card of a node has a button "Show H3 cell". The page no longer draws H3 cells itself; Ask's `set_shapes` also changes their opacity. The pin moves to roadstyle `d7902251`. *Host: pass `h3` (the h3-js module) and `query` to `viewKinds` to get the cells; without `h3` the cells are not drawn.*
- **The turn tables are on the map and have a card** (`edge_graph`, `turn_restrictions`, `turn_path_restrictions`, `turn_permission`, `edge_id_map`, GMNS `movement`; `web/ask/views.js`, roadstyle step 3). A row is one object of three parts: the road in (`from`), the junction or the roads between (`via`), the road out (`to`). The key is two columns, and the card has the facts of the table (`cost`, `uses`, `restriction_type`, ...). The pin moves to roadstyle `bfd28fd6`. *Host: nothing is needed.*
- **`duckosm ask` draws the objects of a result of ids** (`web/ask/views.js` is served at `/ask/views.js` and given to roadstyle's data dock as `kinds`). A result with ids and no geometry column (the `via_node` of a turn restriction, the `node_id` of `way_nodes`, a `route_id`, an `osm_id`) is drawn by the dock itself, a click on a row selects the object, and the card has its facts. A result with a geometry column keeps the page's own drawing until roadstyle can colour a kind (docs/design/dock_kinds_table_views.md, steps 2 to 4). *Host: nothing is needed.*
- **`web/ask/views.js`: what each table of a duckOSM file is on the map** (`viewKinds({ names, hasTags })`, `coverage(names)`; [design](docs/design/table_views.md)). It gives roadstyle's data dock the `kinds` for nodes (also the nodes of a way, from `raw.nodes`), buildings, OSM ways, bus routes, the base-map tables, the boundary, its H3 cells and GMNS links and lanes. The tables that wait for roadstyle (a turn is several parts; the card of an H3 cell) are marked `pending:<step>`. `tests/test_ask_views.py` fails for a table that has no status. No host uses it yet; `objectKinds` stays until the page moves. *Host: nothing is needed.*
- **`duckosm build` makes `main.building_entrances`** with `options.building_faces` (the doors of the area: `kind`, `door`, `access`, `housenumber`, `street`, `level`, `wheelchair`, `name`, `building_id`, `geom`).
  It is the same SQL as the page of Studio, so the editor reads one table everywhere (the Doors section of the Building tab). Before, only Studio made it. Monaco sample: 110 doors.

### Fixed

- **"Changes on the map: Hide" takes the changed roads off the map** (`web/fixes_editor/fixes_panel.js`). Hide made only the editor's own layers invisible. A road of the
  map is in roadstyle's highlight sets `fix-edits` (amber) and `fix-gone` (red), so it stayed lit. Now Hide sets these two sets empty and Show lights them again; the editor keeps
  their rows. Test: `test_editor_turn_changes.py::test_hide_takes_the_changed_roads_off_the_map_and_show_brings_them_back`.
- **`duckosm edit`: the editor showed nothing on the map.** The server gives a JSON column (`ST_AsGeoJSON`) as an object, and the editor
  parses text. `duck.query` of the page now gives such a column as text. The selected object, the changes and the walls show again.
- **`askTables` takes `mode.table` before `name`.** A host entry can have a `name` that is a file name (Studio: `features_traffic.parquet`). Now an entry with `mode` and `table` is named `mode.table`; an entry with only `name` works as before.

### Added

- **Ask sees the base-map tables and has SQL for the common questions** (`web/ask/schema.js`, `docs/design/ask_schema.md`). Studio hid every `features.*` table, so Ask could not
  count the traffic signals ("I can't give a count yet"). `askTables` keeps them (it drops only `raw.*`). The text for the agent has a line for each base-map table: its columns, what `kind` holds
  (`features.traffic` has only `traffic_signals` and `crossing`, no stop signs) and example SQL for the signals, the buildings by levels and the points of interest. `aboutText({ hasTags })`
  leaves out the hints that need `tags` for a file without a `tags` column. A Monaco test runs the SQL of each hint (22 signals, 179 buildings with more than 5 levels).
- **`web/ask/schema.js` says which column of a result row is a node or a building** (`objectKinds`, `hasTags`; `docs/design/ask_schema.md`). A node is `node_id`, `source`, `target`
  or `via_node` (the geometry and the H3 cell come from the nodes table) or an `osm_id` of `features.traffic`; a building is `building_id` of a wall or an `osm_id` of
  `features.buildings`. `geometryBy(sql, column)` gives one SELECT with the geometry and the facts of the card. It leaves `tags` out for a file whose base-map tables have none.
  No host calls it yet: roadstyle's data dock gets the hook for it.

### Changed

- **What Ask knows about the duckOSM tables is one module, `web/ask/schema.js`** (`docs/design/ask_schema.md`). It has the notes of 40 tables, the text for the agent
  (`aboutText`: it names a table only when the table is there, and tells the agent that `tags` is a MAP), `roadsBy`, the first SQL line of the data dock, the columns of
  the main table and the real road classes. `duckosm ask` imports it from `/ask/schema.js`. duckOSM Studio can copy `web/ask/` and use the same file, so the two pages
  differ only in how they reach the data. The page lost its own copies of these. The text for the agent keeps its old facts and adds the layers of `features`, the turn tables and the MAP hint.
- **`duckosm ask`: many more example questions, in 14 groups.** The list has 115 questions. It has new groups for the road tags (surface,
  bridges, tunnels, roundabouts), the junctions and turn restrictions, the traffic signals and crossings, the levels, heights and roofs of the
  buildings, the walls and their streets, and the look of the shapes. The groups that were already there have more questions, and *Charts of the roads*
  is now *Charts of the data*. The SQL of each new question was run on the Monaco sample and gave rows. Ask itself needs an AI key and was not run.
  The list is now one file, `web/ask/questions.js` (`export const QUESTIONS`). The page imports it (the server gives it at `/ask/questions.js`), and a host page
  of its own can copy it, as it copies `web/fixes_editor/`. Guide: `docs/guides/ask-questions.md`.
- **The fixes editor colours the roads with roadstyle's highlight sets** (`docs/design/fixes_editor.md`). A new optional hook,
  `roadRows(osmId)`, gives the map's rows of an OSM way. With it, the picked road is blue, the changes are amber, the deleted roads
  are red, the roads that leave a junction are green and the road under the pointer in the Turn list is dark, on the road's own fill
  at the road's width. Without it, the editor's own line layers stay. `duckosm edit` gives the hook. The roadstyle pin of
  `duckosm-edit` moves to `e1b0617a` (roadstyle #58, #59, #60).

## 0.7.0 (2026-11-06)

The Rust engine is the default and is in the wheel (wheels for 5 systems). `duckosm edit` opens the fixes editor on a page on your computer. The fixes editor shows a box for an object that is already in your changes.

### Fixed

- **`duckosm edit`: the page has roadstyle's current look.** It asked roadstyle for the old docked layout (`layout: dock`), so it did not look like roadstyle's own pages and duckOSM Studio. Now it uses roadstyle's default layout (the panel is a card over the map), at roadstyle main `0689bc25`. The docs show the page: four pictures in the guide *Edit OSM data*, and a new page *The Rust engine*.
- **The fixes editor: an edit could go under a later rule of the same object.** `findRule` read the first record of an object and
  `upsertRule` replaced the first, but the build applies the records in file order. Now the editor reads the records joined as the build
  applies them, and each write puts one record in the place of all of them.
- **The fixes editor: the bar of a tab hid no list row any more.** The bar ("Add to changes") sticks to the bottom of the
  panel, but a sticky box stops at the padding of its scroll box, so 12 px of the list showed under the bar (Kaveh's
  screenshot of the Turn tab). `.fix-bar` now has `bottom: -12px`, the padding of `.fix-body`.
- **The fixes editor: the junctions of the Turn tab are a dropdown list** (Kaveh). Before, each junction at the end of the
  road was a row of two lines, and a road with many junctions filled the tab. The `<select id="fix-via">` has one option for
  each junction, with the number of roads that leave it. The map shows the junction that you pick.

### Changed

- **The Rust engine is the default** (`options.engine: rust`; `--engine python` runs the SQL steps). The Rust code of the
  repository `duckosm-rust` is in `rust/` now, and `duckosm._rust` is its Python module, inside the `duckosm` wheel. The wheel is
  not pure Python any more: it has one build for each of 5 systems (Linux x86_64 and aarch64, macOS x86_64 and arm64, Windows
  x64; abi3, Python 3.9 and newer), and a source install needs a Rust toolchain. The module has no DuckDB; the package uses its own
  `duckdb`. The network step (edges, private edges, nodes, `edge_id_map`, edge graph, turn restrictions) runs in Rust: Estonia
  takes 19 s and 1.5 GB, against 1,545 s and 10 GB. The osmium cut, `raw.*`, the features, the bus routes and the levels
  stay Python for now. They move to Rust one by one, each with its own equality test. `edge_id` and all tables are
  the same as in 0.6.1 (Monaco: every row of 20 tables; the connector rows are equal in both engines since the fixed order of
  `PathConnector`). The Python engine stays as the reference for one release; the tag `python-default-final` is the last commit
  with the Python default. The old wheel `duckosm_rust` (the repository `duckosm-rust`, last version 0.3.0) is not used any more.
  Plan: `docs/design/rust_engine_move.md`.

### Added

- **`duckosm edit`: edit the OSM data on your computer** (`docs/design/edit_page.md`). `duckosm edit area.duckdb` opens a page on
  127.0.0.1: roadstyle's dashboard of the database with the panel **Edit OSM data**. Each change goes into `osm_fixes.yaml` beside the
  database (or `--fixes`) at once, and **Build again** runs the build of the database again with the file. The server is Rust
  (`rust/duckosm-edit`), with the safety rules of roadstyle-serve (127.0.0.1 only, `Host` and `Origin` checks, only the fixes file is
  written). `duckosm-rust edit` is the same page in the Rust program. A build writes its record for this in
  `main.visualization_metadata` (`build_config`, `build_dir`). The editor has a new optional hook, `ctx.buildOff`: Build again is off,
  with the reason under it.
- **The fixes editor: the box "This road is in your changes"** (Kaveh's design). When you pick an object that is in your changes, a box
  shows over the form: **Edit it** opens the form with your changes, **Delete it** removes them (Undo brings them back). For a road, a
  building with its walls, a node, the turns of a road at a junction, and a new road. A file with two or more rules for one object: the
  box says how many, Edit joins them into one (for the same key the later rule wins, as in the build), and the Changes tab marks the
  object ("2 rules").
- **The fixes editor is in duckOSM** (`web/fixes_editor/`): the panel "Edit OSM data" of the duckOSM page, with its unit tests
  (`node web/fixes_editor/fixes_units.mjs`, in CI). The page loads it at the same duckOSM rev as its build engine. Guide:
  `docs/guides/edit-osm-data.md`. Design: `docs/design/fixes_editor.md`. A change from the page's panel: the map shows no wall
  numbers. The wall under the pointer (on the map, or its row in the list of walls) is lit and shows its number; with no pointer (a
  phone), the selected wall shows its number.
- **`extra.tag_edits` in `raw.*`.** The tag edits of a fixes file now change `raw.ways`, `raw.nodes` and `raw.relations`, so the layers
  (`features.*`), the bus routes and the lane profile see them, and the Python engine supports them too. Before, only the networks of the
  Rust engine saw them (Monaco: 6 tables were different from the page).
- **`options.building_faces`** (off): `main.building_faces`, the walls of the buildings with a type each, and the wall rules
  (`extra.building_faces`) of a fixes file. Rust engine only. `tests/test_page_fixes.py`: a fixes file from the page with every fix kind
  gives the same rows as the page in all 44 tables that the two builds have (Monaco).

- **`edges.z_vertices`.** `duckosm elevation` also samples the DEM at every vertex of each edge and writes the heights, in the
  order of the geometry, as `<mode>.edges.z_vertices` (DOUBLE[], 2 dp; `z_vertices_<suffix>` for a second surface). A reader gets the
  heights along a way without the raster. SimReady's `import-elevation` reads them (its open item `P7.dem-interior`).

### Fixed

- **Walk + drive obeys turn restrictions.** `route_multimodal`, `route_multimodal_points` and the Walk + drive route of `route-map` searched
  over junctions, so a driving leg could take a turn that OSM forbids (Monaco: 29 of 16,149 turns in 300 trips). Every leg now follows the
  `edge_graph` of its mode, as `route()` does. No route gets shorter. Design: `docs/design/multimodal_edge_graph.md`.
- `scripts/route_map_compare.py` compares the page with `route_multimodal_points` on random trips.
- **Shortest works in Walk + drive** (`route-map`). The page routed Walk + drive by time only, so Fastest and Shortest gave the same route.
  For Shortest the search now adds metres, and a change of mode counts 0.1 m (Monaco: 1,880 m in place of 1,901 m). The time and the
  length that the panel shows do not change. `route_multimodal` still routes by time.
- **The code runs on DuckDB 2.0.0 and on 1.5.3.** Lambdas use the `lambda x:` form, each connection sets
  `geometry_always_xy = false`, `duckosm.utils.finish()` closes an open result before `register()`, `create_function()` and `sql()`,
  and stored geometry and `NULL` columns get explicit types. The pin stays `duckdb<2`. On 1.5.3 the output does not change, but an
  empty GMNS `curb_seg` table now has the column types of a filled one. Open items: `docs/design/duckdb2_upgrade.md`.

## 0.6.1 (2026-10-22)

### Changed
- **The `levels` extra needs roadstyle 0.24.2.** Its pages cut no tiles of the roads source in simple mode, so a zoom no longer freezes the page at each tile (the page took 40 to 70 ms to read the reply of each tile). A road is picked on the pieces of the road layer.

## 0.6.0 (2026-10-21)

### Added

- **A feature layer `paved`**: squares (`place=square`), pedestrian areas (`area:highway=*`, a pedestrian way with `area=yes`, or a
  multipolygon relation), piers, breakwaters and quays, helipads, marketplaces. They were in no table, so the map had empty places
  where OSM has areas (Monaco: Place du Casino and 82 more). Design: `docs/design/missing_polygons.md`.
- `leisure=swimming_pool` is in `water_polygons` (`kind` = `swimming_pool`), `leisure=beach_resort` and `natural=shrubbery` are in
  `land`, and `tourism=zoo` is in `sites`. `amenity=grave_yard` is in `land` and `railway=platform` in `sites` (found by `features-check` on Norrmalm).
- **`duckosm features-check DB`**: lists the polygons of `raw.*` that no feature table holds, grouped by tag, with the reason when the
  build leaves a group out on purpose. A group with no reason is a possible miss.

### Changed

- The `viz` extra needs mapstyle 0.7.0: the maps (`duckosm viz`, `route-map`, `levels-edit`) have the button and the row for buildings in 3D. The docs maps are drawn again with it.
- `duckosm route-map` has `--google-3d` / `--no-google-3d`, as `viz` has: the Google 3D window is on when
  `GOOGLE_MAPS_KEY` is set.
- `duckosm route-map`: a new side panel. The mode and the weight are segmented buttons. The start and the
  end are pins with the letters A (violet) and B (amber), and the panel shows the name of their road, with a swap button. The summary has a bar of the legs, and the
  steps are a timeline with icons. The settings and the `edge_id` list are in a closed block. The panel
  is light, with a button for a dark theme, and on a phone it is a sheet under the map. Design: `docs/design/route_planner_panel.md`.

## 0.5.0 (2026-10-20)

### Added

- `duckosm levels-edit DB`: edit the drawing order (levels) of a database in a browser. It uses
  mapstyle's base map and shows every road, also the roads in `private_edges`. The editor and the
  maps use one port (8000).
- The build makes the drawing-order tables (`visualization.edge_levels`, `level_edits`,
  `level_heads`, `level_caps`, `level_unmatched`) as its last step (`options.levels`, default
  `true`). A rebuild keeps your edits. A clip build copies the rows of its parent. The `levels` extra
  needs roadstyle 0.22.2 or later.
- `driving.private_edges` holds the roads that cars cannot use freely, each with a label: `private`,
  `bus`, `delivery` or `pedestrian`. Every `highway=pedestrian` street stays in the driving build.
- `edge_graph` has the turns of every restricted edge, labelled in the new `uses` column.
  `routed_graph(..., uses=)`, `to_networkx(..., uses=)`, `write_graph(uses=)` and
  `export-graph --uses` select the labels. By default a reader gets only the edges of its own mode.
- `export-gis --format geoparquet` and `to_gis(..., fmt="geoparquet")` write one GeoParquet file per
  layer into a folder: `edges_<mode>.parquet`, `nodes_<mode>.parquet` and `boundary.parquet`, and
  `layers.json` (the list of the files), with one file per `features.*` table. GeoLibre
  (`?data=<url>`) and RustGIS open these files.
- Each edges file gets the columns of `visualization.edge_levels` (all but `edge_id`) when the table
  exists. An edge with no row there gets NULL values.
- Turn restrictions in full: exceptions, conditions, vehicle classes and via ways (a via-way
  restriction can name several `to` ways). SUMO joined junctions keep their turns.
- `osm_overrides` rule `remove`: drops any way from `raw.ways`.
- `duckosm viz`: `--google-3d` (default on when `GOOGLE_MAPS_KEY` is set), one page for each mode,
  and a blank base map in the roadstyle script. The `viz` extra needs mapstyle 0.5.1 or later.
- Boundaries for Norrmalm and Sundbyberg (`data/boundaries/`).

### Fixed

- SUMO and GMNS: roads are centred on their line, a path is one lane of path width, and a bridge
  has no turn-back. `edge_attrs` takes ids as text too.
- `global_junctions` counts the pedestrian ways that the driving build keeps. Before, six of them
  were lost (Norrmalm: 2293 driving edges, now 2322).
- `duckosm route-map`: the planner page opens with MapLibre 5 (roadstyle 0.23 and later). Before, it read
  the road data from a field that MapLibre 5 moved, and it did not start.
- OSM fixes: Boulevard du Larvotto and Beausoleil (Monaco), Granville Bridge and Fir Street (lane
  counts, lane arrows), Centralbron at Tegelbacken, and 8 streets in Norrmalm.
