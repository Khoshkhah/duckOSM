# GeoPackage and shapefile

```bash
duckosm export-gis monaco.duckdb                 # -> monaco.gpkg
duckosm export-gis monaco.duckdb -f shp          # -> monaco_gis/, one shapefile per layer
```

For QGIS, ArcGIS, FME, or any tool that imports GIS files (Visum, Aimsun, …). Needs GDAL's
`ogr2ogr` for GeoPackage.

## Layers

One pair per mode, plus the area boundary. Monaco:

| Layer | Features |
|---|---|
| `edges_driving`, `nodes_driving` | 1,940, 1,160 |
| `edges_walking`, `nodes_walking` | 8,948, 3,491 |
| `edges_cycling`, `nodes_cycling` | 8,448, 3,566 |
| `boundary` | 1 |

Every column of the `edges` and `nodes` tables comes along as an attribute: `edge_id`, `source`,
`target`, `osm_id`, `highway`, `name`, `oneway`, `lanes`, `length_m`, `maxspeed_kmh`, `cost_s`, …
([what each means](../data_dictionary.md)); with [elevation](../guides/elevation.md), `ele`,
`z_from`, `z_to` too. Coordinates are longitude / latitude (EPSG:4326).

Only the map layers are exported: the graph of legal turns and the turn restrictions are not GIS
data. For those, use [networkx](networkx.md) or [SUMO](sumo.md).

## Shapefile limits

Use a shapefile only for a tool that needs one:

- **Ids are written as text.** `edge_id`, `node_id`, `osm_id`, `source`, `target` are 64-bit and a
  shapefile can't store them exactly as numbers; read them back as strings. GeoPackage keeps
  them as 64-bit integers.
- Field names are cut to 10 characters: `maxspeed_kmh` becomes `spd_kmh`, `is_reverse` `is_rev`.
- One file set per layer; 2 GB per file.

## Options

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode` | modes to export; repeat for several | every mode in the db |
| `-f`, `--format` | `gpkg` or `shp` | `gpkg` |
| `-o`, `--out` | `.gpkg` file, or folder for shapefiles | `<name>.gpkg` / `<name>_gis/` |
| `--no-boundary` | leave out the boundary layer | included |

```python
from duckosm import to_gis
to_gis(con, "monaco.gpkg")             # -> {"layers": {name: count}, ...}
```

## Check an export

```bash
duckosm gis-debug monaco.gpkg --source-db monaco.duckdb     # -> monaco_gis_debug.html
```

Reads the file back through GDAL, as a GIS tool would, and writes a page with a map of every layer
and a check: feature counts, CRS, and whether every `edge_id` in the file matches the database
exactly. It ends with PASS, CHECK or FAIL. Needs `geopandas`.
