# The duckOSM Python SDK: design

Status: proposal, for Kaveh's approval. Nothing in this document is built yet, except where a table says "exists".

## 1. Goal

A Python user gets a road network from a place name, looks at it, analyses it, routes on it and exports it, with
SDK calls only. The user never writes DuckDB connections, WKB conversion, or display code. Each group of users gets
its own module, in the words and with the objects of the tools that the group uses today.

| Rule | What it means |
|---|---|
| Users first | Each module serves one user group (section 2). A function goes in the module of the group that needs it. |
| One object | `duckosm.Network` is the built file. Each module takes a `Network`. |
| At least the tool they use today | Each module covers the main tool of its group. The first module covers all of OSMnx 2.1. |
| Rust does the work | Python only calls the Rust program and converts the result. Counts, searches and graph work run in Rust. |
| The OSM fixes everywhere | Each output of each module uses the fixes that the file was built with. |
| No old code breaks | The current functions stay. `Network` acts as a path. |
| Good in a notebook | Each object shows a card, long steps show progress, an error says how to fix it. |
| roadstyle for maps | Maps come from roadstyle's duckOSM page (`roadstyle.duckosm.page`), unchanged. |

## 2. The users

### 2.1 How the groups were found

For each kind of work on road networks in Python, I listed the tools that people use, and read their downloads from
PyPI for the last month (pypistats.org, 2026-11-20). General libraries (networkx, geopandas, ortools) show the size
of a field, not the number of road-network users.

| # | Group | What they do | Tools today (PyPI downloads, last month) |
|---|---|---|---|
| 1 | **Street network analysts** | Download a city's streets, compute street statistics, orientation, connectivity; research in urban form | OSMnx 491k, momepy 73k, cityseer 9.7k |
| 2 | **GIS and data engineers** | OSM data into tables and files: roads, buildings, POIs; GeoParquet, GeoPackage, tiles | osmium 1.28M, QuackOSM 32k, geopandas 14.7M |
| 3 | **Routing, accessibility and logistics** | Travel time matrices, isochrones, access to jobs or schools, vehicle routing | openrouteservice 53k, routingpy 11k, r5py 6.7k, pandana 5.2k, ortools 8.5M |
| 4 | **Traffic simulation** | Microscopic and agent simulation | eclipse-sumo 189k, sumolib 83k, traci 69k, matsim-tools 0.3k |
| 5 | **Transport planning** | Demand models, traffic assignment, GMNS networks | AequilibraE 17k, osm2gmns 1.0k, path4gmns 0.4k |
| 6 | **Mobility data** | GPS traces and traffic sensors on roads: map matching, data per road | movingpandas 17k, trackintel 17k, leuvenmapmatching 4.5k, mappymatch 1.3k |
| 7 | **Automated driving and HD maps** | Lane maps and road models for simulators and planners | scenariogeneration 63k, CARLA 25k, Lanelet2 16k, commonroad-io 5.5k |
| 8 | **Graph machine learning** | Road graphs as input to graph neural networks | torch-geometric 1.4M, dgl 82k |
| 9 | **Map makers** | Maps and dashboards of road data | folium 2.7M, keplergl 386k, lonboard 28k, prettymaps 0.5k |

### 2.2 What each group needs, and what duckOSM gives

