# The Rust engine: `options.engine: rust`

Status: implemented on 2026-10-22. Decision by Kaveh (duckosm-rust design, section 12): Python duckOSM
calls the Rust wheel part by part, with a config option as the switch. Part 1 (2026-10-22): the network
step. Part 2 (2026-10-22, duckosm-rust 0.2.0): the turn restrictions and the edge graph.

## What it does

[duckosm-rust](https://github.com/Khoshkhah/duckosm-rust) is the network step of duckOSM in Rust: a cut
PBF in, the tables `edges`, `private_edges`, `nodes`, `edge_id_map`, `edge_graph`, `turn_permission`,
`turn_restrictions` and `turn_path_restrictions` out, with the same `edge_id` values. On Estonia (121 MB PBF, three modes) it takes 19 s and 1.5 GB, and the SQL steps take 1,545 s
and 10 GB. Its Python module is `duckosm_rust` (the wheel of that repository).

With `options.engine: rust` (CLI `--engine rust`), the build runs the network step in Rust and every other
step in Python, as before. The output file has the same tables.

## The steps

The global steps do not change: connect, the osmium cut, `ST_READOSM` into `raw.*`, the raw-way fixes, the
boundary, the sidewalk rule and `global_junctions`. The later steps need `raw.*` (features, bus routes,
restrictions), and these steps are cheap.

One new global step, `rust_build`, runs after `global_junctions`. It calls `duckosm_rust.build()` one time
for all modes, on the cut PBF, with the options of the config (`merge_segments`, `global_junctions`,
`cycling_dismount`, `infer_separate_sidewalks`, `connectivity_rescue`, `connect_snap_m`,
`process_speeds`, `calculate_costs`, `functional_types`, `osm_overrides`, `build_graph`,
`extract_restrictions`). The GeoParquet files go to a
temporary folder next to the output file. Rust does not run the component filter
(`keep_largest_component=False`): `ComponentFilter` runs in Python after the tables are loaded, as before,
so it prunes the edges and the tables of the edge graph and the restrictions together.

For each mode:

| Step | `engine: python` | `engine: rust` |
|---|---|---|
| RoadFilter (`ways`, `way_nodes`, `nodes`) | runs | runs (its tables stay in the file, as with `python`) |
| OSM overrides on `ways` | runs | runs (so `ways` is the same; Rust applies the same rules itself) |
| GraphBuilder, GraphSimplifier, PathConnector, DismountMarker, SpeedProcessor, CostCalculator, FunctionalType, `_split_private` | run | replaced by one step, `rust_network`: load the Parquet files of the mode into the mode's schema |
| RestrictionProcessor (driving), EdgeGraphBuilder | run | do not run: `rust_network` loads `turn_restrictions`, `turn_path_restrictions`, `edge_graph` and `turn_permission` |
| ComponentFilter, H3, indexes, validation | run | run |

`rust_engine.load_mode` creates the tables with `read_parquet`. The only type cast is the geometry:
GeoParquet reads as `GEOMETRY('OGC:CRS84')`, the tables have `GEOMETRY`. The other column types are
equal (checked on Estonia, all tables of the three modes).

A clip build (`source.type: duckdb`) has no PBF, so the engine has no effect there.

## What is equal, and what is not

The duckosm-rust repository tests the network step against duckOSM on Monaco (nine variants, three
modes, every row), Tartu and Estonia. One known difference: a connector between two facing dangling
ends can have the other direction label (`edge_ref`, `is_reverse`, and the columns that follow from
them). duckOSM does not fix this order; see section 6 of the duckosm-rust design.

`tests/test_engine_rust.py` builds the Monaco sample with both engines and compares the tables, with the
edge graph and the turn restrictions. It skips when `duckosm_rust` is not installed. The duckosm-rust
repository also compares the edge graph and the restrictions on Tartu (81 restrictions, 19
`turn_permission` rows, 5 via-way paths): every row is equal.

## Install

The engine needs duckosm-rust 0.2.0 or newer; `rust_engine.module()` checks the version. The wheel is not
on PyPI yet (the name waits for the DuckDB trademark answer). A tag of duckosm-rust makes a GitHub Release
with a wheel for Linux, macOS and Windows (the repository is private for now): `pip install` the wheel of
your system. Or build it from the duckosm-rust repository:

```bash
pip install maturin
maturin build --release --no-default-features -m crates/duckosm-py/Cargo.toml -o wheels   # no DuckDB inside: 2 MB
pip install wheels/duckosm_rust-*.whl
```

`--no-default-features` is correct here: duckOSM has its own DuckDB, and the engine needs only the
GeoParquet output.

## Limits

- The Rust step reads the PBF again (`ST_READOSM` already read it into `raw.*`). This costs about 1 s
  on Estonia and keeps the two programs independent.
- The timing of the build report shows one step, `rust_build`, for the whole network step of all modes.
