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

These commands still run duckOSM's Python code: `gmns-viz`, `gmns-map`, `levels`, `bus`, `multimodal` and `clip-pbf`. The same for the functions `to_gmns`, `to_meso`, `to_micro` and
`check_gmns`.

## Extras

Some features need more packages. Add the extras you want in brackets, for example
`pip install "duckosm[routing,viz]"`.

| Extra | Adds | For |
|---|---|---|
| `routing` | networkx | `to_networkx()`, `to_networkx_nodes()`, `write_graph()` |
| `viz` | roadstyle | `duckosm viz` maps |
| `sumo` | SUMO's `netconvert` | `duckosm sumo` |
| `elevation` | rasterio, pyproj | `duckosm elevation` |

## Tools outside Python

Two command-line tools that pip can't install make some steps better. duckOSM works without them.

| Tool | Used for | Without it | Install |
|---|---|---|---|
| **osmium** | cutting a PBF to a boundary (`build -b`, `clip-pbf`) | `build` uses the whole PBF (with a warning); `clip-pbf` fails | `apt install osmium-tool` · `brew install osmium-tool` |
| **GDAL** (`ogr2ogr`) | reading borders from a PBF (`boundary`, `admin`), one-file GeoPackage (`export-gis`) | `boundary` looks the border up online; `export-gis` writes one file per layer; `admin` fails | `apt install gdal-bin` · `brew install gdal` |

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
