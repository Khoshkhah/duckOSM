# The load step in a memory budget

Status: done for the offline build (`options.engine: rust`), 2026-11-11. The browser (duckOSM Studio) is stage 2, below.

## The problem

The load step makes `raw.nodes`, `raw.ways` and `raw.relations` from the `.osm.pbf` file. Before this change, the two
versions did it in two ways, and neither had a memory limit that we control:

| Version | How | Memory |
|---|---|---|
| Offline (`duckosm build`) | DuckDB `ST_READOSM` from Python. It reads the file three times (one time for each table). | DuckDB fills its `memory_limit` and adds more for each thread. France (5.1 GB file) with `memory_limit` 4 GB: 4.5 GB with 20 threads (measured). |
| Online (Studio) | The Rust cut keeps all of `raw.*` of the area in memory (`Raw`), then writes the Parquet files. | It grows with the area. |

Measured on 2026-11-11 for France with `memory_limit` 4 GB: DuckDB counted 3.8 GB in its buffer, and the process had
0.2 to 1.2 GB more (more with more threads). Only the scan (no table) used 283 MiB. So the memory is in the tables
that are made, not in the reader.

## The design

The Rust engine reads the file and writes the three tables as Parquet while it reads. The memory does not grow with
the size of the file. One setting, the **memory budget**, sets the memory of the step.

```text
.osm.pbf ──> blocks, in rounds ──> each block decoded on its own thread into Arrow columns (a "part")
                                         │
                                         └──> in the order of the file ──> raw_nodes / raw_ways / raw_relations.parquet
                                                                           (a row group closes at its share of the budget)
```

| Part | Where | What it does |
|---|---|---|
| `RawSink`, `pbf::raw_block`, `pbf::stream_raw` | `duckosm-core` | Decode one block and give each element to a sink. Read the blocks in rounds: the caller says, for each block, if it fits in the round. |
| `RawColumns`, `RawWriter`, `load_raw` | `duckosm-parquet` (feature `pbf`) | Build the Arrow columns of a block (a tag costs its bytes and two offsets, not two allocations). Write the parts. Choose the rounds from the budget. |
| `load_raw` | `duckosm-build` | The step on a file: the threads and the three files in a folder. |
| `duckosm._rust.load_raw(pbf, out, memory=...)` | `duckosm-py` | The Python binding. |
| `rust_engine.load_raw` | `src/duckosm/rust_engine.py` | Calls the binding, and DuckDB reads the three files into `raw.*`. `_load_pbf` uses it when `options.engine` is `rust`. |

### How the budget is shared

| Share | Size | How it is kept |
|---|---|---|
| The blocks in work | 50% of the budget | A round takes the next block only if the cost of the round stays in this share. |
| The open row groups | 50% of the budget: 1/12 of the budget for each of the three tables, and the rest as margin for the encoder | A table closes its row group when `ArrowWriter::memory_size` reaches its share, or at 65,536 rows. |

The cost of one block in work is `size × (1 + 4 × ratio)`:

- `size` is the size of the block in the file (compressed). `osmpbf` reads each block with `take`, so a reader that
  counts its bytes gives the size before the block is decoded.
- `ratio` is the largest ratio of the Arrow bytes of a part to the size of its block, so far in the file. Monaco: 11.9.
- `4` (`BLOCK_COST`) is the memory of a block while it is decoded, as a multiple of its part: the decompressed block
  and the builders, which grow by doubling. Measured: 3.2 for Monaco (4.9 MB for a part of 1.5 MB).

The first round has one block, because the ratio is not known yet. A round always takes one block, so **the budget
cannot be less than one block**. A file of Geofabrik has blocks of 8,000 elements: about 5 MB in work for the largest
block of Monaco. A round has at most 2 blocks for each thread.

### The same rows as ST_READOSM

`raw.*` has the rows and the column types of `ST_READOSM`:

- `raw.nodes`: `osm_id`, `lat`, `lon`, `tags`. All nodes.
- `raw.ways`: `osm_id`, `tags`, `refs`. The ways with 2 or more nodes. No `line` column (the Studio files have one,
  because `features.sql` reads it; it needs the coordinates of all nodes, so it is not in this step).
- `raw.relations`: `osm_id`, `tags`, `refs`, `ref_roles`, `ref_types`. All relations. DuckDB casts `ref_types` to
  `ENUM('node', 'way', 'relation')[]`.
- No tags: `tags` is NULL. A member with no role: the role is NULL.

`tests/test_rust_load.py` compares the column types and all rows (`EXCEPT ALL` in both directions) on Monaco, with a
budget of 16 MB.

### The check of the budget

`rust/duckosm-parquet/tests/load_memory.rs` counts the heap with a counting allocator. The input is Monaco 20 times
in a row (a `.osm.pbf` file is a list of blocks, so this is a valid file): 97 MiB of parts, which is more than every
budget of the test. The test fails if the peak goes above the budget. Measured:

| Budget | Threads | Peak | Blocks at most in one round |
|---|---|---|---|
| 16 MiB | 1 | 5.7 MiB | 2 |
| 16 MiB | 8 | 7.4 MiB | 5 |
| 32 MiB | 20 | 11.3 MiB | 12 |
| 64 MiB | 20 | 21.7 MiB | 21 |

The test counts the memory of the Rust code exactly. It does not count the memory of DuckDB when DuckDB reads the
files into `raw.*`. DuckDB has its own `memory_limit`, and the build sets it from the same setting.

## The setting

`options.memory_limit` (for example `"2GB"`) is the budget of the load step and DuckDB's `memory_limit`. The two run
one after the other, so each one can use all of it. With no value, the load step uses 1 GB, and DuckDB uses its
default (80% of the RAM).

## France

TO BE MEASURED.

## What this does not change

- **The cut to a boundary.** A build with a boundary still cuts the file with osmium first (`pbf/` cache), then loads
  the cut file. The Rust cut (`pbf::cut`) keeps the area in memory.
- **The network step.** It reads the file again by itself and keeps the network in memory (France: more than 26 GB,
  measured on 2026-11-11). That is the next step of this work.
- **The browser.** Stage 2.

## The stages

| Stage | Work | Check |
|---|---|---|
| 1 (done) | The load step in Rust with a budget, for the offline build. | `test_rust_load.py` (equal to `ST_READOSM`), `load_memory.rs` (the budget). |
| 2 | Studio: the load step writes the files to OPFS, and DuckDB-Wasm reads them. The cut to the area uses the same rounds: the nodes inside the boundary go to a sorted file, not to a set in memory. | The same tests in the browser build, and a phone check. |
| 3 | The network step reads `raw.*` in blocks (by tile), not the whole file into memory. | The golden tests: every row and every `edge_id` stays equal. |
