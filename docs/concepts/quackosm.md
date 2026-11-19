# QuackOSM table

[QuackOSM](https://github.com/kraina-ai/quackosm) turns an `.osm.pbf` file into one table of OSM features.
`duckosm quack` makes the **same table, with the same rows**. It is faster, it needs less memory, it runs in the
browser too, and it can apply your OSM fixes.

| Reference and format | |
|---|---|
| Format | one table, as [GeoParquet](https://geoparquet.org/) or a DuckDB file |
| Reference tool | [QuackOSM](https://github.com/kraina-ai/quackosm) 0.20.0, with its default options |
| Check | the same rows as QuackOSM on France. [How each output is checked](checks.md) |

```bash
duckosm quack monaco.osm.pbf -o out --name monaco -f geoparquet,duckdb
```

| Option | Does |
|---|---|
| `-f` | the formats: `geoparquet`, `duckdb` or both |
| `--fixes osm_fixes.yaml` | applies your [OSM fixes](#osm-fixes) |
| `--tile-zoom 8` | makes the table tile by tile, for a country |
| `--no-sort` | does not sort the rows (faster) |
| `--gdal` | makes [GDAL's five OSM layers](gdal.md) in place of this table |

In the browser: the [quack page](https://khoshkhah.github.io/duckosm-studio/quack.html) of duckOSM Studio.

## The table

One row for each OSM element with tags.

| Column | Example |
|---|---|
| `feature_id` | `way/23614520` |
| `tags` | `{building=yes, name=Mairie}` |
| `geometry` | POINT, LINESTRING, POLYGON or MULTIPOLYGON |

## OSM fixes

`--fixes` applies the fixes file of [Fix the OSM data](../guides/edit-osm-data.md) before the table is made. Then the
table has the same edits as the network. QuackOSM has no such option. The quack page of duckOSM Studio uses the fixes
file of the area when the area has one.

| Rule of the fixes file | In the table |
|---|---|
| `remove`, `keep_nodes`, `tags` | applied to the way |
| `extra.tag_edits` | applied to the node, the way or the relation |
| `new_ways` | a new way, with the same id as in the network (from 9 000 000 000 000) |
| `oneway`, `lanes`, `lanes_forward`, `lanes_backward`, `layer` | the OSM tag with the value of the rule |
| `exclude_modes`, `turn_lanes_*`, `turn_restrictions` | not applied: no OSM tag of a way holds them |

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB). Sorted rows, GeoParquet.

| Area (input) | | QuackOSM | duckOSM | duckOSM in the browser |
|---|---|---|---|---|
| Monaco (0.7 MB) | Time | | **0.05 s** | |
| | Memory | | **0.05 GB** | |
| | File | | 0.85 MB | |
| Kentucky (141 MB) | Time | 13.7 s | **4.0 s** | 9.6 s |
| | Memory | 5.8 GB | **1.2 GB** | 1.0 GB |
| | File | 311.5 MB | **308.4 MB** | |
| France (5.1 GB) | Time | 16 min 38 s | **2 min 30 s** | 5 min 40 s |
| | Memory | 9.8 GB | **3.8 GB** | 6.9 GB |
| | File | **9.05 GB** | 9.22 GB | |

- **The `.duckdb` file of duckOSM is larger** (France: 22.3 GB against 19.7 GB). DuckDB 1.5.6, which duckOSM uses, does
  not compress the geometry. DuckDB 2.0, which QuackOSM uses, does.

All sizes, and the runs without sort: [docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md).

## How it differs

| | QuackOSM | duckOSM |
|---|---|---|
| Language | Python and DuckDB | Rust |
| Between the steps | Parquet files | no files |
| Polygons | GEOS | `geo`, `i_overlay` |
| Large areas | one run | tile by tile, cut by [pbfcut](https://github.com/Khoshkhah/pbfcut) |
| Browser | no | yes (WebAssembly) |
| OSM fixes | no | yes |
| Filters (tags, ids, a geometry) | yes | no |
