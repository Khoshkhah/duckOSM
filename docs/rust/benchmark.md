# Benchmark: network step, duckOSM and duckosm-rust

Date: 2026-10-21. **Only Monaco is tested.** Large areas are not tested yet.

A small area does not predict the speed on a large area. Monaco has 0.7 MB of data. On such a file, the
start-up of the program and the fixed steps are a large part of the time, and the cost of the real work is
small. Do not use the ratios below as a speed-up for a city or a country.

Machine: Intel Core i5-13600K (20 threads), 23 GB memory, WSL2 (Linux 6.6.87.2), warm file cache.

| Program | Version |
|---|---|
| duckOSM (Python) | source at git `b857caf` (`pyproject.toml` says 0.6.0; the installed metadata says 0.5.0), DuckDB 1.5.3 |
| duckosm-rust | this repository at commit `bd7e158` (later commits change only the default `--format` and docs), `cargo build --release`, `lto = "thin"` |
| Rust | rustc 1.99.1 |
| DuckDB inside duckosm-rust | 1.5.6 (crate `duckdb` 1.10506.1, `bundled`) |

## What is compared

The **network step of the driving mode**: PBF in, driving edges and nodes out. Input: the file
`duckOSM/data/sample/monaco.osm.pbf` (41,709 nodes, 6,249 ways), the file that built `monaco.duckdb`.
The settings are the same for both programs:

- one mode (driving), no boundary (so no `osmium` cut and no component filter), global junctions on,
  `merge_segments` off (`--no-merge-segments`, and `--no-speeds` since duckosm-rust has the speed step; the benchmark was measured before it had the merge or the speeds), no OSM fixes;
- duckOSM: the config `bench/python_network_step.yaml`, with every other step switched off (graph of
  legal turns, H3, speeds, costs, turn restrictions, features, bus routes, drawing order);
- duckosm-rust: `duckosm-rust monaco.osm.pbf -o <dir> --name monaco --no-component-filter --format duckdb`
  and `--format geoparquet`.

**The results are equal.** Only the three tables that both programs write are compared: `driving.edges`,
`driving.private_edges` and `driving.nodes`. The other tables of duckOSM (`edge_graph`, `lane_profile`,
`ways`, `raw.*`, and so on) are not written by duckosm-rust, so they are not compared.

| Table | rows (duckOSM = duckosm-rust) | rows that differ, in any column |
|---|---|---|
| `driving.edges` | 3,092 | 0 |
| `driving.private_edges` | 724 | 0 |
| `driving.nodes` | 1,904 | 0 |

`tools/compare_with_duckosm.py <duckOSM db> <duckosm-rust output>` joins on `edge_id` (nodes on
`node_id`) and compares every column. The geometry is compared as exact bytes (WKB), `length_m` as the
exact 32-bit float. It was run on the `.duckdb` file and on the GeoParquet files of duckosm-rust.

There is a second check with a smaller, cut file and a build with the component filter on
(2,765 edges, 79 dropped), and the golden test `cargo test` (see [development.md](development.md)).
Against `monaco.duckdb` of 2026-10-21, 78 edges differ in `lanes` and nothing else. These are the OSM
fixes (`osm_overrides.yaml` sets the lanes of Boulevard du Larvotto, Avenue Albert II and others).
duckosm-rust has no fixes yet.

## Method

`bench/run_bench.py --pbf <file> --label monaco --runs 10`. For each command: one untimed warm-up run,
then 10 timed runs under `/usr/bin/time -v`. The tables show the minimum and the median of the wall time,
and the largest peak memory (maximum resident set size).

The start-up and the real work are shown apart:

1. **Start-up only.** The program starts, prints its version and stops: `duckosm --version` and
   `duckosm-rust --version`. This is the time to load the program and its libraries (for duckOSM:
   Python, DuckDB, click and others).
2. **Whole command.** The build, from start to end.
3. **Network steps only.** The sum of the times that the program reports for its own steps, inside one
   process, without start-up:
   - duckOSM: `_load_pbf`, `_build_global_junctions`, `_filter_roads`, `_build_edges`,
     `_simplify_graph`, `_split_private`. `bench/python_steps.py` wraps these methods of `DuckOSM` and
     prints their time. The code that runs is the code of `duckosm build`.
   - duckosm-rust (`--timing`): read ways, read nodes, build network.

**Lane profile.** duckOSM always runs the step `_lane_profile` for the driving mode and has no switch for
it. It is not part of the network step. It adds a large cost (see below). The row "lane profile off" runs
`bench/python_steps.py ... --no-lane-profile`, which replaces that step with an empty function. This row is
**not** the command `duckosm build`.

## Results (Monaco, 10 runs after 1 warm-up run)

| Program | wall min (s) | wall median (s) | peak memory (MB) | network steps min (s) | network steps median (s) |
|---|---|---|---|---|---|
| Start-up only, duckOSM (`duckosm --version`) | 0.19 | 0.21 | 70 | - | - |
| Start-up only, duckosm-rust (`duckosm-rust --version`) | 0.00 | 0.00 | 13 | - | - |
| duckOSM: `duckosm build --config config.yaml` | 36.55 | 40.48 | 313 | - | - |
| duckOSM: same, lane profile off | 1.73 | 2.05 | 322 | 0.53 | 0.60 |
| duckosm-rust: `--format duckdb` | 0.17 | 0.31 | 62 | 0.02 | 0.02 |
| duckosm-rust: `--format geoparquet` | 0.03 | 0.03 | 43 | 0.02 | 0.02 |

