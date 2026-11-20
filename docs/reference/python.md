# Python SDK

The Python package `duckosm` has a core and one module for each group of users. The core gets a network, reads its
data and draws it. Each module has the calls of one kind of work. All the work is in the Rust program of duckOSM; the
SDK calls it. The design: [docs/proposals/python-sdk.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/proposals/python-sdk.md).

## Quick start

```python
import duckosm

net = duckosm.network("Monaco")    # get and build Monaco in one call: a duckosm.Network
net                                # in a notebook: a card with the file and the edges, nodes and km of each mode
net.map()                          # the roadstyle map of all modes
```

## Install

```bash
pip install "duckosm[viz]"
```

| Extra | Adds | For |
|---|---|---|
| (none) | the core: build, data reads, routes, the exports | all |
| `viz` | roadstyle, mapstyle, lanestyle, matplotlib | `net.map()`, `net.plot()`, `net.dashboard()`, `duckosm viz` |
| `graph` | networkx, rustworkx | `duckosm.ml` |
| `sim` | eclipse-sumo (`netconvert`) | `duckosm.sim.to_sumo` with the `.net.xml` |
| `widget` | anywidget | `duckosm.AreaPicker` |
| `all` | all of the above | |

The old extras `networkx`, `rustworkx`, `routing` and `sumo` stay.

## The objects

| Object | What it is | Made by |
|---|---|---|
| `Area` | An OSM file of an area and its boundary. `area.build(...)` gives a `Network`. | `duckosm.get(...)` |
| `Network` | A built `.duckdb` file with one or more modes. It is also the path of the file (`os.PathLike`). | `duckosm.network`, `duckosm.build`, `duckosm.open`, `Area.build` |
| `ModeView` | One mode of a `Network`: the same calls with no `mode=` | `net["walking"]`, `net.walking` |

## The methods of `Network`

A method with `mode` takes the mode name; on a `ModeView` (`net.walking`) the same method has no `mode`. A method
that names a module calls the function of that module, with the same other arguments.

| Group | Methods and properties |
|---|---|
| Data | `sql(query, params=None)`, `connect()`, `schema(schema=None)`, `tables()`, `columns(table)`, `summarize(table)`, `summary()`, `modes`, `boundary`, `edges(mode, where=None, crs=None)`, `nodes(mode, crs=None)` |
| Maps | `map(mode=None, theme=None, color_by=None, highlight=None, routes=None, **style)`, `dashboard(mode, **style)`, `save_map(path, mode=None, dashboard=False, **options)`, `plot(mode=None, color_by=None, highlight=None, routes=None, ax=None, **style)` |
| [analysis](sdk/analysis.md) | `stats(mode, area_m2=None)` |
| [routing](sdk/routing.md) | `route(a, b, mode, weight)`, `route_points(a, b, mode)`, `routes(origins, destinations, mode)`, `k_shortest_paths(a, b, k, mode)`, `matrix(origins, destinations, mode)`, `isochrones(origins, minutes, mode)`, `accessibility(origins, targets, minutes, mode)`, `route_multimodal_points(a, b)`, `directions(route, mode)` |
| [ml](sdk/ml.md) | `to_networkx(mode)`, `to_rustworkx(mode)` |
| Exports ([sim](sdk/sim.md), [planning](sdk/planning.md), [hdmap](sdk/hdmap.md), [gis](sdk/gis.md)) | `to_sumo(out_dir, mode)`, `to_matsim(out_path, mode)`, `to_gmns(out_path, mode)`, `to_opendrive(out_path, mode)`, `to_gis(out, mode)` |
| [fixes](sdk/fixes.md) | `fixes`, `rebuild(fixes=None, out=None)`, `diff(other, mode)`, `edit(fixes=None, port=None)` |

`ModeView` also has `save_map(path, **options)` and `summary()` for its mode. `Area` has `build(...)`, which gives a `Network`.

## Get a network

