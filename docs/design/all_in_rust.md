# All of duckOSM in Rust

Rule (Kaveh, 2026-11-17): every duckOSM command and every part of the build is Rust. Python only calls Rust. The Rust
program takes the name `duckosm`.

## Where we are

The Rust program `duckosm-rust` has: the build (with H3, levels, bus routes, lane profile, base-map layers, OSM fixes,
the DEM in the build), `tiles`, `cut`, `quack`, `quack-tiles`, `quack-union`, `edit` and `studio`.

These parts are only in Python (`src/duckosm/`, about 11,000 lines without the old build glue):

| Group | Contents | Python lines |
|---|---|---|
| A. Build extras | `main.boundary_cells`, `component_filter.min_component_edges`, validation, report, multimodal graph (`mm.edges` with costs), indexes, `source.type: duckdb`, `boundary.place` / `bbox` / `h3_cell` / `buffer_m`, `clip.predicate`, layers without a boundary, `features-check` | about 1,150 |
| B. Database tools | `extract`, `admin`, `info`, `way`, `ask`, `init-config`, `boundary` | about 590 |
| C. GMNS | `gmns` (macro, meso, micro), `gmns_check`, `gmns_sumo`, crossings, `gmns-viz`, `gmns-map` | 3,379 |
| D. Other exports | SUMO, GIS (`export-gis`, `gis-debug`), MATSim, MATSim lanes, OpenDRIVE, railML, Lanelet2, GTFS | 2,572 |
| E. Routing | `route`, `Router`, point routing, directions, `export-graph`, lane graph and `route-lanes` | 1,173 |
| F. Maps | `viz`, `route-map`, `levels-edit`, the planner page | about 1,011 |
| G. Elevation | the `elevation` command: DEM sources, download, write back to the database | 566 |

## Order

Each group is one or more PRs. Each port gives the same output as Python on Monaco (and Tartu when needed): the same
rows, or the same file bytes where the format allows. A golden file holds the Python output before the port.

1. **A. Build extras.** After this, every `duckosm build` config runs in Rust.
2. **B. Database tools.**
3. **D. Other exports**, then **C. GMNS** (the largest part). The exports read the `.duckdb` file with the `duckdb` crate.
4. **E. Routing.** A Rust graph in place of networkx.
5. **G. Elevation** and **F. Maps** (with the roadstyle Rust engine).
6. **The switch.**
   - The Rust program is `duckosm`. `duckosm-rust` stays for one release and prints "use duckosm".
   - `pip install duckosm` installs the Rust program and the Rust binding (maturin), for each platform.
   - The Python API (`DuckOSM`, `to_sumo`, `route`, ...) keeps its names and calls Rust.
   - The old Python code goes; the tag `python-engine-final` keeps it.
   - The docs say `duckosm` everywhere.
   - Release 1.0.0. Then move the pins in Studio, sensor-matching, SonoFlow and the other projects.

## Group D: the state

The exports are in the crate `rust/duckosm-export`. The commands are `duckosm-rust <export> ...`, with the options of the
Python CLI. The golden test (`rust/duckosm-export/tests/golden.rs`) compares each Rust export with the Python output on Monaco;
`tools/make_golden_exports.py` makes the golden files.

