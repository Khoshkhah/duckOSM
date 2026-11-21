# duckOSM for OSMnx users

This guide gives the duckOSM call for each OSMnx 2 call. The functions are in the core (`duckosm`) and in the module
`duckosm.analysis`. Install: `pip install "duckosm[analysis]"`.

```python
import duckosm
from duckosm import analysis as an

net = duckosm.network("Monaco")           # ox.graph_from_place("Monaco", network_type="drive"), for 3 modes
drive = net.driving                       # one mode: driving, walking or cycling
an.stats(drive)                           # ox.basic_stats(G, area=...)
G = an.to_osmnx(drive)                    # the OSMnx graph, for each OSMnx function that duckOSM does not have
```

## Get a network

| OSMnx | duckOSM |
|---|---|
| `ox.graph_from_place("Monaco", network_type="drive")` | `net = duckosm.network("Monaco")`, then `net.driving` |
| `ox.graph_from_bbox(bbox)` | `duckosm.network(bbox=(west, south, east, north))` |
| `ox.graph_from_point((lat, lon), dist=500)` | `duckosm.network(point=(lon, lat), dist_m=500)` |
| `ox.graph_from_xml("file.osm")` | `duckosm.network(pbf="file.osm.pbf")` |
| `network_type="walk"`, `"bike"` | `net.walking`, `net.cycling` |
| `ox.load_graphml(path)` | `duckosm.open("data/monaco.duckdb")` |
| `ox.geocode_to_gdf("Monaco")` | `net.boundary` |

## Read the data

| OSMnx | duckOSM |
|---|---|
| `ox.graph_to_gdfs(G)` | `net.edges("driving")`, `net.nodes("driving")` |
| `ox.project_graph(G)` | `net.edges("driving", crs="utm")` |
| `ox.add_edge_speeds(G)`, `ox.add_edge_travel_times(G)` | the edge columns `maxspeed_kmh` and `cost_s` |
| `ox.distance.add_edge_lengths(G)` | the edge column `length_m` |
| `ox.simplify_graph(G)` | the build simplifies; `an.stats(drive)["osmnx"]` gives the numbers of OSMnx's graph |

## Analysis

| OSMnx | duckOSM |
|---|---|
| `ox.basic_stats(G, area=a)` | `an.stats(drive, area_m2=a)`: columns `osmnx` and `duckosm` |
| `ox.stats.count_streets_per_node(G)` | `an.streets_per_node(drive)` |
| `ox.bearing.add_edge_bearings(G)` | `an.bearings(drive)` |
| `ox.bearing.orientation_entropy(Gu)` | `an.orientation_entropy(drive)` |
| `ox.plot_orientation(Gu)` | `an.plot_orientation(drive)` |
| `ox.distance.nearest_nodes(G, X, Y)` | `an.nearest_nodes(drive, X, Y)` (lon, lat; distances in metres) |
| `ox.distance.nearest_edges(G, X, Y)` | `an.nearest_edges(drive, X, Y)`: an `edge_id` for each point |
| `nx.strongly_connected_components(G)` | `an.components(drive)` |
| `ox.truncate.largest_component(G)` | `an.largest_component(drive)`: a new network file |
| `ox.truncate.truncate_graph_bbox(G, bbox)` | `an.clip(net, bbox=(west, south, east, north))` |
| `ox.truncate.truncate_graph_polygon(G, polygon)` | `an.clip(net, polygon=polygon)` |
| `ox.truncate.truncate_graph_dist(G, node, 500)` | `an.clip(net, point=(lon, lat), dist_m=500)` (a circle, not a distance along the roads) |
| `ox.elevation.add_node_elevations_raster(G, "dem.tif")` | `an.add_elevation(net, dem="dem.tif")` |
| `ox.elevation.add_edge_grades(G)` | `an.grades(drive)` |
| `ox.utils_geo.sample_points(Gu, 200)` | `an.sample_points(drive, 200)` |
| `ox.simplification.consolidate_intersections(G, 10)` | not yet |

## Routes and maps

| OSMnx | duckOSM |
|---|---|
| `ox.routing.shortest_path(G, a, b, weight="travel_time")` | `drive.route(a, b)`: `a`, `b` are edge ids or `(lon, lat)` points; turn restrictions apply |
| `ox.routing.route_to_gdf(G, route)` | `net.edges(where=...)` with the `edges` of the route |
| `ox.plot_graph(G)` | `net.plot("driving")` (a picture) or `net.map("driving")` (a live map) |
| `ox.plot_graph_route(G, route)` | `net.map("driving", routes=r)` |
| `ox.save_graph_geopackage(G, path)` | `net.to_gis("out/monaco.gpkg")` |

## What is different

| Topic | OSMnx | duckOSM |
|---|---|---|
| Edge id | `(u, v, key)`, new at each download | `edge_id`, the same after each build ([stable edge ids](../concepts/edge-ids.md)) |
| Direction | a two-way road is 2 edges | the same: 2 rows, one for each direction |
| Nodes | a node only where streets meet or end | also where another mode connects or the tags change: see the two views of [`stats`](../reference/sdk/analysis.md#the-two-views-of-stats) |
| Coordinates | `nearest_*` take `X` (lon) and `Y` (lat) | the same |
| Turn restrictions | no | yes, in routes |
| Size | the Overpass API, a city | an `.osm.pbf` file, up to a country |

Reference: [duckosm.analysis](../reference/sdk/analysis.md). Notebook: [9. Street network analysis](../notebooks/09_analysis.md).
