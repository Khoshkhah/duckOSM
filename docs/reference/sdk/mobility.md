# duckosm.mobility

Mobility data: GPS traces and traffic sensors on the roads. The module comes in PR 5 of the design; it is empty today.

| Call | Returns | Today |
|---|---|---|
| `match(view, traces)` | each GPS point and trace on `edge_id` (map matching) | PR 5 |
| `snap(view, points)` | the nearest edge and the position on it | PR 5 |
| `join(view, data, on="edge_id")` | your data on the edges | PR 5 (today: `net.edges().merge(data, on="edge_id")`) |
| `aggregate(view, matched, by="edge_id", time="hour")` | counts or speeds per edge and time | PR 5 |
| `edge_id_map(old_net, new_net)` | old `edge_id` → new `edge_id` after a rebuild | PR 5 |

Page: [Stable edge ids](../../concepts/edge-ids.md).
