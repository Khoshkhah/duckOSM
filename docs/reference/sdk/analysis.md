# duckosm.analysis

Street network analysis: the functions of OSMnx 2. Each function takes a `ModeView` (`net.driving`), a `Network` with
one mode, or a path with `mode=`. The counts, the bearings, the components and the nearest searches run in Rust
(`rust/duckosm-core/src/analysis.rs`). Install: `pip install "duckosm[analysis]"`.

| Call | Returns | OSMnx |
|---|---|---|
| `stats(view, area_m2=None)` | A DataFrame with the keys of `basic_stats`, in two columns: `osmnx` and `duckosm` (see below). `area_m2`: the area in m² for the densities; None: no densities, as in OSMnx. Also `net.driving.stats()`. | `basic_stats` |
| `streets_per_node(view)` | A Series (`node_id` → streets at the node): 1 dead end, 2 a road passing through, 3 a T junction, 4 a crossing | `count_streets_per_node` |
| `bearings(view)` | A Series (`edge_id` → compass bearing from the source node to the target node, 0 to 360) | `add_edge_bearings` |
| `orientation_entropy(view, num_bins=36, weight=None)` | The entropy of the street bearings: 3.58 for all directions, 1.39 for a perfect grid. `weight="length"`: a long street counts more. | `orientation_entropy` |
| `plot_orientation(view, num_bins=36, title=None, ax=None)` | The polar histogram of the bearings, `(fig, ax)` | `plot_orientation` |
| `nearest_nodes(view, x, y, return_dist=False)` | The `node_id` of each point (lon, lat); arrays in, arrays out. `return_dist=True`: also the distances in metres. | `nearest_nodes` |
| `nearest_edges(view, x, y, return_dist=False)` | The `edge_id` of each point, by the distance to the line. Two rows of a two-way road are at the same distance: the lower `edge_id` comes. | `nearest_edges` |
| `components(view, strong=True)` | A Series (`node_id` → component; 0 is the largest). `strong=False`: weakly connected. | `strongly_connected_components` (networkx) |
| `largest_component(view, out=None)` | A new `Network` file with only the largest component of the mode | `largest_component` |
| `clip(net, bbox= / polygon= / point=, dist_m=, out=None)` | A new `Network` file with the edges inside the area (`duckosm extract`). An edge that crosses the border stays whole. | `truncate_graph_bbox`, `truncate_graph_polygon` |
| `add_elevation(net, dem=None)` | Adds `ele` to the nodes and `z_from`, `z_to`, `z_vertices` to the edges (`duckosm elevation`) | `add_node_elevations_raster` |
| `grades(view)` | A Series (`edge_id` → `(z_to - z_from) / length_m`). Needs `add_elevation` first. | `add_edge_grades` |
| `sample_points(view, n, seed=None)` | A GeoDataFrame of `n` random points on the streets, with their `edge_id`. A long street gets more points. | `sample_points` |
| `to_osmnx(view)` | The `networkx.MultiDiGraph` of OSMnx. Each edge keeps its `edge_id`. It passes `ox.convert.validate_graph`. | the OSMnx graph |

## The two views of `stats`

| View | Nodes and edges | When to use it |
|---|---|---|
| `osmnx` | A node with exactly 2 neighbouring nodes, and its roads passing through, is not a node; its two edges are one edge. This is the graph of OSMnx (`simplify_graph`). | To compare with OSMnx and with published OSMnx numbers |
| `duckosm` | Every row of `<mode>.nodes` and `<mode>.edges` | To count the rows that your data joins to |

A junction is a node with 3 or more **different neighbouring nodes** of the mode. A two-way road through a node has 4
edge rows there but 2 neighbouring nodes. duckOSM splits a road where another mode connects, where its tags change and
where an OSM way ends. In the `duckosm` view these 2-road nodes count as intersections (OSMnx counts a node with 2 or
more streets); the `osmnx` view removes them.

Monaco, driving (`tests/data/monaco.osm.pbf` with `tests/data/monaco.geojson`):

| Key | `osmnx` | `duckosm` |
|---|--:|--:|
| `n` (nodes) | 844 | 1,769 |
| `m` (edges) | 1,487 | 2,844 |
| `intersection_count` | 637 | 1,562 |
| `street_segment_count` | 1,072 | 1,999 |
| `street_length_total` (m) | 65,812 | 65,900 |
| `streets_per_node_avg` | 2.54 | 2.26 |
| `circuity_avg` | 1.17 | 1.07 |

The numbers of the `osmnx` view are equal to `ox.basic_stats(ox.simplify_graph(G))` for `G = to_osmnx(view)` with
`G.graph["simplified"] = False`; the test
`tests/test_analysis.py` checks each key.

## Not in this module

| OSMnx | duckOSM |
|---|---|
| `graph_to_gdfs` | `net.edges()`, `net.nodes()` |
| `project_graph` | `crs=` on `net.edges()` and `net.nodes()` |
| `add_edge_lengths`, `add_edge_speeds`, `add_edge_travel_times` | the edge columns `length_m`, `maxspeed_kmh`, `cost_s` |
| `simplify_graph` | the build simplifies; the `osmnx` view of `stats` |
| `consolidate_intersections` | later |

Guide: [duckOSM for OSMnx users](../../guides/osmnx-users.md). Page: [duckOSM and OSMnx](../../concepts/osmnx.md).
