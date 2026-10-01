# Routing between two points: off-road access and exact partial edges

**Status:** approved by Kaveh 2026-10-01; duckOSM part implemented (`src/duckosm/point_routing.py`,
`tests/test_point_routing.py`); mapstyle's planner page next. Decided with Kaveh: the
radius is a parameter (a control in the planner page); the off-road leg costs walking time; duckOSM
is the reference and mapstyle's planner page mirrors it, so the two can be compared one to one.

## Problem

Today a trip runs from a **whole edge to a whole edge**:

- `route(con, from_edge, to_edge)` sums `cost_s` / `length_m` of **every** edge on the path,
  the first and the last in full. A trip that starts in the middle of a 300 m edge pays 300 m; one
  that starts and ends on the same edge pays the whole edge.
- The route planner (`route_map.py`, now mapstyle's `planner.html`) snaps each marker to the
  nearest road, however far away it is (a marker in the sea finds a road 2 km off), then routes
  edge to edge.
- `route()` builds its graph from `edge_graph`, so an edge with no turn in or out (an isolated
  stub) can't be an end at all ("edge id not in the routing graph").

Kaveh, 2026-10-01: "we should set a maximum radius for a marker that can be off the road (it must
have a special cost), the same for the destination. Also the distance or timing must be accurate:
if the user uses half of an edge, the cost must be half of that edge."

## The rules

1. **A point joins the network at its nearest road point within the radius.** Each marker is
   projected onto the edges of the mode within `radius_m` of it. Each candidate is
   `(edge, fraction, access_m)`: the fraction of the edge's length at the projected point (0 at
   its start, 1 at its end), and the straight distance from the marker to that point. No edge
   within the radius: no route, "no road within R m".
   **The straight walk may not cross another road** (Kaveh, 2026-10-01: "it can't jump from
   roads"): an edge of the mode (you'd have joined that one), or any road for cars, in any mode (no
   walking straight across a main road). Footways and paths may be crossed when driving, so a
   sidewalk between a building and its street doesn't block the car. A road met within 0.5 m of
   the road point only touches the walk (a junction there), so it doesn't count.
2. **The off-road leg costs walking time**: `access_m / access_speed` (default 4.5 km/h, a
   parameter), whatever the mode (you walk to the road, or to the car). Its length counts in the
   trip's distance. It is drawn as a straight dashed line.
3. **Partial edges cost their share.** Starting at fraction `f` of edge `e` costs `(1 − f)` of
   `e`'s `cost_s` and `length_m`; ending at fraction `g` of edge `e'` costs `g` of it; every edge
   in between costs in full. The fraction is by length, so the time share assumes a constant speed
   along the edge (as `cost_s` itself does).
4. **Same edge, forward:** start and end on the same directed edge with `f ≤ g` cost `(g − f)` of
   it, with no graph search. Otherwise (behind you on a one-way edge) the route goes around.
5. **Both directions of a two-way road are candidates** (two directed edges on the same line,
   with mirrored fractions), and every edge within the radius is a candidate, not only the nearest:
   the route is the cheapest over all (start candidate, end candidate) pairs, access included. So a
   marker between a street and a footpath picks whichever gives the faster trip.
6. **Turn restrictions still apply**: from the start edge the route continues by `edge_graph` (its
   legal turns), and arrives on the end edge by one. An edge with no turns can still be a start or an
   end when start and end are on it (rule 4).
7. **Walk + drive** (`route_multimodal`, over `mm.*`, junction to junction): the trip starts and
   ends **walking**, so the markers snap to walking edges; the start point becomes a temporary
   node joined to both ends of its walking edge (`f` and `1 − f` of it: walking goes both ways),
   the end point likewise; then the layered search runs as today. The access legs as above.

`weight="length"` follows the same rules with lengths (the access leg counts its metres).

## duckOSM (the reference)

```python
from duckosm import route_points, route_multimodal_points

r = route_points(con, (7.4155, 43.7285), (7.4400, 43.7480), mode="driving",
                 weight="time", radius_m=50, access_kmh=4.5)
r["time_s"], r["length_m"], r["edges"]           # as route(), with the true totals
r["start"], r["end"]                              # {point, edge_id, fraction, access_m, access_s}
```

- Points are `(lon, lat)`. Candidates come from one spatial query per point (`ST_DWithin` on the
  mode's edges, then the exact projection in metres, a local flat frame as elsewhere).
- The search is the edge-based Dijkstra of `route()` with the partial costs at both ends (several
  start edges at once: a multi-source search), so one run covers every candidate pair.
- `route()` and `route_multimodal()` keep working edge to edge, unchanged.
- Returns `None` when no pair is connected; raises `ValueError` "no road within R m of the start /
  end" when a point has no candidate (the page shows that message).

## mapstyle's planner page (mirrors it)

- A **radius** control in the panel (metres, default 50) and the walking speed for the access leg.
  A marker with no road within the radius shows "no road within R m". Each marker shows its
  radius as a circle (Kaveh: "show that distance area we set for walking without road"), red
  when no road is inside.
- The same rules in JavaScript; the page already holds every edge's geometry, `cost_s` and
  `length_m`.
- The route is drawn as its **exact line**: the partial first and last edges cut at the projected
  points, the access legs dashed from each marker, instead of recolouring whole edges (which
  colours the parts not travelled). The edges stay listed.
- The test (a page vs duckOSM comparison over random point pairs, both modes and walk + drive)
  checks time and length to 0.5 s / 0.5 m and the same edges.

## Checks

- Synthetic: a point beside the middle of an edge costs half the edge plus the access leg; start
  and end on one edge cost the part between; a point beyond the radius gives no route; a one-way
  edge with the end behind the start goes around; walk + drive from mid-edge.
- Monaco: `route_points` equals the page on random point pairs (the comparison above), and
  `route_points` between two edge midpoints with radius 0 equals `route()` minus half of the first
  and last edges.
