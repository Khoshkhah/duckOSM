# The Rust engine

duckOSM builds the road networks with an engine in Rust. It is the default since 2026-11-04. It gives the same tables and the same
`edge_id` values as the old Python engine, and it is much faster.

| Area | Rust engine | Python engine |
|---|---|---|
| Estonia, the network step | 19 s, 1.5 GB of memory | 1,545 s, 10 GB of memory |

## What runs in Rust

| Step | Engine |
|---|---|
| The network: `edges`, `private_edges`, `nodes`, `edge_id_map`, `edge_graph`, turn restrictions | Rust |
| The osmium cut, `raw.*`, the base-map layers (`features.*`), the bus routes, the drawing levels | Python, for now |

The Python steps move to Rust one at a time. Each move has a test that the tables stay equal.

## Choose the engine

The engine is the option `options.engine` of the [configuration](../reference/configuration.md):

```yaml
options:
  engine: rust      # the default; python runs the old SQL steps
```

On the command line, `duckosm build --engine python ...` uses the Python engine. The Python engine stays as the reference for one
release.

## Install

The Rust engine is the module `duckosm._rust`, inside the `duckosm` wheel. You do not install it separately. There are wheels for Linux
(x86_64 and aarch64), macOS (x86_64 and arm64) and Windows (x64). On another system, `pip` builds the package from the source, and that
needs a [Rust toolchain](https://rustup.rs).

## The program duckosm-rust

The folder `rust/` also has a program with no Python: `duckosm-rust`. It builds an area from a config or from a cut `.osm.pbf` file.

```bash
cargo build --release --manifest-path rust/Cargo.toml -p duckosm-cli   # makes rust/target/release/duckosm-rust
rust/target/release/duckosm-rust --config my_area.yaml --allow-skipped
rust/target/release/duckosm-rust edit my_area.duckdb                   # the page of Edit OSM data
```

`--allow-skipped` lets the build go on when the config asks for a step that only Python has (for example the base-map layers).
`duckosm-rust --help` lists all the options.

## In the browser

[duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/) runs the same engine as WebAssembly. It builds the same tables in a web
page, with nothing to install.

## For developers

The design and the measurements are on GitHub: [the engine](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/rust_engine.md),
[the move into duckOSM](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/rust_engine_move.md) and
[the Rust documents](https://github.com/Khoshkhah/duckOSM/tree/main/docs/rust).