| # | Group | Their main problems with today's tools | What duckOSM gives them |
|---|---|---|---|
| 1 | Street network analysts | OSMnx is slow and uses much memory for large areas; results change when OSM changes; no turn rules | Same functions, Rust speed, large areas by tiles, turn rules, fixes, a stable `edge_id` |
| 2 | GIS and data engineers | osmium and QuackOSM give raw features, not a routable network; no fixes | Features and network from one file, all GIS formats, PMTiles, fixes in each output |
| 3 | Routing and accessibility | A routing server (OSRM, ORS) to run, or an online API with limits; r5py needs Java | Routing in-process in Rust, matrices and isochrones, no server, turn rules, multimodal |
| 4 | Traffic simulation | `netconvert` from OSM gives networks with faults that people fix by hand, again after each OSM update | SUMO and MATSim networks from the fixed network, `edge_id` kept, fixes that stay across updates |
| 5 | Transport planning | osm2gmns is slow and not kept up; AequilibraE needs its own network import | GMNS with lanes and movements, AequilibraE project, fixes |
| 6 | Mobility data | Map matching needs a network in a special form; results cannot be joined across versions | Map matching onto `edge_id`, data per edge, `edge_id` stable across builds |
| 7 | Automated driving | OpenDRIVE and Lanelet2 from OSM need several tools; lanes are often wrong | OpenDRIVE, Lanelet2 and CommonRoad from one network, lanes from GMNS, lane fixes |
| 8 | Graph ML | Each project writes its own OSM-to-tensor code | `to_pyg()`, `to_dgl()` with edge features, node-based and edge-based graphs |
| 9 | Map makers | Road maps without casing, levels or bridges; each map styled by hand | roadstyle maps: casing, levels, bridges, themes, dashboards |

### 2.3 Order

| Order | Group | Why this order |
|---|---|---|
| 1 | Street network analysts (OSMnx users) | The largest group that works on road networks; Kaveh's first target |
| 2 | GIS and data engineers | Large; duckOSM already has most of the outputs |
| 3 | Routing and accessibility | Large need; duckOSM has the router in Rust |
| 4 | Traffic simulation | duckOSM's strongest outputs (SUMO, MATSim) |
| 5 | Mobility data | Kaveh's own stack (sensor-matching, SonoFlow) joins on `edge_id` |
| 6 | Transport planning | GMNS exists; AequilibraE is new |
| 7 | Automated driving | OpenDRIVE and Lanelet2 exist; CommonRoad is new |
| 8 | Graph ML | Small work on top of the graphs |
| 9 | Map makers | Comes with the core (`net.map()`) through roadstyle |

## 3. The modules

```text
duckosm                  the core: get a network, the Network object, data reads, modes, maps (all groups)
├── duckosm.analysis     1  street network analysts: the OSMnx functions
├── duckosm.gis          2  GIS and data engineers: features, GIS formats, tiles
├── duckosm.routing      3  routing and accessibility: routes, matrices, isochrones, access
├── duckosm.sim          4  traffic simulation: SUMO, MATSim
├── duckosm.planning     5  transport planning: GMNS, AequilibraE
├── duckosm.mobility     6  mobility data: map matching, data per edge
├── duckosm.hdmap        7  automated driving: OpenDRIVE, Lanelet2, CommonRoad, lane routes
├── duckosm.ml           8  graph machine learning: PyTorch Geometric, DGL
└── duckosm.fixes        all groups: edit the OSM fixes from Python
```

A module is imported only when it is used, so `import duckosm` stays fast and needs only the core dependencies. Each
module has a pip extra with the libraries of its group, for example `pip install "duckosm[sim]"`.

The objects of the core also give the most-used call of each module as a method, so that a user can type `net.` and
see what is possible. The method calls the module function; the logic exists once.

| Method | Module function |
|---|---|
| `net.stats()` | `duckosm.analysis.stats(net)` |
| `net.route(a, b)` | `duckosm.routing.route(net, a, b)` |
| `net.to_sumo(out)` | `duckosm.sim.to_sumo(net, out)` |
| `net.to_gis(out)` | `duckosm.gis.to_gis(net, out)` |

## 4. The core: `duckosm`

### 4.1 The objects

```text
duckosm.network("Monaco")           duckosm.get(...)            duckosm.open("monaco.duckdb")
        │                                  │                               │
        │                               Area  ── .build() ──┐              │
        └───────────────────────────────────────────────────┴──────────► Network
                                                                            │
                                                         net["walking"]  ── ModeView
```

| Object | What it is | Made by |
|---|---|---|
| `Area` | An OSM file of an area, and its boundary | `duckosm.get(...)` (exists) |
| `Network` | A built `.duckdb` file with one or more modes | `duckosm.network`, `duckosm.build`, `duckosm.open`, `Area.build` |
| `ModeView` | One mode of a `Network`, so the user does not repeat `mode=` | `net["walking"]`, `net.walking` |

