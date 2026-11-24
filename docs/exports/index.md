# Exports

duckOSM feeds your existing tools: it turns raw OSM data into clean files for GIS, routing, simulation, GNNs and HD maps, with your fixes applied.

Each table lists:

* **Read by:** the software or libraries that read the file.
* **Export:** the format duckOSM writes.
* **Command:** the call that makes it.

## Exports by level

The levels go from the raw OSM data to the most detailed network. Each level is made from the level before it.

<div class="dk-levels">
<a class="lv" href="#0-osm-file"><span class="n">0</span><b>OSM file</b><span class="db-tag">.osm.pbf</span><span class="desc">Exact boundary cut with your fixes</span><span class="tools">pbfcut</span></a>
<a class="lv" href="#1-osm-data"><span class="n">1</span><b>OSM data</b><span class="db-tag">from .osm.pbf</span><span class="desc">Flat tables of all OSM objects</span><span class="tools">QuackOSM · GDAL · Geofabrik · PMTiles</span></a>
<a class="lv" href="#2-routable-network"><span class="n">2</span><b>Routable network</b><span class="db-tag db">DuckDB</span><span class="desc">Drive, walk, bike<br>Edges, edge_id, turns</span><span class="tools">GeoPackage · networkx</span></a>
<a class="lv" href="#3-gmns-network"><span class="n">3</span><b>GMNS network</b><span class="db-tag db">GMNS DuckDB</span><span class="desc">Macro, meso, micro<br>links, lanes, turns</span><span class="tools">GMNS · MATSim · SUMO</span></a>
<a class="lv" href="#4-hd-map"><span class="n">4</span><b>HD map</b><span class="db-tag db">HD map DuckDB</span><span class="desc">Lane shape in 3D (experimental)</span><span class="tools">Lanelet2 · OpenDRIVE</span></a>
</div>

Your OSM fixes go into every level.

### 0. OSM file

