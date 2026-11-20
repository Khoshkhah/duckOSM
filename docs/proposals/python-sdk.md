# The duckOSM Python SDK: design

Status: proposal, for Kaveh's approval. Nothing in this document is built yet, except where the table says "exists".

## 1. Goal

A Python user gets a road network from a place name, looks at it, analyses it, routes on it and exports it, with
SDK calls only. The user never writes DuckDB connections, WKB conversion, or display code.

| Rule | What it means |
|---|---|
| One object | `duckosm.Network` is the built file. All work starts from it. |
| At least OSMnx | Each public function of OSMnx 2.1 has a duckOSM call (section 9). |
| Rust does the work | Python only calls the Rust program and converts the result. Counts, searches and graph work run in Rust. |
| The OSM fixes everywhere | Each output of a `Network` uses the fixes that the file was built with. A user can edit fixes from Python. |
| No old code breaks | The current functions stay. `Network` acts as a path, so code that used the `Path` from `build()` still works. |
| Good in a notebook | Each object shows a card, long steps show progress, an error says how to fix it. |
| roadstyle for maps | Maps come from roadstyle's duckOSM page (`roadstyle.duckosm.page`), unchanged. |

## 2. The objects

```text
duckosm.network("Monaco")           duckosm.get(...)            duckosm.open("monaco.duckdb")
        │                                  │                               │
        │                               Area  ── .build() ──┐              │
        └───────────────────────────────────────────────────┴──────────► Network
                                                                            │
                                                         net["walking"]  ── ModeView
                                                                            │
                                                              Route  ◄── .route(...)
```

| Object | What it is | Made by |
|---|---|---|
| `Area` | An OSM file of an area, and its boundary | `duckosm.get(...)` (exists) |
| `Network` | A built `.duckdb` file with one or more modes | `duckosm.network`, `duckosm.build`, `duckosm.open`, `Area.build` |
| `ModeView` | One mode of a `Network`, so the user does not repeat `mode=` | `net["walking"]`, `net.walking` |
| `Route` | One route: edges, length, time, geometry, directions | `net.walking.route(...)` and the other route calls |
| `Fixes` | The OSM fixes of an area (`osm_fixes.yaml`) | `duckosm.Fixes(path)`, `net.fixes` |

## 3. Get a network

| Call | What it does | Today |
|---|---|---|
| `duckosm.network("Monaco", modes=None, fixes=None, out="data")` | Gets the OSM file and builds the network, in one call. The download is kept in a cache. | new |
| `duckosm.network(bbox=(w, s, e, n))`, `(point=(lon, lat), dist_m=1000)`, `(address="...", dist_m=...)`, `(osm_id=...)`, `(boundary=gdf or file)`, `(h3_cell=...)`, `(pbf="file.osm.pbf")` | The same, for other kinds of area | new (`get` has place, bbox, osm_id, h3, boundary) |
| `duckosm.get(...)` → `Area` | Only the OSM file and the boundary | exists |
| `area.build(modes=..., fixes=...)` → `Network` | Builds from an `Area` | new |
| `duckosm.build(pbf, ...)` → `Network` | Builds from a file, as today, but returns a `Network` | exists, returns `Path` today |
| `duckosm.open(path)` → `Network` | Opens a file that was built before | new |
| `duckosm.geocode("Monte Carlo")` → `(lon, lat)` | A place name to a point | new (the geocoder is inside `get`) |
| `duckosm.geocode_to_gdf("Monaco")` → `GeoDataFrame` | A place name to its boundary | new (`duckosm.boundary` writes a file) |

Default modes: `["driving", "walking", "cycling"]`, as the build does today.

## 4. The `Network` object

### 4.1 What is in the file

| Call | Returns |
|---|---|
| `net` (last line of a cell) | A card: area name, file, size, build date, the fixes file, and one row for each mode (edges, nodes, km) |
| `net.path` | The `.duckdb` path |
| `net.modes` | `["driving", "walking"]` |
| `net.summary()` | A DataFrame, one row for each mode: edges, nodes, km, intersections, dead ends |
| `net.tables(mode=None)`, `net.columns(table)` | What is in the file |
| `net.boundary` | The boundary as a `GeoDataFrame` |

### 4.2 Read the data

| Call | Returns | OSMnx |
|---|---|---|
| `net.edges(mode=None, where=None, crs=None)` | `GeoDataFrame` of the edges. `edge_id` is `int64` (BIGINT). Float columns are `float64`. | `graph_to_gdfs` |
| `net.nodes(mode=None, crs=None)` | `GeoDataFrame` of the nodes | `graph_to_gdfs` |
| `net.features(tags={"building": True})` | `GeoDataFrame` of OSM features (buildings, shops, ...) in the area | `features_from_*` |
| `net.sql("SELECT ...")` | A DataFrame. Read-only, spatial loaded. | none |

