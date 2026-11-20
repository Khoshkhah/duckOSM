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
| `net.tables()`, `net.columns("driving.edges")` | the tables of the file; the columns of one table and their types |
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
| `layers` | the base map of the file under the roads: `True` (all layers, the default), `False` (the roads only) or a list of roadstyle layer names, for example `["water", "buildings"]`. `plot` draws `ocean`, `landcover`, `water`, `waterways` and `buildings`. With `color_by` or `routes`, the live map has roadstyle's default base map and `layers` has no effect. |
| `highlight` | edge ids in orange; the other edges gray |
| (no `color_by`, all modes) | `plot` gives each road one color for the set of modes that can use it: all modes, driving + walking, driving + cycling, walking + cycling, driving, walking or cycling. These are the colors of the "Modes" option of `net.dashboard()`. |
| `routes` | a route, a list of routes or `{label: route}`: lines on top, each in its own color |

`show` sets how a notebook shows the map of `net.map()` and `net.dashboard()`. The setting `DUCKOSM_MAP_SHOW` gives
the default for all maps; without it the default is `"live"`.

| `show` | The output | Size (Monaco) |
|---|---|---|
| `"live"` | The live roadstyle map in the output | about 6 MB |
| `"link"` | The live map saved as an HTML file (`path=`, default `maps/<file>-<n>.html`) and a link to it | a link; the file about 6 MB |
| `"picture"` | A PNG of the live roadstyle map, taken in headless Chromium (`path=`, default `maps/<file>-<n>.png`). Needs `pip install playwright` and `playwright install chromium`. | about 175 KB |
| `"plot"` | `net.plot()`: a matplotlib picture without roadstyle | a small PNG |

The saved outputs of the [notebooks](../notebooks/index.md) are made with `DUCKOSM_MAP_SHOW=plot`. A map with `color_by` or `routes` uses `roadstyle.render_edges` on the default
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

| Module | Users | Today | Page |
|---|---|---|---|
| `duckosm.analysis` | street network analysts (OSMnx) | `stats`, `streets_per_node`, `bearings`, `orientation_entropy`, `plot_orientation`, `nearest_nodes`, `nearest_edges`, `components`, `largest_component`, `clip`, `add_elevation`, `grades`, `sample_points`, `to_osmnx` | [analysis](sdk/analysis.md) |
| `duckosm.gis` | GIS and data engineers | `to_gis`, `quack`, `admin` | [gis](sdk/gis.md) |
| `duckosm.routing` | routing and accessibility | `route`, `route_points`, `route_multimodal`, `route_multimodal_points`, `directions` | [routing](sdk/routing.md) |
| `duckosm.sim` | traffic simulation | `to_sumo`, `to_matsim`, `to_matsim_lanes` | [sim](sdk/sim.md) |
| `duckosm.planning` | transport planning | `to_gmns`, `check_gmns`, `to_meso`, `to_micro` | [planning](sdk/planning.md) |
| `duckosm.mobility` | mobility data | `snap`, `match`, `join`, `aggregate`, `edge_id_map` | [mobility](sdk/mobility.md) |
| `duckosm.hdmap` | automated driving and HD maps | `to_opendrive`, `to_lanelet2`, `lane_graph`, `route_lanes` | [hdmap](sdk/hdmap.md) |
| `duckosm.ml` | graph machine learning | `to_networkx`, `to_networkx_nodes`, `to_rustworkx` | [ml](sdk/ml.md) |
| `duckosm.fixes` | all groups | comes in PR 6 | [fixes](sdk/fixes.md) |

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
