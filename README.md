<p align="center">
  <img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/images/duckosm-banner.svg" alt="duckOSM" width="720">
</p>

<p align="center">
  <strong>Turn OpenStreetMap into routable networks, GMNS models, and GIS layers.</strong><br>
  One Rust engine, for your computer and your browser.
</p>

<p align="center">
  <a href="https://pypi.org/project/duckosm/"><img alt="PyPI" src="https://img.shields.io/pypi/v/duckosm"></a>
  <a href="https://github.com/Khoshkhah/duckOSM/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Khoshkhah/duckOSM/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://khoshkhah.github.io/duckOSM/"><img alt="Docs" src="https://img.shields.io/badge/docs-online-17191f"></a>
  <img alt="Python 3.10+" src="https://img.shields.io/badge/python-3.10%2B-3776AB">
  <a href="LICENSE"><img alt="License MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
</p>

<p align="center">
  <a href="https://khoshkhah.github.io/duckOSM/"><b>Documentation</b></a>
  &nbsp;•&nbsp;
  <a href="https://khoshkhah.github.io/duckOSM/book/"><b>Notebook&nbsp;Manual</b></a>
  &nbsp;•&nbsp;
  <a href="https://khoshkhah.github.io/duckosm-studio/"><b>duckOSM&nbsp;Studio</b></a>
  &nbsp;•&nbsp;
  <a href="https://khoshkhah.github.io/duckOSM/reference/python/"><b>Python&nbsp;SDK</b></a>
  &nbsp;•&nbsp;
  <a href="https://khoshkhah.github.io/duckOSM/reference/cli/"><b>CLI&nbsp;Reference</b></a>
</p>

---

## What duckOSM is

duckOSM reads an OpenStreetMap `.osm.pbf` file and builds the complete transport network of an area—from GIS layers
and turn-by-turn routing to multi-scale GMNS models, traffic simulations, and Graph Neural Networks. One Rust engine
runs it all: the command line, the Python SDK and the browser.

It processes areas tile by tile, keeping memory low and speeds fast even for an entire country. Fix the OSM data once on
a map: your fixes go into one YAML file and apply at build time, so every downstream table, network layer, and export
inherits the same corrected data.

<table>
<tr>
<td width="50%" valign="top">

**Large areas, little memory**<br>
duckOSM builds an area tile by tile, so the build stays fast and the memory stays low, also for a whole country.

</td>
<td width="50%" valign="top">

**In your browser**<br>
[duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/) runs the same engine in a web page, with no install,
and gives the same rows.

</td>
</tr>
<tr>
<td valign="top">

