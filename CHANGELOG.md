# Changelog

## Unreleased

### Added

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
