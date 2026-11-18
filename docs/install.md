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
| `routing` | networkx | `to_networkx()`, `to_networkx_nodes()`, `write_graph()`. PR 279 renames it to `networkx` |
| `levels` | roadstyle | `duckosm levels` and the drawing levels of `duckosm build` (without it, the build only warns) |
| `viz` | mapstyle, lanestyle | the map functions in Python. `duckosm viz` (Rust) needs no extra |
| `elevation` | rasterio, pyproj | nothing: `duckosm elevation` (Rust) needs no extra. PR 279 removes it |

`route()`, the exports, `duckosm viz` and `duckosm elevation` need no extra. PR 275 adds the extra `rustworkx`.

## Outside tools

| Tool | Only for | Without it |
|---|---|---|
| osmium | `clip-pbf` and `build -b` | `build` uses the whole PBF; `clip-pbf` stops |
| GDAL (`ogr2ogr`) | `export-gis` as one file | `export-gis` writes one file for each layer |

PR 279 removes both: `clip-pbf` and `export-gis` move to Rust.

## Not ported to Rust yet

`build` (in part), `levels`, `bus`, `multimodal` and `clip-pbf`. They work.

`build` and `levels` move to Rust in PR 275, `clip-pbf` in PR 279.

## Quick start

```bash
pip install duckosm

curl -LO https://download.geofabrik.de/europe/monaco-latest.osm.pbf
duckosm build --pbf monaco-latest.osm.pbf --output monaco.duckdb --modes driving
duckosm quack monaco-latest.osm.pbf --gdal -o out --name monaco
```

```python
import duckosm

db = "monaco.duckdb"  # made by duckosm build above

r = duckosm.route_points(db, (7.4155, 43.7285), (7.4400, 43.7480))  # Fontvieille to Larvotto (lon, lat)
r["time_s"], r["length_m"], r["edges"]  # time, length, the edge_ids in order
duckosm.to_gis(db, "monaco.gpkg")  # a GeoPackage for QGIS
```

The function `duckosm.build()` comes in PR 275.

All functions: [Python SDK](reference/python.md), [Rust SDK](reference/rust.md).

## Where files go

A relative path (an output, a config, a report) is relative to the folder you run `duckosm` in.

Next: [build your first network](first_network.md).
