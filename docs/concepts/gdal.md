# duckOSM and GDAL

GDAL's OSM driver reads an `.osm.pbf` file as five layers. `ogr2ogr` and QGIS use it.
`duckosm-rust quack --gdal` makes the **same five layers, with the same rows**: faster, and also in the browser.

```bash
duckosm-rust quack kentucky.osm.pbf --gdal -o out --name kentucky                # out/kentucky_gdal.gpkg
duckosm-rust quack kentucky.osm.pbf --gdal -o out --name kentucky -f geoparquet  # out/kentucky_gdal_<layer>.parquet
duckosm-rust quack france.osm.pbf --gdal -o out --name france --tile-zoom 8      # tile by tile (for a country)
```

| Option | Does |
|---|---|
| `-f gpkg` | one GeoPackage with the five layers (the default), as `ogr2ogr -f GPKG` writes it |
| `-f geoparquet` | one GeoParquet file for each layer |
| `--tile-zoom 8` | make the layers tile by tile, so the memory follows the size of a tile |

In the browser: the [quack page](https://khoshkhah.github.io/duckosm-studio/quack.html) of duckOSM Studio, output
**GDAL layers** (GeoParquet, tile by tile).

## The layers

The rules are the ones of GDAL 3.13 with its default `osmconf.ini`.

| Layer | Rows | Columns |
|---|---|---|
| `points` | each node with a tag that is not `created_by`, `source`, ... | `osm_id`, `name`, `barrier`, `highway`, `ref`, `address`, `is_in`, `place`, `man_made` |
| `lines` | each way that is not an area | `osm_id`, `name`, `highway`, `waterway`, `aerialway`, `barrier`, `man_made`, `railway`, `z_order` |
| `multilinestrings` | each relation of the type `multilinestring` or `route` | `osm_id`, `name`, `type` |
| `multipolygons` | each area (a closed way, `closed_ways_are_polygons` of `osmconf.ini`) and each relation of the type `multipolygon` or `boundary` | `osm_id` (relation), `osm_way_id` (way), `name`, `type`, `building`, `landuse`, ... (22 columns) |
| `other_relations` | each other relation: its member nodes and ways | `osm_id`, `name`, `type` |

Each layer also has `other_tags`: the tags that are not a column, as GDAL writes them (`"key"=>"value",...`).

## Results

Mac mini (12 cores, 16 GB). GDAL 3.13.3 of Homebrew. The memory is the peak footprint.

### France (5.1 GB), GeoParquet, `--tile-zoom 8`

| | `ogr2ogr` | duckOSM | duckOSM in the browser |
|---|---|---|---|
| Time | 14 min 18 s (5 runs) | **3 min 37 s** | 5 min 23 s |
| Time, one read (estimate) | about 6 min 10 s | | |
| Memory | 1.6 GB | **0.9 GB** | 1.3 GB + 4.1 GB in 12 tile workers |
| Files | 15.6 GB | **11.3 GB** | 13.7 GB |

### Kentucky (141 MB)

| | `ogr2ogr` | duckOSM |
|---|---|---|
| GeoPackage, time | 8.8 s | **4.5 s** |
| GeoPackage, time, `--tile-zoom 8` | | 5.3 s |
| GeoPackage, memory | 0.44 GB | **0.28 GB** |
| GeoParquet, time | 12.5 s (5 runs) | **3.7 s** |
| GeoParquet, time, `--tile-zoom 8` | | 4.1 s |
| GeoParquet, memory | 0.58 GB | **0.27 GB** |
| GeoParquet, files | 0.43 GB | **0.37 GB** |

GDAL's Parquet driver writes one layer to one file, so `ogr2ogr -f Parquet` is one run for each layer
(`ogr2ogr -f Parquet points.parquet france.osm.pbf points`, and so on), and each run reads the whole file. One read to all
five layers (`ogr2ogr -f FlatGeobuf`, one file for each layer) took 5.4 s on Kentucky: 0.43 of the five runs. The France
estimate is 0.43 of its five runs; it was not measured. France as FlatGeobuf (about 27 GB) or as a GeoPackage (about 31 GB)
did not fit on the disk of the Mac mini with 30 GB free.

## The rows are the same

| Area | Check |
|---|---|
| Monaco | each layer equals `ogr2ogr -f GPKG` row by row: ids, column values, geometries (a test of the build) |
| Monaco, in the browser | each layer equals the native GeoParquet files (a test of the page, at phone size) |
| Kentucky | all 2 240 511 rows equal GDAL on Linux (x86-64) |

GDAL on the Mac mini (arm64) gives one other polygon on Kentucky: two rings of relation 13011728 touch at a point, and
GDAL's arm64 build puts one ring outside, not in a hole. GDAL on Linux and duckOSM put it in a hole.

## Why the files are smaller

Kentucky, the five layers as GeoParquet, Mac mini:

| | Time | Files |
|---|---|---|
| duckOSM (zstd level 1) | **3.2 s** | 0.37 GB |
| duckOSM with zstd level 9 (a test) | 5.4 s | **0.27 GB** |
| `ogr2ogr` (Snappy, a bbox column) | 13.1 s | 0.43 GB |
| `ogr2ogr -lco COMPRESSION=ZSTD` (level 9, a bbox column) | 15.5 s | 0.30 GB |
| `ogr2ogr -lco COMPRESSION=ZSTD -lco WRITE_COVERING_BBOX=NO` | 15.2 s | 0.28 GB |

The rows, their order and the row groups are the same. The size comes from:

- **The codec.** GDAL writes Snappy, duckOSM zstd. With `COMPRESSION=ZSTD`, GDAL uses level 9 and duckOSM level 1.
- **The bbox column.** GDAL adds a GeoParquet 1.1 bbox column (four numbers for each row): about 10 % of the file.

In the order of the file, the bbox column does not let a reader skip row groups: the box of each row group is 55 % to
73 % of the box of the layer, and a small area of Louisville reads all row groups of `lines`, `multipolygons` and `points`.
The column is useful when the rows are in the order of their place (as the sort of QuackOSM's table).

## Where GDAL is still better

- Its own `osmconf.ini`: other columns, `all_tags`, the metadata of the elements. duckOSM has the default rules only.
- The options of `ogr2ogr` (`-spat`, `-where`, `-sql`, ...) and its other output formats (FlatGeobuf, Shapefile, PostGIS, ...).
- The input: GDAL also reads `.osm` XML files and files in any order. duckOSM reads `.osm.pbf` files sorted by type and
  id, as the files of Geofabrik are.
- QGIS opens an `.osm.pbf` file with GDAL directly, with no file between.

More details: [docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md), "GDAL layers".