`crs=` projects the result (`"utm"` picks the UTM zone). This replaces `project_graph` and `project_gdf`.

### 4.3 Modes

| Case | `net.edges()`, `net.nodes()`, `net.map()` | A call that needs one mode (`route`, `to_networkx`, ...) |
|---|---|---|
| The file has one mode | That mode | That mode |
| The file has more modes | All modes, with a `mode` column. The map shows all modes with a legend. | An error that lists the modes and shows `net["walking"].route(...)` |

`net["walking"]` and `net.walking` give a `ModeView` with the same calls and no `mode=` argument. Scripts can still
give `mode="walking"` to `Network` calls.

### 4.4 Analyse

| Call | Returns | OSMnx | Rust work |
|---|---|---|---|
| `view.stats()` | dict: node and edge counts, street length, intersection count, streets per node, circuity, density (with the area) | `basic_stats` and the `stats` module | new Rust function |
| `view.nearest_nodes(x, y)`, `view.nearest_edges(x, y)` | `node_id` or `edge_id` for each point; arrays in, arrays out | `nearest_nodes`, `nearest_edges` | exists in Rust (point routing), to expose |
| `view.largest_component()` | A new `Network` with only the largest strongly connected part | `largest_component` | `components` exists in Rust |
| `net.clip(bbox=... / polygon=... / point=..., dist_m=...)` | A new `Network` file for the smaller area | `truncate_graph_*` | `extract` exists |
| `view.bearings()`, `view.orientation_entropy()` | edge bearings (degrees), entropy of the street directions | `bearing` module | new Rust function |
| `net.add_elevation(dem=None)` | The same `Network`, with `nodes.ele` and edge grades | `elevation` module | `to_elevation` exists; grades: check |
| `view.sample_points(n)` | `GeoDataFrame` of points on the edges | `sample_points` | small, Python |

Not needed, with the reason:

| OSMnx | Why duckOSM needs no call |
|---|---|
| `simplify_graph` | duckOSM edges are simplified when the network is built |
| `add_edge_speeds`, `add_edge_travel_times` | The edges have `maxspeed_kmh` and `cost_s` from the build |
| `add_edge_lengths` | The edges have `length_m` |
| `consolidate_intersections` | To check: if the build does not join complex intersections, add `view.consolidate_intersections(tolerance_m)` |

### 4.5 Route

| Call | Returns | OSMnx | Today |
|---|---|---|---|
| `view.route(a, b, weight="time")` | `Route`. `a` and `b` can be edge ids, `(lon, lat)` points, or place names | `shortest_path` | `route`, `route_points` exist |
| `view.routes(origins, destinations)` | A list of `Route`, in parallel | `shortest_path` with lists | new (Rust) |
| `view.k_shortest_paths(a, b, k=3)` | A list of `Route` | `k_shortest_paths` | new (Rust) |
| `net.route_multimodal(a, b, modes=...)` | `Route` across modes | none | exists |
| `route.edges` | `GeoDataFrame` of the edges of the route | `route_to_gdf` | new |
| `route.length_m`, `route.time_s`, `route.geometry` | Numbers and a line | none | new |
| `route.directions()` | The turn-by-turn steps | none | `directions` exists |
| `route.map()` | The route on the network map | `plot_graph_route` | new |

### 4.6 Graphs

| Call | Returns | Today |
|---|---|---|
| `view.to_networkx(graph="turns")` | networkx graph. `graph="turns"` (edge-based, with turn rules) or `"nodes"` (node-based) | exists as a function |
| `view.to_networkx(osmnx=True)` | A `MultiDiGraph` that OSMnx accepts (`crs`, `x`, `y`, `street_count`, `osmid`, `length`, `geometry`, `travel_time`), so `ox.plot_graph`, `ox.basic_stats` and `ox.shortest_path` work on it | new |
| `view.to_rustworkx(graph="turns")` | rustworkx graph (the Rust engine) | exists as a function |

Each graph keeps `edge_id` on its edges.

### 4.7 Maps

All maps come from roadstyle's duckOSM page (`roadstyle.duckosm.page`), which reads the file directly, shows all modes
and draws the levels. duckOSM adds no styling of its own. A missing map part is a roadstyle change.

