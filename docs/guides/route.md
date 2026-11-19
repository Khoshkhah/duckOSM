# Route

Find the way from A to B on the network of an area: by car, on foot or by bike, the fastest or the shortest. A route
takes only legal turns, so it never uses a turn that a sign forbids.

The easiest way to try it is the route planner below. Drag **A** and **B**, and pick **Drive**, **Walk**, **Cycle** or
**Walk + drive**. The panel shows the time, the distance and the directions.

<iframe src="../../maps/monaco_route_map.html" allow="clipboard-write" title="A route planner for Monaco, made by duckosm route-map"
        loading="lazy" style="width: 100%; height: 520px; border: 0; border-radius: 8px"></iframe>

## Three ways to route

| Way | For | Example |
|---|---|---|
| **A map page** | look and try | `duckosm route-map monaco.duckdb`: one HTML page, as above |
| **Python** | your own program | `route_points(con, (7.4155, 43.7285), (7.4400, 43.7480))` |
| **The command line** | scripts, many routes at once | `duckosm route-points monaco.duckdb 7.4155 43.7285 7.4400 43.7480` |

## In Python

Give two points as (longitude, latitude). The result has the time, the length and the roads of the route in order:

```python
import duckdb
from duckosm import route_points, directions

con = duckdb.connect("monaco.duckdb", read_only=True)
r = route_points(con, (7.4155, 43.7285), (7.4400, 43.7480))  # by car, fastest
r["time_s"], r["length_m"]  # seconds, meters
for step in directions(con, r):
    print(step["text"])  # "At the roundabout, take the 1st exit onto ..."
```

To route on foot, give `mode="walking"`. For the shortest route, give `weight="length"`.

## All functions and commands

| Function | From, to | Arguments and defaults |
|---|---|---|
| `route(con, a, b)` | two `edge_id` values | `mode="driving"` (or `"walking"`, `"cycling"`), `weight="time"` (or `"length"`) |
| `Router(con, mode, weight)` | the same, many times | builds the graph one time; then `router.route(a, b)` |
| `route_points(con, a, b)` | two `(lon, lat)` points | `mode`, `weight`, `radius_m=50`, `access_kmh=4.5` |
| `route_multimodal(con, a, b)` | two `node_id` values | `start_mode`, `end_mode` (`"walking"`), `allowed_modes` (all), `enforce_sequence=True` |
| `route_multimodal_points(con, a, b)` | two `(lon, lat)` points | `radius_m=50`, `access_kmh=4.5`, `allowed_modes`, `enforce_sequence=True` |
| `directions(con, route)` | a result of one of the functions above | `mode="driving"` |

A route gives `edges` (the `edge_id` values in order), `time_s`, `length_m` and `path` (for each edge: `name`, `highway`,
`length_m`, `cost_s`, `geometry`). It gives `None` when no route connects the two ends.

| Command | Does | Many queries |
|---|---|---|
| `duckosm route DB FROM TO` | `route` | `--pairs FILE` (`FROM TO` on each line) |
| `duckosm route-points DB LON LAT LON LAT` | `route_points` (`--radius`, `--access-kmh`) | `--batch FILE` |
| `duckosm route-multimodal DB SRC DST` | `route_multimodal` (`--modes`) | `--batch FILE` |
| `duckosm route-multimodal-points DB LON LAT LON LAT` | `route_multimodal_points` | `--batch FILE` |
| `duckosm directions DB ROUTE.json` | `directions` (`-` reads the standard input) | a list of routes |

Each mode has its own edges. For `mode="walking"`, use the ids of `walking.edges`. Routes across modes:
[Routing across modes](../concepts/multimodal.md).

## From a point

A point joins the road of the mode nearest to it within `radius_m`. The walk to that road costs walking time (`access_kmh`),
also by car. The first and the last edge count only the part that the trip uses. A point with no road within the radius
gives a `ValueError`.

To find an `edge_id`, click a road on the route map below, or use SQL ([Query the database](query.md#places)):

```sql
SELECT min(edge_id) FROM driving.edges WHERE name = 'Boulevard du Larvotto';
```

## The map page

```bash
duckosm route-map monaco.duckdb -m walk+drive      # writes reports/monaco_route_map.html
```

One HTML page, with no server: the planner at the top of this page. It routes with the same router as
`duckosm route-points`. Click a direction to go to it, or a road to copy its `edge_id`.

| Option | Does | Default |
|---|---|---|
| `-m` | the first mode: `driving`, `walking`, `cycling` or `walk+drive` (needs the `mm` tables) | walking in front |
| `--basemap` | the first background: `voyager`, `positron`, `osm`, ... | the layers of the database |
| `--google-3d` / `--no-google-3d` | a Google 3D window; needs `GOOGLE_MAPS_KEY` | on when the key is set |
| `-o` | the output file | `reports/<name>_route_map.html` |

The page holds all the roads. Use it for a city: above 100,000 edges, the command shows a warning.

## Large areas

The graph of one mode is in memory, with 12 bytes for each turn. On a Mac mini (Apple M6, 12 cores, 16 GB), a route by car
from Paris to Marseille on the France network (25.9 million edges) takes 3.7 s after a load of 11 to 16 s, with a peak of
6.4 GB. All measurements and the details of point routing and directions:
[routing details](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/route_details.md).
