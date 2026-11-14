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
| `--threads N` | all the threads of the program and of DuckDB (QuackOSM `cpu_limit`) | all the cores |

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

For a large area, `quack_diff --large a.parquet b.parquet` keeps only a fingerprint of each row on disk (the id, a hash of
the tags, a hash of the normalized geometry, `ST_Normalize`) and reads again only the rows with another geometry, for
`ST_Equals`. Its memory is DuckDB's limit of 6 GB.

## The result on Kentucky

The input is `kentucky-area.osm.pbf` (141 MB). The computer is a Mac mini with 12 cores and 16 GB. QuackOSM is 0.20.0 with
DuckDB 2.0.0; duckOSM writes with DuckDB 1.5.6. Each case ran 3 times. The time and the CPU time are from `/usr/bin/time`.
The memory is the peak RSS: of the process and all its child processes for QuackOSM (it starts a worker process), of the
process for duckOSM. A file size is in bytes (MB = 10^6 bytes).

| Case | QuackOSM | duckOSM |
|---|---|---|
| GeoParquet, sorted (the default) | 13.7 s, CPU 77 s, 5.9 GB, 311.5 MB | 4.0 s, CPU 16.8 s, 1.9 GB, 308.4 MB |
| GeoParquet, not sorted | 13.0 to 13.5 s, CPU 71.5 s, 5.8 GB, 349.3 MB | 3.1 to 3.6 s, CPU 13.2 s, 1.2 GB, 320.7 MB |
| `.duckdb`, sorted (the default) | 14.8 to 15.3 s, CPU 81 to 85 s, 5.8 GB, 670 MB | 5.2 to 6.2 s, CPU 17.1 s, 2.2 GB, 779 to 780 MB |
| `.duckdb`, not sorted | 14.2 to 14.4 s, 5.8 GB, 702 to 739 MB | 4.1 to 4.6 s, CPU 13.4 s, 1.7 GB, 705 MB |
| In the browser (Chromium, 12 threads) | not possible | 12.5 s sorted, 7.1 s not sorted |

The browser row is the page of duckOSM Studio, which uses the duckOSM of PR 157 (the rows in memory, before the design
for large areas).

The rows are the same:

- Each table has 2 393 139 rows: 556 759 nodes, 1 829 301 ways and 7 079 relations, with the same ids.
- All rows have the same tags. A sum of a hash over all rows gives the same number for the ids and for the tags in the
  two tools.
- 2 393 125 rows (99.9994 %) have an equal geometry. The other 14 rows have the same area to 8e-8 of it. In 12 of them, a
  point where two edges cross differs in its last digits. In the other 2, QuackOSM also keeps lines with no area (a
  GEOMETRYCOLLECTION), and duckOSM keeps only the polygon.
- In the sorted files, 99.93 % of the rows are at the same position, and all rows are in the same row group.

On Monaco, all 9 586 rows are equal.

## The result on France

The input is the Geofabrik file `france-latest.osm.pbf` (5.1 GB). The computer and the versions are the ones above. The
case is the GeoParquet output, not sorted: the first step of every pipeline of QuackOSM (its sort and its `.duckdb`
output come after it). One run of each tool, with nothing else on the computer. Every 2 s, a script took the RSS of the
process tree and the free disk; the peak disk use is the free disk at the start minus the lowest free disk, so it counts
the temporary files and the output.

| France, GeoParquet, not sorted | QuackOSM | duckOSM |
|---|---|---|
| Time | 569.5 s | 129.0 s |
| CPU time | 3 379 s | 356 s |
| Peak memory (RSS) | 9.8 GB | 3.8 GB |
| Peak memory footprint of macOS (main process) | 14.6 GB | 3.5 GB |
| Peak disk use | 39.5 GB | 23.2 GB |
| Output | 11.62 GB | 10.29 GB |

The rows, compared with `quack_diff --large` (2 x 99 million rows in 72 s):

