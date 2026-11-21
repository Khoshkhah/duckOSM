"""Write docs/notebooks/09_analysis.ipynb: chapter 9 of the notebook manual, "Street network analysis".
The style comes from scripts/notebook_book.py (rules: docs/design/notebook_book.md).

    python scripts/notebook_chapter9.py docs/notebooks/09_analysis.ipynb"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from notebook_book import IMAGES, Chapter  # noqa: E402

ch = Chapter(9, "09_analysis", "Street network analysis",
             "The numbers of OSMnx, junctions, street directions, components, nearest roads and heights",
             learn="the two views of the statistics; junctions; street bearings and their entropy; connected "
                   "components; the nearest node and edge of a point; a smaller area; random points; heights and "
                   "grades; OSMnx on a duckOSM network",
             time="about 2 minutes", extras="analysis,viz", more_install="\n    %pip install -q osmnx",
             next_=("10_gis", "10. OSM data for GIS", "OSM features, OSM XML and administrative areas"))

ch.md(f"""## What street network analysis measures

Street network analysis describes the form of a road network with numbers. How many junctions does a city have? How
long is a street on average? Do the streets make a grid? Can you drive from each place to each other place? Urban
planners and researchers ask these questions, and the Python library OSMnx answers them for OpenStreetMap data.

The module `duckosm.analysis` gives the functions of OSMnx 2 on a duckOSM network. The counts, the bearings, the
components and the nearest searches run in Rust, on the tables of the file. Each function takes a mode view
(`net.driving`), a network with one mode, or a path with `mode=`.

| Question | duckOSM | OSMnx |
|---|---|---|
| How large is the network? | `stats` | `basic_stats` |
| How many streets meet at each node? | `streets_per_node` | `count_streets_per_node` |
| In which direction does each street go? | `bearings`, `orientation_entropy`, `plot_orientation` | `add_edge_bearings`, `orientation_entropy`, `plot_orientation` |
| Which parts are connected? | `components`, `largest_component` | networkx, `largest_component` |
| Which node or edge is nearest to a point? | `nearest_nodes`, `nearest_edges` | `nearest_nodes`, `nearest_edges` |
| What is in a smaller area? | `clip` | `truncate_graph_bbox`, `truncate_graph_polygon` |
| How steep is each street? | `add_elevation`, `grades` | `add_node_elevations_raster`, `add_edge_grades` |
| Where are random points on the streets? | `sample_points` | `sample_points` |
| Can OSMnx itself work on it? | `to_osmnx` | the OSMnx graph |

The guide [duckOSM for OSMnx users](../compare/for-osmnx-users.md) gives the duckOSM call for each OSMnx call.""")

ch.md(f"""### Two views of one network

duckOSM and OSMnx cut the roads at different places. OSMnx keeps a node only where the streets meet or end
(`simplify_graph`). duckOSM also cuts a road where another mode joins it, where its tags change and where an OSM way
ends, because your data joins to each of these edges by `edge_id`. So a duckOSM network has more nodes and edges than
the OSMnx graph of the same area.

<img src="{IMAGES}/stats-views.svg" alt="The duckosm view keeps every node, and the osmnx view removes the nodes where only two roads meet" width="100%">

`stats` gives each number in two views:

| View | Nodes and edges | Use it |
|---|---|---|
| `osmnx` | A node with exactly 2 neighboring nodes, and its roads passing through, is not a node. Its two edges are one edge. | to compare with OSMnx and with published OSMnx numbers |
| `duckosm` | every row of `<mode>.nodes` and `<mode>.edges` | to count the rows that your data joins to |

> **Key idea:** the `osmnx` view gives the numbers that OSMnx gives for the same roads. The `duckosm` view counts
> the rows of the file. Use the `osmnx` view to compare cities, and the `duckosm` view to plan your own data.""")

ch.md("""## Open the network of chapter 1

Chapter 1 builds Monaco with three modes (driving, walking and cycling) and saves it as `monaco.duckdb`. This chapter opens that file with `duckosm.open`: it builds nothing. The analysis uses the driving network. `an` is the short name of the module.""")
ch.code("""from duckosm import analysis as an                                           # the module of this chapter
net = duckosm.open("monaco.duckdb")    # the network of chapter 1: Monaco, three modes
drive = net.driving                                                         # the driving mode
drive                                                                       # the card""")

