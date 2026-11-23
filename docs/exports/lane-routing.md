# Lane-level routing (experimental)

A route that says **which lane** to be in, not only which road, with your OSM fixes. It runs on the GMNS database of
[`duckosm gmns`](gmns.md).

| Reference and format | |
|---|---|
| Format | the lane graph as the table `lane_driving.lane_edges` in the GMNS database. A route as GeoJSON |
| Reference tool | none |
| Check | the tests on Monaco. [How each output is checked](../concepts/checks.md) |

```bash
duckosm gmns monaco.duckdb                                            # first: the lanes and the turns
duckosm lane-graph monaco_gmns.duckdb                                 # the lane graph
duckosm route-lanes monaco_gmns.duckdb <from> <to> -o route.geojson   # one route
```

`<from>` and `<to>` are lane ids (`<edge_id>_<lane number>`), or `edge_id`s: then lane 1 of the edge.

## The lane graph

| Link | Joins |
|---|---|
| turn | a lane to each lane that it can turn into, legal turns only |
| lane change | a lane to the lane beside it |

The cost is the length in metres, plus a penalty for each turn and each lane change. A route gives its lanes, its
cost, its maneuvers (right turn, merge, fork, lane change, U-turn) and its shape.

## Lane graph and the GMNS micro network

Both are networks of lanes, from the same GMNS lanes and turns. They differ in the size of a node and in their use:

| | Lane graph (`lane-graph`) | GMNS micro (`gmns --micro`) |
|---|---|---|
| A node is | one whole lane of a road | the start of a cell of about 7 m of a lane |
| A link is | a turn into another lane, or a lane change | a cell, a lane change between cells side by side, or a turn |
| A lane change can be | anywhere along the lane | at each cell |
| Use it for | routing: which lane to be in | simulation: where each vehicle is on its lane |
| Size | small: one node for each lane | large: one node for each 7 m of each lane |

## Speed and memory

| Area (source .osm.pbf) | Time | Memory |
|---|---|---|
| Monaco (0.7 MB) | 0.04 s | 25 MB |
| Kentucky (148 MB) | not run: `duckosm gmns` stops on Kentucky | |
| France (5.1 GB) | not run | |

## Limits

Where OSM has no `turn:lanes` (most roads), any lane can take any legal turn. So a route can turn from a lane that a
real driver leaves first. Use it for planning, not for lane-accurate control.