### 4.2 Get a network

| Call | What it does | Today |
|---|---|---|
| `duckosm.network("Monaco", modes=None, fixes=None, out="data")` | Gets the OSM file and builds the network, in one call. The download is kept in a cache. | new |
| `duckosm.network(bbox=...)`, `(point=(lon, lat), dist_m=...)`, `(address=..., dist_m=...)`, `(osm_id=...)`, `(boundary=gdf or file)`, `(h3_cell=...)`, `(pbf="file.osm.pbf")` | The same, for other kinds of area | new (`get` has place, bbox, osm_id, h3, boundary) |
| `duckosm.get(...)` → `Area`; `area.build(...)` → `Network` | Two steps, for more control | `get` exists |
| `duckosm.build(pbf, ...)` → `Network` | Builds from a file, as today, but returns a `Network` | exists, returns `Path` |
| `duckosm.open(path)` → `Network` | Opens a file that was built before | new |
| `duckosm.geocode(text)`, `duckosm.geocode_to_gdf(text)` | A place name to a point, or to its boundary | new (the geocoder is inside `get`) |

Default modes: driving, walking and cycling, as the build does today.

### 4.3 What is in the file, and the data

| Call | Returns |
|---|---|
| `net` (last line of a cell) | A card: area, file, size, build date, fixes file, and one row for each mode (edges, nodes, km) |
| `net.path`, `net.modes`, `net.boundary` | The file, the list of modes, the boundary (`GeoDataFrame`) |
| `net.summary()` | A DataFrame, one row for each mode: edges, nodes, km |
| `net.tables()`, `net.columns(table)` | What is in the file |
| `net.edges(mode=None, where=None, crs=None)` | `GeoDataFrame` of the edges. `edge_id` is `int64`. Float columns are `float64`. |
| `net.nodes(mode=None, crs=None)` | `GeoDataFrame` of the nodes |
| `net.sql("SELECT ...")` | A DataFrame. Read-only, spatial loaded. |

`crs="utm"` projects to the UTM zone of the area.

### 4.4 Modes

| Case | `edges()`, `nodes()`, `map()` | A call that needs one mode (a route, a graph, an export) |
|---|---|---|
| One mode in the file | That mode | That mode |
| More modes | All modes, with a `mode` column; the map shows all modes with a legend | An error that lists the modes and shows `net["walking"].route(...)` |

`net["walking"]` and `net.walking` give a `ModeView` with the same calls and no `mode=` argument.

### 4.5 Maps (also for group 9, map makers)

All maps come from `roadstyle.duckosm.page`. It reads the file directly, shows all modes and draws the levels.
duckOSM adds no styling of its own. A missing map part is a roadstyle change.

| Call | Returns |
|---|---|
| `net.map(mode=None, theme="liberty", color_by=None, highlight=None, routes=None)` | The live roadstyle map in the notebook |
| `net.dashboard()` | roadstyle's dashboard: the map, a SQL data dock and charts |
| `net.save_map(path, ...)` | The same map as one offline HTML file |
| `net.plot(mode=None, ...)` | A static matplotlib picture (papers, saved notebook outputs) |

`color_by` takes a column of the edges, or a Series or DataFrame keyed by `edge_id` (your own data).

### 4.6 In a notebook

| Item | Behaviour |
|---|---|
| Cards | `Network`, `ModeView`, `Area`, `Route`, `Fixes` show an HTML card and a one-line text form |
| Progress | Long calls show one updating line; `progress=False` stops it |
| Errors | One class, `duckosm.DuckOSMError`, with what to do: the list of modes, the build command, or the `pip install "duckosm[...]"` line |
| Help | Type hints and a short example in each docstring (Shift+Tab in Jupyter) |
| Tables | Plain DataFrames; the manual's CSS gives them the site look |

## 5. The modules, one by one

### 5.1 `duckosm.analysis`: street network analysts (OSMnx users)