ch.md("""## Statistics: `stats`

`stats(view)` gives the numbers of OSMnx `basic_stats` as a DataFrame: one row for each key, one column for each view.
`net.driving.stats()` gives the same table.

| Key | What it is |
|---|---|
| `n`, `m` | the number of nodes and of edges (a two-way road is two edges) |
| `k_avg` | the average degree: 2 × `m` / `n` |
| `edge_length_total`, `edge_length_avg` | the sum and the average of the edge lengths, in metres |
| `streets_per_node_avg` | the average number of streets at a node |
| `intersection_count` | the nodes with 2 or more streets (OSMnx's rule) |
| `street_length_total`, `street_segment_count`, `street_length_avg` | the same as the edge keys, with a two-way road counted once |
| `circuity_avg` | the length of the streets divided by the straight distance between their ends |
| `self_loop_proportion` | the share of edges that start and end at the same node |""")
ch.code("""s = an.stats(drive)                                                       # both views, the keys of OSMnx basic_stats
s.drop(["streets_per_node_counts", "streets_per_node_proportions"])       # the numbers (the two dicts follow)""")
ch.md("""**What the output shows.** The `duckosm` view counts 1,768 nodes and 2,840 edges: the rows of the file. The `osmnx` view counts 843 nodes and 1,484 edges, about half. The lengths are almost the same (65.8 km and 65.9 km of streets), because the same roads are in both views. So the average street is 61 m in the `osmnx` view and 33 m in the `duckosm` view. The 2-road nodes count as intersections in the `duckosm` view: 1,561, not 636. The circuity is 1.17 in the `osmnx` view: the streets of Monaco follow the hills and the coast.""")

ch.md("""### The densities: `area_m2`

OSMnx gives the densities only when you give the area. `area_m2=` is the area in square metres. Here it is the area of
the boundary of the network (`net.boundary`), measured in its UTM zone. The node and intersection densities are counts per km². The edge and street densities are metres per km².""")
ch.code("""b = net.boundary                                                    # the boundary polygon (a GeoDataFrame)
area_m2 = b.to_crs(b.estimate_utm_crs()).area.sum()                 # its area in m2
print(f"area: {area_m2 / 1e6:.2f} km2")
an.stats(drive, area_m2=area_m2).loc[["node_density_km", "intersection_density_km", "edge_density_km",
                                      "street_density_km"]].round(1)          # the four densities""")
ch.md("""The boundary of the city has 2.39 km². In the `osmnx` view Monaco has 353 nodes and 266 intersections per km², and 27,517 m of streets per km². The street density is almost the same in the two views. The node density of the `duckosm` view is twice as large.""")

ch.md("""### The streets at the nodes

The keys `streets_per_node_counts` and `streets_per_node_proportions` are dicts: the number of nodes (or their share)
for each count of streets.""")
ch.code("""s.loc["streets_per_node_counts"].apply(lambda d: dict(list(d.items())[1:]))    # nodes for each count of streets (from 1)""")
ch.md("""**What the output shows.** Both views have 207 dead ends, about 566 T junctions, 44 crossings and 2 nodes with five streets. They differ only in the nodes with two streets: 948 in the `duckosm` view, 24 in the `osmnx` view. OSMnx keeps a 2-street node only where it must, for example where a loop starts and ends at the same node.""")

