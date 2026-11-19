# GDAL's OSM layers

GDAL's OSM driver (`ogr2ogr`, QGIS) reads an `.osm.pbf` file as five layers. `duckosm quack --gdal` makes the **same
five layers, with the same rows**. It is faster, it runs in the browser too, and it can apply your OSM fixes.

| Reference and format | |
|---|---|
| Format | five layers, as one [GeoPackage](https://www.geopackage.org/) or one [GeoParquet](https://geoparquet.org/) file for each layer |
| Reference tool | GDAL's [OSM driver](https://gdal.org/en/stable/drivers/vector/osm.html) 3.13, with its default `osmconf.ini` |
| Check | each row equals GDAL's, on Monaco and on Kentucky (2 240 511 rows). [How each output is checked](checks.md) |

```bash
duckosm quack monaco.osm.pbf --gdal -o out --name monaco                  # one GeoPackage, as ogr2ogr -f GPKG
duckosm quack monaco.osm.pbf --gdal -o out --name monaco -f geoparquet    # one GeoParquet file for each layer
duckosm quack monaco.osm.pbf --gdal -o out --fixes osm_fixes.yaml         # with your OSM fixes
```

The options are those of the [QuackOSM table](quackosm.md): `-f`, `--fixes`, `--tile-zoom` and `--no-sort`.

In the browser: the [quack page](https://khoshkhah.github.io/duckosm-studio/quack.html) of duckOSM Studio, output
**GDAL layers**. The browser writes the GeoPackage directly into its storage, with no copy in memory.

## The layers

| Layer | What goes in | Columns |
|---|---|---|
| `points` | the nodes with tags | `osm_id`, `name`, `barrier`, `highway`, `ref`, `address`, `is_in`, `place`, `man_made`, `other_tags` |
| `lines` | the ways that are not areas | `osm_id`, `name`, `highway`, `waterway`, `aerialway`, `barrier`, `man_made`, `railway`, `z_order`, `other_tags` |
| `multilinestrings` | the relations of the type `multilinestring` or `route` | `osm_id`, `name`, `type`, `other_tags` |
| `multipolygons` | the areas (closed ways), and the relations of the type `multipolygon` or `boundary` | `osm_id`, `osm_way_id`, `name`, `type`, `aeroway`, `amenity`, `admin_level`, `barrier`, `boundary`, `building`, `craft`, `geological`, `historic`, `land_area`, `landuse`, `leisure`, `man_made`, `military`, `natural`, `office`, `place`, `shop`, `sport`, `tourism`, `other_tags` |
| `other_relations` | all other relations | `osm_id`, `name`, `type`, `other_tags` |

Each layer also has its geometry. All columns are text, except `z_order` (an integer). They are the columns of GDAL's
default `osmconf.ini`:

- **`other_tags`** holds all other tags of the object, as GDAL writes them: `"key"=>"value","key2"=>"value2"`.
- **`osm_way_id`** is set for an area that is a closed way. `osm_id` is set for an area that is a relation.
- **`z_order`** is the drawing order of a line, from its road or railway class and its `layer`, `bridge` and `tunnel` tags, by the rule of GDAL.

## OSM fixes

`--fixes` applies the fixes file of [Fix the OSM data](../guides/edit-osm-data.md) to the five layers, with the same
rules as the [QuackOSM table](quackosm.md#osm-fixes). GDAL has no such option. With no `--fixes`, the rows are the rows
of GDAL.

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB), GDAL 3.13.3. GeoParquet output.

| Area (input) | | `ogr2ogr` | duckOSM | duckOSM in the browser |
|---|---|---|---|---|
| Monaco (0.7 MB) | Time | | **0.05 s** | |
| | Memory | | **0.01 GB** | |
| | Files | | 0.9 MB | |
| Kentucky (148 MB) | Time | 12.5 s | **3.7 s** | 7.8 s |
| | Memory | 0.62 GB | **0.29 GB** | 1.2 GB |
| | Files | 0.46 GB | **0.39 GB** | 0.41 GB |
| France (5.1 GB), by tiles | Time | 14 min 18 s | **3 min 37 s** | 5 min 23 s |
| | Memory | 1.7 GB | **1.0 GB** | 5.6 GB |
| | Files | 16.7 GB | **12.1 GB** | 14.4 GB |

- **`ogr2ogr` time:** GDAL's Parquet driver writes one layer to a file, so it runs five times.
- **Smaller files:** duckOSM writes zstd, GDAL writes Snappy and adds a bbox column. The browser writes Snappy, because
  it has no zstd.
- **Browser memory:** the page and its 12 tile workers together.
- **GeoPackage** on Kentucky: `ogr2ogr` 8.8 s, duckOSM **4.5 s**.

The GeoPackage measurements and the estimates for France:
[docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md), "GDAL layers".

## How it differs

| | GDAL | duckOSM |
|---|---|---|
| Speed | | 2 to 4 times faster |
| Browser | no | yes |
| OSM fixes | no | yes |
| Your own `osmconf.ini` | yes | no |
| Filters (`-spat`, `-where`, `-sql`) | yes | no |
| Inputs | `.osm.pbf`, `.osm` XML, files in any order | `.osm.pbf`, sorted |
| Output formats | many | GeoPackage, GeoParquet |