| Call | Returns | OSMnx |
|---|---|---|
| `net.map(mode=None, theme="liberty", color_by=None, highlight=None, routes=None)` | The live roadstyle map in the notebook | `plot_graph`, `plot_graph_route(s)` |
| `net.dashboard()` | roadstyle's dashboard: the map, a SQL data dock and charts | none |
| `net.save_map("monaco.html", ...)` | The same map as one offline HTML file | none |
| `net.plot(mode=None, ...)` | A static matplotlib picture (for papers and for the saved notebook outputs) | `plot_graph`, `plot_figure_ground` |
| `net.plot_orientation(mode)` | A polar chart of the street directions | `plot_orientation` |
| `net.features(...).plot()` | Building footprints | `plot_footprints` |

`color_by` takes a column of the edges, or a Series or DataFrame keyed by `edge_id` (your own data, for example
traffic counts). In the saved notebook outputs (`DUCKOSM_STATIC_MAPS=1`), `map()` gives the `plot()` picture.

### 4.8 Export

Each export call writes files and returns their paths. All exports use the fixes of the file.

| Call | Format | Today |
|---|---|---|
| `net.to_gis(out, fmt="gpkg")` | GeoPackage, GeoParquet, shapefile | exists |
| `view.to_graphml(path)` | GraphML (OSMnx `save_graphml`) | exists (`write_graph`) |
| `net.to_osm_xml(path)` | OSM XML (OSMnx `save_graph_xml`) | new |
| `view.to_sumo(out)`, `view.to_matsim(path)`, `view.to_opendrive(path)` | Traffic simulation | exist |
| `view.to_gmns(path)` → a GMNS file, then `.to_lanelet2(path)`, `.to_matsim_lanes(out)` | Lane networks | exist |
| `net.to_railml(path)` | Rail | exists |
| `net.to_pmtiles(path)`, `net.to_quack(out)` | Vector tiles, QuackOSM features | exist (`quack`) |

### 4.9 Fixes

| Call | What it does |
|---|---|
| `net.fixes` | The `Fixes` the file was built with (or `None`) |
| `fixes = duckosm.Fixes("osm_fixes.yaml")` | Opens or makes a fixes file |
| `fixes.set(way=123, oneway=False, maxspeed=30)`, `fixes.remove(way=456)`, `fixes.exclude(way=789, modes=["driving"])` | Adds a rule (the fields of `osm_fixes.yaml`) |
| `fixes` (last line of a cell) | A table of the rules |
| `net.rebuild(fixes=fixes)` → `Network` | Builds the area again with the fixes |
| `net.diff(other)` | The edges that changed between two networks (by `edge_id`) |
| `net.edit()` | Opens `duckosm edit` (the fixes editor) on the file |

## 5. In a notebook

| Item | Behaviour |
|---|---|
| Cards | `Network`, `ModeView`, `Area`, `Route` and `Fixes` show an HTML card (`_repr_html_`) and a one-line text form (`__repr__`) |
| Progress | `network`, `get`, `build`, `rebuild` and the exports show one updating line; `progress=False` stops it |
| Errors | One class, `duckosm.DuckOSMError`. The message says what to do: the list of modes, the build command, or the `pip install "duckosm[...]"` line |
| Help | Type hints and a short example in each docstring (Shift+Tab in Jupyter) |
| Tables | Plain DataFrames. The manual's CSS shows them in the site's look. No display code in the notebooks |
| Extras | `pip install duckosm` gives the core. `[viz]`: roadstyle maps. `[graph]`: networkx and rustworkx. `[widget]`: `AreaPicker`. `[osmnx]`: the OSMnx form. `[all]`: all of them |

## 6. What stays

| Today | After |
|---|---|
| `duckosm.route(db, ...)`, `duckosm.to_sumo(db, ...)` and the other functions | They stay and take a `Network` or a path. The methods call them, with no second copy of the logic. |
| `build()` returns `Path` | It returns `Network`, which acts as a path (`os.fspath`, `str`, `duckdb.connect(str(net))`) |
| `duckosm.DuckOSM` (old Python importer) | Stays as it is |

## 7. Docs, notebooks and tests

| Part | Content |
|---|---|
| Python reference | A page for each object (`Network`, `ModeView`, `Route`, `Fixes`, `Area`) with a table of its calls |
| Quick start | `net = duckosm.network("Monaco")`, `net`, `net.map()`, at the top of the Python pages and of notebook 1 |
| "duckOSM for OSMnx users" | A table: each OSMnx function and its duckOSM call (section 9) |
| Notebooks | 1 Area to network; 2 OSM fixes; 3 Routing; 4 Exports; 5 Your data by edge_id; 6 Analysis (stats, orientation, components). Each code cell is SDK calls with a comment. No helper code. |
| Tests | A test file for each object, on Monaco. A test that OSMnx accepts the OSMnx form. CI runs the notebooks. |

