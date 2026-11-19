<p align="center">
  <img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/images/duckosm-banner.svg" alt="duckOSM" width="720">
</p>

<p align="center">
  <b>Turn OpenStreetMap into routable networks, map layers and GIS files.<br>
  One Rust engine, for your computer and your browser.</b>
</p>

<p align="center">
  <a href="https://pypi.org/project/duckosm/"><img alt="PyPI" src="https://img.shields.io/pypi/v/duckosm"></a>
  <a href="https://github.com/Khoshkhah/duckOSM/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Khoshkhah/duckOSM/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://khoshkhah.github.io/duckOSM/"><img alt="Docs" src="https://img.shields.io/badge/docs-khoshkhah.github.io%2FduckOSM-17191f"></a>
  <img alt="Python 3.10+" src="https://img.shields.io/badge/python-3.10%2B-3776AB">
  <img alt="License MIT" src="https://img.shields.io/badge/license-MIT-blue">
</p>

<p align="center">
  <a href="https://khoshkhah.github.io/duckOSM/"><b>Documentation</b></a> ·
  <a href="https://khoshkhah.github.io/duckosm-studio/">duckOSM Studio (browser)</a> ·
  <a href="https://khoshkhah.github.io/duckOSM/first_network/">Your first network</a> ·
  <a href="https://khoshkhah.github.io/duckOSM/reference/cli/">Command line</a> ·
  <a href="https://khoshkhah.github.io/duckOSM/reference/database/">Database schema</a>
</p>

---

## What duckOSM makes

All outputs come from one `.osm.pbf` file.

