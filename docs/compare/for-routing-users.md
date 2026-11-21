# duckOSM for routing users

This page is for users of openrouteservice (ORS), OSRM, r5py, pandana and OSMnx. It gives the duckOSM call for each of
their calls. All duckOSM calls are in [`duckosm.routing`](../reference/sdk/routing.md) and run on your computer, on a
file that you build once:

```python
import duckosm

net = duckosm.network("Monaco")      # get the OSM data and build the network
car, walk = net.driving, net.walking  # one view for each mode
```

## Calls

| Task | Their call | duckOSM call |
|---|---|---|
| One route | ORS `directions`, OSRM `route`, OSMnx `shortest_path` | `car.route(a, b)` |
| The route as edges | OSMnx `route_to_gdf` | `car.route(a, b).edges` |
| The shortest route, not the fastest | OSMnx `shortest_path(weight="length")` | `car.route(a, b, weight="length")` |
| Turn-by-turn steps | ORS and OSRM `steps=true` | `car.route(a, b).directions()` |
| Many routes | OSMnx `shortest_path(G, origs, dests)` | `car.routes(origins, destinations)` |
| Other good routes | OSMnx `k_shortest_paths`, OSRM `alternatives=true` | `car.k_shortest_paths(a, b, k=3)` |
| A travel time matrix | ORS `matrix`, OSRM `table`, r5py `TravelTimeMatrix` | `car.matrix(origins, destinations)` |
| A distance matrix | ORS `matrix(metrics=["distance"])` | `car.matrix(origins, destinations, weight="length")` |
| Isochrones | ORS `isochrones`, r5py `Isochrones` | `walk.isochrones(origins, minutes=[5, 10, 15])` |
| Targets within a time, the nearest target | pandana `aggregate`, `nearest_pois` | `walk.accessibility(origins, targets, minutes=[5, 10])` |
| Nearest edge of a point | OSMnx `nearest_edges` | a point is a place: each call snaps it to its nearest edge |
| A place name to a point | ORS `geocode`, OSMnx `geocode` | `duckosm.routing.geocode(car, "Boulevard Albert 1er")` |
| A route across modes | r5py with `transport_modes` | `net.route_multimodal_points(a, b)` |
| The graph for your own algorithm | OSMnx `graph_from_place` | `car.to_networkx()`, `car.to_rustworkx()` |

## Differences

| Subject | Their tools | duckOSM |
|---|---|---|
| Server | ORS and OSRM: a server (or the public server with a quota) | no server: a `.duckdb` file and a Rust module in Python |
| Turns | OSMnx: a node graph, no turn restrictions | an edge graph: a route obeys the OSM turn restrictions |
| A place | a coordinate, or a node id (OSMnx) | an edge id, a `(lon, lat)` point or a place name |
| Matrix | OSMnx: one route for each cell | one Dijkstra search for each origin, the origins in parallel |
| Isochrone polygon | ORS: a concave hull of the reached points | the union of the reached edges, each with a buffer of 25 m (`buffer_m`) |
| Public transport | r5py: GTFS timetables | no timetables; the multimodal graph walks, cycles and drives |
| IDs | node ids that change at each download | `edge_id`, a stable hash: you can join your data to it after a rebuild |

## Example: the shops within 10 minutes on foot

```python
homes = walk.edges(where="highway = 'residential'").edge_id.tolist()[:200]   # origins: residential streets
shops = [(7.4246, 43.7384), (7.4200, 43.7310)]                              # targets: points
acc = walk.accessibility(homes, shops, minutes=[5, 10])                      # a row for each home street
net.map("walking", color_by=acc["n_10min"], cmap="viridis")                  # color each street by its count
```

The notebook [11. Advanced routing](../notebooks/08_routing_advanced.md) runs each call on Monaco, with a map or a chart
for each one.
