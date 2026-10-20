# Turns of the restricted edges in edge_graph

**Status:** approved by Kaveh and implemented 2026-10-20 (with the changes in "What was built").

## Problem

`driving.private_edges` holds the edges that cars may not use. Each has an `access` label:
`bus`, `private`, `pedestrian` or `delivery`. Only the `bus` edges are in `driving.edge_graph`.
Their turns are rows with `uses = 'bus'`
([bus-only edges](bus_only_edges.md)). The other three labels have no row. So a query on
`edge_graph` cannot tell which restricted edges meet at a junction.

## Rule

`edge_graph` gets the rows for every pair of edges where the first ends at the node where the
second starts, and at least one of the two is in `private_edges`. The `uses` value of the row is
the `access` label of the restricted edge. This is the rule that `bus` has today, for all labels.

| The two edges | `uses` of the row |
|---|---|
| a car edge and a restricted edge | the label of the restricted edge |
| two restricted edges with the same label | that label |
| two restricted edges with different labels | no row (no user takes both) |

The rows of the mode's own edges do not change (`uses = 'car'` in driving). A restricted edge has
no turn restriction and no turn-back rule: the same cleaning steps run on all rows, as today.

## Size (to confirm in the test)

| Area | Rows `car` | New rows |
|---|---|---|
| Sundbyberg | 12,549 | `private` 258, `pedestrian` 154 |
| Norrmalm | 3,839 | `pedestrian` about 2,200, `delivery` about 100, `private` about 90 (100 rows between two different labels are dropped) |

Norrmalm has many pedestrian streets, so its table grows by more than half.

## Rule for the readers (Kaveh, 2026-10-20)

Every reader of `edge_graph` takes **only the rows it needs, by default**, and has an option to
add the other uses. Nothing reads a new row unless it is asked to.

- The option is `uses`: a list of the extra labels, from `bus`, `private`, `pedestrian` and
  `delivery`. The default is none (the reader's own rows).
- `routed_graph(con, mode, uses=None)` is the one helper that most readers call. It returns the
  rows with `uses` equal to the mode's own, and the rows of the labels in `uses`. The readers that
  call it get the option by passing it on: `route()`, the route planner, point routing, `export-graph`, `sumo`.
- The CLI commands have `--uses bus,pedestrian` (a comma list).
- A reader that takes extra rows is responsible for them. For routing, the turn onto a restricted
  edge is then a legal move: use it only to look at the graph, not to route cars.

| Reader | Default rows | Option |
|---|---|---|
| `route()`, planner, point routing, `routed_graph` | the mode's own (`car`) | `uses=` |
| `export-graph`, networkx | the mode's own | `--uses` |
| SUMO export (`sumo.py`) | the mode's own; its `bus` rows for the lanes, as today | `--uses` adds the others as connections |
| GMNS export (`gmns.py`) | the mode's own and `bus` (as today: bus movements) | `--uses` adds `private`, `pedestrian`, `delivery` as movements with that `allowed_uses` |
| sensor-matching (`adapters/network.py`) | `uses = 'car'` | none (its own code) |
| Other downstream scripts (traffic_tube: `regime_chain`, `simulation`, `demand`) | not checked | check before the change |

A database that was built before this change has no new rows. Nothing there breaks.

## Where the code changes

- `processors/edge_graph.py`: the second `INSERT` takes all of `private_edges` with its `access`
  label, not only `access = 'bus'`. The label becomes `uses`. `routed_graph` gets `uses=`.
- `routing.py`, `route_map.py`, `point_routing.py`, `sumo.py`: pass `uses` to `routed_graph`.
- `gmns.py`: it reads the rows with `uses IN (own, 'bus', <the option>)`, in the main select and in
  the two `EXISTS` checks. Without this filter, a `pedestrian` row would become a movement with
  `allowed_uses = 'pedestrian'`.
- `cli.py`: `--uses` on `export-graph`, `sumo`, `gmns` and the routing commands.
- Docs: `reference/database.md` (the `uses` row and the `private_edges` text),
  `design/bus_only_edges.md` (a link to this note), the CLI reference.

## What was built (2026-10-20)

- `edge_graph` has the rows of all four labels in every mode (driving: `bus`, `private`, `pedestrian`, `delivery`;
  walking and cycling: `private`). A turn between two restricted edges of different labels has no row.
- `routed_graph(con, mode, uses=None)` is the one place that decides the rows. `check_uses` names a wrong label.
- The `uses` option is where the data allows it: `routed_graph`, `to_networkx` (the restricted edges get `access`), `write_graph` and
  `duckosm export-graph --graph edge --uses`. It is **not** on `route()`, `Router`, point routing, the route map, SUMO or
  GMNS. Their output needs the geometry and the lanes of the edges: a restricted edge that is not a link or a road of the net
  would have no data there. They read the own rows (SUMO and GMNS also the `bus` rows, as before).
- GMNS reads `uses IN (own, 'bus')`, in the main select and in the two `EXISTS` checks, so a `pedestrian` row never becomes a
  movement.
- Checked downstream: sensor-matching and four traffic_tube scripts filter `uses = 'car'`; urbanstyle copies only the rows whose two
  edges are in `edges`; the other traffic_tube script builds its SUMO net with duckOSM's own export. Nothing needed a change.
- Monaco: the `car` rows are the same as before; Sundbyberg: 12,545 `car`, 258 `private`, 154 `pedestrian`, 2 `bus`.

## Decision

The rows are in `edge_graph` (option A), not in a second table. The default of every reader is its
own rows, so a reader that does not use the option sees no change.

## Test

- Monaco: the rows with `uses = 'car'` are the same as before (same pairs).
- Every new row has one label, and both of its edges are in `edges` or `private_edges`.
- Rows between two different labels: none.
- The GMNS tables (`link`, `movement`) and the SUMO net of Monaco are the same as before, with no `uses` option.
- `route()` gives the same routes as before. With `uses=("pedestrian",)`, `routed_graph` also has the pedestrian rows.
- `--uses` with a label that does not exist is an error that lists the labels.
