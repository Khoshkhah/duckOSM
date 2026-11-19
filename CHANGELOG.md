# Changelog

## Unreleased

- **Level 0 of the outputs: the OSM file** (`docs/exports/osm-file.md`). `duckosm get "Monaco"` finds the boundary
  (a name, an OSM id, a box, an H3 cell or a GeoJSON file), the smallest Geofabrik extract that holds it (the rule of
  Studio's page, now in Rust: `duckosm_build::get::covering`), downloads it to `~/.cache/duckosm` (resume, progress, MD5
  check) and cuts it. `duckosm cut --fixes FILE` writes the OSM fixes into the `.osm.pbf` file with the rules of
  `quack --fixes`, and each turn rule as a restriction relation. Python: `duckosm.get()`. The new page
  `docs/reference/boundary.md` gives the format of a boundary file. `get` and `cut` stop a boundary that is not in WGS 84
  longitude and latitude, and a ring that is not closed.

- **One roadstyle version, roadstyle main `53135c49`.** The crates pinned three roadstyle commits, and the `duckosm` pages
  (`viz`, `route-map`, `edit`, `ask`, `studio`, `gmns-map`) had an old dashboard. All roadstyle crates now take one rev from
  `[workspace.dependencies]` in `rust/Cargo.toml`. The docs maps are made again with it.
- **`duckosm quack --osm2pgsql`: the four tables of osm2pgsql** (`planet_osm_point`, `planet_osm_line`,
  `planet_osm_polygon`, `planet_osm_roads`), as osm2pgsql 2.3.1 writes them with its `default.style` and `--hstore`
  (`rust/duckosm-core/src/osm2pgsql.rs`, `docs/exports/osm2pgsql.md`). Into a PostGIS database (`--postgis`), as GeoParquet or
  as DuckDB, in EPSG:3857 or EPSG:4326 (`--latlong`), with the OSM fixes (`--fixes`) and tile by tile. On Monaco each row is
  equal to osm2pgsql's, byte for byte (the golden test `quack_osm2pgsql_monaco.rs`).

## 0.8.0 (2026-11-18)

The program is `duckosm`, in Rust, and `pip install duckosm` installs it (wheels for 5 systems). The exports, the routers, the maps, the editors and GMNS run in Rust. The Python API keeps its names and calls Rust. The commands `levels`, `bus`, `multimodal` and `clip-pbf`, and the build steps of `DuckOSM.run()`, still run the Python code.

- **The switch, step 6 of `docs/design/all_in_rust.md`.** The Rust program is `duckosm`. `duckosm-rust` stays for one release:
  it prints "use duckosm" and runs the same command. The wheel has the whole program in the module `duckosm._rust` (with
  DuckDB), and the command `duckosm` of the wheel runs it. The Python API keeps its names and calls Rust
  (`src/duckosm/_api.py`): `to_sumo`, `to_gis`, `to_matsim`,
  `to_matsim_lanes`, `to_opendrive`, `to_railml`, `to_lanelet2`, `to_elevation`, `route`, `Router`, `route_multimodal`,
  `route_points`, `route_multimodal_points`, `directions`, `build_lane_graph`, `route_lanes`. The GMNS functions and the
  commands `levels`, `bus`, `multimodal` and `clip-pbf` still run the Python code. `Router` has no `.graph` (the graph is
  in Rust). A connection to an in-memory database runs the Python code. `DuckOSM.run()` is as before (the Python steps
  with the Rust network step): the whole build in Rust is not yet equal to it (`docs/design/all_in_rust.md`, step 6).
  `import duckosm` loads the names when they are first used.

