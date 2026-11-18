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
