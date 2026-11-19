# Base-map layers

A build from a PBF also makes the layers of a base map: water, land use, buildings, points of interest and place names.
They are tables in the schema `features`, one table for each layer.

| Reference and format | |
|---|---|
| Format | tables `features.<layer>` in the [DuckDB file](https://duckdb.org/docs/stable/internals/storage) of the build, with the classes of the [Shortbread](https://shortbread-tiles.org/schema/1.0/) schema; as files: [GeoParquet](https://geoparquet.org/) |
| Reference tool | none |
| Check | each row of each layer equals the golden file of Monaco, with and without a boundary. [How each output is checked](../concepts/checks.md) |

```bash
duckosm build --pbf monaco.osm.pbf -b monaco.geojson                              # -> monaco.duckdb, with the layers
duckosm monaco.osm.pbf -o out --name monaco --boundary monaco.geojson --features   # the native build: -> out/monaco.duckdb
duckosm export-gis monaco.duckdb -f geoparquet                                     # -> monaco_geoparquet/features_<layer>.parquet
```

## Get them

| As | Do |
|---|---|
| DuckDB tables | open the `.duckdb` file: `SELECT kind, count(*) FROM features.land GROUP BY kind` |
| GeoParquet files | `duckosm export-gis -f geoparquet`: one file `features_<layer>.parquet` for each layer ([GeoPackage, GeoParquet and shapefile](gis.md#geoparquet)) |
| a map | [`duckosm viz`](../guides/draw-map.md) and [duckOSM Studio](../guides/studio.md) draw them |

A database cut from a larger database has no `raw.*`, so it has no base-map layers.

## The layers

The layers, their columns and the OSM tags of each: [Base-map layers](../reference/features.md).

To list the OSM polygons that no layer holds:

```bash
duckosm features-check monaco.duckdb
```

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB). Time: the wall-clock time. Memory: the peak memory footprint (`/usr/bin/time -l`). 1 MB = 10⁶ bytes.

| Area | Time | Memory | Output size |
|---|---|---|---|
| Monaco (0.7 MB) | 0.27 s | 134 MB | 13.6 MB |
| Kentucky (148 MB) | 44 s | 3.7 GB | 2.8 GB |
| France (5.1 GB) | not run, needs 13 GB |  |  |

The command: `duckosm FILE.osm.pbf --boundary B.geojson -f duckdb --features`: the full build (driving) with the layers. France: an estimate from Kentucky.
