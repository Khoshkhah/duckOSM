# The load step in Rust: `read_osm` and a fast PBF decoder

Status: 2026-11-11. A whole-file build of `duckosm-rust` (way D) makes `raw.*` with `read_osm`. `duckosm build`
(Python) keeps `ST_READOSM`: its wheel has no DuckDB inside, so it cannot load a function of the Rust module. The
browser (way C) is a later stage: see "The stages".

## Summary

The load step makes duckOSM's `raw.nodes`, `raw.ways` and `raw.relations` from a `.osm.pbf` file. Before this work,
DuckDB's `ST_READOSM` (C++, the spatial extension) did it. This work gives the load step in Rust, in two parts:

1. **A fast PBF decoder** (`duckosm-core`, `pbfraw.rs`). It reads the protobuf bytes in place. It is 5 times faster
   than the `osmpbf` crate, and faster than the two C++ readers (`ST_READOSM` and libosmium) for this job.
2. **A DuckDB table function `read_osm(path, kind)`** (`duckosm-duckdb`, `read_osm.rs`). DuckDB calls it from all
   its threads and writes the rows into its own tables, as it does with `ST_READOSM`.

France (5.1 GB file), all of `raw.*`, in the same DuckDB: **`read_osm` uses about 30% less CPU time than
`ST_READOSM`**, with the same rows and the same memory.

The work also measured four other designs. They were slower, and this note keeps their numbers, so that nobody tries
them again without a new reason.

## How a dashboard holds its data: ways C and D

roadstyle's guide "How a dashboard holds its data" names four ways to run a dashboard. Two of them build data:

| Way | duckOSM program | Where DuckDB is | Load step |
|---|---|---|---|
| C. Made in the browser (online) | duckOSM Studio | DuckDB-Wasm, a separate WebAssembly module | Rust writes Parquet files to the browser disk (OPFS); DuckDB-Wasm reads them |
| D. Made by a local program (offline) | `duckosm-rust studio` | built into the Rust program (`duckdb` crate) | `read_osm` in the program's DuckDB |

In way C, the Rust module and DuckDB-Wasm have separate memories, and they can only share files. So the browser
needs the Parquet hand-over. In way D, DuckDB is in the same program, so the rows go into DuckDB directly.

## The test data and the computer

| Item | Value |
|---|---|
| File | `france-latest.osm.pbf` of Geofabrik, 2026-11-11, 5,112,749,993 bytes, sorted by type (`Sort.Type_then_ID`) |
| Content | 529,233,838 nodes, 73,692,922 ways, 1,106,136 relations, 75,506 data blocks |
| Computer | WSL2, 20 threads, 23 GB of RAM, 24 GB of swap |
| DuckDB | 1.5.6 (the `duckdb` crate 1.10506.1, built in) for the comparisons of `read_osm` and `ST_READOSM`; 1.5.3 (Python) for the earlier designs |
| Settings | `memory_limit` 4 GB, `threads` 20, `preserve_insertion_order` false, unless a table says other values |

**Other sessions used the computer during many measurements** (load average 9 to 21). The wall times change by up
to 2 times from one run to the next. The CPU time (user seconds) changes less, so the tables give both. A run that
comes first can also be slower, because the file is not yet in the file cache of Linux. The comparisons run in both
orders for this reason.

## The PBF decoder

### Why `osmpbf` was slow

The way blocks of France (9,214 blocks), 20 threads, nothing stored:

| Step | Time |
|---|---|
| Read the blocks from the file | 0.1 s |
| Inflate the zlib data only | 0.7 s |
| `osmpbf`: inflate and decode (`to_primitiveblock`) | 6.2 s |
| `osmpbf`: and visit each tag and each node id | 6.7 s |

So almost all of the time is the protobuf decode of `osmpbf`. It uses `rust-protobuf`, which makes owned objects: new
vectors for the keys, the values and the node ids of each way (more than 200 million allocations for France), and it
also decodes the metadata (`Info`: version, time, user) that `raw.*` does not use. A change of the zlib backend
(`zlib-rs` in place of `miniz_oxide`) did not change the time, because the inflate is only 0.7 s.

### The new decoder

`pbfraw.rs` reads the bytes of a block in place, as `ST_READOSM` (protozero) does:

- It checks the string table as UTF-8 one time for each block, and then it uses `&str` slices of it.
- It reads the keys, the values and the node ids into buffers that it uses again for each element.
- It skips the metadata and the groups of the kinds that the caller does not want.
- It gives the coordinates with the formula of `osmpbf`, `1e-9 * (offset + granularity * value)`.
- It supports raw and zlib blocks. A block with lzma, lz4 or zstd gives an error.

The test `pbfraw::tests::equals_osmpbf` compares it with `osmpbf` on Monaco: the same elements, the same tags and the
same coordinates, to the bit.

| The way blocks of France | `osmpbf` | `pbfraw` |
|---|---|---|
| Decode and visit each tag and node id | 6.7 s | **1.3 s** |
| Decode and build the Arrow columns | 9.0 s | **2.3 s** |

