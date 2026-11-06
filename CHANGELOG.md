# Changelog

## Unreleased

### Changed

- **The fixes editor is in roadstyle's editor frame, and the roads are in roadstyle's highlight sets** (Kaveh,
  `docs/design/fixes_editor.md`). `initFixesPanel(panel, ctx)` takes the dashboard's panel (`#sb`) and `ctx.dashboard` (`window.rs`,
  roadstyle `f715e32f` or later). The frame has the title "Edit OSM data", a back button, the six tabs in its head row, the form in its
  body and the buttons of each tab in its one bar at the bottom edge. The grid of grey tabs and the bar of each tab are gone. New hooks:
  `roadRows(osmId)` (the map's rows of a way), `onBack` and `backLabel`. With `roadRows`, the picked road is blue, the changes amber,
  the deleted roads red, the roads that leave a junction green and the road under the pointer in the Turn list dark, at the road's
  own width. Without it, the old line layers stay. The editor returns `frame`. `duckosm edit` gives the new hooks, shows the file name
  in the title line and the status in the bar, and has a card "Edit OSM data" that opens the editor after **← Map**.

### Fixed

- **`duckosm edit`: the editor showed nothing on the map.** The server gives a JSON column (`ST_AsGeoJSON`) as an object, and the editor
  parses text. `duck.query` of the page now gives such a column as text. The selected object, the changes and the walls show again.

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
