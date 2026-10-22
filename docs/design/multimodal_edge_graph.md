# Design: Walk + drive on the edge graph

Status: proposal. No code is written. It waits for Kaveh's sign-off.

## Problem

`route_multimodal` (Walk + drive, walk + cycle) searches a graph of junctions: `mm.edges` has a
`source` and a `target` for each road, and nothing else. It does not read `edge_graph`. So the
driving part of a trip can use a turn that OSM forbids. The one-mode routes (`route()`,
`route_points`) read `edge_graph` and never do.

Test on Monaco: 300 random Walk + drive trips have 16,149 turns from one driving road to the next.
29 of them are not in `driving.edge_graph`. All 29 are banned by a row of `driving.turn_restrictions`
(9 by `no_*`, 20 by `only_*`).

The first version chose a junction graph because it is the simplest (`docs/design/multimodal.md`:
"Start node-based"). That note also says that for turn restrictions on the whole trip you need an
edge-based search. This note is that step.

## Rule

Every part of a trip follows the `edge_graph` of its own mode. Walk + drive gives the same turns as
Drive alone on the driving part, and the same turns as Walk alone on the walking part.

## Search

A state is `(mode, edge_id, phase)`. `phase` is the same as today: 0 before the vehicle,
1 in the vehicle, 2 after it. The label of a state is the time at the end of that edge.

| Move | Condition | Cost added |
|---|---|---|
| Next road | `(e, e')` is a row of the `edge_graph` of the mode | `cost_s(e')` |
| Change of mode | at the junction `n = e.target`, a row of `mm.transfers` for `(n, mode, mode')`, and the phase rule | the cost of the transfer |

A change of mode goes through a node state `(mode', n, phase')`. From it, the trip can go onto
every road of `mode'` that leaves `n`: `cost_s(e')`. A turn restriction is a rule between two roads
of the same mode, so the first driving road after a change of mode has none. A driving road
that arrives at `n` is already legal, so the walk that follows has none either.

This graph has a state for each road of each mode, and the number of moves is about the number of
rows in the three `edge_graph` tables. Monaco has 25,641 roads in `mm.edges`.

**Start and end.**
- `route_multimodal(con, src_node, dst_node)`: the start is the node state `(walking, src_node, 0)`.
  The trip ends on a walking road with `target == dst_node` (or at once when `src_node == dst_node`).
  `start_mode` and `end_mode` keep their meaning.
- `route_multimodal_points`: the start and the end are the candidates of `_ends`, on the
  walking roads, in the same way as `route_points`. A start candidate gives the label
  `access_s + (1 - fraction) * cost_s` to its road. An end candidate adds `fraction * cost_s` and
  `access_s` when the search leaves the road before it. A start and an end on one road, in the right
  direction, stay a direct walk.

The result keeps its shape: `time_s`, `length_m`, `edges`, `legs`, `transfers`, `nodes` (and
`start`, `end`, `access_s` for points). `enforce_sequence`, `allowed_modes` and the transfer costs
keep their meaning.

## The browser planner

`planner.js` already holds the `edge_graph` of every mode: `D.graphs[mode]` has the roads (`k`), the
moves (`next`) and the costs, and `routeOne` uses it. `routeMulti` changes in the same way as the Python code
and reads `D.graphs`. It still needs the `source` and `target` junction of each road, from
`D.mm.edges`, and the transfers from `D.mm.transfers`. The format of the planner data does not change,
so mapstyle and the plugin need no change except the new template.

The panel note "Turn restrictions don't apply across modes" goes away, and so does the `NO_MM` text in the same place.

## What changes for users

- A Walk + drive route never uses a forbidden turn. Where the old route used one, the new route is
  longer, or it goes another way.
- No route gets shorter. All other routes stay the same.
- One more `edge_graph` per mode is read. A call is a little slower, and the page holds the same
  data as before (`D.graphs` is already in it).

## Files

- `src/duckosm/routing.py`: `route_multimodal`.
- `src/duckosm/point_routing.py`: `route_multimodal_points` (the `_virtual` argument goes away).
- `src/duckosm/templates/planner.html`: `routeMulti`, the note; `docs/maps/monaco_route_map.html` is made again.
- `tests/test_multimodal.py`, `tests/test_point_routing.py`, and the browser check of the route map.
- Docs: `docs/concepts/multimodal.md` (cut the limit "No turn restrictions"), `docs/guides/route.md`,
  `docs/design/multimodal.md`, `docs/design/route_map.md`, `CHANGELOG`.
- geolibre-roadstyle: `planner/planner.js` (made again by `tools/make_planner.py`) and its guide.

## Tests

1. **No banned turn.** The Monaco test above gives 0 turns outside `edge_graph` (was 29).
2. **No shorter route.** For the same 300 trips, `time_s` is never less than the old value.
   It is equal when the old route used no banned turn.
3. **One mode equals `route()`.** `allowed_modes={"walking"}` and `{"driving"}` give the time of `route()`.
4. **One real case.** A trip that used a `no_*` turn and a trip that used an `only_*` turn: the new route avoids them.
5. **The page equals Python.** The browser check compares the page with `route()` and
   `route_multimodal()` on the 8 Monaco trips and on the cases of test 4.

## Open points

- **Speed on a big area.** `route_multimodal` reads all tables at each call, as before.
  A prebuilt graph (like `Router`) is a separate task.
- **Cycling.** The cycling part follows the cycling `edge_graph`. I think this is right, and it needs no extra work.
- **Old databases.** A database without `mm` or without `edge_graph` keeps the same error as today.