| Output | Command | Format |
|---|---|---|
| **Road networks**: driving, walking and cycling, and the graph of legal turns | `duckosm build` | DuckDB |
| **Base-map layers**: water, land use, buildings, POIs | `duckosm build` | DuckDB (`features.*`) |
| **Map data**: the drawing order of the roads, the walls of the buildings | `duckosm build` | DuckDB |
| **The OSM features table** of [QuackOSM](https://khoshkhah.github.io/duckOSM/concepts/quackosm/) | `duckosm quack`, [in the browser](https://khoshkhah.github.io/duckosm-studio/quack.html) | GeoParquet, DuckDB |
| **GDAL's five OSM layers**, as [QGIS reads them](https://khoshkhah.github.io/duckOSM/concepts/gdal/) | `duckosm quack --gdal`, [in the browser](https://khoshkhah.github.io/duckosm-studio/quack.html) | GeoPackage, GeoParquet |
| **Traffic simulation networks** | `duckosm sumo`, `matsim`, `gmns` | SUMO, MATSim, GMNS |
| **GIS files and graphs** of the network | `duckosm export-gis`, `export-graph` | GeoPackage, shapefile, networkx |
| **Heights**: the ground height of each node | `duckosm elevation` | DuckDB |
| **Maps and pages**: maps, a route planner, an editor, Ask | `duckosm viz`, `route-map`, `edit`, `ask` | HTML |

## Fix the OSM data

OSM has errors: a wrong one-way, a missing road, a wrong bridge layer. Fix them on a map. duckOSM keeps your fixes in one
file, `osm_fixes.yaml`, and applies them at each build, so a new OSM file does not remove them.

<img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/images/edit-turn.png" alt="The Turn tab: road 688731299 is picked, the junction box shows its junction, and the list shows the two roads that leave it, each with Allowed. On the map, the road is blue, the roads that leave are green and the road that only arrives is violet" width="720">

| Where | How |
|---|---|
| Browser | [duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/), step **Edit OSM data** |
| Your computer | `duckosm edit my_area.duckdb` opens the same editor |
| Build | `duckosm build --fixes osm_fixes.yaml` |

| You can change | Examples |
|---|---|
| Roads | one-way, lanes, speed, road class, access; delete a road |
| Buildings | floors, height, roof; the type of each wall and its door |
| Nodes | traffic signals, crossings |
| Turns | a turn that a driver cannot make, or must make |
| New roads | a road that OSM does not have |

The network, all its exports, and the quack outputs (`duckosm quack --fixes`) use your fixes. More: [Edit OSM data](https://khoshkhah.github.io/duckOSM/guides/edit-osm-data/).

## Two versions, one engine

| | Native | Browser |
|---|---|---|
| Program | `duckosm` on your computer | [duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/), a web page |
| Install | `pip install duckosm` | none |
| Rows | the same | the same |

In the browser, open:

- [**duckOSM Studio**](https://khoshkhah.github.io/duckosm-studio/): choose an area, build its network, edit the OSM data, and export.
- [**The quack page**](https://khoshkhah.github.io/duckosm-studio/quack.html): the OSM features table of QuackOSM and GDAL's five layers.

Both build every area by map tiles, cut by [pbfcut](https://github.com/Khoshkhah/pbfcut), so the memory does not grow with the area
([Browser and native builds](https://khoshkhah.github.io/duckOSM/concepts/browser-native/)).

## Speed on France

France: 5.1 GB `.osm.pbf`, Mac mini (Apple M6, 12 cores, 16 GB).

| Job | Native | Browser | Other tool |
|---|---|---|---|
| Road network, driving | 3 min 36 s | 5 min 16 s | |
| OSM features table (`quack`) | 2 min 9 s | 4 min 54 s | QuackOSM: 9 min 30 s |
| GDAL's layers, GeoParquet (`quack --gdal`) | 3 min 37 s | 5 min 23 s | `ogr2ogr`: 14 min 18 s |

## One edge id for each direction

```text
edge_id = hash(osm_id, source, target)
```

**Each direction of a road has its own id**: a two-way street is two edges, A → B and B → A, so data per direction joins
to the right edge. The id comes only from OSM ids (the way and its two end nodes), so it stays the same after a rebuild,
a cut and every export ([how](https://khoshkhah.github.io/duckOSM/concepts/edge-ids/)).

## Quick start

```bash
pip install "duckosm[routing]"

duckosm get "Monaco"  # -> monaco.osm.pbf, monaco.geojson
duckosm build --pbf monaco.osm.pbf --boundary monaco.geojson --modes driving
duckosm quack monaco.osm.pbf --gdal -o out --name monaco
```

```python
import duckosm

area = duckosm.get("Monaco")  # as duckosm get
db = duckosm.build(pbf=area.pbf, boundary=area.boundary)  # as duckosm build

r = duckosm.route_points(db, (7.4155, 43.7285), (7.4400, 43.7480))   # (lon, lat): Fontvieille to Larvotto
r["time_s"], r["length_m"], r["edges"]   # the fastest path, door to door, and the ordered edge_ids
duckosm.to_gis(db, "monaco.gpkg")         # a GeoPackage for QGIS
```

All functions: [Python SDK](https://khoshkhah.github.io/duckOSM/reference/python/). Rust: [Rust SDK](https://khoshkhah.github.io/duckOSM/reference/rust/).

Each output is compared with a reference tool: QuackOSM, GDAL, SUMO, MATSim and the GMNS specification
([how each output is checked](https://khoshkhah.github.io/duckOSM/concepts/checks/)).

Experimental exports: OpenDRIVE, Lanelet2, railML, lane-level routing. See the
[documentation](https://khoshkhah.github.io/duckOSM/exports/).

## For AI agents

- To use duckOSM: the agent skill [`skills/duckosm/SKILL.md`](skills/duckosm/SKILL.md) (also a
  Claude Code plugin: `/plugin marketplace add Khoshkhah/duckOSM`), and the whole documentation as
  one text file: [`llms-full.txt`](https://khoshkhah.github.io/duckOSM/llms-full.txt)
  ([`llms.txt`](https://khoshkhah.github.io/duckOSM/llms.txt) is its index).
- `duckosm info DB --json` says what a database holds; `way`, `route-lanes` and `gis-debug` also
  take `--json`.
- To change duckOSM: the rules are in [`AGENTS.md`](AGENTS.md).

## Data and credits

- Map data © [OpenStreetMap contributors](https://www.openstreetmap.org/copyright), under the
  [ODbL](https://opendatacommons.org/licenses/odbl/). Outputs made from OSM data keep this license.
- The region files come from [Geofabrik](https://www.geofabrik.de/)'s free [download server](https://download.geofabrik.de/).
  `duckosm quack --geofabrik` follows Geofabrik's [layer rules](https://download.geofabrik.de/osm-data-in-gis-formats-free.pdf).

## License

MIT. duckOSM is built on [DuckDB](https://duckdb.org) and its spatial extension. It is an
independent project, not affiliated with or endorsed by the DuckDB Foundation.
