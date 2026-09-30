# Access tags: forbidden roads out, private roads visible but not routable

**Status:** implemented 2026-09-30 (approved by Kaveh the same day).

## Today

The road filters ignore most access tags: `access=private`, `motor_vehicle=no` or `foot=no`
roads stay in driving and walking, so routes and exports use them. Monaco: 108 driving and 242
walking edges. (Cycling already drops `access=private/no` and `bicycle=no`.)

## Rule

For each mode, the **most specific** access tag decides (OSM's hierarchy):

| Mode | Tags, most specific first |
|---|---|
| driving | `motorcar`, `motor_vehicle`, `vehicle`, `access` |
| walking | `foot`, `access` |
| cycling | `bicycle`, `vehicle`, `access` |

- **Forbidden** (`no`; for driving also `agricultural`, `forestry`, `emergency`, `psv`): the way is
  **not in that mode** at all. It stays in the other modes that may use it (a bus-only road is in
  walking and cycling) and in `raw.ways`.
- **Private** (`private`): the way is **visible but never routable** in that mode (below).
- Anything else (`yes`, `destination`, `delivery`, `permissive`, …): a normal road.

A more specific tag can reopen a road: `access=private` + `motor_vehicle=yes` is a normal road for
driving.

## Private roads: their own table

A private road is built like any other (split at junctions, same `edge_id` formula), then moved
from `<mode>.edges` to **`<mode>.private_edges`** (same columns) before the graph of legal turns is
made. So:

- `edges`, `edge_graph`, `route()`, `route_multimodal`, networkx and **every export** (SUMO,
  MATSim, GMNS, GIS, OpenDRIVE…) see only roads you may use, with no code change and no filter to
  forget.
- A public road reachable only through a private one ends up cut off, and the component filter
  drops it (correct: you can't legally get there).
- **The maps draw them:** `duckosm viz` and `route-map` add `private_edges`, greyed out, with
  `private: yes` in the popup. The route planner never snaps a marker to them.
- Querying them: `SELECT * FROM driving.private_edges`.

## Checks

- Tests: the hierarchy per mode (forbidden out, private moved, reopened by a more specific tag);
  `private_edges` has no rows in `edge_graph`; exports don't contain private `edge_id`s.
- Monaco: counts before / after per mode; a private road visible (grey) on the viz and route maps,
  and no route through it.
- Docs: What each network contains, the data dictionary, the Query guide.
