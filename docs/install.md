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
| `sumo` | SUMO (`netconvert`) | `duckosm sumo` builds the `.net.xml` |
| `routing` | networkx | `to_networkx()`, `to_networkx_nodes()`, `write_graph()` |
| `levels` | roadstyle | `duckosm levels` (not ported to Rust yet) |
| `viz` | mapstyle, lanestyle | map functions in Python |
| `elevation` | rasterio, pyproj | not needed by `duckosm elevation` (Rust); this extra goes away |

`route()`, the exports and `duckosm elevation` need no extra.

## Outside tools

| Tool | Only for | Without it |
|---|---|---|
| osmium | `clip-pbf` and `build -b` | `build` uses the whole PBF; `clip-pbf` stops |
| GDAL (`ogr2ogr`) | `export-gis` as one file | `export-gis` writes one file for each layer |

Both are being replaced by Rust code.

## Not ported to Rust yet

`build` (in part), `levels`, `bus`, `multimodal` and `clip-pbf`. They work, and they move to Rust next.

## Quick start

```bash
pip install duckosm

curl -LO https://download.geofabrik.de/europe/monaco-latest.osm.pbf
duckosm build --pbf monaco-latest.osm.pbf --output monaco.duckdb --modes driving
duckosm quack monaco-latest.osm.pbf --gdal -o out --name monaco
```

```python
import duckosm

db = duckosm.build(pbf="monaco-latest.osm.pbf", output="monaco.duckdb")  # as duckosm build

r = duckosm.route_points(db, (7.4155, 43.7285), (7.4400, 43.7480))  # Fontvieille to Larvotto (lon, lat)
r["time_s"], r["length_m"], r["edges"]  # time, length, the edge_ids in order
duckosm.to_gis(db, "monaco.gpkg")  # a GeoPackage for QGIS
```

Next: [Your first network](first_network.md). All functions: [Python SDK](reference/python.md), [Rust SDK](reference/rust.md).

## Where files go

A relative path (an output, a config, a report) is relative to the folder you run `duckosm` in.

Next: [build your first network](first_network.md).
