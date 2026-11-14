# duckOSM and QuackOSM

[QuackOSM](https://github.com/kraina-ai/quackosm) is a Python library that converts an `.osm.pbf` file into one table of
features, in GeoParquet or in DuckDB. `duckosm-rust quack` writes the same table, with the same rows, from duckOSM's Rust
engine. So you can use QuackOSM's table without Python, also in the browser, and you can compare the two tools on the same
output.

## Make the table

```bash
duckosm-rust quack kentucky.osm.pbf -o out --name kentucky -f duckdb,geoparquet
```

| Option | Does | Default |
|---|---|---|
| `-o, --out DIR` | the output folder | `.` |
| `--name NAME` | the files are `DIR/NAME_quack.parquet` and `DIR/NAME_quack.duckdb` | `area` |
| `-f, --format LIST` | `geoparquet`, `duckdb` (the table `main.quackosm`), or both in one pass | both |
| `--no-sort` | keep the rows in the order in which they are made (QuackOSM `sort_result=False`) | sorted |
| `--threads N` | the threads of the program and of DuckDB (QuackOSM `cpu_limit`) | all the cores |

The input must be sorted by type and id, as the files of Geofabrik are. To make the table of an area, cut the region file
first with `duckosm-rust cut`.

In the browser, the page `quack.html` of duckOSM Studio makes the same table from a file on your computer and gives the
GeoParquet file to download: <https://khoshkhah.github.io/duckosm-studio/quack.html>. The file does not leave your
computer.

## What the table holds

The table is QuackOSM 0.20.0's table with its default options. It has three columns: `feature_id` (`node/<id>`,
`way/<id>` or `relation/<id>`), `tags` (`MAP(VARCHAR, VARCHAR)`) and `geometry`.

- A node with tags is a POINT.
- A way with tags, with 2 or more nodes and all its nodes in the file, is a LINESTRING. It is a POLYGON when it is closed and
  its tags match QuackOSM's polygon rules (for example `building`, `landuse`, `area=yes`; `area=no` is always a line).
- A relation of the type `multipolygon` or `boundary` with all its member ways in the file is a POLYGON or a MULTIPOLYGON.
  A relation with a ring that is not closed is not in the table.
- The metadata tags (`source`, `note`, `fixme`, `created_by`, `ele`, `area` and some more) are not in `tags`. An element
  with no other tag is not in the table.
- The coordinates are rounded to 7 decimals.
- By default, the rows are sorted as QuackOSM sorts them: in vertical strips of the area, so that each row group of 100 000
  rows holds the rows of a small area. A reader with a bounding box can then skip the other row groups.

The table is for analysis of OSM features: buildings, land use, points of interest. It is not a road network: it has no
node lists and no topology. For a routable network, use `duckosm build`.

## Compare the two tools

The example `quack_diff` of the crate `duckosm-duckdb` compares two tables of this form, row by row. Each side is a
GeoParquet file or a `.duckdb` file with the table `main.quackosm`:

```bash
cargo run --release --manifest-path rust/Cargo.toml -p duckosm-duckdb --example quack_diff -- out/kentucky_quack.duckdb quackosm.parquet
```

It shows the rows of each kind, the ids that are in one table only, the rows with other tags, and the rows with another
geometry (`ST_Equals` is false), with examples.

## The result on Kentucky

The input is `kentucky-area.osm.pbf` (141 MB). The computer is a Mac mini with 12 cores and 16 GB. QuackOSM is 0.20.0 with
DuckDB 2.0.0; duckOSM writes with DuckDB 1.5.6. Each case ran 3 times. The memory is the peak of the process and all its
child processes. A file size is in bytes (MB = 10^6 bytes).

| Case | QuackOSM | duckOSM |
|---|---|---|
| GeoParquet, sorted (the default) | 13.7 s, CPU 77 s, 5.9 GB, 311.5 MB | 3.4 s, CPU 13 s, 2.1 GB, 308.4 MB |
| GeoParquet, not sorted | 13.0 to 13.5 s, CPU 71.5 s, 5.8 GB, 349.3 MB | 3.0 s, CPU 11.6 s, 1.1 GB, 320.1 MB |
| `.duckdb`, sorted (the default) | 14.8 to 15.3 s, CPU 81 to 85 s, 5.8 GB, 670 MB | 4.2 s, CPU 13.5 s, 2.8 GB, 779 MB |
| `.duckdb`, not sorted | 14.2 to 14.4 s, 5.8 GB, 702 to 739 MB | 3.4 s, 1.8 GB, 704 to 709 MB |
| GeoParquet, sorted, 1 thread | 41.7 s | 8.3 s |
| GeoParquet, sorted, 4 threads | 18.2 s | 4.0 s |
| In the browser (Chromium, 12 threads) | not possible | 12.5 s sorted, 7.1 s not sorted |

The rows are the same:

- Each table has 2 393 139 rows: 556 759 nodes, 1 829 301 ways and 7 079 relations, with the same ids.
- All rows have the same tags. A sum of a hash over all rows gives the same number for the ids and for the tags in the
  two tools.
- 2 393 125 rows (99.9994 %) have an equal geometry. The other 14 rows have the same area to 8e-8 of it. In 12 of them, a
  point where two edges cross differs in its last digits. In the other 2, QuackOSM also keeps lines with no area (a
  GEOMETRYCOLLECTION), and duckOSM keeps only the polygon.
- In the sorted files, 99.93 % of the rows are at the same position, and all rows are in the same row group.

On Monaco, all 9 586 rows are equal.

Two results are not in favor of duckOSM:

- The sorted `.duckdb` file is larger: 779 MB against 670 MB. DuckDB 1.5.6 stores the geometry with no compression, and
  DuckDB 2.0 compresses it. The same data loaded from duckOSM's Parquet file into DuckDB 1.5.6 is also 779 MB.
- In the browser, Kentucky uses 3.6 to 3.8 GiB of WebAssembly memory (the limit is 4 GiB), because the GeoParquet file
  stays in memory until the download.

## Why duckOSM is faster

QuackOSM runs its steps as DuckDB queries that write Parquet files between the steps, and it reads the PBF file with
DuckDB's `ST_READOSM`. Each step reads the files of the step before it. For the `.duckdb` output, it writes the whole
Parquet file first and then loads it.

duckOSM makes the rows in one pass over the file:

1. Its own PBF decoder reads the blocks in parallel. A node keeps only its rounded point, and a node with tags becomes a
   row at once.
2. A way becomes a row as soon as it is read, because its nodes came before it in the file.
3. The relations come at the end, in parallel.
4. The rows go to the writer while the file is read. For the `.duckdb` output, a table function in Rust gives the rows to
   DuckDB's insert, with no Parquet file between.

The polygon operations (the ring check, the repair of an invalid ring, the union and the difference) use the pure-Rust
libraries `geo` and `i_overlay`. QuackOSM uses GEOS through DuckDB's spatial extension. Because duckOSM needs neither
Python nor GEOS, the same code builds for WebAssembly, and the table can be made in a browser.

## What QuackOSM has and duckOSM does not have

- The filters: tags (`tags_filter`), OSM ids, custom SQL, and a geometry (`geometry_filter`).
- The download of an extract for a geometry, and the geocoding of a place name.
- More than one PBF file in one call.
- The tags as columns (`explode_tags`), the Hilbert sort, and the WKT output.

`duckosm-rust quack` makes the table of QuackOSM's default options, with or without the sort. The design and the
measurements in full are in
[docs/rust/quack.md](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/quack.md).
