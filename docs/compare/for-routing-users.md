# duckOSM for routing users

This page is for users of openrouteservice (ORS), OSRM, r5py, pandana, and OSMnx. It gives the direct duckOSM equivalent for every standard routing operation. All routing functions are in the core SDK (`duckosm.routing` — [reference](../reference/sdk/routing.md)) and execute via an embedded, multi-threaded Rust routing engine directly inside Python. The notebook [8. Advanced routing](https://khoshkhah.github.io/duckOSM/book/08_routing_advanced.html) runs these workflows on Monaco.

```bash
pip install "duckosm[viz]"      # roadstyle (for interactive route maps)
```

```python
import duckosm

net = duckosm.network("Monaco")
car, walk, bike = net.driving, net.walking, net.cycling
```

## Routing Operations

| Task | Standard tool call | duckOSM call | What duckOSM does |
|---|---|---|---|
| **Fastest route** | ORS `directions`, OSRM `route`, OSMnx `shortest_path` | `car.route(a, b)` | Rust Dijkstra on the turn graph, with the OSM turn restrictions |
| **Shortest route** | OSMnx `shortest_path(weight="length")` | `car.route(a, b, weight="length")` | Minimizes physical distance in metres |
| **Route edges** | OSMnx `route_to_gdf` | `car.route(a, b).edge_ids` | The list of stable `edge_id`s of the route; `.edges` gives them as a GeoDataFrame |
| **Turn-by-turn guidance** | ORS / OSRM `steps=true` | `car.route(a, b).directions()` | Gives each step its type, its direction, the street name, the point, the length and a line of text |
| **Multi-origin / destination** | OSMnx `shortest_path(G, origs, dests)` | `car.routes(origins, destinations)` | Vectorized parallel routing across coordinate or edge lists |
| **Alternative routes (KSP)** | OSMnx `k_shortest_paths`, OSRM `alternatives` | `car.k_shortest_paths(a, b, k=3)` | Returns distinct viable detour paths |
| **Travel time matrix (OD)** | ORS `matrix`, OSRM `table`, r5py `TravelTimeMatrix` | `car.matrix(origins, destinations)` | Parallel single-source searches computing full OD impedance matrix in seconds |
| **Distance matrix** | ORS `matrix(metrics=["distance"])` | `car.matrix(origins, destinations, weight="length")` | Full OD distance table in metres |
| **Isochrones** | ORS `isochrones`, r5py `Isochrones` | `walk.isochrones(origins, minutes=[5, 10, 15])` | Generates buffered reachable road polygons (`buffer_m=25`) |
| **Cumulative accessibility** | pandana `aggregate`, `nearest_pois` | `walk.accessibility(homes, shops, minutes=[5, 10])` | Counts reachable destinations within time thresholds for every origin |
| **Nearest edge snapping** | OSMnx `nearest_edges` | Built-in: pass `(lon, lat)` points | Coordinates snap automatically to the nearest legal road edge |
| **Geocoding** | ORS `geocode`, OSMnx `geocode` | `duckosm.routing.geocode(car, "Boulevard Albert 1er")` | A street name of the network (exact match, any case); else Nominatim, which needs the internet |
| **Multimodal route** | r5py with `transport_modes` | `net.route_multimodal_points(a, b)` | Seamless walk-drive-bike journey across co-located transfer nodes |
| **Custom graph algorithm** | OSMnx `graph_from_place` | `car.to_networkx()`, `car.to_rustworkx()` | The turn graph for NetworkX or rustworkx |

## Architectural Differences

| Dimension | External Routing Engines (ORS, OSRM) | NetworkX / OSMnx | duckOSM Routing |
|---|---|---|---|
| **Architecture** | Requires dedicated HTTP server, daemon processes, and GBs of RAM | In-memory Python interpreter (slow single-thread traversal) | Embedded in-process Rust engine; zero server setup, zero HTTP latency |
| **Turn restrictions** | Enforced by pre-compiled graphs | Ignored (routes may execute illegal U-turns or banned turns) | Edge-based graph strictly obeying all OSM turn relations and conditional restrictions |
| **Input location** | Requires latitude/longitude | Node integer ID | Flexible: `edge_id`, `(lon, lat)` tuple, or street name string |
| **OD matrix scale** | Rate limited or memory-constrained | Loops over individual shortest path calls ($O(N \times M)$) | Multi-core parallel single-source Dijkstra ($O(N)$ tree searches) |
| **Isochrone accuracy** | Often rough concave hulls connecting point clouds | Point buffers along network | Buffer around exact reached road centerline geometries |
| **ID persistence** | Internal graph vertex IDs change every build | Ephemeral `(u, v, key)` changes every run | Persistent 64-bit `edge_id` hash; results join directly back to DuckDB |

## Example: 10-Minute Walk Accessibility to Amenities

```python
import duckosm

net = duckosm.network("Monaco")
walk = net.walking

# 1. Select residential streets as origins
homes = walk.edges(where="highway = 'residential'").edge_id.tolist()[:200]

# 2. Define amenity destination coordinates (shops, clinics, transit stops)
amenities = [(7.4246, 43.7384), (7.4200, 43.7310), (7.4280, 43.7420)]

# 3. Calculate accessibility within 5 and 10 minutes
acc = walk.accessibility(homes, amenities, minutes=[5, 10])

# 4. Color residential streets by reachable count on an interactive map
net.map("walking", color_by=acc["n_10min"], cmap="viridis")
```
