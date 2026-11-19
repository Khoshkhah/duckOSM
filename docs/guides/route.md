# Route

Find the fastest or the shortest route between two roads or two points. A route follows the graph of legal turns, so it
never takes a banned turn.

```python
import duckdb
from duckosm import route, route_points, directions

con = duckdb.connect("monaco.duckdb", read_only=True)
r = route(con, from_edge, to_edge)  # by car, fastest
r = route_points(con, (7.4155, 43.7285), (7.4400, 43.7480))  # from (lon, lat) to (lon, lat)
for step in directions(con, r):
    print(step["text"])  # "At the roundabout, take the 1st exit onto ..."
```

The same in the program `duckosm`, which prints JSON:

```bash
duckosm route monaco.duckdb FROM_EDGE TO_EDGE
duckosm route-points monaco.duckdb 7.4155 43.7285 7.4400 43.7480
```

## The functions

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

## On a map

```bash
duckosm route-map monaco.duckdb -m walk+drive      # writes reports/monaco_route_map.html
```

One HTML page, with no server. Drag the two markers to set the start and the end, and select Drive, Walk, Cycle or
Walk + drive. The page routes with the same router as `duckosm route-points`, built into the page. The panel shows the
directions; click one to go to it. Click a road to copy its `edge_id`. The map is drawn by roadstyle.

<iframe src="../../maps/monaco_route_map.html" allow="clipboard-write" title="A route planner for Monaco, made by duckosm route-map"
        loading="lazy" style="width: 100%; height: 520px; border: 0; border-radius: 8px"></iframe>

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