| Call | OSMnx | Today | Rust work |
|---|---|---|---|
| `stats(view)` | `basic_stats` and the `stats` module | new | new |
| `bearings(view)`, `orientation_entropy(view)`, `plot_orientation(view)` | `bearing` module, `plot_orientation` | new | bearings: new |
| `nearest_nodes(view, x, y)`, `nearest_edges(view, x, y)` | `nearest_nodes`, `nearest_edges` | in Rust, not public | expose |
| `largest_component(view)` → `Network` | `largest_component` | new | `components` exists |
| `clip(net, bbox= / polygon= / point=, dist_m=)` → `Network` | `truncate_graph_*` | `extract` exists | none |
| `add_elevation(net, dem=None)` | `elevation` module | `to_elevation` exists; grades: check | check |
| `sample_points(view, n)` | `sample_points` | new | none |
| `to_osmnx(view)` → `MultiDiGraph` | the OSMnx graph | new | none |
| `consolidate_intersections(view, tolerance_m)` | `consolidate_intersections` | to check if the build does it | to check |

Not needed: `simplify_graph` (the build simplifies), `add_edge_speeds`, `add_edge_travel_times`, `add_edge_lengths`
(the edges have `maxspeed_kmh`, `cost_s`, `length_m`), `project_graph` (`crs=` in the core). The full OSMnx table
is in section 7.

### 5.2 `duckosm.gis`: GIS and data engineers

| Call | What it gives | Their tool | Today |
|---|---|---|---|
| `features(net or area, tags={"building": True})` | `GeoDataFrame` of OSM features | OSMnx `features_from_*`, QuackOSM | `quack` writes files |
| `to_gis(net, out, fmt="gpkg" / "parquet" / "shp")` | GIS files of the networks | ogr2ogr | exists |
| `to_quack(area, out)` | QuackOSM features table | QuackOSM | exists (`quack`) |
| `to_geofabrik(area, out)` | Geofabrik's 20 layers | Geofabrik shapefiles | exists (`quack --geofabrik`) |
| `to_pmtiles(area, out)` | Vector tiles (Shortbread) | Tilemaker, Planetiler | exists (`quack --pmtiles`) |
| `to_osm_xml(net, path)` | OSM XML | OSMnx `save_graph_xml`, osmium | new |
| `admin_boundaries(net)` | `GeoDataFrame` of administrative areas | OSMnx `geocode_to_gdf` | `duckosm admin` exists |

### 5.3 `duckosm.routing`: routing, accessibility and logistics

| Call | What it gives | Their tool | Today | Rust work |
|---|---|---|---|---|
| `route(view, a, b, weight="time")` → `Route` | One route; `a`, `b` are edge ids, points or place names | ORS, OSRM, OSMnx `shortest_path` | `route`, `route_points` exist | none |
| `routes(view, origins, destinations)` | Many routes, in parallel | OSMnx `shortest_path` with lists | new | parallel |
| `k_shortest_paths(view, a, b, k)` | Other routes | OSMnx `k_shortest_paths` | new | new |
| `matrix(view, origins, destinations=None, weight="time")` | A travel time or distance matrix (DataFrame) | ORS matrix, OSRM table, r5py `TravelTimeMatrix`, ortools input | new | new (one-to-many search) |
| `isochrones(view, origins, minutes=[5, 10, 15])` | `GeoDataFrame` of polygons | ORS isochrones, r5py | new | new |
| `accessibility(view, origins, targets, minutes)` | The count or the nearest target for each origin | pandana, r5py | new | uses `matrix` |
| `route_multimodal(net, a, b, modes=...)` | A route across modes | r5py | exists | none |
| `Route` object: `.edges`, `.length_m`, `.time_s`, `.geometry`, `.directions()`, `.map()` | One route | OSMnx `route_to_gdf` | new | none |

### 5.4 `duckosm.sim`: traffic simulation