**Python SDK**<br>
`pip install duckosm`, then `import duckosm`: build, route, match GPS traces and export from Python
([Python SDK](https://khoshkhah.github.io/duckOSM/reference/python/)).

</td>
<td valign="top">

**The notebook manual**<br>
15 chapters, each one a Jupyter notebook on Monaco that opens in Colab
([notebook manual](https://khoshkhah.github.io/duckOSM/book/)).

</td>
</tr>
<tr>
<td valign="top">

**One id for each road direction**<br>
`edge_id = hash(osm_id, source, target)` stays the same after a rebuild, so your data joins to the right edge
([how](https://khoshkhah.github.io/duckOSM/concepts/edge-ids/)).

</td>
<td valign="top">

**Fix the OSM data once**<br>
Edit roads, turns and buildings on a map: your fixes go into one YAML file and apply at build time
([Edit OSM data](https://khoshkhah.github.io/duckOSM/guides/edit-osm-data/)).

</td>
</tr>
<tr>
<td valign="top">

**GMNS in DuckDB**<br>
The open standard (General Modeling Network Specification) in DuckDB: links, lanes, and turning movements for simulation and traffic assignment
([GMNS network](https://khoshkhah.github.io/duckOSM/exports/gmns-network/)).

</td>
<td valign="top">

**Traffic simulation**<br>
SUMO and MATSim exports, and GMNS tables that DTALite reads; SUMO and MATSim can join the edges of a road into one (`--merge`)
([Exports](https://khoshkhah.github.io/duckOSM/exports/)).

</td>
</tr>
</table>

## What duckOSM makes

All outputs come from one `.osm.pbf` file, in five levels, and all take your OSM fixes. Each level keeps the `edge_id` of the routable network.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/images/levels-dark.svg">
    <img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/images/levels-light.svg" alt="The five levels of duckOSM's outputs: 0 OSM file (pbfcut), 1 OSM data from .osm.pbf (QuackOSM, GDAL, Geofabrik, PMTiles), 2 routable network in DuckDB (GeoPackage, networkx), 3 GMNS network in GMNS DuckDB (GMNS, MATSim, SUMO), 4 HD map with 3D lane geometry, experimental (Lanelet2, OpenDRIVE)" width="900">
  </picture>
</p>

| Level | Database / Format | Read by / Target tools | Command |
|---|---|---|---|
| [0. OSM file](https://khoshkhah.github.io/duckOSM/exports/osm-file/) | `.osm.pbf` (cut by `pbfcut`) | Exact boundary cut with your fixes | `duckosm get`, `duckosm cut` |
| [1. OSM data](https://khoshkhah.github.io/duckOSM/exports/osm-data/) | `from .osm.pbf` | QuackOSM · GDAL · Geofabrik · osm2pgsql · PMTiles | `duckosm quack` |
| [2. Routable network](https://khoshkhah.github.io/duckOSM/exports/routable-network/) | `DuckDB` | Drive, walk, bike · GeoPackage · networkx | `duckosm build`, `export-gis`, `export-graph` |
| [3. GMNS network](https://khoshkhah.github.io/duckOSM/exports/gmns-network/) | `GMNS DuckDB` | Multi-scale (macro, meso, micro) · GMNS · MATSim · SUMO | `duckosm gmns`, `sumo`, `matsim` |
| [4. HD map](https://khoshkhah.github.io/duckOSM/exports/hd-map/) *(experimental)* | `3D geometry` | Lane shape in 3D · Lanelet2 · OpenDRIVE | `duckosm lanelet2`, `opendrive` |

## Quick start

```bash
pip install duckosm

duckosm get "Monaco"                                            # -> monaco.osm.pbf, monaco.geojson
duckosm build --pbf monaco.osm.pbf --boundary monaco.geojson    # -> Level 2 routable database (monaco.duckdb)
duckosm gmns monaco.duckdb                                      # -> Level 3 GMNS database (monaco_gmns.duckdb)
duckosm quack monaco.osm.pbf --gdal -o out --name monaco        # -> Level 1 GIS layers
```

```python
import duckosm

area = duckosm.get("Monaco")
db = duckosm.build(pbf=area.pbf, boundary=area.boundary)  # Level 2 routable database

# Route on the turn graph
r = duckosm.route_points(db, (7.4155, 43.7285), (7.4335, 43.7468))   # Fontvieille to Larvotto
r["time_s"], r["length_m"], r["edges"]

# Export to Level 3 GMNS or GIS
duckosm.to_gmns(db, "monaco_gmns.duckdb")   # multi-scale GMNS network
duckosm.to_gis(db, "monaco.gpkg")           # GeoPackage for QGIS
```

All functions: [Python SDK](https://khoshkhah.github.io/duckOSM/reference/python/) and [Rust SDK](https://khoshkhah.github.io/duckOSM/reference/rust/).

## Learn with the notebook manual

The [**notebook manual**](https://khoshkhah.github.io/duckOSM/book/) teaches the Python SDK in 15 chapters: your first
network, maps, routing, street network analysis, map matching, traffic simulation, transport planning, HD maps and
graph machine learning.

<p>
  <a href="https://khoshkhah.github.io/duckOSM/book/"><img alt="Read the notebook manual" src="https://img.shields.io/badge/Read-the%20notebook%20manual-f0d35b?style=for-the-badge"></a>
  <a href="https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/01_area_to_network.ipynb"><img alt="Open chapter 1 in Colab" src="https://img.shields.io/badge/Open%20chapter%201-in%20Colab-F9AB00?style=for-the-badge&logo=googlecolab&logoColor=white"></a>
  <a href="https://khoshkhah.github.io/duckosm-studio/"><img alt="Try duckOSM Studio" src="https://img.shields.io/badge/Try-duckOSM%20Studio-17191f?style=for-the-badge"></a>
</p>

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
