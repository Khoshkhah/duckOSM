# duckosm.routing

Routing, accessibility and logistics. Each function takes a `Network`, a `ModeView` (`net.walking`), a path or a DuckDB
connection. With a path or a connection, the mode default is `"driving"`.

| Call | Returns | Today |
|---|---|---|
| `route(net, a, b, mode=None, weight="time")` | the fastest route (`weight="length"`: the shortest) from `a` to `b`: two edge ids, or two `(lon, lat)` points. A dict `{edges, time_s, length_m, path}`, or `None`. | yes |
| `route_points(net, a, b, mode=None, weight="time", radius_m=50.0, access_kmh=4.5)` | the route between two `(lon, lat)` points; each point joins its nearest road within `radius_m` | yes |
| `route_multimodal(net, src_node, dst_node, schema="mm", start_mode="walking", end_mode="walking", ...)` | a route across the modes between two nodes. Needs the [`mm` schema](../../concepts/multimodal.md). | yes |
| `route_multimodal_points(net, a, b, ...)` | a route across the modes between two points: the legs of each mode | yes |
| `directions(net, route, mode=None)` | the turn-by-turn steps of a route | yes |
| `routes(view, origins, destinations)` | many routes, in parallel | PR 3 |
| `k_shortest_paths(view, a, b, k)` | other routes | PR 3 |
| `matrix(view, origins, destinations=None)` | a travel time or distance matrix | PR 3 |
| `isochrones(view, origins, minutes=[5, 10, 15])` | polygons of the area reached | PR 3 |
| `accessibility(view, origins, targets, minutes)` | the targets reached from each origin | PR 3 |
| `Route` | a route object with `.edges`, `.geometry`, `.directions()`, `.map()` | PR 3 |

```python
r = net.walking.route((7.4155, 43.7285), (7.4335, 43.7468))   # = duckosm.routing.route(net.walking, ...)
net.walking.map(routes=r)                                     # the route on the map
```

The old names of this module stay: `to_networkx`, `to_networkx_nodes`, `write_graph` and `Router` (the Python router,
with `.graph`). The top-level `duckosm.Router` is the Rust router. Guide: [Route](../../guides/route.md).