ch.md("""## Junctions: `streets_per_node`

`streets_per_node(view)` gives the number of streets at each node of the `duckosm` view, as a Series with the
`node_id` as its index. A two-way road counts once.

| Streets | The node is |
|---|---|
| 1 | a dead end |
| 2 | a road passing through (or a place where the tags change) |
| 3 | a T junction |
| 4 | a crossing |

The chart counts the nodes of each class. The map shows the dead ends, the T junctions and the crossings.""")
ch.code("""import matplotlib.pyplot as plt

spn = an.streets_per_node(drive)                                             # node_id -> streets
nodes = net.driving.nodes().merge(spn.rename("streets").reset_index(), on="node_id")  # the nodes with their count
fig, (a, b) = plt.subplots(1, 2, figsize=(12, 5), gridspec_kw={"width_ratios": [1, 2]})    # a chart and a map
spn.value_counts().sort_index().plot.bar(ax=a, color="#1f6feb", xlabel="streets at the node", ylabel="nodes")    # the counts
net.driving.edges().plot(ax=b, color="#cccccc", linewidth=0.6)                       # the roads in gray
for k, color, label in [(1, "#d1242f", "dead end"), (3, "#bf8700", "T junction"), (4, "#2ea043", "crossing")]:    # 3 classes
    nodes[nodes.streets == k].plot(ax=b, color=color, markersize=6, label=label)    # the nodes of the class
b.legend(loc="lower right"); b.set_axis_off(); plt.tight_layout(); plt.show()   # the picture""")
ch.md("""**What the output shows.** The chart shows the classes of the `duckosm` view: 948 nodes with two streets, 567 T junctions, 207 dead ends and 44 crossings. Monaco has few crossings: its streets meet mostly at T junctions, on the slopes. Many dead ends are at the edge of the city, where the boundary cuts the streets, and at the entries of garages.""")

ch.md("""## Street directions

### The bearing of each edge: `bearings`

`bearings(view)` gives the compass bearing of each edge, from its source node to its target node: 0 is north, 90 is
east, 180 is south and 270 is west. The result is a Series with the `edge_id` as its index. A self-loop has `NaN`.""")
ch.code("""brg = an.bearings(drive)                    # edge_id -> bearing in degrees
brg.describe().round(1)                     # the spread of the bearings""")

ch.md("""### Grid or not: `orientation_entropy`

`orientation_entropy(view)` puts the bearings into `num_bins` bins (default 36 bins of 10 degrees) and gives the
entropy of the bins. The entropy is large when the streets go in all directions, and small when they go in few.

| Network | Entropy |
|---|---|
| the streets go in all directions equally | ln(36) = 3.58 |
| a perfect grid (four directions) | ln(4) = 1.39 |

`weight="length"` lets a long street count more than a short street.""")
ch.code("""print("orientation entropy:", round(an.orientation_entropy(drive), 3))                        # each street counts once
print("weighted by length:", round(an.orientation_entropy(drive, weight="length"), 3))         # a long street counts more""")
ch.md("""The entropy of Monaco is 3.528, near the largest value of 3.58: the streets go in almost all directions. With `weight="length"` it is 3.48: the long streets have a few more common directions, along the coast from southwest to northeast.""")

ch.md("""### The polar histogram: `plot_orientation`

`plot_orientation(view)` draws the bearings as a polar histogram, as OSMnx does. The area of each bar shows its share.
It gives `(fig, ax)`, so you can put it into your own figure with `ax=` and give it a `title=`.""")
ch.code("""fig, ax = an.plot_orientation(drive, title="Monaco, driving")          # the polar histogram
plt.show()                                                             # the picture""")
ch.md("""The bars are of almost equal size in all directions. The longest bars point to the northeast and to the southwest, along the coast. A city with a grid shows four long bars at right angles.""")

ch.md("""## Connected parts

### The component of each node: `components`

A **strongly connected component** is a set of nodes in which you can go from each node to each other node, along the
directions of the edges. `components(view)` gives the component of each node, as a Series with the `node_id` as its
index. Component 0 is the largest, 1 is the next, and so on. `strong=False` gives the weakly connected components: the
directions of the edges do not count.""")
ch.code("""comp = an.components(drive)                                               # node_id -> part (0 = the largest)
print(f"{comp.nunique()} parts; the largest has {(comp == 0).sum():,} of {len(comp):,} nodes")    # the sizes
weak = an.components(drive, strong=False)                                 # the directions do not count
print(f"weakly connected: {weak.nunique()} parts; the largest has {(weak == 0).sum():,} nodes")""")
ch.md("""**What the output shows.** The driving network has 122 strongly connected parts. The largest has 1,600 of the 1,768 nodes. The other 121 parts are small: most are one node, for example the end of a one-way garage ramp that only leaves the network. When the directions do not count, the network has only 16 parts, and the largest has 1,718 nodes.""")