## 8. Plan of work

| PR | Content | Rust work |
|---|---|---|
| 1 | `Network`, `ModeView`, `open`, `network()`, `Area.build`, data reads (4.1–4.3), maps through `roadstyle.duckosm.page` (4.7), graphs (4.6), export methods (4.8), cards, progress, errors, extras. Notebooks 1–5 rewritten. | none |
| 2 | `Route` object, `routes`, `k_shortest_paths`, `nearest_*` (4.5, 4.4) | k shortest paths, many routes, nearest as an API |
| 3 | `stats`, `bearings`, `orientation_entropy`, `largest_component`, `clip`, `sample_points`, `features`, `geocode*`, `to_osm_xml`, `point`/`address` areas. Notebook 6. "duckOSM for OSMnx users" page. | stats, bearings |
| 4 | `Fixes` object, `rebuild`, `diff`, `edit` (4.9). Notebook 2 uses it. | none |

Release: 0.9.0 after PR 1, because the API changes (`build` returns `Network`). PRs 2–4 then come in 0.9.x
releases. The Colab badges work from 0.9.0.

## 9. OSMnx 2.1 and duckOSM

| OSMnx module | OSMnx functions | duckOSM |
|---|---|---|
| graph | `graph_from_place`, `_bbox`, `_point`, `_address`, `_polygon`, `_xml` | `duckosm.network(...)` with `place`, `bbox`, `point`, `address`, `boundary`, `pbf` |
| features | `features_from_*` | `net.features(tags)`, or `duckosm.features(area, tags)` |
| geocoder | `geocode`, `geocode_to_gdf` | `duckosm.geocode`, `duckosm.geocode_to_gdf` |
| convert | `graph_to_gdfs`, `graph_from_gdfs`, `to_digraph`, `to_undirected` | `net.edges()`, `net.nodes()`, `view.to_networkx(...)` |
| distance | `nearest_nodes`, `nearest_edges`, `great_circle`, `euclidean`, `add_edge_lengths` | `view.nearest_nodes`, `view.nearest_edges`; lengths are in the edges |
| routing | `shortest_path`, `k_shortest_paths`, `route_to_gdf`, `add_edge_speeds`, `add_edge_travel_times` | `view.route`, `view.routes`, `view.k_shortest_paths`, `route.edges`; speeds and times are in the edges |
| stats | `basic_stats` and 11 more | `view.stats()` |
| bearing | `add_edge_bearings`, `orientation_entropy` | `view.bearings()`, `view.orientation_entropy()` |
| elevation | `add_node_elevations_*`, `add_edge_grades` | `net.add_elevation(dem)` |
| simplification | `simplify_graph`, `consolidate_intersections` | built in; consolidation: to check |
| truncate | `largest_component`, `truncate_graph_*` | `view.largest_component()`, `net.clip(...)` |
| projection | `project_graph`, `project_gdf` | `crs=` on `edges()` and `nodes()` |
| plot | `plot_graph`, `plot_graph_route(s)`, `plot_figure_ground`, `plot_footprints`, `plot_orientation` | `net.map(...)`, `net.plot(...)`, `route.map()`, `net.plot_orientation()` |
| io | `save_graphml`, `load_graphml`, `save_graph_geopackage`, `save_graph_xml` | `view.to_graphml`, `net.to_gis`, `net.to_osm_xml`; `duckosm.open` loads |
| utils_geo | `sample_points`, `interpolate_points`, `bbox_from_point` | `view.sample_points(n)`; the others are shapely work |

What duckOSM has and OSMnx does not: the OSM fixes in every output, turn rules, multimodal routes, lanes (GMNS,
Lanelet2), SUMO, MATSim, OpenDRIVE, vector tiles, `edge_id` that other projects join on, and large areas by tiles.

## 10. Questions for Kaveh

| # | Question | My proposal |
|---|---|---|
| 1 | `build()` returns `Network` in place of `Path`. Is that change acceptable? | Yes: it acts as a path, so old code works |
| 2 | Release number for PR 1 | 0.9.0, not 0.8.1 |
| 3 | `duckosm.network("Monaco")` default modes | driving, walking and cycling, as the build does today |
| 4 | Four PRs in this order, or another order? | The order of section 8 |