| Export | Rust command | State |
|---|---|---|
| MATSim | `duckosm-rust matsim` | Done. The same bytes as Python on Monaco and Tartu (gzip: the same text). |
| railML | `duckosm-rust railml` | Done. The same bytes as Python on Monaco and Tartu. |
| GIS | `duckosm-rust export-gis` | Done. GeoPackage, GeoParquet, shapefile: the same columns and rows as Python on Monaco and Tartu; `layers.json`: the same bytes. |
| GIS check | `duckosm-rust gis-debug` | Done. The same page and the same `--json` as Python (a GeoPackage, shapefiles, and shapefiles with errors; Monaco and Tartu). The export is read through GDAL (`ST_Read`); the page is `src/duckosm/templates/gis_debug.html` for both. |
| OpenDRIVE | `duckosm-rust opendrive` | Done. The same bytes as Python on Monaco and Tartu, with and without `--junctions`. A CRS that is not UTM or EPSG:3006 gets its proj4 text from PROJ's `projinfo` (pyproj uses the same PROJ function). |
| MATSim lanes | `duckosm-rust matsim-lanes` | Done. The same bytes as Python on Monaco and Tartu. |
| Lanelet2 | `duckosm-rust lanelet2` | Done. The same bytes as Python on Monaco and Tartu. The offset line of shapely is ported from GEOS 3.13.1 in pure Rust: the raw offset curve (`src/offset.rs`), and the buffer polygon of the line (`src/geos.rs`: GEOS's noder, planar graph, depths and polygon builder). On all 53,614 lane sides of Monaco and Tartu it gives the same points as GEOS (within 1e-17 m). Not ported: a closed line (GEOS buffers both sides of the ring) and GEOS's second try with less precision when its buffer fails; the Rust export stops with an error there. |
| SUMO | `duckosm-rust sumo` | Done for the command. The plain XML and the `.netccfg`: the same bytes as Python on Monaco (three modes) and Tartu; the `.net.xml` of netconvert: the same but its header comment. Also the second netconvert pass of the turn permissions. The parts of the Python API for GMNS (`gmns_sumo`: the join of nodes, the turn arrows, `bus_edges`, `edge_attrs`, `lanes`) move with group C. |

The exports of a GMNS db read the test tables of `tests/data/exports/gmns_driving/` (the columns that they read, as
Parquet), until group C makes the GMNS db in Rust.

`sumo_compare.py` is not a command: `lane_diff` compares the lanes of duckOSM with the lanes of SUMO's own OSM import, for a
test (`tests/test_levels.py`). It moves with group C (the lane profile).

GTFS (`gtfs.py`) is not an export: it reads the stops of a feed for the GMNS `location` table. It moves with group C.

## Group G: the state

`duckosm-rust elevation` is in the crate `rust/duckosm-tools` (`elevation.rs`, with the catalog `dem_sources.rs`). It has the
options of `duckosm elevation` and writes the same columns and rows: `ele`, `z_from`, `z_to`, `z_vertices` (and the same with
`--suffix`) and `main.elevation_metadata`. The golden test (`elevation_equals_duckosm`, `tools/make_golden_elevation.py`) keeps the
Python output on Monaco. Checked: Monaco with a DEM file, a DEM crop with nodata and `--source copernicus`; Tartu with
`--source copernicus` (6 s, Python 4 min 17 s). All rows are equal. France driving (12.3 M nodes) takes 7 min on a Mac mini.

- The DEM is read in pure Rust with the reader of the build (`duckosm_build::elevation::Dem`, the `tiff` crate): a GeoTIFF in
  EPSG:4326 with 32-bit float values, a file or an HTTP address. Python reads all the formats and CRSs of GDAL. A projected
  national DTM (for example EPSG:3006) needs a Rust projection first.
- `--source copernicus` downloads each 1° tile with DuckDB's httpfs (Python reads parts of the tile over HTTP). `--source eudtm`
  downloads the GeoTIFF of the box from OpenTopography. The AWS Terrain Tiles are not a source of the command: the ground rule
  (`docs/concepts/ground-reference.md`) does not allow them as the ground.
- The pixel of a point is found as rasterio finds it: the inverse affine transform, then floor. Before, the reader of the build
  divided by the pixel size, and one point of Tartu on a pixel edge took the next pixel.
- DuckDB updates a list column as a delete and an insert, and updates an R-tree index row by row. So the command does not
  update `edges`: it makes the table again in one transaction (`CREATE OR REPLACE TABLE … POSITIONAL JOIN` the new heights) and
  sets its indexes, column defaults and comments again (Tartu: the update took 64 s for the cycling edges; the whole command now takes 6 s).

## Group F: the state

The maps use roadstyle's Rust engine: its reader of a duckOSM file (`roadstyle_io::duckosm`, the port of mapstyle's `render_map`)
and its page writer (`page_options::page_from`). The code is in `rust/duckosm-edit` next to the page of `duckosm edit`, which uses
the same engine.

| Part | State |
|---|---|
| `viz` and the build's `viz.enabled` | Done: `duckosm-rust viz` (`maps.rs`), the same pages and options as `duckosm viz`. The config has no skipped step now. |
| `route-map` and the planner page (the router in WebAssembly) | Done: `duckosm-rust route-map` (`route_map.rs`, `page/planner.html`). The page routes with `duckosm-planner` as WebAssembly. |
| `levels-edit` | Done: `duckosm-rust levels-edit DB`, the page of `duckosm-rust studio` on one file (`Studio::for_file`) with card 8 open (`?area=<name>&step=8`). |

- The pages are roadstyle's `duckosm.page(db, mode, only=...)`: the "liberty" theme and the "google" paths of roadstyle's port.
  mapstyle's own `render_map` (the "osm" theme) does not run with the current roadstyle (`palette_to_dict` is gone), so
  `duckosm viz` in Python fails today.
- `levels-edit` uses roadstyle's level-edit part and its WebAssembly level editor, as Studio's card 8. The Python command serves
  roadstyle's Python editor with mapstyle's layers; the Rust page uses Studio's map of the area. `--area`, `--reports` and
  `--plain` are not ported: the work folder is in the page, the maps have their own command (`viz`), and the map is always Studio's.
