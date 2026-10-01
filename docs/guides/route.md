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
duckosm multimodal monaco.duckdb               # optional: the tables Walk + drive needs
duckosm route-map monaco.duckdb -m walk+drive  # -> reports/monaco_route_map.html
```

One HTML page, no server: drag the two markers to set the start and the end, and pick Drive, Walk,
Cycle, or Walk + drive (walk to the car, drive, walk from it; it needs `duckosm multimodal` first).
A marker joins the roads within the off-road distance by a walk, and the first and last roads count
only the part travelled, as in [`route_points()`](#between-two-points). The panel lists the
directions ("Turn left onto …", "At the roundabout, take the 2nd exit …"), the same as
[`directions()`](#turn-by-turn-directions); click one to go to it on the map. Click a road to copy its
`edge_id`; the Street View button shows the street. The Roads box hides road classes, bridges,
tunnels, private roads or bus lanes, as on the [drawn maps](draw-map.md). The map under the route is
drawn by mapstyle; the routing is duckOSM's.

<iframe src="../../maps/monaco_route_map.html" allow="clipboard-write" title="A route planner for Monaco, made by duckosm route-map"
        loading="lazy" style="width: 100%; height: 520px; border: 0; border-radius: 8px"></iframe>

Needs `pip install "duckosm[viz]"`. Everything is inside the page, so keep it to a city: above
100,000 edges the command warns. `-m driving` puts that mode in front and picks it first;
`-m walk+drive` shows every mode and opens on Walk + drive, as the map above does.

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
route(con, from_edge, to_edge)["time_s"]                            # 287 s, 3.7 km by car
```

**On the map:** in the route map above, click a road to copy its `edge_id`.

Each mode has its own edges: for `mode="walking"`, take the ids from `walking.edges`.

## Between two points

`route_points` routes from one place to another, not from edge to edge. Each point joins the
network at its nearest road point within `radius_m`; the straight walk to it costs walking time
(`access_kmh`), whatever the mode, and never crosses another road (one of the mode, or any road
for cars: it joins the first road in its way); and the first and last edges count only the part
you travel:
starting in the middle of a 300 m street costs 150 m of it.

```python
from duckosm import route_points

r = route_points(con, (7.4155, 43.7285), (7.4400, 43.7480), mode="driving",   # (lon, lat)
                 radius_m=200, access_kmh=4.5)        # Larvotto's point is on the beach, ~100 m from a road
r["time_s"], r["length_m"]        # door to door: the walks to the road + the parts of edges used
r["start"]                        # {point, road_point, edge_id, fraction, access_m, access_s}
r["path"][0]["from_fraction"]     # where on the first edge the trip begins (0 to 1)
```

Every road of the mode within the radius is tried, both directions of a two-way street, so a
point between a street and a footpath takes whichever gives the faster trip. A point with no road
within the radius raises `ValueError` ("no road within 50 m of the start"). For walk → drive →
walk, `route_multimodal_points(con, a, b, radius_m=50)` does the same over the `mm` tables (below).
Design: [point routing](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/point_routing.md).

## Turn-by-turn directions

`directions(con, route)` turns any route (from `route`, `route_points`, `route_multimodal` or
`route_multimodal_points`) into steps, the ones the route map lists:

```python
from duckosm import directions

for s in directions(con, r):                     # r from route_points above
    print(s["text"], round(s["length_m"]))
```

```text
Head northwest on Avenue des Castelans 23
At the roundabout, take the 1st exit onto Avenue Albert II 179
At the roundabout, take the 3rd exit onto Avenue Albert II 189
Continue onto Tunnel Rocher Palais 207
…
At the roundabout, take the 4th exit onto Avenue Princesse Grace 172
Arrive at your destination 0
```

Each step also has `type` (`depart`, `turn`, `fork`, `new name`, `roundabout`, `mode`, `arrive`),
`modifier` (`left`, `slight right`, `uturn`, …), `name`, `mode`, `at` (the `(lon, lat)` where it
happens) and, for a roundabout, `exit`. A step's `length_m` is the distance to the next one. The
rules are route-guidance's (the OSRM model); the route map runs the same rules in the page.

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
Public transport isn't included. How it works: [Routing across modes](../concepts/multimodal.md).