## The table function `read_osm`

```sql
CREATE TABLE raw.nodes AS SELECT * FROM read_osm('france.osm.pbf', 'node');
CREATE TABLE raw.ways AS SELECT * FROM read_osm('france.osm.pbf', 'way');
CREATE TABLE raw.relations AS
  SELECT osm_id, tags, refs, ref_roles, CAST(ref_types AS ENUM('node', 'way', 'relation')[]) AS ref_types
  FROM read_osm('france.osm.pbf', 'relation');
```

`read_osm::raw_sql(path)` gives these statements. The columns and the types are the same as duckOSM's `raw.*` from
`ST_READOSM`: `ref_types` is text from the function and DuckDB casts it to the ENUM list.

How it works:

1. **A block index.** `pbf::block_index` reads the file one time and skips the data of each block: it notes where
   each block starts and how long it is, and if the header says `Sort.Type_then_ID`.
2. **Only the blocks of the kind.** In a sorted file, `pbf::kind_range` does a binary search: it decodes some blocks
   to find where the ways and the relations start. The pass for the ways of France reads 9,214 blocks of the 75,506
   (measured). A file that is not sorted gives all blocks to each pass.
3. **A parallel read.** DuckDB calls the function from all its threads (`set_max_threads`). Each call takes the
   next block number from a shared counter and reads the block at its place in the file (`read_at`), with no lock.
4. **The copy into DuckDB.** The call decodes the block with `pbfraw` into Arrow columns, and copies up to one
   vector (2,048 rows) into DuckDB's vectors: the ids and the coordinates as flat copies, the tags as a `MAP`, the
   node ids and the members as lists. The rest of the block waits in a queue for the next call.
5. **DuckDB writes.** DuckDB's own insert writes the table. So the memory is DuckDB's `memory_limit`, the extra
   memory of DuckDB's threads, and one block for each thread in Rust.

`read_osm` is only for a program with DuckDB built in (way D). DuckDB-Wasm cannot load a function of our Rust module.

## The comparison with the C++ readers

To measure again: `cargo run --release -p duckosm-duckdb --example read_osm_compare -- <file.osm.pbf> <folder> <rust|st> [memory_limit] [threads]`
makes `raw.*` in `<folder>/<rust|st>.duckdb` and prints the time of each table. Run both in both orders.

### Decode only

All 75,506 blocks of France, read and decoded, nothing stored:

| Reader | Language | Wall time | CPU time (user) | Peak RSS |
|---|---|---|---|---|
| **`pbfraw`** (this work) | Rust | **3.5 to 4.7 s** | **57 s** | 73 MB |
| `osmium fileinfo -e --no-crc` (libosmium 2.18) | C++ | 8.7 to 10.0 s | 80 to 85 s | 264 MB |
| `osmium fileinfo -e` (with the CRC32 of the data) | C++ | 13.8 to 14.5 s | 96 to 98 s | 256 MB |

The comparison is of the same result for `raw.*`, not of the same work. `osmium` is a general tool: it decodes the
metadata and makes full objects. `pbfraw` does only what `raw.*` needs.

### The ways only, in DuckDB, no table

`SELECT count(*), sum(len(refs)), sum(cardinality(tags)) FROM ...`, so that both read every column of the 73.7
million ways:

| Function | Language | Wall time | CPU time (user) |
|---|---|---|---|
| **`read_osm(path, 'way')`** | Rust | **3.8 to 4.2 s** | **58 to 62 s** |
| `ST_READOSM(path) WHERE kind = 'way'` | C++ | 5.0 to 9.4 s | 85 to 92 s |

A probe that skipped one column showed the cost of the copy into DuckDB: the tags about 14 CPU seconds, the node ids
about 6.

### All of `raw.*`, in DuckDB

| Run (order) | Function | nodes | ways | relations | **Total** | CPU (user) | Peak RSS |
|---|---|---|---|---|---|---|---|
| 1 | `read_osm` (Rust) | 32.0 s | 31.4 s | 5.8 s | **69.2 s** | 338 s | 6.0 GB |
| 2 | `ST_READOSM` (C++) | 53.2 s | 24.3 s | 12.9 s | **90.4 s** | 462 s | 6.0 GB |
| 3 | `read_osm` (Rust) | 22.3 s | 15.7 s | 5.0 s | **42.9 s** | 325 s | 6.1 GB |
| 4 | `ST_READOSM` (C++) | 38.2 s | 44.0 s | 11.2 s | **93.5 s** | 466 s | 6.1 GB |

On a quieter computer earlier in the day, `ST_READOSM` took 39.2 s and 42.9 s for the same work. So the wall times
of the table are not a fair answer alone. The CPU time is: **`read_osm` does the same work with about 30% less CPU.**
The peak memory is the same, because DuckDB's limit and its threads set it.

The rows are equal: on Monaco, the three tables of `read_osm` and of `ST_READOSM` have the same column types and the
same rows (`EXCEPT ALL` in both directions).

