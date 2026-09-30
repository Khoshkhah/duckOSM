# Install

duckOSM needs Python 3.10 or newer.

```bash
pip install duckosm
duckosm --version          # duckosm, version 0.1.0
```

## Extras

Some features need more packages. Add the extras you want in brackets, for example
`pip install "duckosm[routing,viz]"`.

| Extra | Adds | For |
|---|---|---|
| `routing` | networkx | `route()`, `Router`, `to_networkx()`, `duckosm export-graph` |
| `viz` | roadstyle | `duckosm viz` maps |
| `sumo` | SUMO's `netconvert` | `duckosm sumo` |
| `elevation` | rasterio, pyproj | `duckosm elevation` |

## Tools outside Python

Two command-line tools that pip can't install make some steps better. duckOSM works without them.

| Tool | Used for | Without it | Install |
|---|---|---|---|
| **osmium** | cutting a PBF to a boundary (`build -b`, `clip-pbf`) | `build` uses the whole PBF (with a warning); `clip-pbf` fails | `apt install osmium-tool` · `brew install osmium-tool` |
| **GDAL** (`ogr2ogr`) | reading borders from a PBF (`boundary`, `admin`), one-file GeoPackage (`export-gis`) | `boundary` looks the border up online; `export-gis` writes one file per layer; `admin` fails | `apt install gdal-bin` · `brew install gdal` |

## Where files go

Every relative path (outputs, configs, reports) is relative to the folder you run `duckosm` in.

Next: [build your first network](first_network.md).
