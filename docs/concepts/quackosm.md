# duckOSM and QuackOSM

[QuackOSM](https://github.com/kraina-ai/quackosm) turns an `.osm.pbf` file into one table of OSM features.
`duckosm quack` makes the **same table, with the same rows**: faster, with less memory, and also in the browser.

```bash
duckosm quack france.osm.pbf -o out --name france -f geoparquet,duckdb
```

| Option | Does |
|---|---|
| `--no-sort` | do not sort the rows (faster) |
| `--tile-zoom 8` | make the table tile by tile (for a country) |
| `--fixes osm_fixes.yaml` | apply the OSM fixes of [Edit OSM data](#osm-fixes) |

In the browser: the [quack page](https://khoshkhah.github.io/duckosm-studio/quack.html) of duckOSM Studio.

## Results

Mac mini (12 cores, 16 GB). duckOSM is **3 to 7 times faster** and needs **2 to 5 times less memory**.

### France (5.1 GB)

| | QuackOSM | duckOSM | duckOSM in the browser |
|---|---|---|---|
| Time | 9 min 30 s | **2 min 9 s** | 4 min 54 s |
| Time, sorted | 16 min 38 s | **2 min 30 s** | 5 min 40 s |
| Memory | 9.8 GB | **3.8 GB** | 6.9 GB |

### Kentucky (141 MB)

| | QuackOSM | duckOSM | duckOSM in the browser |
|---|---|---|---|
| Time | 13.0 s | **3.1 s** | 5.6 s |
| Time, sorted | 13.7 s | **4.0 s** | 9.6 s |
| Memory | 5.8 GB | **1.2 GB** | 1.0 GB |

France sorted in the browser used at most 17.9 GB of browser storage on the disk, for a GeoParquet file of 11.7 GB.

The rows are the same: 99.9998 % of the 99 million France rows have an equal geometry, and the rest differ only in the
last digits.

### Output size

MB = 10^6 bytes, GB = 10^9 bytes. QuackOSM is 0.20.0 on the Mac mini; it was not measured on Monaco and Tartu.

| Area (input) | Output | QuackOSM | duckOSM |
|---|---|---|---|
| Monaco (0.7 MB) | GeoParquet, sorted | | 0.85 MB |
| | GeoParquet, not sorted | | 0.83 MB |
| | `.duckdb`, sorted | | 3.2 MB |
| | `.duckdb`, not sorted | | 2.6 MB |
| Tartu (3.0 MB) | GeoParquet, sorted | | 4.7 MB |
| | GeoParquet, not sorted | | 5.1 MB |
| | `.duckdb`, sorted | | 11.3 MB |
| | `.duckdb`, not sorted | | 11.0 MB |
| Kentucky (141 MB) | GeoParquet, sorted | 311.5 MB | **308.4 MB** |
| | GeoParquet, not sorted | 349.3 MB | **320.5 MB** |
| | `.duckdb`, sorted | **670 MB** | 780 MB |
| | `.duckdb`, not sorted | 702 to 739 MB | 706 MB |
| France (5.1 GB) | GeoParquet, sorted | **9.05 GB** | 9.22 GB |
| | GeoParquet, not sorted | 11.62 GB | **10.29 GB** |
| | `.duckdb`, not sorted | **19.7 GB** | 22.3 GB |

The `.duckdb` file of duckOSM is larger because DuckDB 1.5.6 does not compress the geometry. DuckDB 2.0, which QuackOSM
uses, compresses it.

Monaco and Tartu ran on Linux (x86-64), Kentucky and France on the Mac mini; the Kentucky sizes of duckOSM were measured
again on 2026-11-18 and are the same as before.

## How it differs

| | QuackOSM | duckOSM |
|---|---|---|
| Language | Python + DuckDB | Rust |
| Steps | Parquet files between the steps | no files between the steps |
| Polygons | GEOS | `geo`, `i_overlay` |
| Large areas | one run | tile by tile, cut by [pbfcut](https://github.com/Khoshkhah/pbfcut) |
| Browser | no | yes (WebAssembly) |

## The table

| Column | Example |
|---|---|
| `feature_id` | `way/23614520` |
| `tags` | `{building=yes, name=Mairie}` |
| `geometry` | POINT, LINESTRING, POLYGON or MULTIPOLYGON |

One row for each OSM element with tags, as QuackOSM 0.20.0 with its default options.

## OSM fixes

`--fixes FILE` applies the OSM fixes file of `duckosm build --fixes` (the file of Edit OSM data) to the OSM data before
the table is made. Then the table has the same edits as the network. The result is the same with and without
`--tile-zoom`. The quack page of duckOSM Studio uses the fixes file of the area when the area has one.

| Rule of the fixes file | In the output |
|---|---|
| `remove`, `keep_nodes`, `tags` | applied to the way |
| `extra.tag_edits` | applied to the node, the way or the relation |
| `new_ways` | a new way, with the same id as in the network (from 9 000 000 000 000) |
| `oneway`, `lanes`, `lanes_forward`, `lanes_backward`, `layer` | OSM tags with the value of the rule: `oneway=yes` or `no`, `lanes`, `lanes:forward`, `lanes:backward`, `layer` |
| `exclude_modes`, `turn_lanes_*`, `turn_restrictions` | not applied: they have no OSM tag of a way |

## GDAL layers

`--gdal` makes the five layers of GDAL's OSM driver in place of this table: see [duckOSM and GDAL](gdal.md).

## Where QuackOSM is better

- Its `.duckdb` file is smaller (France: 19.7 GB against 22.3 GB).
- It has filters (tags, ids, a geometry) and more output options.

More details: [docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md).
