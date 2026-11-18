# duckOSM and GDAL

GDAL's OSM driver (`ogr2ogr`, QGIS) reads an `.osm.pbf` file as five layers.
`duckosm-rust quack --gdal` makes the **same five layers, with the same rows**: faster, and also in the browser.

```bash
duckosm-rust quack kentucky.osm.pbf --gdal -o out --name kentucky                # one GeoPackage, as ogr2ogr -f GPKG
duckosm-rust quack kentucky.osm.pbf --gdal -o out --name kentucky -f geoparquet  # one GeoParquet file for each layer
duckosm-rust quack france.osm.pbf --gdal -o out --name france --tile-zoom 8      # tile by tile (for a country)
```

In the browser: the [quack page](https://khoshkhah.github.io/duckosm-studio/quack.html) of duckOSM Studio, output **GDAL layers**.

## Results

Mac mini (12 cores, 16 GB), GDAL 3.13.3, GeoParquet. 1 GB = 10⁹ bytes; the memory is the peak footprint.

### France (5.1 GB)

| | `ogr2ogr` | duckOSM | duckOSM in the browser |
|---|---|---|---|
| Time | 14 min 18 s¹ | **3 min 37 s** | 5 min 23 s |
| Memory | 1.7 GB | **1.0 GB** | 1.3 GB + 4.3 GB² |
| Files | 16.7 GB | **12.1 GB** | 14.4 GB³ |

### Kentucky (148 MB)

| | `ogr2ogr` | duckOSM | duckOSM in the browser |
|---|---|---|---|
| Time | 12.5 s¹ | **3.7 s** | –⁴ |
| Memory | 0.62 GB | **0.29 GB** | –⁴ |
| Files | 0.46 GB | **0.39 GB** | –⁴ |

¹ Five runs, one for each layer: GDAL's Parquet driver writes one layer to a file. One read takes about 0.43 of this (Kentucky, measured); France one read: about 6 min 10 s (an estimate).
² In the 12 tile workers. ³ Snappy: the browser has no zstd. ⁴ Not measured on the Mac mini.
As a GeoPackage, Kentucky takes 8.8 s with `ogr2ogr` and 4.5 s with duckOSM.

## The layers

| Layer | What goes in |
|---|---|
| `points` | the nodes with tags |
| `lines` | the ways that are not areas |
| `multilinestrings` | the relations of the type `multilinestring` or `route` |
| `multipolygons` | the areas (closed ways) and the relations of the type `multipolygon` or `boundary` |
| `other_relations` | all other relations |

The rules and the columns are those of GDAL's default `osmconf.ini`; the other tags go into `other_tags`.

## The rows are the same

On Monaco (all layers, also in the browser) and Kentucky (2 240 511 rows), each row equals GDAL's.⁵

⁵ GDAL on Linux. GDAL on the Mac mini (arm64) gives one other polygon on Kentucky.

## Why the files are smaller

| Reason | Effect on the files of GDAL |
|---|---|
| GDAL writes Snappy, duckOSM zstd | larger |
| GDAL adds a bbox column (GeoParquet 1.1) | about 10 % larger |
| With `-lco COMPRESSION=ZSTD`, GDAL uses level 9, duckOSM level 1 | about 20 % smaller than duckOSM, 4 times slower |

## Where GDAL is better

- Your own `osmconf.ini`, and the options of `ogr2ogr` (`-spat`, `-where`, `-sql`).
- More inputs (`.osm` XML, files in any order) and more output formats.
- QGIS opens an `.osm.pbf` file with GDAL directly.

More details: [docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md), "GDAL layers".
