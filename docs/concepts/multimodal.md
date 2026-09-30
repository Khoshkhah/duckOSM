# Routing across modes

`duckosm multimodal` joins the walking, cycling and driving networks of a database into one graph,
so that a trip can change mode on the way: walk to the car, drive, walk from the car. How to run it
and route with `route_multimodal`: [Route across modes](../guides/route.md#across-modes-walk--drive--walk).
*Experimental.*

## What it adds

It writes a schema `mm` into the database (or the build does, with `multimodal.enabled: true`). It
needs walking and at least one other mode.

| Table | Contents |
|---|---|
| `mm.edges` | a view of every mode's `edges`, with a `mode` column: `mode`, `edge_id`, `source`, `target`, `cost_s`, `length_m`, `highway`, `name`, `geometry`. The key is `(mode, edge_id)`, because [the same road has the same `edge_id` in every mode](edge-ids.md#the-same-id-in-every-mode) |
| `mm.transfers` | one row per change of mode: `node_id`, `from_mode`, `to_mode`, `cost_s`, `kind` |

A transfer is made at every node that walking shares with another mode, in both directions. A
`node_id` is the OSM node id, the same in every mode, so a shared node is the same junction.

| From → to | `kind` |
|---|---|
| walking → driving / driving → walking | `park` / `retrieve` |
| walking → cycling / cycling → walking | `bike_park` / `bike_unpark` |

There is no transfer between driving and cycling: every change of mode goes through walking.

Monaco: walking shares 729 nodes with driving and 3,082 with cycling, so `mm.transfers` has 7,622
rows (`duckosm multimodal monaco.duckdb`).

## The model

A point in the graph is a pair `(node_id, mode)`. An edge of a mode moves you inside that mode; a
transfer moves you to another mode at the same node. Everything is in seconds, so a trip's time is
the sum of its edges' `cost_s` plus the sum of its transfers' `cost_s`.

By default (`enforce_sequence=True`) a trip has this shape:

```text
walk*  (drive | cycle)*  walk*
```

It starts and ends in `start_mode` and `end_mode` (both `walking` by default), has at most one
vehicle leg, and gets on and off that vehicle on foot. With `enforce_sequence=False` any sequence of
modes the transfers allow is possible.

## Transfer cost

Each transfer costs the same number of seconds: 60 by default, `--transfer-cost` on the command
line. In a config file the costs can differ by direction:

```yaml
multimodal:
  enabled: true
  transfer_s: 60                   # every change of mode
  transfer_costs:                  # optional, per direction
    "walking->driving": 60
    "driving->walking": 30
```

## Limits

- **A vehicle is waiting at every shared junction.** Transfers aren't limited to car parks or bike
  stands, so a trip can "pick up a car" anywhere. Good for comparing modes, not a real trip planner.
- **No turn restrictions.** `route_multimodal` routes over nodes, not over the graph of legal
  turns, so a driving leg can take a banned turn. For one mode, use `route()`, which obeys them.
- **No public transport.** OSM has stops and lines but no timetables.
- The graph is held in memory: fine for a city.