Wall time of each run (s):

- duckOSM `duckosm build`: 40.04, 39.18, 51.13, 41.63, 44.60, 53.63, 38.87, 40.93, 39.46, 36.55.
- duckosm-rust: see `bench/run_bench.py` output; the `duckdb` runs vary from 0.17 to 0.31 s because the
  machine was busy.

Reading the table:

- **Start-up:** duckOSM needs about 0.2 s before it does anything. duckosm-rust needs less than 10 ms.
  For a file as small as Monaco, this is a large part of the 1.7-2.1 s of the whole duckOSM run with the
  lane profile off.
- **Network steps alone (no start-up):** 0.53-0.60 s in duckOSM and 0.02 s in duckosm-rust. This is the
  best estimate of the real work, but it is for 3,092 edges. It does not tell what happens at 3 million.
- **Output writing:** the `.duckdb` file of duckosm-rust adds about 0.13 s (loading the GeoParquet files
  with DuckDB). GeoParquet alone takes 0.005 s.

### Steps inside the programs (one run, seconds)

duckOSM: `_connect` 0.06, `_load_pbf` 0.08, `_build_global_junctions` 0.02, `_filter_roads` 0.06,
`_build_edges` 0.03, `_simplify_graph` 0.35, `_split_private` 0.03, `_create_indexes` 0.07, `_cleanup` 0.01,
`_generate_metadata` 0.01, `_checkpoint` 0.09. Total inside the process: 1.51 s.

duckosm-rust: read ways 0.011, read nodes 0.006, build network 0.006, write GeoParquet 0.005,
write `.duckdb` 0.134. Total: 0.162 s.

### The lane profile step

`driving.lane_profile` has one row for each lane of each driving edge (columns `edge_id`, `lane_num`,
`use`, `width_m`, `turn`, `source`, `width_source`). The GMNS export reads it. It does not change `edges`.
The step is `LaneProfile` in `processors/lane_profile.py`.

The plain command `duckosm build` took 36-54 s on Monaco, and 1.7-2.1 s with this step off. The step is
slow because it writes its rows with `con.executemany("INSERT ... VALUES (?, ...)", rows)` (line 158),
which commits each row. Test on a copy of `monaco.duckdb`, on this disk:

| How the rows are inserted | time |
|---|---|
| `executemany`, 3,000 rows, one commit for each row (the code of duckOSM) | 30.0 s (10 ms for each row) |
| `executemany`, 3,000 rows, inside one `BEGIN ... COMMIT` | 1.4 s |
| 300,000 rows in one `INSERT ... SELECT * FROM <DataFrame>` | 0.2 s |

The cost for each row depends on the disk and on what the machine does at the same time. This is why the
plain command took 22 s in an earlier series and 36-54 s in this one. The cost grows with the number of
lane rows, so the step will be much slower on a large area. This was **not** measured here. Other places
in duckOSM use the same call for lists that can be large (for example `component_filter.py` line 67 and
`restrictions.py` line 203).

## Caveats

- **Only Monaco.** One area of 0.7 MB. Large areas are not tested yet. The speed on a large area can be
  different, in both programs.
- **The machine was shared.** Other sessions ran builds and servers at the same time. The load average was
  between 8 and 19 during the runs. The Rust `.duckdb` runs vary from 0.17 to 0.31 s for this reason, and
  the plain duckOSM runs from 36 to 54 s.
- **The programs do not do the same work in every way.** duckOSM also loads all OSM ways, nodes and
  relations into `raw.*`, and writes `ways`, `way_nodes`, `main.global_junctions`, `lane_profile`,
  metadata and the time zone. duckosm-rust keeps only the highway ways and the nodes they use. Both give the
  same `edges`, `private_edges` and `nodes`.
- **Threads.** duckOSM runs on DuckDB, which can use all threads for the SQL steps. duckosm-rust reads the
  PBF with all threads (rayon) and builds the network on one thread. Monaco is too small for threads to
  matter.
- **Memory** is the peak of the whole process. In Python it includes the interpreter and DuckDB.
- **Not measured:** `merge_segments`, the component filter, the walking and cycling modes. They are not in
  duckosm-rust yet (the component filter is in the code and tested on the cut Monaco file; it is off in the
  benchmark because duckOSM runs it only with a boundary).
- **The `.duckdb` writer** loads the GeoParquet files with DuckDB, so `--format duckdb` includes writing
  GeoParquet to disk first.

## Update: the lane profile fix

The fix is merged into `main` of duckOSM (commit `8b5a101`, fast-forward, not pushed). The step now writes its rows with one `INSERT ... SELECT` from a DataFrame.
On a copy of `monaco.duckdb`: 10.9 s before and 0.21 s after (3,176 rows). `driving.lane_profile` has the same
rows and the same column types (checked with `EXCEPT` in both directions). The tests `test_lane_profile.py`,
`test_lane_routing.py`, `test_road_filter_lanes.py`, `test_gmns.py`, `test_levels.py` and `test_sumo.py` pass
(88 tests). The other `executemany` calls on large lists (see above) are not changed.
The benchmark tables on this page were measured before the fix.