- A file with no stored levels: the engine solves them while it draws (mapstyle's rule).
- The planner page: `duckosm-planner` holds the area in memory (the page's data `RM`: each road's line, each mode's roads in
  table order, the turns, walk + drive) and gives the JSON of `route-points`, `route-multimodal-points` and `directions`.
  `duckosm-edit/build.rs` builds it for wasm32 (no wasm-bindgen: the page calls its exports) into the program, and the page
  holds the module as base64. `duckosm-edit/tests/route_map.rs` compares it with the program on Monaco (420 questions);
  `tests/test_route_map_rust.py` compares the page in Chromium with the commands, and checks the pages at 390 x 844.
- A tie: the database gives the roads near a point in the order of its R-tree index, the page in the order of the table. Two
  routes with the same time and length can then differ (2 of 386 routes on Monaco). Python has the R-tree order too.
- The page's Shortest walk + drive (by length, a change of mode 0.1 m) is not in duckOSM: the router has `Weight::Length` for it.
- The other planner pages (the GeoLibre plugin, Studio) can use the same module: load it, `rm_load` the data, `rm_plan` each
  request.

## Group C: the state

The GMNS export is ported step by step into `rust/duckosm-export/src/gmns.rs`, in the order of `to_gmns`, with the same SQL.
The golden file `tests/data/exports/monaco_gmns_columns.tsv` has one line for each column of each table of Python's
`duckosm gmns` on Monaco: the type, the number of rows and the MD5 of the values as sorted text. The test `gmns_equals_python`
compares each column of each table that the Rust export makes; `GMNS_PENDING` lists the columns that a later step changes.
The command `duckosm-rust gmns` comes when all the steps are ported.

### How the lane geometry is checked (Kaveh, 2026-11-18)

Python's lane geometry is not a reference: its result changed from run to run. At a round corner the joint of two pieces is
the centre of the fillet arc, so every arc point is the same distance from it (to 1e-15 m), and GEOS's `project` picked one
from the last bit of numpy's `lstsq` (OpenBLAS threads change that bit). Both exports now cut a run at the middle of the tied
arc, on the corner's bisector (`_project_tol` in `gmns.py`, `shapely::project_tol` in Rust).

So the computed lane geometry (`lane.geom`, and the lane geometry of later steps) is checked by its properties, not by
equality with Python:

- each lane line is at its offset from the link's line within 1 mm, except near the joins, the tapers and the varying gaps;
- no lane crosses itself, and the lanes of a link do not cross each other;
- the number of lanes agrees with `link.lanes` (`gmns_check`);
- the lane ends meet the movements and connectors at the nodes within 1 mm (when those steps are ported);
- two runs give the same geometry.

The golden file `tests/data/exports/monaco_gmns_lanes.tsv` holds only a small, stable summary that the Rust export makes:
the counts, the total lengths to 1 mm and the results of the checks. The other columns (ids, attributes, counts, the
geometry copied from the source db) still compare with Python's `monaco_gmns_columns.tsv`, where Python is deterministic.

| Step | State |
|---|---|
| `config`, `use_definition`, `use_group`, `node`, the inherited lane counts, `link`, `geometry` | Done: 176 columns equal Python on Monaco. |
| The roads a footpath runs along (`link_along`, `link.along_*`, `link.parent_link_id`) | Done: equal on Monaco. shapely's linear referencing, line distance and the order of its `STRtree` (with libstdc++'s `std::sort`, for equal envelopes) are in `src/shapely.rs`: a tie between two roads goes to the first in that order, as in Python. |
| Lanes (`lane` without `geom`) and `curb_seg` | Done: equal on Monaco. The lanes of driving come from `driving.lane_profile`; an older db without it, and walking and cycling, use the tag rules (`_rule_lanes`). |
| Lane geometry (`lane.geom`) | Done: checked by its properties (below). On Monaco Rust and the fixed Python agree within 1 mm on every lane. The checks find 7 lanes that cross themselves (Monaco: driving 4, cycling 3) and 2 pairs of crossing lanes of one cycling link: the same in Python, open. |
| Movements (`movement`, without their lanes) | Done: equal to Python after this step on Monaco. The lanes of the movements, and the rows that the SUMO step adds in driving and cycling, come with that step. |
| Walking frame (`--walk-frame`, off by default) | Not started. |
| The lanes of the walking movements, `signal_controller`, `location`, the signs, the node types, `zone` | Done: equal on Monaco. |
| `gmns_sumo` (netconvert: the lanes of the movements, `lane_check`, `lane_connector`, `lane.geom_cut`), the lanes that meet | Done: every column but the geometry equals Python on Monaco; the lane geometry agrees with Python within 1 mm, and the connectors meet the ends of their lanes within 1 mm (`lane_report`). `sumo::to_sumo_with` has the options that GMNS gives: the bus-only edges, GMNS's lanes with their arrows (netconvert's OSM import), the lane middles as shapes. The Rust CI job installs netconvert 1.27.0. |
| Crossings, GTFS stops, walk joins | Not started. |
| Walk joins, `gmns_all`, CSV, meso, micro, `gmns_check`, `gmns-viz`, `gmns-map` | Not started. |

## Not changed

- The tables and `edge_id` stay the same: other projects join on them.
- Studio keeps its own exports in the browser. When a Rust export is ready, Studio can use it through WebAssembly.
