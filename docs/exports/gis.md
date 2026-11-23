# GeoPackage, GeoParquet and shapefile

`duckosm export-gis` writes the network of each mode as GIS layers, with your OSM fixes, for QGIS, ArcGIS and FME.

| Reference and format | |
|---|---|
| Format | [GeoPackage](https://www.geopackage.org/), [GeoParquet](https://geoparquet.org/) or shapefile, in longitude and latitude (EPSG:4326) |
| Check | GDAL reads each file back, and each `edge_id` equals the database. [How each output is checked](../concepts/checks.md) |

```bash
duckosm export-gis monaco.duckdb                 # -> monaco.gpkg
duckosm export-gis monaco.duckdb -f geoparquet   # -> monaco_geoparquet/
duckosm export-gis monaco.duckdb -f shp          # -> monaco_gis/
```

## The layers

| Layer | Columns |
|---|---|
| `edges_<mode>` | the columns of [`edges`](../reference/database.md): `edge_id`, `source`, `target`, `osm_id`, `highway`, `name`, `oneway`, `lanes`, `maxspeed_kmh`, `length_m`, `cost_s`, … |
| `nodes_<mode>` | the columns of [`nodes`](../reference/database.md): `node_id`, … |
| `boundary` | the area boundary |

## The formats

| | GeoPackage | GeoParquet | Shapefile |
|---|---|---|---|
| Files | one file | one file for each layer | a set of files for each layer |
| `edge_id`, `osm_id` | integers | integers | text |
| Column names | full | full | 10 characters (`spd_kmh`) |

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB). France is the driving network only.

| Area (input DuckDB) | Format | Time | Memory | Output |
|---|---|---|---|---|
| Kentucky (~1.2 GB) | GeoPackage | 14 s | 663 MB | 843 MB |
| | GeoParquet | 0.33 s | 2.1 GB | 386 MB |
| | shapefile | 17 s | 606 MB | 3.2 GB |
| France (~35 GB) | GeoParquet | 6.9 s | 7.4 GB | 3.6 GB |

All options: [Command line](../reference/cli.md#export-gis).
