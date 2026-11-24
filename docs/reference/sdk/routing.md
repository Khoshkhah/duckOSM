# duckosm.routing

Routing, accessibility and logistics. Each function takes a `Network`, a `ModeView` (`net.walking`), a path or a DuckDB
connection. With a path or a connection, the mode default is `"driving"`. Each function is also a method of the network
and of a mode view: `net.walking.matrix(...)` is `duckosm.routing.matrix(net.walking, ...)`.

## Places

A place (`a`, `b`, an item of `origins`, `destinations` or `targets`) is one of these:

| Place | Example | How it joins the network |
|---|---|---|
| an edge id | `1234567890123` | that edge |
| a `(lon, lat)` point | `(7.4155, 43.7285)` | `route` with two points: the nearest road within `radius_m`, plus the walk to it. The other calls: the nearest edge. |
| a place name | `"Boulevard Albert 1er"` | `geocode`: the middle of the longest edge of that name in the network, else Nominatim in the box of the network (needs the internet) |

## Calls

| Call | Returns |
|---|---|
| `route(view, a, b, weight="time")` | the fastest route (`weight="length"`: the shortest), a `Route`, or `None` when no route connects `a` and `b` |
| `route_points(view, a, b, weight="time", radius_m=50.0, access_kmh=4.5)` | the route between two points; each point joins its nearest road within `radius_m`. A `Route` with `start` and `end`. |
| `routes(view, origins, destinations, weight="time")` | the route from each origin to the destination at the same place in the list (`zip`), in parallel. A list of `Route` (`None`: not connected). |
| `k_shortest_paths(view, a, b, k=3, weight="time")` | the `k` best routes that use no edge two times, the best first. A list of `Route`. |
| `matrix(view, origins, destinations=None, weight="time", max_cost=None)` | a DataFrame: a row for each origin, a column for each destination. The cells are seconds (`weight="length"`: metres). `NaN`: not connected, more than `max_cost`, or an edge that is not in the graph of the mode (for example a bus-only edge). |
| `isochrones(view, origins, minutes=(5, 10, 15), buffer_m=25.0, weight="time")` | a GeoDataFrame with a polygon for each origin and each time: `origin`, `minutes`, `n_edges`, `area_km2`, `geometry` |
| `accessibility(view, origins, targets, minutes=15, weight="time")` | a DataFrame with a row for each origin: `n_<m>min` (the number of targets within `m` minutes; `minutes` can be a list, then one column for each value), `nearest`, `nearest_s` |
| `geocode(view, name)` | the `(lon, lat)` of a place name |
| `routed_graph(view)` | the SQL relation of the rows of `<mode>.edge_graph` that the mode routes on: the graph of all calls |
| `directions(view, route)` | the turn-by-turn steps of a route |
| `route_multimodal(net, src_node, dst_node, schema="mm", start_mode="walking", end_mode="walking", ...)` | a route across the modes between two nodes. Needs the [`mm` schema](../../concepts/multimodal.md). |
| `route_multimodal_points(net, a, b, ...)` | a route across the modes between two points: the legs of each mode |

