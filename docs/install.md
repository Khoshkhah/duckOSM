# Install

duckOSM needs Python 3.10 or newer.

```bash
pip install duckosm
duckosm --version
```

The wheel has the Rust program in it (the module `duckosm._rust`, no other install). The command `duckosm` and the Python
API (`to_sumo`, `route`, `to_elevation`, ...) run the Rust code. There is a wheel for Linux (x86_64 and aarch64), macOS (x86_64
and arm64) and Windows (x64). On another system, `pip` builds the package from the source, and that needs a
[Rust toolchain](https://rustup.rs).

These commands still run duckOSM's Python code: `levels`, `bus` and `multimodal`. The same for the functions `to_gmns`, `to_meso`, `to_micro` and
`check_gmns`.

## Extras

Some features need more packages. Add the extras you want in brackets, for example
`pip install "duckosm[viz,sumo]"`. `route()`, `duckosm elevation` and the other exports need no extra.

| Extra | Adds | For |
|---|---|---|
| `viz` | mapstyle (with roadstyle and geopandas), lanestyle | `duckosm viz`, `route-map`, `gmns-map` |
| `levels` | roadstyle | `duckosm levels`: the drawing levels |
| `networkx` | networkx | `to_networkx()`, `to_networkx_nodes()`, `write_graph()`; `routing` is its old name, for one more release |
| `sumo` | eclipse-sumo (SUMO's `netconvert`) | `duckosm sumo` |

## The program without Python

Each [GitHub release](https://github.com/Khoshkhah/duckOSM/releases) has the program `duckosm` as one file for each system.
To build it from the source, use a Rust toolchain:

```bash
cargo install --git https://github.com/Khoshkhah/duckOSM duckosm-cli
duckosm --help
```

The program `duckosm-rust` is the old name. It stays for one release: it prints "use duckosm" and runs the same command.

## Where files go

Every relative path (outputs, configs, reports) is relative to the folder you run `duckosm` in.

Next: [build your first network](first_network.md).
