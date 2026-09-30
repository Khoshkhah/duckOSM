# Route

A route goes from one edge (a directed piece of road) to another over the graph of legal turns, so
it never takes a banned turn. Needs `pip install "duckosm[routing]"`.

```python
import duckdb
from duckosm import route, Router

con = duckdb.connect("monaco.duckdb", read_only=True)

r = route(con, from_edge, to_edge)                  # fastest, by car
r["edges"], r["time_s"], r["length_m"]              # ordered edge_ids, seconds, metres
r["path"]                                           # per edge: name, highway, length_m, cost_s, geometry

route(con, from_edge, to_edge, weight="length")     # shortest instead of fastest
route(con, from_edge, to_edge, mode="walking")      # on the walking network (or "cycling")

router = Router(con, mode="driving")                # many routes: build the graph once
router.route(from_edge, to_edge)
```

The graph is held in memory: fine for a city, heavy for a country.

## On a map

```bash
duckosm route-map monaco.duckdb -m driving     # -> reports/monaco_route_map.html
```

One HTML page, no server: click the map for the start, again for the end, and drag the markers to
move them. The route is computed in the page over the same graph, so it gives the same answer as
`route()`:

<iframe src="../../maps/monaco_route_map.html" allow="clipboard-write" title="A route planner for Monaco, made by duckosm route-map"
        loading="lazy" style="width: 100%; height: 520px; border: 0; border-radius: 8px"></iframe>

Needs `pip install "duckosm[viz]"`. Everything is inside the page, so keep it to a city: above
100,000 edges the command warns.

## Find the edges to route between

**By street name:**

```python
q = "SELECT min(edge_id) FROM driving.edges WHERE name = ?"
from_edge = con.execute(q, ["Boulevard du Larvotto"]).fetchone()[0]
```

**Nearest to a point** (latitude first, see [Query the database](query.md#places)):

```python
con.execute("LOAD spatial")
near = """SELECT edge_id FROM driving.edges
          ORDER BY ST_Distance_Sphere(ST_FlipCoordinates(ST_Centroid(geometry)), ST_Point(?, ?)) LIMIT 1"""
from_edge = con.execute(near, [43.7285, 7.4155]).fetchone()[0]     # Fontvieille
to_edge = con.execute(near, [43.7480, 7.4400]).fetchone()[0]       # Larvotto
route(con, from_edge, to_edge)["time_s"]                            # 324 s, 3.8 km by car
```

**On the map:** in a [`duckosm viz`](draw-map.md) map, click a road to copy its `edge_id`.

Each mode has its own edges: for `mode="walking"`, take the ids from `walking.edges`.

## Across modes (walk → drive → walk)

`duckosm multimodal` joins the walking, cycling and driving networks at the junctions they share,
adding the `mm` tables to the database. `route_multimodal` then finds the fastest trip that may
change mode on the way. *Experimental.*

```bash
duckosm multimodal monaco.duckdb             # --transfer-cost 60: seconds per change of mode
```

It goes from one junction to another, so it takes OSM node ids, not edge ids:

```python
from duckosm import route_multimodal

near_node = """SELECT node_id FROM walking.nodes WHERE node_id > 0
               ORDER BY ST_Distance_Sphere(ST_FlipCoordinates(geom), ST_Point(?, ?)) LIMIT 1"""
start = con.execute(near_node, [43.7285, 7.4155]).fetchone()[0]   # Fontvieille
end = con.execute(near_node, [43.7480, 7.4400]).fetchone()[0]     # Larvotto

r = route_multimodal(con, start, end)
[(leg["mode"], round(leg["time_s"])) for leg in r["legs"]]      # [('walking', 7), ('driving', 265)]
r["time_s"], r["transfers"]                                       # 393 s including 2 mode changes
```

| Argument | Does | Default |
|---|---|---|
| `start_mode`, `end_mode` | the mode at the start and at the end | `"walking"` |
| `allowed_modes` | limit the modes, e.g. `["walking", "cycling"]` | all |
| `enforce_sequence` | at most one vehicle leg, reached and left on foot | `True` |

It returns `None` if the two points aren't connected. Changing mode is allowed at any shared
junction, so a trip can "pick up a car" anywhere: good for comparing modes, not a real trip planner.
Public transport isn't included. How it works: [Routing across modes](../multimodal.md).