- **`duckosm gmns-map`: the lane map of a GMNS file is in Rust** (`rust/duckosm-edit/src/gmns_map.rs`,
  `docs/design/all_in_rust.md`, group C), with the options of `duckosm gmns-map` (`--source-db`, `--area`, `-o`). It calls
  lanestyle's Rust engine (the crate `lanestyle` 0.4, a Git dependency): the roads of the level area as roadstyle draws them, each
  as wide as its lanes, with the lanes, lane lines, arrows, zebras and sidewalks. lanestyle's items equal those of the Python
  lanestyle 0.3.1 on Monaco and Tartu (lanestyle's golden check). The level area is still made by `duckosm levels`. Monaco: the
  page in 3 s.

- **`duckosm levels-edit DB`: the level editor in the Rust program** (`docs/design/all_in_rust.md`, group F). It is the page of
  `duckosm studio` on the one file, with card 8 (Edit levels) open: roadstyle's level editor over the area's map. "← Steps"
  writes the drawing order into the file. Options: `--port` (default 8000) and `--no-open`.

- **`duckosm route-map`: the route planner page is in Rust** (`rust/duckosm-edit/src/route_map.rs`,
  `docs/design/all_in_rust.md`, group F), with the options of `duckosm route-map`. The page routes with the program's own
  router: the new crate `duckosm-planner`, built as WebAssembly into the page, in place of the page's JavaScript copy of the
  router. The routes and the directions are those of `duckosm route-points`, `route-multimodal-points` and `directions`
  (Monaco: 420 questions; a tie of equal cost can take another road). The multimodal router has a length weight
  (`Weight::Length`) for the page's Shortest walk + drive. A new CI job checks the page in Chromium, also at phone width.

- **`duckosm viz`: the maps are in Rust** (`rust/duckosm-edit/src/maps.rs`, `docs/design/all_in_rust.md`, group F), with
  the options of `duckosm viz`: one page with every mode and one page for each mode with only its roads, drawn by roadstyle's
  Rust engine (the engine of the `duckosm edit` page). The build's `viz.enabled` now draws them too, so the config of
  `duckosm` has no skipped step. Monaco: the four pages in 3 s.

- **`duckosm lanelet2`: the Lanelet2 export is in Rust** (`rust/duckosm-export/src/lanelet2.rs`,
  `docs/design/all_in_rust.md`, group D). The offset line of shapely (`offset_curve`) is a pure Rust port of GEOS 3.13.1, with
  no GEOS library: the raw offset curve, and the buffer polygon of the line (GEOS's noder, planar graph, depths and polygon
  builder, `src/geos.rs`). On Monaco and Tartu the `.osm` file has the same bytes as the Python export.

- **The Rust router holds less for a large area** (France on a Mac mini, `docs/guides/route.md`, "Large areas"). The graph is
  flat arrays of arcs in both directions (12 bytes for each turn), the searches keep dense labels, and a trip across modes
  reads only the modes of `--modes` and makes its indexes one time. France driving (25.9 million edges): the route Paris to
  Marseille took 7.0 s with a peak of 8.7 GB, now 3.7 s and 6.4 GB. Île-de-France, Versailles to Bastille on foot and by car:
  2.6 GB with the two modes, 4.4 GB with all. The routes are the same as before (and as Python's).

- **`duckosm elevation`** (`docs/design/all_in_rust.md`, group G), in `rust/duckosm-tools`, with the options of
  `duckosm elevation` (`--dem`, `--source`, `--mode`, `--nodata-fill`, `--suffix`, `--product`, `--vertical-datum`, `--license`,
  `--not-reference`). It writes the same heights and the same `main.elevation_metadata` as Python on Monaco (a DEM file, a crop
  with nodata, `--source copernicus`) and Tartu (`--source copernicus`, 6 s in place of 4 min 17 s). The DEM is a GeoTIFF in
  EPSG:4326 with 32-bit floats; the downloads use DuckDB's httpfs. The DEM catalog (`dem_sources.json`) has a Rust reader too.
  The DEM reader of the build now finds the pixel of a point as rasterio does, so a point on a pixel edge gets the same
  height as in Python.
  It writes `edges` again in one step (not a row-by-row UPDATE) and loads one Copernicus tile at a time, so it runs on
  France driving (12.3 M nodes) in 7 min on a Mac mini with 16 GB ([Add elevation](docs/guides/elevation.md#in-rust)).

- **`duckosm lane-graph` and `route-lanes`** (`docs/design/all_in_rust.md`, group E), with the options of the Python
  commands: the lane graph of a GMNS file (the same SQL, the same rows in the same order) and the lane route (a port of
  networkx's `dijkstra_path`). On Monaco and Tartu, `lane_<mode>.lane_edges` and 126 lane routes (with and without the stored
  graph) equal Python's, with the text, `--json` and the `-o` GeoJSON (`duckosm-route/tests/lanes.rs`). With this, every part of
  group E is in Rust.

- **`duckosm directions`** (`docs/design/all_in_rust.md`, group E): the turn-by-turn steps of `directions.py` for any route
  result (one mode, across modes, between points), in `duckosm-router` (no DuckDB). On Monaco and Tartu, the steps of 1,423
  routes (about 23,000 steps) equal Python's, with their texts (`duckosm-route/tests/multimodal_points.rs`).

- **`duckosm route-multimodal`, `route-points` and `route-multimodal-points`** (`docs/design/all_in_rust.md`, group E): the
  trip across modes (`route_multimodal`, every part on the edge graph of its mode, walk* vehicle* walk*), and the routes between
  two points (`route_points`, `route_multimodal_points`), in `duckosm-router` (no DuckDB). A point joins its roads with GEOS's
  own formulas (distance, project, interpolate, length), so `fraction`, `access_m` and `road_point` are the same floats as
  shapely gives. Each command prints duckOSM's dict as JSON, and `--batch FILE` answers many queries on one graph. On Monaco and
  Tartu, 1,500 queries give the same JSON as Python, floats and error texts too (`duckosm-route/tests/multimodal_points.rs`).

- **`duckosm route` and `export-graph`** (`docs/design/all_in_rust.md`, group E), in the new crates `rust/duckosm-router` (the
  algorithms: no DuckDB, no threads, it builds for WebAssembly) and `rust/duckosm-route` (the database, GraphML, the CLI). The
  routing graph is in memory with the order of networkx's graph, and the route is a port of networkx's bidirectional Dijkstra
  (the same heap keys), so a tie gives the path of duckOSM's `routing.route`. `route DB FROM TO` prints `route()` as JSON;
  `--pairs FILE` routes many pairs on one graph, as `Router` does. `export-graph` writes the same GraphML files as duckOSM
  (node graph and edge graph), byte for byte; `gpickle` (a Python pickle) stays in Python. On Monaco and Tartu, 1,200 routes of
  driving and walking, by time and by length, have the same edges, `time_s` and `length_m` as Python
  (`duckosm-route/tests/golden.rs`). With `weight=length` Python's graph comes from a join with no fixed row order, so its ties
  change from run to run; the Rust graph keeps the order of the edge graph. Python's text of a float (`place::py_float`) and
  the JSON of the tools now round a tie of the shortest digits to the even digit, as Python does.

- **`duckosm ask`** (`docs/design/all_in_rust.md`, group B): the page of `duckosm ask` (roadstyle's data dock and Ask on a
  database), with the native database in place of Python's (`DB`, `--fixes`, `--port`, default 8791). The page and the answers of
  `/api/info`, `/api/query`, `/api/sql`, `/ask/*.js` are the same bytes as Python's on Monaco. With this, every command of group B
  is in Rust.

- **`duckosm extract` and `admin`** (`docs/design/all_in_rust.md`, group B), with the options of the Python commands. `admin`
  reads the boundary relations of the PBF with GDAL in DuckDB's spatial extension (no `ogr2ogr`), and `--gpkg` writes or reuses
  a GeoPackage; `extract` cuts a database to an area by name, OSM id or GeoJSON file, with the time zone of the Rust build. On
  Monaco (all three kinds of area, both `--gpkg` cases) and with the Tartu PBF, every table equals the Python output
  (`duckosm-tools/tests/golden.rs`).

- **`duckosm boundary`** (`docs/design/all_in_rust.md`, group B): the options of `duckosm boundary` (`NAME`, `--pbf`,
  `--osm-id`, `--offline`, `-o`). It looks in the borders of the PBF with GDAL in DuckDB's spatial extension (no `ogr2ogr`), then
  on Nominatim, and writes the same GeoJSON file, byte for byte, with the same lines (the other matches too). A boundary that a
  build makes from `boundary.place`, `bbox` or `h3_cell` is now written in the same format as Python's.

- **`duckosm info`, `way` and `init-config`** (`docs/design/all_in_rust.md`, group B), in the new crate `rust/duckosm-tools`.
  They have the options of the Python commands and give the same output: the text and `--json` of `info`, and the text (DuckDB's
  table print), `--json` and the `--out` files (.csv, .json, .parquet) of `way`; `init-config` writes the same template. Checked on
  Monaco (both builds, a clip) and Tartu; a golden test keeps the Python output (`tests/data/golden_tools/`). Two differences:
  a `.parquet` file of `way --out` names DuckDB 1.5.6 in its metadata (its rows are equal), and `way` takes a negative id with no
  `--` before it.

- **`duckosm`: the bus routes of a build with no boundary.** A build of the whole file (the `.duckdb` output) reads the
  route relations, their member ways and the elements of the fixes from `raw.*` (`RawInput::raw_part`), applies the fixes, and
  makes `bus.routes` and `bus.route_edges`, as duckOSM does. Before, it made no bus tables. Monaco (`tests/bus_whole_file.rs`)
  and Tartu (the cut file as a whole file) equal the Python build row by row.

- **`duckosm --config`: the base-map layers without a boundary, and `duckosm features-check`, are in Rust**
  (`docs/design/all_in_rust.md`, group A). Without a boundary, the layers run on the `raw.*` of the whole file with duckOSM's
  own `geom.way_line`, and the sea uses the box of the nodes, as in duckOSM. The OSM fixes change `raw.*` first, as
  duckOSM's `_raw_way_fixes` and the arrows of `OSMOverrides` do (`after::raw_fixes`). `features-check` (`rust/duckosm-build/src/
  features_check.rs`) prints duckOSM's report; groups with equal keys come in the order of their names. On Monaco's whole sample
  file, the 11 layers and the lines of the report equal the Python build (`tests/features_no_boundary.rs`). With this, every
  key of a `duckosm build` config runs in `duckosm --config` with no skip, except `viz.enabled` (the maps, group F).


- **`duckosm --config`: `source.type: duckdb` is in Rust** (`rust/duckosm-build/src/clip.rs`, `docs/design/all_in_rust.md`,
  group A). The build copies the rows of the parent database in the boundary, with `clip.predicate`, and then makes
  `main.boundary`, `main.visualization_metadata`, the drawing order, the indexes, the checks and the report. On Monaco
  (`intersects` and `within`) every table of the clip, `main.boundary`, the metadata and the indexes equal the Python clip
  (`tests/clip.rs`). Two differences: the Rust clip has `turn_path_restrictions` (the SQL of duckOSM fails in DuckDB, and
  duckOSM drops the error), and the drawing order has the numbers of the Rust solve, as in a Rust build from a PBF.

- **`duckosm --config`: `boundary.place`, `bbox`, `h3_cell`, `buffer_m` and `options.boundary_cells` are in Rust**
  (`rust/duckosm-build/src/place.rs`, `docs/design/all_in_rust.md`, group A). The box, the cell and the place become
  `<name>.boundary.geojson` next to the database, as in duckOSM. A place is looked up in the borders of the PBF with GDAL in
  DuckDB's spatial extension (the polygon equals the one of `ogr2ogr`), then on Nominatim. On Monaco the cut networks,
  `main.boundary` and the ids of `main.boundary_cells` equal the Python build (`tests/boundary_kinds.rs`). The outline of an H3
  cell (`h3_cell`, the cells) comes from `h3o` and differs from the H3 C library by about 1e-14 degrees.

- **`duckosm --config`: the indexes, the checks, the report and the multimodal graph are in Rust**
  (`rust/duckosm-build/src/after.rs`, `docs/design/all_in_rust.md`, group A). The `.duckdb` file gets duckOSM's indexes,
  `validation:` runs duckOSM's checks (a failed check stops the build with `fail_on_error`), `report:` writes
  `reports/<name>_<time>.md` and `.html`, and `multimodal:` keeps `mm.*` with the costs of `transfer_s` and `transfer_costs`
  (without `multimodal.enabled`, the file has no `mm.*`, as in duckOSM). `clip.predicate` and `clip.min_component_edges` have no
  effect on a build from a PBF, so they are no longer errors. On Monaco the tables, the indexes, `mm.transfers` and the results
  of the checks are equal to the Python build (`tests/after.rs`). The report counts the rows of the tables; the Python engine
  counts the edges before the private roads move to `private_edges`.


- **The check page of a GIS export is in Rust**: `duckosm gis-debug` writes the same page and the same `--json` as
  `duckosm gis-debug`. It reads the export through GDAL (DuckDB's `ST_Read`), not geopandas. The page template moved to
  `src/duckosm/templates/gis_debug.html`, and Python and Rust both use it.

- **The SUMO export is in Rust**: `duckosm sumo` writes the same plain XML and `.netccfg` as `duckosm sumo` and
  runs netconvert in the same way (`SUMO_HOME`, the PATH, or the `eclipse-sumo` package of the active Python environment).

- **The OpenDRIVE and MATSim lanes exports are in Rust**: `duckosm opendrive` (also `--junctions`) and
  `duckosm matsim-lanes` write the same files as the Python exports (Monaco and Tartu).

- **The GIS export is in Rust**: `duckosm export-gis` (GeoPackage, GeoParquet, shapefile) writes the same
  columns and rows as `duckosm export-gis` (Monaco and Tartu). It uses `ogr2ogr` for one GeoPackage file, as the Python export.

- **The MATSim and railML exports are in Rust** (`docs/design/all_in_rust.md`, group D): `duckosm matsim` and
  `duckosm railml`, in the new crate `rust/duckosm-export`. They write the same file as `duckosm matsim` and
  `duckosm railml` (Monaco and Tartu). A golden test holds the Python output on Monaco.

- **The sorted join of quack by tiles keeps less on the disk.** `quack_join_to` drops each part when it has read all its rows,
  so the caller can delete the file of the part before the join ends (the browser does). The sort writes the pieces of each
  strip over the pieces of the strip before, so its temporary file holds one strip, not all of them. Chromium on the Mac mini,
  sorted: the peak of the browser storage on the disk of Sweden 9.4 GB to 3.3 GB; France now finishes, in 340.2 s with at
  most 17.9 GB (it stopped at 37.6 GB before).

- **The sorted quack by tiles: each tile sorts its rows, and the join merges the sorted parts** (`docs/rust/quack.md`, "The
  sort tile by tile"). `quack_tile_to` gets `sort`: the part has its rows in the order (x, y, row number), in row groups of 2 048
  rows. `quack_join_to` merges the parts with a heap and makes QuackOSM's strips with no runs on disk, and it gives its progress.
  The order is the order of `str_order`, as before. `wkb_center` reads the points in place. Chromium on the Mac mini, sorted:
  the join of Île-de-France 8.9 s to 5.8 s and 965 MiB to 626 MiB; the tiles 14.1 s to 17.7 s (one large tile sorts alone).

- **The bus routes and the levels after the union of a build by tiles: memory that does not grow with the area**
  (`docs/design/tail_steps_by_tiles.md`). The bus step puts the route relations and the member ways into temporary files sorted
  by batch, and each batch reads only its own data; the read of the union edges decodes only the columns of the walk. The levels
  keep the numbers of an edge only until the last tile that reads them starts. France (Mac mini, native): the bus step 14 s to
  9.3 s and 609 MB to 171 MB of heap; the tiles of the levels 187 MB to 67 MB. The bus tables and `edge_levels` are equal to
  before, row by row (Kentucky, Sweden, France, both level methods), and Kentucky by tiles equals the whole build.
- **`duckosm tiles --keep-tiles` keeps the tables of the tiles.** The union deleted them for each table with border rows.
- **The tile cut is the pbfcut library** (`Khoshkhah/pbfcut`, one pinned git `rev` in `rust/duckosm-core/Cargo.toml`). duckOSM's own
  cut (`tilecut.rs`, `tilecut/write.rs`) is gone; `duckosm_core::tilecut` keeps its names and gives pbfcut the rule of the junctions
  (`NodeUses`). `boundary`, `tiles`, the temporary store of `spill`, `util::FxHasher` and `cutstream::CutCounts` are now the types
  of pbfcut (re-exports), so the cut and the build share them. Before the move, the two cuts gave the same elements in every tile file
  (Kentucky zoom 8: 20 files, France zoom 8: 104 files, `pbfcut compare`).
- **The tile cut uses all threads in each part** (`tilecut.rs`): the relation blocks are decoded on all threads, the
  outside rows are encoded on the pool, and in part 5 each blob of a tile file with its extra elements is one job, so one
  large tile also uses all the threads. In the browser, part 5 makes all the tiles on the pool at the same time. Sweden at
  zoom 6 in Chromium: the cut 49.5 s to 29.2 s. Kentucky at zoom 9 natively: part 5 0.93 s to 0.55 s, peak 497 MB to
  523 MB, the same elements in all 44 tile files. The quack cut and the cut of the build by tiles are again one cut.
- **Quack tile tables: `duckosm quack-tiles` and `quack-union`** (`docs/rust/quack.md`, "Tile tables"). The cut makes the file of each
  map tile of one zoom; each tile alone makes its table (the nodes in the tile, the ways with a node in it, the relations that its file has
  whole) with a column `owner`, so nothing waits for another tile; the union keeps the rows with `owner`. France at zoom 9: 697 tables in
  232.6 s (3.7 GB RSS); the union (43.6 s) is byte for byte the table of one run.
- **Quack by tiles is faster natively**: no zlib in the temporary tile files, and 4 tiles at a time. France: 224.9 s to 171.3 s (one run:
  104.3 s); peak RSS 1.8 GB to 3.7 GB.
- **`duckosm quack --tile-zoom Z`: tile by tile with the map tiles of one zoom** (`TileSize::Zoom`), in place of the quadtree of
  `--tiles N`. Île-de-France in Chromium: zoom 8 (13 tiles) 42.5 s and 474 MiB, zoom 9 (32 tiles) 41.7 s and 467 MiB. France natively: zoom 8
  (241 tiles) 200.4 s, zoom 9 (697 tiles) 222.5 s.
- **`duckosm quack --tiles N`: the features table of QuackOSM tile by tile** (`docs/rust/quack.md`, "By tiles"). The file is cut into a
  quadtree of tiles with at most `N` nodes, and each tile gives the rows that it owns, so the memory follows the size of a tile: the mode of the
  browser for a large file. The same rows as one run (Monaco test; Kentucky and France with `quack_diff --large`). France: 223.5 s and 2.0 GB of
  RSS (one run: 105.9 s, 4.6 GB). The tile cut can now keep its files in the temporary store (`tilecut::cut_to_store`, OPFS in the browser) and
  takes tiles of mixed zooms. The on-disk sort of quack keeps a large strip on disk in sorted pieces. In the browser, quack makes the relations in batches
  of 4 million points (before: all at once). Île-de-France in Chromium: 41.4 s and 429 MiB tile by tile (one run: more than 4 GiB).
- **The Rust build computes the global junctions one time** (`duckosm-core`, `build_network_junctions`). Before, it computed them for each mode and again for
  `main.global_junctions`. The output is the same. The gain is small (the junctions take 0.06 s for Estonia, 0.19 s for Kentucky, on a Mac mini); the change
  lets a later step give the network step the junctions from DuckDB (`docs/design/streaming_load.md`, stage 4).

- **`read_osm`, the load of `raw.*` in Rust** (`duckosm-duckdb`, `docs/design/streaming_load.md`). A DuckDB table function, `read_osm(path, kind)`, gives the
  nodes, the ways or the relations of a `.osm.pbf` file with the columns and the types of `ST_READOSM`, and DuckDB calls it from all its threads. A whole-file
  build of `duckosm` now has `raw.nodes`, `raw.ways` and `raw.relations` (before: none), and it reads `options.memory_limit` and `options.threads` for this
  step. France, all of `raw.*`: about 30% less CPU than `ST_READOSM`. A test compares it with `ST_READOSM` on Monaco. `duckosm build` (Python) keeps `ST_READOSM`.
- **A fast PBF decoder, `pbfraw`** (`duckosm-core`). It reads the protobuf bytes in place: 5 times faster than the `osmpbf` crate on the way blocks of France,
  and faster than libosmium (`osmium fileinfo -e --no-crc`) for the data of `raw.*`. A test compares it with `osmpbf` on Monaco.
- **The Parquet load in a memory budget** (`duckosm-parquet`, feature `pbf`), for the browser: `load_raw` writes `raw.*` as Parquet shards while it reads,
  and a test with a counting allocator fails if it goes above its budget. Nothing calls it yet.

### Host changes

For a host that runs the fixes editor (duckOSM Studio, `duckosm edit`). Each entry says what the host must do.

- **The tile cut is pbfcut** (duckOSM: the pbfcut move). `tilecut::cut_to_sink` keeps its arguments and its tile files (no zlib; with `memory`,
  zlib, in place of the parts in memory). `spill::set_temp_store` also sets the store of the cut (one store). *Host: nothing is
  needed; Studio moves its pin.*
- **The editor reads the map's road table itself** (duckOSM #43). `roadIdOfRow` and `roadRows` are now optional. Without them the editor reads `window.RS_SIDE.props`
  (then `rsGetProps` and `rsQuery`) and builds the index of the roads by OSM way. A row of the table is an object, or an array when the host gives the new option
  `roadKeys` (the column names). `roadRows: false` keeps the editor's own line layers. *Host: nothing is needed. Studio can drop its own `roadIdOfRow` and `roadRows` and give `roadKeys`.*
- **The editor files are a library crate, `duckosm-editor`** (duckOSM #45). It has the four files of `web/fixes_editor/` (`FILES`, `file(name)`). *Host: nothing is needed. A Rust host can add the crate
  at the same revision as `duckosm-core`, serve `FILES` from one folder, and remove its step that fetches and copies the folder.*
- **The Ground step: `web/ground_step/`** (`initGroundStep`). It shows the DEM sources that cover the area, best first, and gives the build the file with its product, vertical datum and licence from the catalog, or a DEM from the AWS Terrain Tiles (product `DSM`, label "Ground: AWS Terrain Tiles", not a reference). *Host: duckOSM Studio moves its pin, copies `web/ground_step/` and `src/duckosm/data/dem_sources.json` in `build.sh`, and uses the step in place of its plain DEM field. Other hosts: nothing is needed.*
- **Download in the data step and in the Ground step** (`web/build_steps/data_step.js`, `web/ground_step/ground_step.js`). A host with a folder gives the new option `files`: each extract row and the chosen DEM row then have Download (into the folder), then Use or In use, as Studio's rows, and the build does not download. A chosen extract keeps its outline on the map until it is deselected. *Host: nothing is needed; a host with no `files` (Studio in the browser) is as before.*
- **The Visualization step opens the map in a new tab** and does not open it by itself at the start (`auto` is now false). *Host: nothing is needed.*
- **`askTables` names an entry `mode.table` when it has both** (`web/ask/schema.js`). *Host: nothing is needed; a host that passes its entries as they are (Studio, whose `name` is a file name) moves its duckOSM pin.*

- **A host can give the data dock `kinds` as a function and read it again** (roadstyle `f3d1efc5`, PR 134 and 136; `viewKinds` needs no change). A host whose tables change after the start (Studio: a build with other layers) gives `kinds: () => viewKinds({ names, hasTags, h3, query })`
  and calls `await data.refreshKinds()` after the change. The dock then draws the rows of the new tables. *Host: nothing is needed. `duckosm ask` adds no table while it runs, so its page still gives the object. Studio replaced its proxy of kinds with the function.*

### Fixed

- **`duckosm studio`: a click on a road in card 8 (Edit levels) selected nothing** (roadstyle `9df3ceda`, PR 171). The editor needs the whole road table, and the
  map of way D keeps none in the page: roadstyle's level part now borrows it from the server (`/areas/<name>/road_table`, `road_ends`) while the editor is open.
- **`duckosm studio`: the map has the 3D walls by type** (`GET /areas/<name>/walls`; `page::walls_of`, Studio's wallsData on the server). Each wall of
  `main.building_faces` with the height of its building, cut into tiles of the area's map when the walls first show (roadstyle's `set_walls`).
- **`duckosm studio` on a phone: the base map stays blank when an area opens** (roadstyle `d07131ab`, PR 169). The page asked for the blank map before the first base map (OpenFreeMap) had come, and the late base map came back over the area. Now only the last choice of a base map is used.
- **`duckosm studio` has no patch for a page with no roads** (roadstyle `8dfd5e2c`, PR 153; `rust/duckosm-edit/page/studio.js`, `web/build_steps/steps.css`, `web/ground_step/ground_step.css`). The roadstyle dashboard now shows no banner "0 road features" on a page with no roads, opens on the world, and sets `text-transform` and `letter-spacing` of an option row (`label.rs-opt`) back. The page removed its three workarounds: the script that removed the banner, the jump to Europe, and the two CSS resets. *Host: move the roadstyle pin to `8dfd5e2c` and remove the same patches. A host that sets its own first view keeps it.*

### Changed

- **`duckosm studio`: the map of an area is roadstyle's way D** (roadstyle PR 168; `rust/duckosm-edit/src/studio.rs`, `page/studio.js`). The server keeps a
  `roadstyle_local::Local` for each open area (the last 3) and answers its addresses at `/areas/<name>/`: `style.json`, the tiles, the rows and the lines of the
  roads, and, in the program `duckosm-rust`, the queries and the schema over the area's `.duckdb` file (read only). The page gives the dashboard
  `rsHttpAdapter`, so a click on a road asks the server for its fields and its line. `GET /api/style/<name>` and `GET /tiles/<name>/...` are removed.
  The Python module has no DuckDB inside: there the map has no queries.
- **`duckosm studio`: step 1 and step 3 compared with Studio, action by action.** The page was run next to Studio in headless Chromium with the same actions (type "burnaby", hover each result, click one, each Draw tool and its key, the File and H3 tabs); the differences were fixed: the text "a point" of a place with no boundary, the File tab text and a file dropped on the map, the boxes of the Draw hint and of the H3 note, the gap between the search field and the list, the size of a tab icon, the panel "?", and no extract is chosen at the start (the person picks the file, as in Studio). *Host: nothing is needed.*
- **`duckosm studio`: the buffer, the fixes file and the build details of Studio.** The card "The area" has **Buffer around the area** (0 to 50,000 m, roadstyle's `rs.number`): the page asks the server (`POST /api/buffer`, DuckDB `ST_Buffer` in the UTM zone, as Studio does in the browser), and the area on the map, its numbers and the build use the grown area. The card Build has **Fixes**: the path of a fixes file, checked by the server, goes into the config as `osm_overrides` (Load gives it back). **Build details** shows the time of each step and the total. `Host::buffer` is a new method of the trait (a default that refuses). `initAreaStep` has the option `buffer`, `initSettingsStep` the option `checkFile`. *Host: nothing is needed.*
- **`duckosm studio`: the H3 tab and the hover of the search are Studio's.** A place in the list shows its outline on the map (purple, with its name) when the mouse or the keyboard is on it, and a second click on the picked place takes the area away. The tab H3 has the level list (5 to 10), the grid of the hexagons in view, the lit hexagon under the pointer, the picked cells as the area (at most 500 cells and 5,000 km²), and H3 cell IDs in the search box. h3-js 4.2.1 loads at the first use. *Host: nothing is needed.*
- **`duckosm studio`: the search and the Draw tab are Studio's.** The search starts while the person types (after 300 ms, from 2 letters): Photon lists the places, then one Nominatim lookup gives their boundaries (a place with no boundary is listed with no choice), through a queue of one request each second. The Draw tab has the tools of Studio (terra-draw: rectangle, polygon, circle, freehand, select, Merge, Delete, Clear; the keys R, P, C, F, S) in place of one box button. `initAreaStep` options `geocode` (now `(text, signal)`), `boundaries` and `reverse`. *Host: nothing is needed.*
- **`duckosm studio` has the cards of duckOSM Studio** (Kaveh: "all steps should have almost the same interface as Studio except the new step 4"). The page ports Studio's flow: the cards 1 Choose an area (tabs with icons, "What is here?"), 2 The area (name, number tiles, Boundary, Unselect, zoom), 3 The data, 4 Ground (new), 5 Build (the networks, the base map, More options, Build) and 6 The result (a table of the networks), the check mark of a step that is done, the line "after step N", one open step, and the "?" of each card. The base map is OpenFreeMap Liberty, as in Studio. `initAreaStep` takes `detail` (the card "The area") and `rs: { search, drop, tabs, toggle, kpi }`, and `initDataStep` takes `rs: { tabs }`. *Host: duckOSM Studio does not use these two steps; nothing is needed.*

### Added

- **`duckosm studio`: card 8, Edit levels** ([guide](docs/guides/studio.md); roadstyle `1c909e5b`, PR 170). roadstyle's level-edit part (`levelEditPart`, unchanged) over the map of the open area.
  The server gives it the roads of the level area (`GET /areas/<name>/levels/edges`), the stored levels (`levels/stored`) and the save (`POST levels/save`:
  one SQL script, `page::save_levels_sql`, that each host runs on a writing connection, `Host::write`), and way D draws each change again (`POST relevel`).
  roadstyle's page files and the editor's WebAssembly package are at `/rs/` (`roadstyle_serve::embedded`). `rust/duckosm-edit/editor_pkg.sh` builds the
  package in the roadstyle checkout of the pin; the CI job studio-page runs it. The Python `_Db` opens the file in an instance of its own (an attached
  database), so a second reader does not get the locked settings of the page of `/edit/`. Tests: `test_card_8_edits_the_levels_of_an_area`,
  `the_level_editor_save_writes_the_tables_and_stops_on_other_edges`.
- **`duckosm` makes the H3 columns and the base-map layers** (`rust/duckosm-build`). `nodes.h3_cell` and `edges.from_cell`, `to_cell`, `lca_res` (`options.h3_indexing`, `options.h3_resolution`; the Rust crate `h3o`), `features.*` with `raw.*` (`options.build_features`, with a boundary; the SQL of `features.sql` on the DuckDB of the program) and `features.ocean` (`options.sea`, flag `--sea`; needs the network). The `.duckdb` file now equals the file of `duckosm build` for these tables; `rust/duckosm-build/tests/features_h3.rs` compares them on Monaco. `options.boundary_cells` is still skipped. New flags of `duckosm`: `--features`, `--sea`, `--h3-resolution`. *Host: nothing is needed.*
- **`duckosm studio [OUT_DIR]`: a page on your computer that makes the data of an area** ([guide](docs/guides/studio.md)). Six steps as in duckOSM Studio: the area (a place name, a box on the map or a GeoJSON file), the OSM data (a Geofabrik extract, or a local `.osm.pbf`), the settings, the ground (the Ground step), the build with its progress, and the result, which is the page of `duckosm edit` served at `/edit/`. The server is the Rust server of `duckosm edit` (`rust/duckosm-edit/src/studio.rs`); `duckosm studio` is the program (all steps in Rust, with no base-map layers or H3 columns yet); `duckosm studio` is the same page with the Python build, for the cross-check. The page is roadstyle's dashboard (the pin of `duckosm-edit` moves to roadstyle `34e21f30`), and each step is a card as in Studio. The card **Your areas** lists the areas on the disk, with Open (the map and the editor) and Load (the area and the settings back into the steps). The steps are the new parts `web/build_steps/` (`initAreaStep`, `initDataStep`, `initSettingsStep`, `initBuildStep`, `initResultStep`). `duckosm edit` now writes the record of the build also after a Python build, so Build again works on an area that `duckosm studio` made. *Host: duckOSM Studio moves its pin and can use the parts `initAreaStep`, `initSettingsStep`, `initBuildStep` and `initResultStep`, and `core.js` (`covering`, `settingsYaml`); `initDataStep` needs a path on disk, so it is for a local host only. Other hosts: nothing is needed.*
- **The catalog of DEM sources** (`src/duckosm/data/dem_sources.json`, `duckosm.dem_sources.sources_for_bbox(bbox)`). One entry for each source of [Elevation sources](docs/reference/elevation-sources.md): coverage, resolution, product, CRS, vertical datum, licence, commercial use, access and `cors`. The function gives the sources that cover a box: a national 1 m DTM, EU-DTM, FABDEM, Copernicus GLO-30, then the others. Two tables of the docs page are made from the file (`scripts/update_elevation_sources_doc.py`); a test checks them. Tests: Monaco, Tartu, Stockholm.
- **The ground id: `main.elevation_metadata.sha256`** ([The ground reference](docs/concepts/ground-reference.md)). The SHA-256 of the bytes of the DEM file, the same value as `sha256sum`. `duckosm elevation` and the Rust build (`--dem` and `Params.dem_bytes`) write it. It is NULL when no whole file was read: Copernicus tiles streamed from AWS, or a `--dem` URL. A database from an older duckOSM gets the column on the next `duckosm elevation`.
- **The config names the area's DEM** (section `elevation`: `dem`, `product`, `vertical_datum`, `license`, `reference`; [Configuration](docs/reference/configuration.md#elevation)). The Rust build reads it, so the DEM is set one time for each area, and `duckosm edit` (Build again) keeps it. A `--dem` on the command line wins over the section. The Python `duckosm build` keeps the key and does not sample.
- **A reference DEM needs its product, its vertical datum and its license.** `duckosm elevation --dem FILE` stops without `--product` (`DTM` or `DSM`), `--vertical-datum` and `--license`, unless `--not-reference` (or `--suffix`) is given. The Rust build has the same rule (`--dem-product`, `--dem-vertical-datum`, `--dem-license`, `--dem-not-reference`; `Params.dem_product`, `dem_vertical_datum`, `dem_license`, `dem_not_reference`), and it now writes them instead of `unknown`. A build with no DEM does not change. *Host: a host that builds with `Params.dem_bytes` (duckOSM Studio) gives the product, the datum and the license of the file, or sets `dem_not_reference`, when it moves its duckOSM pin.*
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

- **The Ground step: a drop zone and the ground id** (`initGroundStep({ drop })`, with roadstyle's `rs.drop`). The file field becomes a drop zone, and after a file the step shows its product, vertical datum, licence and ground id (the SHA-256). Without `drop` the step is as before. *Host: nothing is needed.*
- **The Ground step chooses no source at the start** (`web/ground_step/`). It chose the best source, so a file that the person selected without changing the choice got the product, the datum and the licence of that source (a Copernicus DSM file was labelled EU-DTM, a DTM). Now the person names the source first; with no choice `dem()` gives `null`, and a new area clears the choice. *Host: nothing is needed. A test that selects a file must choose the source first (duckOSM Studio's `ground_e2e.py`).*
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

- **`duckosm quack --gdal`** (`docs/rust/quack.md`, "GDAL layers"): the five layers of GDAL's OSM driver (`points`,
  `lines`, `multilinestrings`, `multipolygons`, `other_relations`, GDAL's default `osmconf.ini`), made by the quack engine in one
  pass, as a GeoPackage (default) or one GeoParquet file for each layer (`-f geoparquet`), also by tiles (`--tile-zoom`). On
  Monaco, each layer equals `ogr2ogr -f GPKG` of GDAL 3.13.3 row by row (`duckosm-build/tests/quack_gdal_monaco.rs`). Kentucky
  on the Mac mini: 4.2 s and 0.25 GB against 8.9 s and 0.44 GB for `ogr2ogr`.

- **quack --gdal tile by tile in the browser** (`docs/rust/quack.md`, "GDAL layers"): `quack_gdal_tile_to` gives the rows that one
  tile owns, alone (the tile of the first node of the first member way owns a multipolygon), and `quack_gdal_join_to` joins the files
  of all tiles into the five layers; duckOSM Studio's quack page uses them. On Monaco, each tile alone with the join equals
  `ogr2ogr` (`duckosm-build/tests/quack_gdal_monaco.rs`).

- **quack --gdal: France on the Mac mini** (`docs/concepts/quackosm.md`): GeoParquet by tiles in 3 min 24 s against 14 min 18 s
  for `ogr2ogr -f Parquet` (five runs, one for each layer: GDAL's Parquet driver writes one layer to one file), 5.0 GB against
  1.6 GB of memory, 11.3 GB against 15.6 GB of files. The files of the tiles of the browser are Snappy frames now (France: about
  30 GB of rows without them).

- **quack --gdal by tiles: 0.9 GB on France, and no stop** (`docs/rust/quack.md`, "GDAL layers"). Each tile sends its rows at
  once (a group of four tiles kept 8.8 GB of rows), and the writer encodes on a pool of its own (on the engine's pool, a run of
  France stopped: the writer and the relations waited for each other). France: 3 min 37 s and 0.9 GB, against 5.0 GB before
  and 1.6 GB for `ogr2ogr`. `duckosm-build/tests/quack_gdal_full.rs` runs Monaco with full channels.

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
  written). `duckosm edit` is the same page in the Rust program. A build writes its record for this in
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
