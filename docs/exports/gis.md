# GeoPackage and shapefile

```bash
duckosm export-gis monaco.duckdb                 # -> monaco.gpkg
duckosm export-gis monaco.duckdb -f shp          # -> monaco_gis/monaco_edges_driving.shp, … (one per layer)
```

For QGIS, ArcGIS, FME, or any tool that imports GIS files (Visum, Aimsun, …). GeoPackage uses
GDAL's `ogr2ogr` to put every layer in one file; without it you get one `.gpkg` per layer.

## Layers

One pair per mode, plus the area boundary. Monaco:

| Layer | Features |
|---|---|
| `edges_driving`, `nodes_driving` | 2,765, 1,719 |
| `edges_walking`, `nodes_walking` | 10,706, 3,996 |
| `edges_cycling`, `nodes_cycling` | 10,228, 4,128 |
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
| `-f`, `--format` | `gpkg` or `shp` | `gpkg` |
| `-o`, `--out` | `.gpkg` file, or folder for shapefiles | `<name>.gpkg` / `<name>_gis/` |
| `--name` | the file name | the db's name |
| `--no-boundary` | leave out the boundary layer | included |

```python
import duckdb
from duckosm import to_gis
con = duckdb.connect("monaco.duckdb", read_only=True)
to_gis(con, "monaco.gpkg")             # -> {"layers": {name: count}, ...}
```

## Check an export

```bash
duckosm gis-debug monaco.gpkg --source-db monaco.duckdb     # -> monaco_gis_debug.html
```

Reads the file back through GDAL, as a GIS tool would, and writes a page with a map of every layer
and a check: feature counts, CRS and, with `--source-db`, whether every `edge_id` in the file matches
the database exactly. It ends with PASS, CHECK or FAIL. Needs `geopandas`.
