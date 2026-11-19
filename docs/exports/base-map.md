# Base-map layers

A build also makes the layers of a base map: water, land use, buildings, points of interest and place names. Each
layer is one table in the schema `features` of the database. The layers take your OSM fixes, as the network does.

| Reference and format | |
|---|---|
| Format | tables `features.<layer>` in the [DuckDB file](https://duckdb.org/docs/stable/internals/storage) of the build, with the classes of the [Shortbread](https://shortbread-tiles.org/schema/1.0/) schema. As files: [GeoParquet](https://geoparquet.org/) |
| Reference tool | none |
| Check | each row of each layer equals the golden file of Monaco, with and without a boundary. [How each output is checked](../concepts/checks.md) |

```bash
duckosm build --pbf monaco.osm.pbf -b monaco.geojson --fixes osm_fixes.yaml   # -> monaco.duckdb, with the layers
duckosm export-gis monaco.duckdb -f geoparquet                                # -> one GeoParquet file for each layer
```

| To get them as | Do |
|---|---|
| DuckDB tables | open the `.duckdb` file: `SELECT kind, count(*) FROM features.land GROUP BY kind` |
| GeoParquet files | `duckosm export-gis -f geoparquet`: one file `features_<layer>.parquet` for each layer ([GeoPackage and shapefile](gis.md)) |
| a map | [`duckosm viz`](../guides/draw-map.md) and [duckOSM Studio](../guides/studio.md) draw them |

- **Turn them off:** `--no-features`, or `options.build_features: false` in the config file.
- **A database cut from a larger one** has no base-map layers: it has no `raw.*` tables to make them from.
- **OSM polygons that no layer holds:** `duckosm features-check monaco.duckdb` lists them.

## The layers

In drawing order, bottom first.

| Layer | Shape | What goes in |
|---|---|---|
| `ocean` | area | the sea, from Overture Maps' ocean polygons (they are made from OSM's coastline) |
| `land` | area | land use and land cover: forest, grass, residential, park, cemetery |
| `water_polygons` | area | lakes, rivers as areas, reservoirs, swimming pools |
| `water_lines` | line | rivers, streams, canals, ditches |
| `sites` | area | parking, schools, hospitals, stations, platforms |
| `paved` | area | squares, pedestrian areas, piers, quays |
| `buildings` | area | each building |
| `streets` | line | each way with a `highway` tag, and the railways, for drawing |
| `public_transport` | point | bus stops, stations, tram stops, ferry terminals |
| `pois` | point | amenities, shops, tourism, offices |
| `traffic` | point | traffic signals and crossings |
| `place_labels` | point | cities, towns, villages, suburbs, islands |

Each layer has the same columns:

| Column | Type | Holds |
|---|---|---|
| `osm_id` | BIGINT | the OSM id, unique together with `osm_type` |
| `osm_type` | VARCHAR | `node`, `way` or `relation` |
| `kind` | VARCHAR | the class, for example `forest` or `bus_stop` |
| `name` | VARCHAR | the OSM `name` |
| `tags` | MAP(VARCHAR, VARCHAR) | every OSM tag |
| `geom` | GEOMETRY | the shape, in longitude and latitude |

`traffic` also has `bearing`: the direction of the road under a crossing, in degrees from north. `ocean` has `source`
(the Overture release) in place of `tags`.

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB). The time of the whole build (driving) with the layers.

| Area (input) | Time | Memory | Output |
|---|---|---|---|
| Monaco (0.7 MB) | 0.27 s | 134 MB | 13.6 MB |
| Kentucky (148 MB) | 44 s | 3.7 GB | 2.8 GB |
| France (5.1 GB) | not run: the Mac mini has too little free disk | | |

The OSM tags and the classes of each layer: [Base-map layers](../reference/features.md) in the Reference.
