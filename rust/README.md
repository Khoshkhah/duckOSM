# The Rust engine of duckOSM

The Cargo workspace of the Rust engine. It came from the repository
[duckosm-rust](https://github.com/Khoshkhah/duckosm-rust) (commit `53b3196`, version 0.3.0).
The plan is `docs/design/rust_engine_move.md`. The notes of the engine are in `docs/rust/`.

| Crate | Job |
|---|---|
| `duckosm-core` | Read a PBF. Build the edges, nodes, `edge_id`, turn restrictions and edge graph. |
| `duckosm-parquet` | Write the tables as GeoParquet. |
| `duckosm-duckdb` | Write the tables into a `.duckdb` file (DuckDB 1.5.6, bundled). |
| `duckosm-build` | The whole build, from a config file or from options. |
| `duckosm-cli` | The program `duckosm-rust` (it gets a new name in a later step). |
| `duckosm-py` | The Python module `duckosm_rust` (it becomes `duckosm._rust` in a later step). |

```bash
cd rust
CARGO_BUILD_JOBS=4 cargo test --release -p duckosm-core -p duckosm-duckdb -p duckosm-parquet -p duckosm-build
```

The tests read the Monaco files in `../tests/data`.
