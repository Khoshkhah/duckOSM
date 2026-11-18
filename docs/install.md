# Install

```bash
pip install duckosm
duckosm --version
```

| | |
|---|---|
| You get | the program `duckosm` (Rust) and a Python package that calls it |
| Python | 3.10 or newer |
| Ready-made packages | Linux (x86_64, aarch64), macOS (x86_64, arm64), Windows (x64) |
| Other systems | `pip` builds from the source; this needs a [Rust toolchain](https://rustup.rs) |

## No install: the browser

| Page | Does |
|---|---|
| [duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/) | choose an area, build its network, edit the OSM data, export |
| [The quack page](https://khoshkhah.github.io/duckosm-studio/quack.html) | the OSM features table of QuackOSM and GDAL's five layers |

## No Python: the program alone

Each [GitHub release](https://github.com/Khoshkhah/duckOSM/releases) has the program `duckosm` as one file for each system.
Or build it with Rust:

```bash
cargo install --git https://github.com/Khoshkhah/duckOSM duckosm-cli
```

`duckosm-rust` is the old name of the program. It works for one more release.

## Extras

Optional. Add them in brackets, for example `pip install "duckosm[sumo]"`.

| Extra | Adds | For |
|---|---|---|
| `sumo` | SUMO (`eclipse-sumo`, with `netconvert`) | `duckosm sumo` builds the `.net.xml` |
| `networkx` | networkx | `to_networkx()`, `to_networkx_nodes()`, `write_graph()`. `routing` is its old name, for one more release |
| `levels` | roadstyle | `duckosm levels` (not ported to Rust yet) |
| `viz` | mapstyle, lanestyle | the maps of `duckosm build` with `viz.enabled: true` (not ported to Rust yet), and the map functions in Python |

`route()`, the exports, `export-graph` to GraphML and `duckosm elevation` need no extra. No duckOSM command needs osmium or GDAL.

## Not ported to Rust yet

`build` (in part), `levels`, `bus` and `multimodal`. They work, and they move to Rust next.

## Where files go

A relative path (an output, a config, a report) is relative to the folder you run `duckosm` in.

Next: [build your first network](first_network.md).
