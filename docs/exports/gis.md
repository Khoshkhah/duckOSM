# GeoPackage, GeoParquet and shapefile

```bash
duckosm export-gis monaco.duckdb                 # -> monaco.gpkg
duckosm export-gis monaco.duckdb -f shp          # -> monaco_gis/monaco_edges_driving.shp, … (one per layer)
duckosm export-gis monaco.duckdb -f geoparquet   # -> monaco_geoparquet/edges_driving.parquet, … (one per layer)
```

For QGIS, ArcGIS, FME, or any tool that imports GIS files (Visum, Aimsun, …). GeoPackage uses
GDAL's `ogr2ogr` to put every layer in one file; without it you get one `.gpkg` per layer.

## GeoParquet

GeoParquet is a column file format for geodata. Use it for web and cloud tools. GeoLibre
opens a file from a URL (`?data=<url>`). RustGIS also opens it. DuckDB, GeoPandas and GDAL read it too.

- `out` is a folder. The command writes one file per layer: `edges_<mode>.parquet`,
  `nodes_<mode>.parquet` and `boundary.parquet`. The layers are the same as in the GeoPackage.
- DuckDB writes the GeoParquet `geo` metadata. Coordinates are longitude / latitude (EPSG:4326).
- Column names stay as they are (no shapefile renames). `edge_id` stays a 64-bit integer. As in
  the GeoPackage, list columns such as `refs` are not written.
- If the database has the table `visualization.edge_levels`, each `edges_<mode>.parquet` file also
  gets all its columns except `edge_id`. The join uses `edge_id`. An edge with no row there gets
  NULL values. The columns are `casing_start`, `casing_level`, `casing_end` and `fill_level`, and,
  if the table has them, `head_start_m`, `head_end_m`, `cap_start` and `cap_end`. The `nodes` and
  `boundary` files do not get them. duckOSM only reads that table. Without it, the command writes
  the edges as they are and logs one line.

## Layers

One pair per mode, plus the area boundary. Monaco:

| Layer | Features |
|---|---|
| `edges_driving`, `nodes_driving` | 2,765, 1,719 |
| `edges_walking`, `nodes_walking` | 10,952, 4,116 |
| `edges_cycling`, `nodes_cycling` | 10,274, 4,151 |
| `boundary` | 1 |

Every column of the `edges` and `nodes` tables comes along as an attribute, except lists such as
`refs`: `edge_id`, `source`,
`target`, `osm_id`, `highway`, `name`, `oneway`, `lanes`, `length_m`, `maxspeed_kmh`, `cost_s`, …
([what each means](../reference/database.md)); with [elevation](../guides/elevation.md), `ele`,
`z_from`, `z_to` too. `length_m` and `cost_s` are rounded to 2 decimals. Coordinates are longitude / latitude (EPSG:4326).

Only the map layers are exported: the graph of legal turns and the turn restrictions are not GIS
data. For those, use [networkx](networkx.md) or [SUMO](sumo.md).

## Shapefile limits

Use a shapefile only for a tool that needs one:

- **Ids are written as text.** Every 64-bit column (`edge_id`, `node_id`, `osm_id`, `source`,
  `target`, the H3 cells) is too big for a shapefile number; read them back as strings. GeoPackage keeps
  them as 64-bit integers.
- Field names can have 10 characters, so `maxspeed_kmh` is renamed `spd_kmh`, `is_reverse`
  `is_rev`, `admin_level` `adm_level`.
- One file set per layer; 2 GB per file.

## Options

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode` | modes to export; repeat for several | every mode in the db |
| `-f`, `--format` | `gpkg`, `geoparquet` or `shp` | `gpkg` |
| `-o`, `--out` | `.gpkg` file, or folder for shapefiles and GeoParquet | `<name>.gpkg` / `<name>_gis/` / `<name>_geoparquet/` |
| `--name` | the file name | the db's name |
| `--no-boundary` | leave out the boundary layer | included |

```python
import duckdb
from duckosm import to_gis
con = duckdb.connect("monaco.duckdb", read_only=True)
to_gis(con, "monaco.gpkg")             # -> {"layers": {name: count}, ...}
to_gis(con, "monaco_geoparquet", fmt="geoparquet")   # a folder of .parquet files
```

## Check an export

```bash
duckosm gis-debug monaco.gpkg --source-db monaco.duckdb     # -> monaco_gis_debug.html
```

Reads the file back through GDAL, as a GIS tool would, and writes a page with a map of every layer
and a check: feature counts, CRS and, with `--source-db`, whether every `edge_id` in the file matches
the database exactly. It ends with PASS, CHECK or FAIL. Needs `geopandas`.
