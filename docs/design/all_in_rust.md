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
| GIS check | `gis-debug` | Not started. |
| OpenDRIVE | `duckosm-rust opendrive` | Done. The same bytes as Python on Monaco and Tartu, with and without `--junctions`. A CRS that is not UTM or EPSG:3006 gets its proj4 text from PROJ's `projinfo` (pyproj uses the same PROJ function). |
| MATSim lanes | `duckosm-rust matsim-lanes` | Done. The same bytes as Python on Monaco and Tartu. |
| Lanelet2 | | Not started. It needs the offset line of shapely (GEOS): see group C. |
| SUMO | | Not started. |

The exports of a GMNS db read the test tables of `tests/data/exports/gmns_driving/` (the columns that they read, as
Parquet), until group C makes the GMNS db in Rust.

GTFS (`gtfs.py`) is not an export: it reads the stops of a feed for the GMNS `location` table. It moves with group C.

## Not changed

- The tables and `edge_id` stay the same: other projects join on them.
- Studio keeps its own exports in the browser. When a Rust export is ready, Studio can use it through WebAssembly.