- Each table has 99 412 535 rows: 25 925 027 nodes, 73 201 509 ways and 285 999 relations, with the same ids.
- All rows have the same tags.
- 99 412 366 rows (99.99983 %) have an equal geometry. The other 169 rows (165 relations and 3 ways) have the same area
  to 1.6e-7 of it: points where two edges cross differ in their last digits, and one GEOMETRYCOLLECTION of QuackOSM.

The `.duckdb` output, not sorted, with the same method:

| France, `.duckdb`, not sorted | QuackOSM | duckOSM |
|---|---|---|
| Time | 678.1 s | 143.8 s |
| CPU time | 3 748 s | 370 s |
| Peak memory (RSS) | 7.7 GB | 4.6 GB |
| Peak memory footprint of macOS (main process) | 13.9 GB | 15.9 GB |
| Peak disk use | 43.6 GB | 31.7 GB |
| Output | 19.7 GB | 22.3 GB |

Both files have the same 99 412 535 rows of each kind. QuackOSM needed a DuckDB memory limit to finish on this computer
(`duckdb_conn_kwargs={"config_kwargs": {"memory_limit": "8GB"}}`). With its defaults, two runs failed: one at 592 s,
when DuckDB could not read the Parquet file of the step before (`TProtocolException: Invalid data`), and one at 888 s,
when macOS stopped the process. The footprint of duckOSM is larger because the DuckDB buffer pool keeps the table pages
until the end; its RSS is lower. The file of duckOSM is larger because DuckDB 1.5.6 does not compress the geometry
(DuckDB 2.0 compresses 46 million of the 99 million geometries in QuackOSM's file).

The sorted GeoParquet output (the default of both tools), with the same method:

| France, GeoParquet, sorted | QuackOSM | duckOSM |
|---|---|---|
| Time | 998.2 s | 150.2 s |
| CPU time | 4 694 s | 488 s |
| Peak memory (RSS) | 10.6 GB | 4.5 GB |
| Peak memory footprint of macOS (main process) | 14.5 GB | 3.8 GB |
| Peak disk use | 40.0 GB | 23.5 GB |
| Output | 9.05 GB | 9.22 GB |

duckOSM keeps the rows of the sort on disk, compressed, so the sort does not add disk use to the case that is not
sorted; it adds 21 s.

## The results that are not in favor of duckOSM

- The `.duckdb` files are larger: on Kentucky 779 MB against 670 MB sorted, on France 22.3 GB against 19.7 GB. DuckDB 1.5.6 stores the geometry with no
  compression, and DuckDB 2.0 compresses it. The same data loaded from duckOSM's Parquet file into DuckDB 1.5.6 is also
  779 MB.
- The sorted GeoParquet file of France is 9.22 GB, against 9.05 GB of QuackOSM; on Kentucky the sorted output needs 1.9
  GB, against 1.2 GB not sorted (the rows of the sort wait in a buffer of 512 MB).
- In the browser, Kentucky uses 3.6 to 3.8 GiB of WebAssembly memory (the limit is 4 GiB), because the GeoParquet file
  stays in memory until the download.

## Why duckOSM is faster

QuackOSM runs its steps as DuckDB queries that write Parquet files between the steps, and it reads the PBF file with
DuckDB's `ST_READOSM`. Each step reads the files of the step before it. For the `.duckdb` output, it writes the whole
Parquet file first and then loads it.

duckOSM reads the file twice and keeps each table in temporary files, not in memory, so its memory does not grow with the
area:

1. Its own PBF decoder reads the blocks in parallel. The ways and the relations go to temporary files; the nodes of each
   way go into about 100 buckets by node id.
2. On the second read, the nodes come in the order of their ids. The buckets come in the same order, each sorted in
   memory on all threads, so each node gives its point to the ways that need it. A node with tags becomes a row at once.
3. The points, in buckets by way and sorted in memory, make the ways. A way becomes a row at once.
4. The relations come from a join of their member ways on disk, and they are made on all threads, one at a time.
5. The rows go to the writer while the file is read. For the `.duckdb` output, a table function in Rust gives the rows to
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