| Call | Does |
|---|---|
| `network(place=None, *, modes=("driving", "walking", "cycling"), fixes=None, out="data", name=None, pbf=None, progress=None, **area)` | Gets the OSM file and builds the network in one call. The area: `place`, or `bbox=(west, south, east, north)`, `point=(lon, lat)` with `dist_m=`, `osm_id=`, `h3_cell=`, `boundary=` (a GeoJSON file, a dict or a GeoDataFrame), or `pbf=` (your own file). The download stays in a cache. |
| `get(...)` → `Area`; `area.build(modes=None, output=None, fixes=None)` → `Network` | The same in two steps |
| `build(pbf=None, ...)` → `Network` | Builds from a file (the table under [The functions of the package](#the-functions-of-the-package)) |
| `open(path)` → `Network` | Opens a file that you built before |

Later (PR 2 of the design): `network(address=...)`, `geocode(text)`, `geocode_to_gdf(text)`.

## What is in the file

| Call | Returns |
|---|---|
| `net` (the last line of a cell) | a card: the file, its size, the build time, the fixes file, and one row for each mode |
| `net.path`, `net.modes`, `net.boundary` | the file, the list of modes, the boundary (`GeoDataFrame`) |
| `net.summary()` | a DataFrame, one row for each mode: `edges`, `nodes`, `km`, `oneway_km` |
| `net.schema()`, `net.schema("driving")` | one row for each table: the schema, the table, the number of rows and of columns, and the description |
| `net.tables()`, `net.columns("driving.edges")` | the tables of the file; the columns of one table, with their types and descriptions |
| `net.summarize("driving.edges")` | DuckDB's `SUMMARIZE`: for each column the minimum, the maximum, the distinct values, the quartiles and the NULL percentage |
| `net.connect()` | a read-only DuckDB connection with the extension spatial: for Arrow (`.arrow()`), Polars (`.pl()`) or DuckDB relations |
| `net.edges(mode=None, where=None, crs=None)` | a `GeoDataFrame` of the edges: all columns, `edge_id` as `int64`, float columns as `float64`. `where`: a SQL filter, put into the query as it is (give only text that you trust). `crs`: for example `3857`, or `"utm"` for the UTM zone of the area. |
| `net.nodes(mode=None, crs=None)` | a `GeoDataFrame` of the nodes |
| `net.sql("SELECT ...", params=None)` | a DataFrame; read-only, with the extension spatial |

## Modes

| Case | `edges()`, `nodes()`, `map()`, `plot()` | A call for one mode (a route, a graph, an export) |
|---|---|---|
| One mode in the file | that mode | that mode |
| More modes | all modes; the data has a `mode` column, the map shows all modes | a `DuckOSMError` that lists the modes and shows `net["walking"].route(...)` |

`net["walking"]` and `net.walking` give a `ModeView`. Each call above also takes `mode=`.

## Maps

| Call | Returns |
|---|---|
| `net.map(mode=None, *, theme="liberty", color_by=None, highlight=None, routes=None, **style)` | the live roadstyle map in a notebook (`roadstyle.duckosm.page`: the roads of all modes, the levels, the base map of the file). `theme`: `"liberty"`, `"osm"`, `"google"`, `"grey"` or `"roadstyle"`. |
| `net.dashboard(mode=None)` | roadstyle's dashboard: the map and the panel with the road card and the color menu |
| `net.save_map(path, mode=None, *, dashboard=False, **options)` | the same map as one offline HTML file |
| `net.plot(mode=None, *, color_by=None, highlight=None, routes=None, ax=None)` | a static matplotlib picture |

| Option | Gives |
|---|---|
| `color_by` | a column of the edges, or your own data: a Series keyed by `edge_id`, or a DataFrame with `edge_id` and one value column. Numbers get a color ramp (`cmap=`); classes need `colors={value: color}`. |
| `highlight` | edge ids in orange; the other edges gray |
| `routes` | a route, a list of routes or `{label: route}`: lines on top, each in its own color |

`DUCKOSM_STATIC_MAPS=1` makes `net.map()` draw `net.plot()`: the saved outputs of the [notebooks](../notebooks/index.md)
are pictures, not megabytes of map HTML. A map with `color_by` or `routes` uses `roadstyle.render_edges` on the default
base map until `roadstyle.duckosm.page` takes them.

## In a notebook

| Item | Behavior |
|---|---|
| Cards | `Network`, `ModeView` and `Area` show an HTML card and a one-line text form |
| Progress | `get`, `build`, `network` and the exports of a `Network` show one line that updates. `progress=False` shows nothing; a function gets each line. Outside a notebook they show nothing. |
| Errors | `duckosm.DuckOSMError` (a `RuntimeError`) says what to do: the list of modes, the build command, or the `pip install "duckosm[...]"` line |
| Help | Type hints and a short example in each docstring (Shift+Tab in Jupyter) |

## The modules

The plan of each module and its decision: [Python SDK: the modules](python-modules.md). Each module takes a
`Network`, a `ModeView`, a path or a DuckDB connection. The methods of `Network` call them:
`net.walking.route(a, b)` is `duckosm.routing.route(net.walking, a, b)`.

| Module | Users | Functions and classes |
|---|---|---|
| [`duckosm.analysis`](sdk/analysis.md) | street network analysts (OSMnx) | `stats`, `streets_per_node`, `bearings`, `orientation_entropy`, `plot_orientation`, `nearest_nodes`, `nearest_edges`, `components`, `largest_component`, `clip`, `add_elevation`, `grades`, `sample_points`, `to_osmnx` |
| [`duckosm.gis`](sdk/gis.md) | GIS and data engineers | `features`, `admin_boundaries`, `to_osm_xml`, `to_gis`, `quack`, `admin` |
| [`duckosm.routing`](sdk/routing.md) | routing and accessibility | `Route`, `route`, `route_points`, `routes`, `k_shortest_paths`, `matrix`, `isochrones`, `accessibility`, `geocode`, `routed_graph`, `route_multimodal`, `route_multimodal_points`, `directions`, `Router`, `to_networkx`, `to_networkx_nodes`, `write_graph` |
| [`duckosm.sim`](sdk/sim.md) | traffic simulation | `to_sumo`, `to_matsim`, `to_matsim_lanes`, `sumo_net`, `edge_map` |
| [`duckosm.planning`](sdk/planning.md) | transport planning | `to_gmns`, `check_gmns`, `to_meso`, `to_micro`, `to_path4gmns`, `to_aequilibrae` |
| [`duckosm.mobility`](sdk/mobility.md) | mobility data | `snap`, `match`, `join`, `aggregate`, `edge_id_map` |
| [`duckosm.hdmap`](sdk/hdmap.md) | automated driving and HD maps | `to_opendrive`, `to_lanelet2`, `to_commonroad`, `build_lane_graph`, `route_lanes` |
| [`duckosm.ml`](sdk/ml.md) | graph machine learning | `to_networkx`, `to_networkx_nodes`, `to_rustworkx`, `graph_arrays`, `to_pyg`, `to_dgl` |
| [`duckosm.fixes`](sdk/fixes.md) | all groups | `Fixes`, `diff` |

Each module page has one row for each function: its arguments, what it returns and the OSMnx or other tool function that it matches.

## The functions of the package

The top-level functions stay, with the same arguments: `duckosm.route(db, ...)`, `duckosm.to_sumo(db, ...)` and the
others. Their mode default is `"driving"`, as before. `build` gives a `Network`, which acts as a path.

### Rules for all functions

| Rule | Description |
|---|---|
| Database argument | `db` (also `con` or `source`) is a path to a `.duckdb` file, or a DuckDB connection to a database file. |
| Option names | The option names are the command options with `_` in place of `-`. `--h3-resolution` is `h3_resolution=`. |
| An option that you do not give | It has the default of the command. |
| A list option | A Python list. `modes=["driving", "walking"]` is `-m driving -m walking`. |
| An error | The function raises `RuntimeError` with the message of the command. |

### Build and areas


| Function | Main options | Returns | Command |
|---|---|---|---|
| `get(place=None, *, osm_id=None, bbox=None, h3_cell=None, boundary=None, point=None, dist_m=1000, out=".", name=None, fixes=None, cache=None, refresh=False, progress=None)` | Give one of `place`, `osm_id`, `bbox` (a tuple `(west, south, east, north)`), `h3_cell`, `point` (`(lon, lat)`, with the box `dist_m` metres around it) and `boundary`. `boundary`: a GeoJSON file, a GeoJSON dict, or an object with `__geo_interface__` (shapely, geopandas). `fixes`: the cut file has the [OSM fixes](../guides/edit-osm-data.md#the-fixes-file) in it. | an object with `pbf` and `boundary`, the `Path`s of the two files | [`duckosm get`](cli.md#get) |
| `build(pbf=None, *, config=None, boundary=None, modes=None, output=None, source_db=None, h3_cell=None, fixes=None, features=None, levels=None, graph=None, h3_index=None, h3_resolution=None)` | `config` gives the settings of a [config file](../guides/build.md#the-config-file). An option that is not `None` overrides the file. Without a config file, `None` is the default of the command (`modes=["driving"]`, the switches on, `h3_resolution=8`). | a [`Network`](#network): the `.duckdb` file | [`duckosm build`](cli.md#build) |
| `tiles(region=None, *, boundary=None, out=None, config=None, zoom=8, memory="2G", jobs=None, tile_threads=2, keep_tiles=False, build_args=[])` | `build_args`: the build options of each tile (the options after `--`) | the `Path` of the output folder | [`duckosm tiles`](cli.md#tiles) |
| `quack(pbf, *, out=".", name="area", formats=None, gdal=False, fixes=None, threads=None, sort=True, tiles=None, tile_zoom=None)` | `formats`: `["geoparquet"]`, `["duckdb"]` or both. With `gdal=True`: `["gpkg"]` or `["geoparquet"]`. `fixes`: an [OSM fixes file](../guides/edit-osm-data.md#the-fixes-file). `tile_zoom`: make the table tile by tile (zoom 0 to 14). | a list of the `Path`s of the files that it wrote | [`duckosm quack`](cli.md#quack) |
| `extract(source, db, *, name=None, osm_id=None, boundary=None)` | Give one of `name`, `osm_id` and `boundary`. | the `Path` of the new database | [`duckosm extract`](cli.md#extract) |
| `boundary(name=None, *, pbf=None, osm_id=None, offline=False, out=None)` | `pbf`: search the borders in this PBF first | the `Path` of the GeoJSON file | [`duckosm boundary`](cli.md#boundary) |
| `levels(db)` | Solves the drawing order again with the edits in the database | the `Path` of the database | [`duckosm levels`](cli.md#levels) |
| `init_config(path="duckosm.yaml", *, force=False)` | `force`: overwrite the file | the `Path` of the file | [`duckosm init-config`](cli.md#init-config) |

### Routes, graphs and exports

The other top-level functions are on the pages of their modules: [routing](sdk/routing.md), [ml](sdk/ml.md),
[sim](sdk/sim.md), [planning](sdk/planning.md), [hdmap](sdk/hdmap.md) and [gis](sdk/gis.md).
`to_railml(source, out_path)` (experimental) and `to_elevation(db, dem=None, ...)` ([Add elevation](../guides/elevation.md))
stay in the core.