Each call that has `weight` also has `missing="time"` (see [Your own cost](#your-own-cost)).

The rows and the columns of a matrix (and the rows of `accessibility`) have the labels of the places when all places
are edge ids or names, else the numbers `0, 1, 2 ...`.

## Your own cost

`weight` sets the cost that a route makes smallest:

| `weight` | The cost of an edge |
|---|---|
| `"time"` (the default) | `cost_s`, the travel time in seconds |
| `"length"` | `length_m` |
| the name of a column, for example `"toll_eur"` | the value of that numeric column of `<mode>.edges`; you can add the column with SQL |
| a pandas Series (or a dict) keyed by `edge_id` | your value for each edge: a toll, CO2, danger, noise |

Your own cost replaces the time of the edge. The turn costs of the edge graph are not added. A cost must be 0 or more.
`missing` sets what an edge with no value (not in the Series, or `NULL` in the column) costs:

| `missing` | An edge with no value |
|---|---|
| `"time"` (the default) | costs its time (`cost_s`) |
| `"skip"` | is not used: no route goes on it |

```python
toll = net.driving.edges().set_index("edge_id").cost_s       # the time of each edge
toll[toll.index.isin(louis_ii)] += 600                        # 10 minutes more on Boulevard Louis II
r = net.driving.route(a, b, weight=toll)
r.weight, r.cost, r.time_s                                    # "cost_s", the cost with the toll, the real time
```

The routing stays in Rust: duckOSM gives the cost of each edge to the router. In `matrix`, the cells are your cost. In
`isochrones` and `accessibility`, a minute is 60 units of your cost. In `route_points`, the access legs from the points
to the roads cost 0 in your cost.

## The Route object

A `Route` is also the dict of the old `route`: `r["edges"]` is the list of edge ids, and `net.map(routes=r)` and
`directions` take it.

| Member | Gives |
|---|---|
| `r.edge_ids` | the edge ids, in the order of the route |
| `r.edges` | a GeoDataFrame of the edges in the order of the route: `seq`, `edge_id` and the columns of `<mode>.edges` |
| `r.cost`, `r.weight` | the cost that the route makes smallest, and its name: `"time"` (then `r.cost` is `r.time_s`), `"length"` (`r.length_m`), the column, the name of your Series, or `"own"` for a dict. With your own cost, `r["cost"]` is the sum of your costs of the edges |
| `r.length_m`, `r.time_s` | the sums of `length_m` and `cost_s` of the edges. A route between two points also includes the access legs: `length_m` includes `start.access_m` and `end.access_m` (the distance from each point to its road), `time_s` includes `access_s` (walking time), and only the used parts of the first and the last edge count |
| `r.geometry` | one shapely line (a `MultiLineString` where the edges do not join) |
| `r.directions()` | the turn-by-turn steps |
| `r.map()` | the route on the roadstyle map of the network |

In a notebook, a `Route` shows a card with the number of edges, the length and the time.

## How the results agree

All calls search the same edge graph as `route`: a node is an edge, an arc is a legal turn, and an arc costs the time
(or the length) of its from-edge. The tests (`tests/test_routing_sdk.py`) check these rules on Monaco:

| Call | Rule |
|---|---|
| `routes` | each route has the edges and the time of `route` of the same pair |
| `k_shortest_paths` | the first route is `route` of the same pair; the costs do not decrease |
| `matrix` | each cell is `time_s` (`length_m`) of `route` of the same two edge ids: from the start of the origin edge to the end of the destination edge |
| `isochrones` | an edge is in the polygon of `m` minutes when its `matrix` cell is at most `m × 60` s |
| `accessibility` | the counts and the nearest target come from `matrix` |

## The algorithms

| Call | Algorithm (Rust, `rust/duckosm-router/src/graph.rs`) |
|---|---|
| `route`, `routes` | bidirectional Dijkstra (the path of networkx on a tie); `routes` runs the pairs in parallel (rayon) |
| `k_shortest_paths` | Yen's algorithm: the first path from bidirectional Dijkstra, the other paths from Dijkstra searches without the used arcs |
| `matrix`, `accessibility` | one Dijkstra search from each origin to all edges, the origins in parallel, not one search for each cell. `max_cost` stops each search early. |
| `isochrones` | one Dijkstra search from each origin, stopped at the largest time. The polygon is the union of the reached edges, each with a buffer of `buffer_m` metres in the UTM zone of the area: not a concave hull, so a park with no road stays out. |

Speed on Monaco (2,844 driving edges, 11,934 walking edges; WSL, 20 threads). The times include the Python calls and
the reads of the database file:

| Call | Time |
|---|---|
| `route`, one pair | 58 ms |
| `routes`, 100 pairs | 110 ms |
| `k_shortest_paths`, `k=3` | 127 ms |
| `matrix`, 100 × 100 edges | 117 ms |
| `matrix`, 200 × 2,844 edges (all) | 158 ms |
| `isochrones`, walking, 1 origin × 3 times | 0.96 s |
| `accessibility`, walking, 11,934 origins × 3 targets | 0.45 s |

```python
net = duckosm.network("Monaco")
r = net.driving.route((7.4155, 43.7285), (7.4335, 43.7468))   # a Route
r.map()                                                       # the route on the map
m = net.driving.matrix(r.edge_ids[:10])                       # 10 x 10 travel times in seconds
iso = net.walking.isochrones([(7.4246, 43.7384)], minutes=[5, 10])
```

The old names of this module stay: `to_networkx`, `to_networkx_nodes`, `write_graph` and `Router` (the Python router,
with `.graph`). The top-level `duckosm.Router` is the Rust router. Guides: [Route](../../guides/route.md),
[duckOSM for routing users](../../compare/for-routing-users.md).