| Call | What it gives | Their tool | Today |
|---|---|---|---|
| `to_sumo(view, out, netconvert=True)` | SUMO network (`edge_id` kept) | netconvert, osmWebWizard | exists |
| `sumo_net(path)` | The network read back with `sumolib` | sumolib | new (small) |
| `to_matsim(view, path)` | MATSim `network.xml` | matsim-tools, pt2matsim | exists |
| `to_matsim_lanes(gmns, out)` | MATSim lanes and signals | MATSim | exists |
| `edge_map(sumo_or_matsim_path)` | SUMO or MATSim edge id → duckOSM `edge_id` (DataFrame) | none | new: to join results back |

### 5.5 `duckosm.planning`: transport planning

| Call | What it gives | Their tool | Today |
|---|---|---|---|
| `to_gmns(view, path)` → `Gmns` | GMNS network with lanes and movements | osm2gmns | exists |
| `check_gmns(path)` | GMNS checks | GMNS validator | exists |
| `to_meso(gmns)`, `to_micro(gmns)` | Meso and micro networks | osm2gmns | exist |
| `to_aequilibrae(view, project_dir)` | An AequilibraE project (links, nodes) | AequilibraE OSM import | new |
| `to_path4gmns(gmns)` | Input for path4gmns and DTALite | path4gmns | exists as GMNS files; check |

### 5.6 `duckosm.mobility`: mobility data

| Call | What it gives | Their tool | Today | Rust work |
|---|---|---|---|---|
| `match(view, traces)` | Each GPS point and trace on `edge_id` (map matching) | leuvenmapmatching, mappymatch | new | new |
| `snap(view, points)` | The nearest edge and the position on it | own code | nearest exists | expose |
| `join(view, data, on="edge_id")` | Your data on the edges (`GeoDataFrame`) | own code | new | none |
| `aggregate(view, matched, by="edge_id", time="hour")` | Counts or speeds per edge and time | movingpandas, own code | new | none |
| `edge_id_map(old_net, new_net)` | Old `edge_id` → new `edge_id` after a rebuild | none | new | new |

Kaveh's projects (sensor-matching, SonoFlow) are users of this module.

### 5.7 `duckosm.hdmap`: automated driving and HD maps

| Call | What it gives | Their tool | Today |
|---|---|---|---|
| `to_opendrive(view, path, junctions=False)` | ASAM OpenDRIVE `.xodr` | scenariogeneration, CARLA | exists |
| `to_lanelet2(gmns, path)` | Lanelet2 map | Lanelet2 | exists |
| `to_commonroad(view, path)` | CommonRoad scenario map | commonroad-scenario-designer | new |
| `lane_graph(gmns)`, `route_lanes(gmns, a, b)` | Lane graph and lane routes | Lanelet2 routing | exist |

### 5.8 `duckosm.ml`: graph machine learning

| Call | What it gives | Their tool | Today |
|---|---|---|---|
| `to_pyg(view, graph="nodes" / "turns", features=[...])` | `torch_geometric.data.Data` with edge features | own code | new |
| `to_dgl(view, ...)` | `dgl.DGLGraph` | own code | new |
| `to_networkx(view, graph=...)`, `to_rustworkx(view, graph=...)` | Graphs | networkx, rustworkx | exist |

### 5.9 `duckosm.fixes`: all groups

| Call | What it does |
|---|---|
| `net.fixes` | The fixes the file was built with |
| `Fixes(path)` | Opens or makes a fixes file (`osm_fixes.yaml`) |
| `fixes.set(way=..., oneway=False, maxspeed=30)`, `.remove(way=...)`, `.exclude(way=..., modes=[...])` | Adds a rule |
| `net.rebuild(fixes=fixes)` → `Network` | Builds the area again with the fixes |
| `net.diff(other)` | The edges that changed between two networks |
| `net.edit()` | Opens `duckosm edit` on the file |

## 6. What stays

| Today | After |
|---|---|
| `duckosm.route(db, ...)`, `duckosm.to_sumo(db, ...)` and the other functions | They stay, and call the module functions |
| `build()` returns `Path` | It returns `Network`, which acts as a path (`os.fspath`, `str`) |
| `duckosm.DuckOSM` (old Python importer) | Stays as it is |

## 7. OSMnx 2.1 and duckOSM

