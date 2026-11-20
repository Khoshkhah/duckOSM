# Python SDK: the modules

The Python SDK has one core and one module for each group of users. This page shows the plan and the decision for
each module. The full design is in the
[SDK design document](https://github.com/Khoshkhah/duckOSM/blob/main/docs/proposals/python-sdk.md).

```mermaid
flowchart LR
    core["<b>duckosm</b> (core)<br/>Network · ModeView · Area<br/>network() · open()<br/>edges() · map()"]
    core --> analysis["<b>duckosm.analysis</b><br/>street network analysts (OSMnx users)"]
    core --> gis["<b>duckosm.gis</b><br/>GIS and data engineers"]
    core --> routing["<b>duckosm.routing</b><br/>routing and accessibility"]
    core --> sim["<b>duckosm.sim</b><br/>traffic simulation"]
    core --> mobility["<b>duckosm.mobility</b><br/>mobility data"]
    core --> planning["<b>duckosm.planning</b><br/>transport planning"]
    core --> hdmap["<b>duckosm.hdmap</b><br/>automated driving"]
    core --> ml["<b>duckosm.ml</b><br/>graph machine learning"]
    core --> fixes["<b>duckosm.fixes</b><br/>OSM fixes, all groups"]
```

## The modules and the decisions

| # | Module | Users (their tools today) | What the module gives (exists today in bold) | Decision |
|---|---|---|---|---|
|  | `duckosm` (core) | All | `Network`, `ModeView`, `Area`, `network()`, `open()`, `edges()`, `nodes()`, `sql()`, `summary()`, `map()`, `dashboard()`; **`get`, `build`** | Approved |
| 1 | `duckosm.analysis` | Street network analysts (OSMnx, momepy, cityseer) | Statistics, bearings, nearest nodes and edges, largest component, clip, elevation, `to_osmnx` | Approved |
| 2 | `duckosm.gis` | GIS and data engineers (osmium, QuackOSM) | Features as a GeoDataFrame, OSM XML; **GIS formats, QuackOSM, Geofabrik, PMTiles** | Approved |
| 3 | `duckosm.routing` | Routing, accessibility, logistics (openrouteservice, OSRM, r5py, pandana) | Many routes, k shortest paths, travel time matrices, isochrones, accessibility; **routes, point routes, multimodal, directions** | Approved |
| 4 | `duckosm.sim` | Traffic simulation (SUMO, MATSim) | The map from SUMO and MATSim ids back to `edge_id`; **SUMO, MATSim, MATSim lanes** | Approved |
| 5 | `duckosm.mobility` | Mobility data (movingpandas, trackintel, leuvenmapmatching) | Map matching onto `edge_id`, snap, join, aggregate, the `edge_id` map between builds | Approved |
| 6 | `duckosm.planning` | Transport planning (AequilibraE, osm2gmns, path4gmns) | AequilibraE project; **GMNS, meso and micro networks, checks** | Approved |
| 7 | `duckosm.hdmap` | Automated driving (scenariogeneration, CARLA, Lanelet2, CommonRoad) | CommonRoad; **OpenDRIVE, Lanelet2, lane graph, lane routes** | Approved |
| 8 | `duckosm.ml` | Graph machine learning (PyTorch Geometric, DGL) | `to_pyg`, `to_dgl`; **networkx, rustworkx** | Approved |
| 9 | `duckosm.fixes` | All groups | Edit the OSM fixes from Python, rebuild, diff; **the fixes file, `duckosm edit`** | Approved |

The decision for each module is one of these:

| Decision | What it means |
|---|---|
| Approved | The module is in the plan of work |
| Later | The module waits; the functions that exist today stay where they are |
| No | The module is not made |

## Decisions inside the modules

### `duckosm.analysis`

| # | Point | Decision |
|---|---|---|
| 1 | `stats()` returns two sets of numbers | **OSMnx view**: a point where only another mode connects (for example a walking way that meets a driving road) is not a node of this mode, and the two pieces count as one segment, as in OSMnx's graph. **duckOSM view**: every node and edge of the mode's tables. Intersections (3 or more roads of the mode) are the same in both. |
| 2 | Close connection points (for example two roundabout entries 0.8 m apart) | Each connection point counts as one intersection, as in OSMnx. Joining close points into one (`consolidate_intersections`) comes later. |
| 3 | The graph that `stats()` uses | The node graph, as in OSMnx |

These change only the calculation. The tables of the file do not change.