ch.md("""### The largest part: `largest_component`

`largest_component(view, out=...)` writes a new network file with only the largest component of the mode. The mode
keeps the nodes, the edges and the turn tables of that component. The other modes of the file do not change. Use it
before an analysis that needs a connected graph, for example a matrix of all pairs. The map shows the edges outside
the largest part in orange.""")
ch.code("""big = an.largest_component(drive, out="data/monaco_largest.duckdb")       # a new network file with the largest part
edges = net.driving.edges()                                                       # all edges
print(len(big.driving.edges()), "of", len(edges), "edges are in the largest part")
net.driving.plot(highlight=edges.edge_id[~edges.edge_id.isin(big.driving.edges().edge_id)])    # the edges outside the largest part""")
ch.md("""The largest part has 2,622 of the 2,840 edges. The 218 orange edges are outside it: garage ramps, private service roads, and short roads at the boundary of the city that you can enter but not leave (or leave but not enter).""")

ch.md("""## From points to the network

`nearest_nodes(view, x, y)` and `nearest_edges(view, x, y)` join points to the network: addresses, sensors, GPS
points or bus stops. `x` is the longitudes and `y` the latitudes, as numbers or arrays. The result has the same shape:
an array for arrays, a number for a number. `return_dist=True` also gives the distances in metres.

`nearest_edges` measures the distance to the line of the edge. The two edges of a two-way road are at the same
distance, so the edge with the lower `edge_id` comes.

Here the points are ten random points in the box of the network (black). The edge of each point is orange.""")
ch.code("""import numpy as np

rng = np.random.default_rng(1)                                    # random points that do not change
w, south, e, north = net.driving.edges().total_bounds                     # the box of the network
xs, ys = rng.uniform(w, e, 10), rng.uniform(south, north, 10)     # 10 points (lon, lat)
ids, dist = an.nearest_edges(drive, xs, ys, return_dist=True)     # the nearest edge of each point
node_ids = an.nearest_nodes(drive, xs, ys)                        # the nearest node of each point
print({"edge_id": ids[:3], "distance_m": dist[:3].round(1), "node_id": node_ids[:3]})    # the first three
ax = plt.subplots(figsize=(7, 7))[1]                              # a new picture
net.driving.plot(highlight=ids, ax=ax)                                    # the edges of the points in orange
ax.scatter(xs, ys, color="black", s=20, zorder=3); plt.show()     # the points""")
ch.md("""The arrays have the `edge_id` and the `node_id` of each point, and the distance to the edge in metres. The first two points are 375 m and 618 m from a road: they are in the sea, in the box of the network but outside the city. The third point is 12 m from its road. The map shows each point and its edge.

> **Tip:** to put your own data on the network, join the `edge_id` of `nearest_edges` to your rows. Chapter
> [4. Join your data by edge_id](04_join_by_edge_id.md) shows the join.""")

ch.md("""## A smaller area: `clip`

`clip(net, ...)` writes a new network file with the edges inside an area. Give one area:

| Argument | The area |
|---|---|
| `bbox=(west, south, east, north)` | a box in degrees |
| `polygon=` | a shapely polygon or a GeoJSON |
| `point=(lon, lat)` with `dist_m=` | the circle of `dist_m` metres around the point |

An edge that crosses the border stays whole, as in a build (chapter [1. Get and build an area](01_area_to_network.md)).
Here the area is 500 m around the casino square.""")
ch.code("""small = an.clip(net, point=(7.4280, 43.7393), dist_m=500, out="data/monaco_casino.duckdb")    # a new network file
print(len(small.driving.edges()), "of", len(net.driving.edges()), "edges")      # the size
small.driving.plot()                                                    # its roads by class""")
ch.md("""The circle of 500 m around the casino square holds 702 of the 2,840 edges. The new file is a full network, with its own nodes and turn tables, so every call of this chapter and of the routing chapters works on it.""")

