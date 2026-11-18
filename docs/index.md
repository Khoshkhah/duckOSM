---
hide:
  - navigation
  - toc
---

<div class="dk-hero" markdown>
![duckOSM](assets/duckosm-mark.svg)
<div markdown>
# duckOSM

OpenStreetMap → road networks, map layers and GIS files.
One Rust program, **on your computer and in the browser**.
</div>
</div>

[Get started](first_network.md){ .md-button .md-button--primary }
[Open duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/){ .md-button }
[View on GitHub](https://github.com/Khoshkhah/duckOSM){ .md-button }

## What duckOSM makes

All outputs come from one `.osm.pbf` file.

| Output | Command | Format | More |
|---|---|---|---|
| **Road networks**: driving, walking and cycling, and the graph of legal turns | `duckosm build` | DuckDB | [Build a network](guides/build.md) |
| **Base-map layers**: water, land use, buildings, POIs | `duckosm build` | DuckDB (`features.*`) | [Base-map layers](reference/features.md) |
| **Map data**: the drawing order of the roads, the walls of the buildings | `duckosm build` | DuckDB | [Drawing order](guides/drawing-order.md), [walls](concepts/building-walls.md) |
| **The OSM features table** of QuackOSM | `duckosm quack` | GeoParquet, DuckDB | [duckOSM and QuackOSM](concepts/quackosm.md) |
| **GDAL's five OSM layers**, as QGIS reads them | `duckosm quack --gdal` | GeoPackage, GeoParquet | [duckOSM and GDAL](concepts/gdal.md) |
| **Traffic simulation networks** | `duckosm sumo`, `matsim`, `gmns` | SUMO, MATSim, GMNS | [Exports](exports/index.md) |
| **GIS files and graphs** of the network | `duckosm export-gis`, `export-graph` | GeoPackage, shapefile, networkx | [GIS](exports/gis.md), [networkx](exports/networkx.md) |
| **Heights**: the ground height of each node | `duckosm elevation` | DuckDB | [Add elevation](guides/elevation.md) |
| **Maps and pages**: maps, a route planner, an editor, Ask | `duckosm viz`, `route-map`, `edit`, `ask` | HTML | [Draw a map](guides/draw-map.md) |

## Two versions, one engine

| | Native | Browser |
|---|---|---|
| Program | `duckosm` on your computer | [duckOSM Studio](guides/studio.md), a web page |
| Install | `pip install duckosm` | none |
| Storage | the disk | the browser's storage (OPFS) |
| Rows | the same | the same |

Both build every area by map tiles, so the memory does not grow with the area
([Browser and native builds](concepts/browser-native.md)).

## Speed on France

France: 5.1 GB `.osm.pbf`, Mac mini (12 cores, 16 GB).

| Job | Native | Browser | Other tool |
|---|---|---|---|
| Road network, driving | 3 min 36 s | 5 min 16 s | |
| OSM features table (`quack`) | 2 min 9 s | 4 min 54 s | QuackOSM: 9 min 30 s |
| GDAL's layers, GeoParquet (`quack --gdal`) | 3 min 37 s | 5 min 23 s | `ogr2ogr`: 14 min 18 s |

## Edge ids that stay the same

Other tools number their edges `0, 1, 2, …`, and the numbers change at each rebuild. duckOSM's `edge_id` is a hash of
OSM ids:

```text
edge_id = hash(osm_id, source, target)
```

The same OSM data gives the same ids after a rebuild, in a city cut from a country, and in every export
([how the id is built](concepts/edge-ids.md)).

## Quick start

```bash
pip install "duckosm[routing]"

curl -LO https://download.geofabrik.de/europe/monaco-latest.osm.pbf
duckosm build --pbf monaco-latest.osm.pbf --output monaco.duckdb --modes driving
duckosm quack monaco-latest.osm.pbf --gdal -o out --name monaco
```

```python
import duckosm

db = duckosm.build(pbf="monaco-latest.osm.pbf", output="monaco.duckdb")   # as duckosm build

r = duckosm.route_points(db, (7.4155, 43.7285), (7.4400, 43.7480))   # (lon, lat): Fontvieille to Larvotto
r["time_s"], r["length_m"], r["edges"]   # the fastest path, door to door, and the ordered edge_ids
duckosm.to_gis(db, "monaco.gpkg")         # a GeoPackage for QGIS
```

All functions: [Python SDK](reference/python.md). Rust: [Rust SDK](reference/rust.md).

Next: [Your first network](first_network.md).

## How each output is checked

| Output | Check |
|---|---|
| OSM features table | each row equals QuackOSM's |
| GDAL's layers | each row equals GDAL's (Monaco, Kentucky) |
| SUMO | SUMO's `netconvert` |
| MATSim | MATSim's DTD and XSD schemas |
| GMNS | the GMNS table specification |
| GeoPackage, shapefile | `duckosm gis-debug` compares each `edge_id` through GDAL |
| Browser build | the tables equal the native build's |

Experimental exports: OpenDRIVE, Lanelet2, railML, lane-level routing ([Exports](exports/index.md)).

---

duckOSM is MIT-licensed and built on [DuckDB](https://duckdb.org) and its spatial extension.
It is an independent project, not affiliated with or endorsed by the DuckDB Foundation.
