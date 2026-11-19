# Python SDK

The Python package `duckosm` has one function for each step of duckOSM. Each function calls the Rust
program. Python does not do the work: the result is the same as the result of the command.

```bash
pip install duckosm
```

## From a PBF file to a route and an export

```python
import duckosm

area = duckosm.get("Monaco")                               # area.pbf, area.boundary
db = duckosm.build(pbf=area.pbf, boundary=area.boundary,
                   modes=["driving", "walking"])           # Path("monaco.duckdb")

r = duckosm.route_points(db, (7.4155, 43.7285), (7.4400, 43.7480))   # (lon, lat): Fontvieille to Larvotto
r["time_s"], r["length_m"], r["edges"]                     # the fastest route, door to door
for step in duckosm.directions(db, r):
    print(step)

duckosm.to_sumo(db, "sumo/")                               # SUMO files of the driving network
duckosm.to_gis(db, "monaco.gpkg", modes=["driving"])       # a GeoPackage for QGIS
```

To build from a config file, use `duckosm.build(config="duckosm.yaml")`. See
[The config file](../guides/build.md#the-config-file).

## Rules for all functions

| Rule | Description |
|---|---|
| Database argument | `db` (also `con` or `source`) is a path to a `.duckdb` file, or a DuckDB connection to a database file. |
| Option names | The option names are the command options with `_` in place of `-`. `--h3-resolution` is `h3_resolution=`. |
| An option that you do not give | It has the default of the command. |
| A list option | A Python list. `modes=["driving", "walking"]` is `-m driving -m walking`. |
| An error | The function raises `RuntimeError` with the message of the command. |

## Build and areas


| Function | Main options | Returns | Command |
|---|---|---|---|
| `get(place=None, *, osm_id=None, bbox=None, h3_cell=None, boundary=None, out=".", name=None, fixes=None, cache=None, refresh=False)` | Give one of `place`, `osm_id`, `bbox` (a tuple `(west, south, east, north)`), `h3_cell` and `boundary`. `fixes`: the cut file has the [OSM fixes](../guides/edit-osm-data.md#the-fixes-file) in it. | an object with `pbf` and `boundary`, the `Path`s of the two files | [`duckosm get`](cli.md#get) |
| `build(pbf=None, *, config=None, boundary=None, modes=None, output=None, source_db=None, h3_cell=None, fixes=None, features=None, levels=None, graph=None, h3_index=None, h3_resolution=None)` | `config` gives the settings of a [config file](../guides/build.md#the-config-file). An option that is not `None` overrides the file. Without a config file, `None` is the default of the command (`modes=["driving"]`, the switches on, `h3_resolution=8`). | the `Path` of the `.duckdb` file | [`duckosm build`](cli.md#build) |
| `tiles(region=None, *, boundary=None, out=None, config=None, zoom=8, memory="2G", jobs=None, tile_threads=2, keep_tiles=False, build_args=[])` | `build_args`: the build options of each tile (the options after `--`) | the `Path` of the output folder | `duckosm tiles` |
| `quack(pbf, *, out=".", name="area", formats=None, gdal=False, fixes=None, threads=None, sort=True, tiles=None, tile_zoom=None)` | `formats`: `["geoparquet"]`, `["duckdb"]` or both. With `gdal=True`: `["gpkg"]` or `["geoparquet"]`. `fixes`: an [OSM fixes file](../guides/edit-osm-data.md#the-fixes-file). `tile_zoom`: make the table tile by tile (zoom 0 to 14). | a list of the `Path`s of the files that it wrote | `duckosm quack` |
| `extract(source, db, *, name=None, osm_id=None, boundary=None)` | Give one of `name`, `osm_id` and `boundary`. | the `Path` of the new database | [`duckosm extract`](cli.md#extract) |
| `boundary(name=None, *, pbf=None, osm_id=None, offline=False, out=None)` | `pbf`: search the borders in this PBF first | the `Path` of the GeoJSON file | [`duckosm boundary`](cli.md#boundary) |
| `levels(db)` | Solves the drawing order again with the edits in the database | the `Path` of the database | [`duckosm levels`](cli.md#levels) |
| `init_config(path="duckosm.yaml", *, force=False)` | `force`: overwrite the file | the `Path` of the file | [`duckosm init-config`](cli.md#init-config) |

## Routing

| Function | Main options | Returns | Command |
|---|---|---|---|
| `route(con, from_edge, to_edge, mode="driving", weight="time")` | `weight`: `"time"` or `"length"` | a dict `{edges, time_s, length_m, path}`, or `None` when no route connects the two edges | `duckosm route` |
| `Router(con, mode="driving", weight="time")` | Makes the graph one time. `.route(from_edge, to_edge)` gives the same dict as `route`. | a router | |
| `route_points(con, a, b, mode="driving", weight="time", radius_m=50.0, access_kmh=4.5)` | `a`, `b`: `(lon, lat)` points | a dict `{time_s, length_m, edges, start, end, path}`, or `None` | `duckosm route-points` |
| `route_multimodal(con, src_node, dst_node, schema="mm", start_mode="walking", end_mode="walking", enforce_sequence=True, allowed_modes=None)` | Needs the [`mm` schema](../concepts/multimodal.md) | a dict with the legs of each mode | `duckosm route-multimodal` |
| `route_multimodal_points(con, a, b, radius_m=50.0, access_kmh=4.5, schema="mm", allowed_modes=None, enforce_sequence=True)` | `a`, `b`: `(lon, lat)` points | a dict with the legs of each mode | `duckosm route-multimodal-points` |
| `directions(con, route, mode="driving")` | `route`: a result of a route function | a list of the turn-by-turn steps | `duckosm directions` |
| `to_networkx(con, mode="driving", weight="time")` | | a `networkx.DiGraph`: a node is an `edge_id` | `duckosm export-graph` |
| `to_networkx_nodes(con, mode="driving")` | | a `networkx.MultiDiGraph`: a node is a junction, an edge a road | `duckosm export-graph` |
| `to_rustworkx(con, mode="driving", graph="turns", weight="time")` | `graph`: `"turns"` (the graph of `to_networkx`) or `"junctions"` (the graph of `to_networkx_nodes`). Needs `duckosm[rustworkx]`. | a `rustworkx.PyDiGraph`; `g.attrs["index"]` gives the node index of an id | |

Guide: [Route](../guides/route.md).

## Exports

| Function | Main options | Returns | Command |
|---|---|---|---|
| `to_sumo(con, out_dir, mode="driving", net_name="network", run_netconvert=True, connections=True, config=None)` | `run_netconvert`: also make `.net.xml` with SUMO's `netconvert` | a dict of the file paths | [`duckosm sumo`](cli.md#sumo) |
| `to_matsim(source, out_path, mode="driving", crs=None, gzip=True)` | `mode`: one mode or a list | a dict of the numbers | [`duckosm matsim`](cli.md#matsim) |
| `to_gis(con, out, modes=None, fmt="gpkg", boundary=True, name=None, srs="EPSG:4326")` | `fmt`: `"gpkg"`, `"geoparquet"` or `"shp"` | a dict `{path, dir, files}` | [`duckosm export-gis`](cli.md#export-gis) |
| `to_gmns(source_db, out_path, modes=None, to_csv=None, lane_geometry=True, combined=False)` | `to_csv`: also write the CSV files to this folder | a dict `{path, csv, combined, modes}` | [`duckosm gmns`](cli.md#gmns) |
| `to_opendrive(source, out_path, mode="driving", crs=None, junctions=False)` | Experimental | a dict of the numbers | [`duckosm opendrive`](cli.md#opendrive) |
| `to_lanelet2(gmns_db, out_path, mode="driving", snap_m=0.01)` | Reads a [GMNS database](../exports/gmns.md). Experimental. | a dict of the numbers | [`duckosm lanelet2`](cli.md#lanelet2) |
| `to_railml(source, out_path)` | Experimental | a dict of the numbers | [`duckosm railml`](cli.md#railml) |
| `to_elevation(db, dem=None, source="auto", modes=None, vertical_datum=None, license=None, product=None, reference=True)` | Adds the heights to the database. `product`: `"DTM"` or `"DSM"`. | a dict with the source and the numbers | [`duckosm elevation`](cli.md#elevation) |

Guides: [Exports](../exports/index.md), [Add elevation](../guides/elevation.md).
