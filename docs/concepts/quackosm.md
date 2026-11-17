# duckOSM and QuackOSM

[QuackOSM](https://github.com/kraina-ai/quackosm) turns an `.osm.pbf` file into one table of OSM features.
`duckosm-rust quack` makes the **same table, with the same rows**: faster, with less memory, and also in the browser.

```bash
duckosm-rust quack france.osm.pbf -o out --name france -f geoparquet,duckdb
```

| Option | Does |
|---|---|
| `--no-sort` | do not sort the rows (faster) |
| `--tile-zoom 8` | make the table tile by tile (for a country) |

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

## Where QuackOSM is better

- Its `.duckdb` file is smaller (France: 19.7 GB against 22.3 GB).
- It has filters (tags, ids, a geometry) and more output options.

More details: [docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md).