ch.md("""## Random points on the roads: `sample_points`

`sample_points(view, n, seed=...)` gives `n` random points on the streets, as a GeoDataFrame with the `edge_id` of
each point. A street is chosen with a chance that follows its length, so a longer street gets more points. The two
edges of a two-way road count as one street. The same `seed` gives the same points. Use the points as random trip
starts or to test a sensor plan.""")
ch.code("""pts = an.sample_points(drive, 200, seed=1)                  # a GeoDataFrame of points with edge_id
print(len(pts), "points on", pts.edge_id.nunique(), "edges")
ax = plt.subplots(figsize=(7, 7))[1]                         # a new picture
net.driving.plot(ax=ax)                                              # the roads
pts.plot(ax=ax, color="#2ea043", markersize=8); plt.show()   # the points""")

ch.md("""## Heights and grades

### Add the ground height: `add_elevation`

`add_elevation(net)` adds the ground height to the nodes (`ele`) and to the edges (`z_from`, `z_to`, `z_vertices`). It
changes the file in place. `dem=` takes a DEM file (GeoTIFF) of your own. Without it, the call finds a source for the
area (`source="auto"`) and downloads its tiles into the cache. For Monaco the source is Copernicus.
[Add elevation](../guides/elevation.md) gives the sources and the options.""")
ch.code("""an.add_elevation(drive)                                     # the heights of a DEM source for the area
net.driving.nodes()["ele"].describe().round(1)                      # the heights of the nodes in metres""")
ch.md("""The progress lines show the source (Copernicus, one tile of 12.6 MB) and the 1,768 nodes that got a height. The heights go from -0.5 m at the sea to 336 m, and the median is 34 m. Monaco rises fast from the coast.""")

ch.md("""### The grade of each edge: `grades`

`grades(view)` gives `(z_to - z_from) / length_m` for each edge, as a Series with the `edge_id` as its index. 0.05 is
5 % up, and -0.05 is 5 % down. It needs `add_elevation` first.""")
ch.code("""g = an.grades(drive)                                                       # edge_id -> grade
e = net.driving.edges().set_index("edge_id")                                       # the edge columns
ok = g[(e.length_m >= 50) & e.tunnel.isna() & e.bridge.isna()]             # edges of 50 m or more, on the ground
print(f"all edges: {g.abs().max():.0%} the steepest; on the ground and 50 m or more: {ok.abs().max():.0%}")
ok.abs().describe(percentiles=[0.5, 0.9]).round(3)                          # the spread of the grades""")
ch.md("""Over all edges the steepest grade is 880 %: a wrong value on a tunnel or a short edge. For the 471 edges of 50 m or more that are not tunnels or bridges, the steepest is 35 %, the median is 5.1 % and 90 % of the edges are below 15.8 %. Monaco is a steep city.

> **Warning:** a DEM gives the height of the ground. A tunnel or a bridge is not on the ground, so its grade from the
> DEM is wrong. A short edge on a slope can also get a large grade from the cells of the DEM. Remove these edges
> before you use the grades.""")

ch.md("""## OSMnx on the network: `to_osmnx`

`to_osmnx(view)` gives the mode as the `networkx.MultiDiGraph` of OSMnx. Each OSMnx function works on it: the graph
passes `ox.convert.validate_graph`. The node keys are the `node_id`. Each edge keeps its `edge_id` and has the OSMnx
names: `osmid`, `length`, `geometry`, `highway`, `name`, `oneway`, `lanes`, `speed_kph` and `travel_time`.

Here OSMnx finds a route on the largest part (`big`, above) with its own `shortest_path`, and draws it.""")
ch.code("""import osmnx as ox

G = an.to_osmnx(big.driving)                                                 # the largest part as a networkx MultiDiGraph
print(G)                                                                     # the size of the graph
orig, dest = an.nearest_nodes(big.driving, [7.4155, 7.4370], [43.7310, 43.7480])  # two nodes (duckOSM finds them)
route = ox.routing.shortest_path(G, orig, dest, weight="travel_time")        # OSMnx's route
fig, ax = ox.plot.plot_graph_route(G, route, route_color="#d1242f", node_size=0, bgcolor="white",
                                   edge_color="#999999", figsize=(7, 7))    # OSMnx's picture""")

