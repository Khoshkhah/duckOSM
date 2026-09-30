# Lane-level routing → lane graph + `route_lanes`

`duckosm lane-graph` / `route-lanes` (`build_lane_graph` / `route_lanes`). A **lane graph**
(lane→lane adjacency) and a router over it, so a route can be planned **per lane** (which lane to
be in, when to change), not just road to road.

## The idea — mirror `edge_graph` → `Router`, at lane resolution

duckOSM already routes roads via `edge_graph` (edge→edge adjacency) + a Dijkstra `Router`. Lane routing
is the same pattern one level down:

| road routing | lane routing |
|---|---|
| node = **edge** (`edge_id`) | node = **lane** (`lane_id`) |
| edge = `edge_graph` (edge→edge turn) | edge = **turn** (lane→lane) + **lane-change** (adjacent lanes) |
| cost = edge length / time | cost = lane length + turn/lane-change penalties |

Compact: ~3,193 lane nodes on Södermalm (vs 70k micro cells), so a route is a clean **lane sequence**.

## Building the lane graph (from a GMNS db)

Nodes are `gmns_<mode>.lane` rows. Two edge kinds:

1. **turn** (lane → lane across a junction) — from `gmns_<mode>.movement` (`ib_link_id` → `ob_link_id`,
   restrictions already honoured). For each movement, connect the inbound link's lane(s) to the
   outbound link's lane(s):
   - with `turn:lanes` (`start_ib_lane`/`end_ib_lane`): the specified inbound lanes → the outbound lane;
   - without (common): **all inbound lanes → all outbound lanes** (permissive) — over-connects, but keeps
     every legal path reachable; lane discipline is then modelled by lane-change edges. *(Documented
     simplification — the fidelity ceiling where OSM lacks `turn:lanes`.)*
   - cost = downstream lane length + a turn penalty (by `movement.type`: left/right/uturn heavier).
2. **lane-change** (lane ↔ adjacent lane, same link) — lanes of a link with `lane_num` differing by 1,
   both directions. cost = lane length + a lane-change penalty (discourages needless weaving).

Written as `lane_<mode>.lane_edges(from_lane, to_lane, kind, cost)` in the GMNS db (queryable, and the
router reads it). `build_lane_graph(gmns_db, mode)` → counts.

## Routing — `route_lanes`

```python
from duckosm import route_lanes
path = route_lanes("monaco_gmns.duckdb", from_lane, to_lane, mode="driving")
# -> {"lanes": [lane_id, …], "cost": …, "geometry": "LINESTRING(…)", "maneuvers": ["change L→R", "left turn", …]}
```

- Dijkstra over `lane_edges` (reuse the existing shortest-path machinery / networkx), from a source lane
  to a target lane. Convenience: accept a source/target **edge_id** (or point) → pick a representative
  lane → route → return the lane sequence.
- **Output**: the ordered `lane_id`s, total cost, the **concatenated lane-centerline geometry** (drive
  this / draw this), and a human-readable **maneuver list** (turns + lane-changes) derived from the edge
  kinds — the thing that makes it "lane-level".

## Scope

**v1:** `build_lane_graph` (turn + lane-change edges, penalised costs) + `route_lanes` (lane sequence +
geometry + maneuvers), driving mode. Enough to plan and draw a lane-level route.

**Out of scope → later:** per-lane turn assignment beyond `turn:lanes` (needs the data), lane-change
*feasibility windows* (length needed to change), multi-modal lane routing, and cell-resolution routing
(the `micro` network already exists if sub-lane detail is ever needed).

## Fidelity

Lane **connectivity structure** is real (turns honour restrictions; lane-changes are geometric
adjacency). Lane-to-lane **turn assignment** is permissive where `turn:lanes` is untagged (~97% here),
so the router may allow a turn from a lane a real driver would change out of first — acceptable for
lane-level *planning*, not lane-accurate *control*. Costs/penalties are heuristic defaults, tunable.

## Tests

Build a GMNS db with a 2-lane link A → {B, C} (a turn choice): `lane_edges` has lane-change edges
between A's two lanes and turn edges A-lanes → B/C-lanes; `route_lanes` from an A-lane to a C-lane
returns a lane sequence ending in C with a "right turn" maneuver; an unreachable target returns no path;
costs are positive and turn/lane-change penalised.
