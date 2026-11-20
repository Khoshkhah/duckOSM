# duckosm.analysis

Street network analysis: the functions of OSMnx 2.1. The module comes in PR 2 of the design; it is empty today.

| Call | OSMnx | Today |
|---|---|---|
| `stats(view)` | `basic_stats` | PR 2 |
| `bearings(view)`, `orientation_entropy(view)`, `plot_orientation(view)` | the `bearing` module, `plot_orientation` | PR 2 |
| `nearest_nodes(view, x, y)`, `nearest_edges(view, x, y)` | `nearest_nodes`, `nearest_edges` | PR 2 |
| `largest_component(view)` | `largest_component` | PR 2 |
| `clip(net, bbox= / polygon= / point=, dist_m=)` | `truncate_graph_*` | PR 2 |
| `add_elevation(net, dem=None)` | the `elevation` module | PR 2 (today: `duckosm.to_elevation`) |
| `sample_points(view, n)` | `sample_points` | PR 2 |
| `to_osmnx(view)` | the OSMnx graph | PR 2 |
| `consolidate_intersections(view, tolerance_m)` | `consolidate_intersections` | PR 2 |

What OSMnx calls `graph_to_gdfs` is `net.edges()` and `net.nodes()`; `project_graph` is `crs=` on them; the edges have
`length_m`, `maxspeed_kmh` and `cost_s`. Page: [duckOSM and OSMnx](../../concepts/osmnx.md).
