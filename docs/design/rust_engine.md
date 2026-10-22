# The Rust engine: `options.engine: rust`

Status: implemented on 2026-10-22. Decision by Kaveh (duckosm-rust design, section 12): Python duckOSM
calls the Rust wheel part by part, with a config option as the switch. This is the first part.

## What it does

[duckosm-rust](https://github.com/Khoshkhah/duckosm-rust) is the network step of duckOSM in Rust: a cut
PBF in, the tables `edges`, `private_edges`, `nodes` and `edge_id_map` out, with the same `edge_id`
values. On Estonia (121 MB PBF, three modes) it takes 19 s and 1.5 GB, and the SQL steps take 1,545 s
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
`process_speeds`, `calculate_costs`, `functional_types`, `osm_overrides`). The GeoParquet files go to a
temporary folder next to the output file. Rust does not run the component filter
(`keep_largest_component=False`): `ComponentFilter` runs in Python after the edge graph, as before, so
the edge graph and `edge_id_map` are pruned together.

For each mode:

| Step | `engine: python` | `engine: rust` |
|---|---|---|
| RoadFilter (`ways`, `way_nodes`, `nodes`) | runs | runs (the restrictions read `way_nodes`) |
| OSM overrides on `ways` | runs | runs (so `ways` is the same; Rust applies the same rules itself) |
| GraphBuilder, GraphSimplifier, PathConnector, DismountMarker, SpeedProcessor, CostCalculator, FunctionalType, `_split_private` | run | replaced by one step, `rust_network`: load the four Parquet files of the mode into the mode's schema |
| Turn restrictions, EdgeGraph, ComponentFilter, H3, indexes, validation | run | run |

`rust_engine.load_mode` creates the tables with `read_parquet`. The only type cast is the geometry:
GeoParquet reads as `GEOMETRY('OGC:CRS84')`, the tables have `GEOMETRY`. The other column types are
equal (checked on Estonia, all tables of the three modes).

A clip build (`source.type: duckdb`) has no PBF, so the engine has no effect there.

## What is equal, and what is not

The duckosm-rust repository tests the network step against duckOSM on Monaco (nine variants, three
modes, every row), Tartu and Estonia. One known difference: a connector between two facing dangling
ends can have the other direction label (`edge_ref`, `is_reverse`, and the columns that follow from
them). duckOSM does not fix this order; see section 6 of the duckosm-rust design.

`tests/test_engine_rust.py` builds the Monaco sample with both engines and compares the tables. It
skips when `duckosm_rust` is not installed.

## Install

The wheel is not on PyPI yet (the name waits for the DuckDB trademark answer). Build it from the
duckosm-rust repository and install it into the venv:

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
