# duckOSM and GDAL

GDAL's OSM driver (`ogr2ogr`, QGIS) reads an `.osm.pbf` file as five layers.
`duckosm quack --gdal` makes the **same five layers, with the same rows**: faster, and also in the browser.

```bash
duckosm quack kentucky.osm.pbf --gdal -o out --name kentucky                # one GeoPackage, as ogr2ogr -f GPKG
duckosm quack kentucky.osm.pbf --gdal -o out --name kentucky -f geoparquet  # one GeoParquet file for each layer
duckosm quack france.osm.pbf --gdal -o out --name france --tile-zoom 8      # tile by tile (for a country)
```

In the browser: the [quack page](https://khoshkhah.github.io/duckosm-studio/quack.html) of duckOSM Studio, output **GDAL layers**, format GeoParquet, GeoPackage or both.

## Results

Mac mini (12 cores, 16 GB), GDAL 3.13.3, GeoParquet. 1 GB = 10⁹ bytes; the memory is the peak footprint.

### France (5.1 GB), `--tile-zoom 8`

| | `ogr2ogr` | duckOSM | duckOSM in the browser² |
|---|---|---|---|
| Time, GeoPackage | ≈ 10 min³ | ≈ 4 min 40 s³ | |
| Time, GeoParquet | 14 min 18 s¹ | **3 min 37 s** | 5 min 23 s |
| Memory, GeoPackage | ≈ 1.3 GB³ | ≈ 1.0 GB³ | |
| Memory, GeoParquet | 1.7 GB | **1.0 GB** | 1.3 GB + 4.3 GB⁴ |
| Files, GeoPackage | ≈ 37 GB³ | ≈ 30 GB³ | |
| Files, GeoParquet | 16.7 GB | **12.1 GB** | 14.4 GB⁵ |

### Kentucky (148 MB)

| | `ogr2ogr` | duckOSM | duckOSM in the browser² |
|---|---|---|---|
| Time, GeoPackage | 8.8 s | **4.5 s** | 13.0 s |
| Time, GeoParquet | 12.5 s¹ | **3.7 s** | 7.8 s |
| Memory, GeoPackage | 0.47 GB | **0.30 GB** | 0.27 GB + 0.96 GB⁴ |
| Memory, GeoParquet | 0.62 GB | **0.29 GB** | 0.27 GB + 0.96 GB⁴ |
| Files, GeoPackage | 1.01 GB | **0.97 GB** | 0.89 GB |
| Files, GeoParquet | 0.46 GB | **0.39 GB** | 0.41 GB⁵ |

¹ Five runs, one for each layer: GDAL's Parquet driver writes one layer to a file. One read takes about 0.43 of this
(Kentucky, measured); France one read: about 6 min 10 s (an estimate).
² France: GeoParquet only. ³ An estimate from the Kentucky ratios: France as a GeoPackage does not fit on the disk of the Mac mini
with 30 GB free. ⁴ In the 12 tile workers. ⁵ Snappy: the browser has no zstd.

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

On Monaco (all layers, also in the browser) and Kentucky (2 240 511 rows), each row equals GDAL's.⁵ The GeoPackage of the
browser has the rows of the native GeoPackage on Monaco (0.9 s, 3.6 MB) and on Kentucky (relation rows can be in another order).
SQLite's `integrity_check` is ok, and `ogrinfo` opens it. The browser writes it directly into its file in the browser storage, with no
copy in memory.

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