| OSMnx module | OSMnx functions | duckOSM |
|---|---|---|
| graph | `graph_from_place`, `_bbox`, `_point`, `_address`, `_polygon`, `_xml` | `duckosm.network(...)` |
| features | `features_from_*` | `duckosm.gis.features` |
| geocoder | `geocode`, `geocode_to_gdf` | `duckosm.geocode`, `duckosm.geocode_to_gdf` |
| convert | `graph_to_gdfs`, `graph_from_gdfs`, `to_digraph`, `to_undirected` | `net.edges()`, `net.nodes()`, `analysis.to_osmnx` |
| distance | `nearest_nodes`, `nearest_edges`, `add_edge_lengths`, `great_circle`, `euclidean` | `analysis.nearest_*`; lengths are in the edges |
| routing | `shortest_path`, `k_shortest_paths`, `route_to_gdf`, `add_edge_speeds`, `add_edge_travel_times` | `routing.route`, `routing.routes`, `routing.k_shortest_paths`, `Route.edges` |
| stats | `basic_stats` and 11 more | `analysis.stats` |
| bearing | `add_edge_bearings`, `orientation_entropy` | `analysis.bearings`, `analysis.orientation_entropy` |
| elevation | `add_node_elevations_*`, `add_edge_grades` | `analysis.add_elevation` |
| simplification | `simplify_graph`, `consolidate_intersections` | built in; consolidation to check |
| truncate | `largest_component`, `truncate_graph_*` | `analysis.largest_component`, `analysis.clip` |
| projection | `project_graph`, `project_gdf` | `crs=` in the core |
| plot | `plot_graph`, `plot_graph_route(s)`, `plot_figure_ground`, `plot_footprints`, `plot_orientation` | `net.map`, `net.plot`, `Route.map`, `analysis.plot_orientation` |
| io | `save_graphml`, `load_graphml`, `save_graph_geopackage`, `save_graph_xml` | `gis.to_gis`, `gis.to_osm_xml`, `to_graphml`; `duckosm.open` |
| utils_geo | `sample_points`, `interpolate_points` | `analysis.sample_points` |

## 8. Docs, notebooks and tests

| Part | Content |
|---|---|
| Python reference | One page for the core and one for each module, with a table of its calls |
| Quick start | `net = duckosm.network("Monaco")`, `net`, `net.map()` |
| One guide for each group | "duckOSM for OSMnx users", "for SUMO users", "for routing", ...: the tool they use today, call by call |
| Notebooks | One for the core and one for each module. Each code cell is SDK calls with a comment. No helper code. |
| Tests | A test file for each module, on Monaco. CI runs the notebooks. |

## 9. Plan of work

| PR | Content | Rust work |
|---|---|---|
| 1 | The core (section 4): objects, `network()`, `open()`, data, modes, maps, cards, progress, errors. The module structure with the functions that exist today moved into their modules. Notebooks rewritten. | none |
| 2 | `analysis` (group 1): all OSMnx functions. Guide "duckOSM for OSMnx users". | stats, bearings, nearest API |
| 3 | `routing` (group 3): `Route`, matrices, isochrones, accessibility | one-to-many, k shortest |
| 4 | `gis` (group 2): `features`, `to_osm_xml` | none |
| 5 | `mobility` (group 6): map matching, join, `edge_id_map` | map matching |
| 6 | `fixes` (all groups) | none |
| 7 | `sim`, `planning`, `hdmap`, `ml` (groups 4, 5, 7, 8): the new calls (`edge_map`, AequilibraE, CommonRoad, PyG, DGL) | none expected |

Release: 0.9.0 after PR 1 (the API changes). Each later PR comes in a 0.9.x release.

## 10. Questions for Kaveh

| # | Question | My proposal |
|---|---|---|
| 1 | The nine groups and their order (section 2) | As in section 2.3 |
| 2 | One module for each group (section 3) | Yes |
| 3 | `build()` returns `Network` in place of `Path` | Yes: it acts as a path, so old code works |
| 4 | Release number for PR 1 | 0.9.0 |
| 5 | The order of the PRs (section 9) | As in section 9 |
