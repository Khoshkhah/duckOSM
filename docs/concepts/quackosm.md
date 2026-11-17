# duckOSM and QuackOSM

[QuackOSM](https://github.com/kraina-ai/quackosm) is a Python library that converts an `.osm.pbf` file into one table of
features (GeoParquet or DuckDB). `duckosm-rust quack` writes the same table, with the same rows, from duckOSM's Rust
engine: with no Python, faster, with less memory, and also in the browser.

## Make the table

```bash
duckosm-rust quack france.osm.pbf -o out --name france -f geoparquet,duckdb
```

| Option | Does | Default |
|---|---|---|
| `-f, --format LIST` | `geoparquet`, `duckdb` (the table `main.quackosm`), or both in one pass | both |
| `--no-sort` | keep the rows in the order in which they are made (QuackOSM `sort_result=False`) | sorted |
| `--threads N` | the threads of the program and of DuckDB (QuackOSM `cpu_limit`) | all the cores |
| `--tile-zoom Z` | make the table tile by tile, with the map tiles of zoom `Z` (zoom 8 for a country) | one run |

The input must be sorted by type and id, as the files of Geofabrik are. In the browser, the
[quack page](https://khoshkhah.github.io/duckosm-studio/quack.html) of duckOSM Studio makes the same table from a file on
your computer; the file does not leave your computer.

## How the approach differs

| | QuackOSM | duckOSM |
|---|---|---|
| Engine | Python; each step is a DuckDB query that writes Parquet files for the next step | Rust; one pass of each step, with no files between the steps |
| Read of the PBF file | DuckDB's `ST_READOSM` | its own decoder, all blocks in parallel, the file read two times |
| The nodes of the ways | joins in DuckDB | buckets by node id on disk, each sorted in memory, so memory does not grow with the area |
| Polygons | GEOS (DuckDB's spatial extension) | the pure-Rust libraries `geo` and `i_overlay` |
| `.duckdb` output | writes the Parquet file first, then loads it | gives the rows to DuckDB's insert directly |
| Large areas | one run | also tile by tile: the cut is the [pbfcut](https://github.com/Khoshkhah/pbfcut) library (one read gives one complete `.osm.pbf` for each map tile), each tile makes its rows alone, and the parts are joined |
| Browser | not possible | the same Rust code as WebAssembly |

Because duckOSM needs neither Python nor GEOS, the same code runs on a computer and in a browser. Tile by tile, the memory
of each part follows the size of a tile, not of the area: in the browser, each tile has its own worker with its own
4 GiB of memory.

## What the table holds

QuackOSM 0.20.0's table with its default options, with three columns: `feature_id` (`node/<id>`, `way/<id>` or
`relation/<id>`), `tags` (`MAP(VARCHAR, VARCHAR)`) and `geometry`.

- A node with tags is a POINT. A way with tags and all its nodes is a LINESTRING, or a POLYGON when it is closed and its
  tags match QuackOSM's polygon rules. A `multipolygon` or `boundary` relation with all its member ways is a POLYGON or a
  MULTIPOLYGON.
- The metadata tags (`source`, `note`, `fixme`, `created_by` and some more) are not in `tags`; an element with no other tag
  is not in the table. The coordinates are rounded to 7 decimals.
- Sorted (the default), each row group of 100 000 rows holds a small area, so a reader with a bounding box can skip the
  other row groups.

The table is for the analysis of OSM features (buildings, land use, points of interest). It is not a road network; for
that, use `duckosm build`.

## Results

A Mac mini with 12 cores and 16 GB; QuackOSM 0.20.0; Geofabrik files (`kentucky-area.osm.pbf`, 141 MB;
`france-latest.osm.pbf`, 5.1 GB). The memory is the peak RSS of all processes of the tool.

| Case | QuackOSM | duckOSM |
|---|---|---|
| Kentucky, GeoParquet, sorted | 13.7 s, 5.9 GB | 4.0 s, 1.9 GB |
| Kentucky, GeoParquet, not sorted | 13.0 s, 5.8 GB | 3.1 s, 1.2 GB |
| France, GeoParquet, not sorted | 569.5 s, 9.8 GB | 129.0 s, 3.8 GB |
| France, GeoParquet, sorted | 998.2 s, 10.6 GB | 150.2 s, 4.5 GB |
| France, `.duckdb`, not sorted | 678.1 s, 7.7 GB (only with a DuckDB memory limit of 8 GB) | 143.8 s, 4.6 GB |
| France in the browser, tile by tile, not sorted | not possible | 294.4 s, 1.2 GB in the page and 5.7 GB in 12 tile workers |

**The rows are the same.** Kentucky: 2 393 139 rows in both, with the same ids and tags; 99.9994 % with an equal
geometry. France: 99 412 535 rows in both, with the same ids and tags; 99.99983 % with an equal geometry. The other rows
have the same area to 1.6e-7: points where two edges cross differ in their last digits.

**In the browser** (headless Chromium on the same Mac mini, zoom 8, 241 tiles): the cut 72.9 s, the rows of the tiles
168.0 s (11.9 of 12 workers busy), the join 44.2 s. The browser storage needs about 8 times the input file (39.8 GB for
France).

## Where duckOSM is not better

- The `.duckdb` files are larger (France 22.3 GB against 19.7 GB): DuckDB 1.5.6 does not compress the geometry, and
  DuckDB 2.0 does.
- The sorted GeoParquet file of France is a little larger: 9.22 GB against 9.05 GB.
- QuackOSM has options that duckOSM does not have: filters (tags, ids, SQL, a geometry), the download of extracts and the
  geocoding of a place name, more than one input file, the tags as columns, the Hilbert sort, WKT output, the file
  settings, its own polygon rules, a cache, and progress bars.

## Compare two tables

The example `quack_diff` compares two tables of this form row by row (GeoParquet, or a `.duckdb` file with
`main.quackosm`); `--large` keeps only a fingerprint of each row, for files of a country:

```bash
cargo run --release --manifest-path rust/Cargo.toml -p duckosm-duckdb --example quack_diff -- out/kentucky_quack.duckdb quackosm.parquet
```

The design and the measurements in full are in
[docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md).
