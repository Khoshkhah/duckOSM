# Lane-level routing (experimental)

A route that says **which lane** to be in, not only which road. It runs on a
[GMNS database](gmns.md):

```bash
duckosm gmns monaco.duckdb
duckosm lane-graph monaco_gmns.duckdb       # 3,182 lanes, 4,999 turn + 834 lane-change links
duckosm route-lanes monaco_gmns.duckdb <from> <to> -o route.geojson
```

```text
route: 112 lanes, cost 4146, maneuvers: right turn, right turn, right turn, right turn, U-turn
```

`<from>` and `<to>` are lane ids (`<edge_id>_<lane number>`, e.g. `4737533508550016661_1`), or
`edge_id`s (then their lane 1). An unknown id gives "no lane route", as an unreachable lane does. The
lane graph links each lane to the lanes it can turn into (legal turns only) and to the lanes beside
it (a lane change). The cost is metres, plus a penalty for each turn and lane change.
`lane-graph` saves the graph in the GMNS database (`lane_driving.lane_edges`) so that repeated
routes are faster; `route-lanes` also works without it. Needs `pip install "duckosm[routing]"`.

```python
from duckosm import build_lane_graph, route_lanes
build_lane_graph("monaco_gmns.duckdb")
r = route_lanes("monaco_gmns.duckdb", from_lane, to_lane)
r["lanes"], r["cost"], r["maneuvers"], r["geometry"]
```

**Limits:** where OSM has no `turn:lanes` (most roads), any lane may take any legal turn, so a
route can turn from a lane a real driver would leave first. Good for planning, not for lane-accurate
control. Driving by default; `-m cycling` runs too, but cycling roads mostly have one lane.