## The designs that were slower

France, all of `raw.*`. The wall times are from runs at different load of the computer, so they show the size of the
difference, not exact values.

| Design | Total | Peak memory | Why it was slower |
|---|---|---|---|
| `ST_READOSM` (the old load, before this work) | 65 s | 4.5 GB | (the reference) |
| Rust writes Parquet (one writer, zstd), DuckDB copies the files | 274 s | Rust 0.5 GB, DuckDB 3.1 GB | One thread compresses all the files |
| The same, with DELTA_BINARY_PACKED and BYTE_STREAM_SPLIT | slower | | The encoders cost more for Rust and for DuckDB (nodes copy: 116 s, not 47 s) |
| The same, with no compression | 307 s | DuckDB 3.3 GB | The writer is still one thread |
| Parquet shards (one writer for each thread), no compression | 186 s | Rust 0.7 GB, DuckDB 3.3 GB | 22 GB written and read again |
| The same, with LZ4 | 191 s | DuckDB 3.6 GB | Less to write, but a slower copy |
| Rust appends Arrow batches to DuckDB (one connection for each thread) | 236 s (1 GB budget), 563 s (4 GB) | 2.0 to 3.3 GB | The conversion of each batch in Rust, and many commits on the same tables |
| An Arrow stream from Python into DuckDB (only the nodes) | 108 s for the nodes | 3.7 GB | DuckDB reads one stream on one thread |
| `read_osm` with `osmpbf` and one reader lock | 68 s | 6.2 GB | One thread read the file, and each pass decoded all blocks |

The lesson: the slow part was never the language. It was the way that the rows went into DuckDB (files, conversions
or one thread), and the decode of `osmpbf`. When DuckDB pulls the rows from a table function and the decode reads the
bytes in place, Rust is faster than the C++ readers for this job.

## The Parquet load for the browser (way C)

`duckosm-parquet` has the load that writes Parquet files in a memory budget, for the browser:

- `pbf::stream_raw` reads the blocks in rounds and decodes a round in parallel with `pbfraw`.
- `load_parts` chooses the blocks of a round so that they stay in the budget. The cost of a block is its size in the
  file times `1 + 4 × ratio`, where `ratio` is the largest ratio of the Arrow bytes of a part to the size of its block
  (Monaco: 11.9).
- `load_raw` gives each thread its own shard of three files, `<table>_<shard>.parquet`. A row group closes when it
  holds its share of the budget.

`rust/duckosm-parquet/tests/load_memory.rs` counts the heap with a counting allocator, on Monaco 20 times in a row
(97 MiB of rows). The test fails if the peak goes above the budget:

| Budget | Threads | Peak |
|---|---|---|
| 16 MiB | 1 | 5.9 MiB |
| 16 MiB | 8 | 6.7 MiB |
| 32 MiB | 20 | 10.9 MiB |
| 64 MiB | 20 | 19.8 MiB |

France with a 1 GB budget: the Rust part used 0.7 GB at most. In the browser, DuckDB-Wasm must read the files from
OPFS with `registerFileHandle`, not `registerFileBuffer` (which copies each file into its memory), and it does not copy
them into tables: a query reads the files. A copy into a `.duckdb` file is only for the export.

## The settings

| Setting | Effect |
|---|---|
| `options.threads` | DuckDB's threads. In `duckosm-rust`, also the threads of `read_osm` (DuckDB calls it from its threads). Not set: all cores. |
| `options.memory_limit` | DuckDB's `memory_limit`. In `duckosm-rust`, the setting is read for the load of `raw.*` (before this work, the program ignored it). Not set: DuckDB's default, 80% of the RAM. |

The Parquet load (way C) takes its budget as an argument; the page sets it when stage 3 connects it.

DuckDB uses more memory than its limit: about 0.2 to 1.2 GB for its threads (France, 4 to 20 threads, measured with
`ST_READOSM` and with `read_osm`). Fewer threads make this smaller.

## The stages

| Stage | Work | Check |
|---|---|---|
| 1 (done) | `pbfraw`, `read_osm`, the Parquet load with its budget, the measurements. | `pbfraw::tests::equals_osmpbf`, `duckosm-parquet/tests/load_memory.rs` |
| 2 (done) | Way D: a whole-file build of `duckosm_build::build` makes `raw.*` with `read_osm`. The Python `duckosm build` keeps `ST_READOSM`. | `duckosm-duckdb/tests/read_osm.rs` compares `read_osm` with `ST_READOSM` on Monaco; the golden tests |
| 2b | Way D with a boundary: the cut in rounds (the nodes inside the boundary on disk), then `read_osm` on the cut. Today an area still gets `raw.*` from the cut in memory. | The golden tests |
| 3 | Way C: Studio writes the Parquet files to OPFS and DuckDB-Wasm reads them with `registerFileHandle`. | The same tests in the browser build, and a phone check |
| 4 | The network step uses `pbfraw` in place of `osmpbf`, and reads in blocks. | The golden tests |
