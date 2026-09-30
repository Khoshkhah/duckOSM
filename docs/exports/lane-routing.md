# Lane-level routing (experimental)

A route that says **which lane** to be in, not only which road. It runs on a
[GMNS database](gmns.md):

```bash
duckosm gmns monaco.duckdb
duckosm lane-graph monaco_gmns.duckdb       # 2,276 lanes, 3,792 turn + 672 lane-change links
duckosm route-lanes monaco_gmns.duckdb <from> <to> -o route.geojson
```

```text
route: 81 lanes, cost 4646, maneuvers: right turn, right turn, right turn, right turn
```

`<from>` and `<to>` are lane ids, or `edge_id`s (then their first lane). The lane graph links each
lane to the lanes it can turn into (legal turns only) and to the lanes beside it (a lane change).
The cost is metres, plus a penalty for each turn and lane change.

```python
from duckosm import build_lane_graph, route_lanes
build_lane_graph("monaco_gmns.duckdb")
r = route_lanes("monaco_gmns.duckdb", from_lane, to_lane)
r["lanes"], r["cost"], r["maneuvers"], r["geometry"]
```

**Limits:** where OSM has no `turn:lanes` (most roads), any lane may take any legal turn, so a
route can turn from a lane a real driver would leave first. Good for planning, not for lane-accurate
control. Driving only.
