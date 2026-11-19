# Routing across modes

The `mm` tables join the walking, cycling and driving networks into one graph. Then a trip can change mode: walk to the
car, drive, and walk from the car. *Experimental.*

```python
from duckosm import route_multimodal_points

r = route_multimodal_points(con, (7.4155, 43.7285), (7.4400, 43.7480))   # (lon, lat)
[(leg["mode"], round(leg["time_s"])) for leg in r["legs"]]                # walking, driving, walking
```

The functions and the commands: [Route](../guides/route.md#the-functions).

## Where the tables come from

| How | Makes the `mm` tables |
|---|---|
| `duckosm <PBF> -o DIR` | yes, always |
| `duckosm --config FILE` and `duckosm build --config FILE` | only with `multimodal.enabled: true` |
| `duckosm multimodal DB` | yes, on a built database |

The database must have walking and at least one other mode. `mm.transfers` is a table: after you build a mode again,
make it again.

| Table | Contents |
|---|---|
| `mm.edges` | a view of the `edges` of each mode, with a `mode` column. The key is `(mode, edge_id)`, because [the same road has the same `edge_id` in each mode](edge-ids.md#the-same-id-in-every-mode) |
| `mm.transfers` | one row for each change of mode: `node_id`, `from_mode`, `to_mode`, `cost_s`, `kind` |

## Transfers

A transfer is at each node that walking shares with another mode, in the two directions: `park` and `retrieve` for
driving, `bike_park` and `bike_unpark` for cycling. Every change of mode goes through walking. Each transfer costs 60 s
(`duckosm multimodal --transfer-cost`, or `multimodal.transfer_s` and `multimodal.transfer_costs` in a config, for
example `"driving->walking": 30`).

## The trip

A trip is a sequence of edges and transfers, and its time is the sum of their `cost_s`. With `enforce_sequence=True` (the
default), a trip starts and ends on foot and has one vehicle leg at most. Each leg follows the turn restrictions of its
mode. A vehicle waits at each shared junction, so a trip can take a car anywhere: use it to compare modes, not as a real
trip planner. Public transport is not included, because OSM has no timetables.

Design: [turns across modes](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/multimodal_edge_graph.md).