ch.md("""### The osmnx view equals OSMnx

The `osmnx` view of `stats` gives the numbers of `ox.basic_stats` on the simplified OSMnx graph. To simplify the
graph with OSMnx, set `G.graph["simplified"] = False` first: `to_osmnx` marks the graph as simplified, because each
edge has its line. The tests of duckOSM check each key.""")
ch.code("""H = an.to_osmnx(drive)                                     # the whole driving network
H.graph["simplified"] = False                              # let OSMnx simplify it
ours = an.stats(drive)["osmnx"]                            # the osmnx view of duckOSM
theirs = ox.basic_stats(ox.simplify_graph(H))              # OSMnx itself
{k: (ours[k], theirs[k]) for k in ("n", "m", "intersection_count", "street_segment_count")}""")
ch.md("""The graph of the largest part has 1,600 nodes and 2,622 edges, the same as `big`. OSMnx found a route on it and drew it. For the whole network, the `osmnx` view of duckOSM and `ox.basic_stats` of OSMnx give the same numbers: 843 nodes, 1,484 edges, 636 intersections and 1,070 street segments.""")

ch.md("""## Try it yourself

**1.** Give the statistics of the walking network of Monaco, and compare `streets_per_node_avg` with driving.

<details><summary>Show the answer</summary>

```python
walk = duckosm.open("monaco.duckdb")
an.stats(walk.walking).loc[["n", "m", "streets_per_node_avg"]]
```
</details>

**2.** Count the dead ends of the network of 500 m around the casino square (`small`, above).

<details><summary>Show the answer</summary>

```python
(an.streets_per_node(small.driving) == 1).sum()
```
</details>

**3.** Clip the network to a box around the port, and give its orientation entropy.

<details><summary>Show the answer</summary>

```python
port = an.clip(net, bbox=(7.418, 43.730, 7.430, 43.738), out="data/port.duckdb")
an.orientation_entropy(port.driving)
```
</details>

**4.** Draw the edges whose grade is more than 10 % (on the ground, 50 m or more).

<details><summary>Show the answer</summary>

```python
steep = ok[ok.abs() > 0.10].index
net.plot(highlight=list(steep))
```
</details>""")

ch.glossary([
    ("street network analysis", "the description of the form of a road network with numbers"),
    ("view", "a way to count the nodes and edges: `osmnx` (2-road nodes removed) or `duckosm` (every row)"),
    ("intersection", "a node where 2 or more streets meet (OSMnx's rule)"),
    ("dead end", "a node with one street"),
    ("circuity", "the length of a street divided by the straight distance between its ends"),
    ("bearing", "the compass direction of an edge, from 0 (north) to 360 degrees"),
    ("orientation entropy", "a number for the spread of the street directions: 1.39 for a grid, 3.58 for all directions"),
    ("strongly connected component", "a set of nodes where you can go from each node to each other node along the edge directions"),
    ("DEM", "a digital elevation model: a raster of ground heights"),
    ("grade", "the change of height of an edge divided by its length"),
])

ch.summary([
    ("`an.stats(view, area_m2=...)`", "the numbers of OSMnx `basic_stats`, in two views"),
    ("`an.streets_per_node(view)`", "the streets at each node"),
    ("`an.bearings(view)`", "the compass bearing of each edge"),
    ("`an.orientation_entropy(view, weight=...)`", "the entropy of the street directions"),
    ("`an.plot_orientation(view, title=...)`", "the polar histogram of the bearings"),
    ("`an.components(view, strong=...)`", "the connected component of each node"),
    ("`an.largest_component(view, out=...)`", "a new file with the largest component"),
    ("`an.nearest_nodes(view, x, y)`, `an.nearest_edges(view, x, y)`", "the nearest node and edge of each point"),
    ("`an.clip(net, point=..., dist_m=...)`", "a new file with the edges inside an area"),
    ("`an.sample_points(view, n, seed=...)`", "random points on the streets"),
    ("`an.add_elevation(net, dem=...)`, `an.grades(view)`", "the heights, and the grade of each edge"),
    ("`an.to_osmnx(view)`", "the network as an OSMnx graph"),
], reference="[duckosm.analysis](../reference/sdk/analysis.md)")

ch.write(sys.argv[1])