The OSM data cut to your boundary with your fixes, powered by **[pbfcut](https://github.com/Khoshkhah/pbfcut)**. It cuts extracts to exact polygons and partitions large areas into tiles so whole countries build in low memory.

| Read by | Export | Command |
|---|---|---|
| Any OSM tool (QGIS, OSMnx, osm2pgsql, SUMO) | [the OSM file](osm-file.md) with your fixes | `duckosm get`, `duckosm cut --fixes` |

### 1. OSM data

The OSM objects as tables, with no network. Each row is one OSM object.

| Read by | Export | Command |
|---|---|---|
| DuckDB, pandas, Spark | the OSM features table of QuackOSM, as GeoParquet ([duckOSM and QuackOSM](../concepts/quackosm.md)) | `duckosm quack` |
| QGIS, ArcGIS | GDAL's five OSM layers, as GeoPackage or GeoParquet ([duckOSM and GDAL](../concepts/gdal.md)) | `duckosm quack --gdal` |
| QGIS, ArcGIS | Geofabrik's 20 layers (roads, POIs, buildings, land use, ...), as GeoPackage, shapefiles or GeoParquet ([duckOSM and Geofabrik](geofabrik.md)) | `duckosm quack --geofabrik` |
| PostGIS map servers, OSM Carto | the four tables of osm2pgsql, into PostGIS or as GeoParquet ([osm2pgsql tables](osm2pgsql.md)) | `duckosm quack --osm2pgsql` |
| MapLibre, OpenLayers, QGIS | a base map in one file: vector tiles of the Shortbread schema ([Vector tiles (PMTiles)](pmtiles.md)) | `duckosm quack --pmtiles` |

### 2. Routable network

The routable network of each mode: edges with their `edge_id`, nodes, and the legal turns.

| Read by | Export | Command |
|---|---|---|
| Custom routers | [the turn graph](turn-graph.md) `edge_graph` in the database | `route()`, `Router` ([Route](../guides/route.md)) |
| QGIS, map styles | the [base-map layers](base-map.md) `features.*`: water, land use, buildings, POIs | `duckosm build` |
| QGIS, ArcGIS, FME | the network as [GeoPackage or shapefile](gis.md) | `duckosm export-gis` |
| networkx | [a networkx graph](networkx.md), GraphML or gpickle (`duckosm export-graph -f gpickle`) | `to_networkx()`, `duckosm export-graph` |
| rustworkx | [a rustworkx graph](networkx.md): an API like networkx with a Rust engine, faster for large areas | `to_rustworkx()` |
| petgraph (Rust) | [a petgraph graph](../reference/rust.md) | `duckosm_route::graphs::to_petgraph()` |
| Rail planning: OpenTrack, RailSys | [railML 2.4](railml.md): the rail tracks, switches and stations, from `raw.*` | `duckosm railml` |

### 3. GMNS network

The multi-resolution network in the GMNS standard, from macro (links) to micro (lanes, movements, and cells), with the simulation exports. `matsim-lanes` and the HD map exports read the GMNS db (`<area>_gmns.duckdb`). SUMO and MATSim read the GMNS db, or the Level 2 db for large areas.

| Read by | Export | Command |
|---|---|---|
| DTALite, Path4GMNS | [GMNS](gmns.md) tables: macro | `duckosm gmns` |
| SUMO | [SUMO](sumo.md) plain XML and `.netccfg` (netconvert makes the `.net.xml`) | `duckosm sumo` |
| MATSim, BEAM, eqasim | [MATSim](matsim.md) `network.xml`, lanes and signals (GMNS db; `network.xml` also from the Level 2 db) | `duckosm matsim`, `matsim-lanes` |
| DTALite, Path4GMNS | [GMNS meso and micro](gmns.md#meso-and-micro-networks) | `duckosm gmns --meso --micro` |

### 4. HD map

Each lane with its own shape and its connections, for driving simulators and autonomous driving.

| Read by | Export | Command |
|---|---|---|
| Your own lane router | [a lane graph](lane-routing.md) | `duckosm lane-graph`, `route-lanes` |
| Autoware, lanelet2 | [Lanelet2](lanelet2.md) `.osm` | `duckosm lanelet2` |
| CARLA, esmini, Vissim | [OpenDRIVE](opendrive.md) `.xodr` | `duckosm opendrive` |

OpenDRIVE, Lanelet2, railML and the lane graph are **experimental**: tested for structure, not yet loaded in the target tools.

## The same id everywhere

Each network export keeps the **`edge_id`** as the id in the target format: the SUMO edge id, the MATSim link id, the GMNS
`link_id`, the OpenDRIVE road id, a GIS attribute. Data that you join on an `edge_id` in duckOSM (counts, speeds, sensor data)
joins on the exported network too.

## Two starting points

Most exports read the database `duckosm build` writes (`monaco.duckdb`). The lane-level exports read the **GMNS database** (`monaco_gmns.duckdb`) that `duckosm gmns` makes:

* **`monaco.duckdb` (Level 2):** GIS formats, networkx / rustworkx, PyTorch Geometric and DGL, routing on `edge_graph`, railML and OpenDRIVE; SUMO and MATSim for large areas.
* **`monaco_gmns.duckdb` (Level 3 and 4):** the GMNS CSV tables, SUMO and MATSim with the GMNS lanes, MATSim lanes and signals (`matsim-lanes`), Lanelet2, CommonRoad, the lane graph and OpenDRIVE junctions (`opendrive --junctions`).

See each level page below for its dedicated export diagram and details.

## Coordinates

Most exports write longitude / latitude (EPSG:4326). MATSim and OpenDRIVE need metres, so they
reproject to the **UTM zone of the data** (EPSG:32632 for Monaco); `--crs` picks another, e.g.
`EPSG:3006` for Sweden's national grid.

## Elevation

After [`duckosm elevation`](../guides/elevation.md), the heights go along: GIS layers get `ele` /
`z_from` / `z_to` attributes, MATSim nodes a `z`, and OpenDRIVE roads an elevation profile.
