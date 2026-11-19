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

## Quick start

```bash
curl -LO https://download.geofabrik.de/europe/monaco-latest.osm.pbf
duckosm build --pbf monaco-latest.osm.pbf --output monaco.duckdb --modes driving
duckosm quack monaco-latest.osm.pbf --gdal -o out --name monaco
```

```python
import duckosm

db = duckosm.build(pbf="monaco-latest.osm.pbf", output="monaco.duckdb")  # as duckosm build (in the next release)

r = duckosm.route_points(db, (7.4155, 43.7285), (7.4400, 43.7480))  # Fontvieille to Larvotto (lon, lat)
r["time_s"], r["length_m"], r["edges"]  # time, length, the edge_ids in order
duckosm.to_gis(db, "monaco.gpkg")  # a GeoPackage for QGIS
```

All functions: [Python SDK](reference/python.md), [Rust SDK](reference/rust.md).


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

Next: [Your first network](first_network.md), step by step.
