"""Write docs/notebooks/08_routing_advanced.ipynb: chapter 8 of the notebook manual, "Advanced routing".
The style comes from scripts/notebook_book.py (rules: docs/design/notebook_book.md).

    python scripts/notebook_chapter8.py docs/notebooks/08_routing_advanced.ipynb"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from notebook_book import IMAGES, Chapter  # noqa: E402

ch = Chapter(8, "08_routing_advanced", "Advanced routing",
             "The Route object, many routes, the k best routes, travel time matrices, isochrones and accessibility",
             learn="the graph that all routing calls search; the members of a `Route`; the fastest or the shortest "
                   "route; the k best routes; many routes in parallel; a travel time matrix; isochrones; accessibility; your own cost of each edge and of each change of mode; a delivery round with ortools; "
                   "cafés near every junction with pandana",
             time="about 2 minutes", extras="widget,viz,graph", more_install="\n    %pip install -q ortools pandana",
             next_=("09_analysis", "9. Street network analysis", "the numbers of OSMnx, junctions, components and points"))

ch.md(f"""## One graph for all routing calls

[7. Routing](07_routing.md) finds one route from A to B. This chapter uses the other calls of the module
`duckosm.routing`. They answer larger questions: the next best routes, a route for each of 100 pairs, the travel time
between all pairs of 100 places, the area that you reach in 10 minutes on foot, and the nearest stop from each street.

All these calls search **the same graph**: the turn graph of the mode (the table `<mode>.edge_graph`). A node is an
edge of the road network, and an arc is a legal turn ([7. Routing](07_routing.md), section "How duckOSM finds a
route"). The search runs in Rust. Each call opens the database file, reads the graph once, and searches it with
Dijkstra's algorithm.

| Call | Question | Algorithm |
|---|---|---|
| `route(a, b)` | the best route from A to B | bidirectional Dijkstra |
| `routes(origins, destinations)` | the best route for each pair, in parallel | bidirectional Dijkstra, the pairs in parallel |
| `k_shortest_paths(a, b, k)` | the `k` best routes from A to B | Yen's algorithm |
| `matrix(origins, destinations)` | the time from each origin to each destination | one Dijkstra search from each origin |
| `isochrones(origins, minutes)` | the area reached within each time | one Dijkstra search, stopped at the largest time |
| `accessibility(origins, targets, minutes)` | the targets reached from each origin, and the nearest | `matrix` with a time limit |

Each call is a function of the module, `duckosm.routing.matrix(net.driving, ...)`, and also a method of a mode view,
`net.driving.matrix(...)`. This chapter uses the methods.

> **Key idea:** all routing calls search the same turn graph, with the same turn rules. So a cell of the matrix is
> the time of `route` for the same pair, the first of the k best routes is `route`, and an isochrone holds the edges
> whose matrix time is in the limit.""")

ch.md("""## Open the network of chapter 1

Chapter 1 builds Monaco with three modes (driving, walking and cycling) and saves it as `monaco.duckdb`. This chapter
opens that file with `duckosm.open`: it builds nothing. The routes use the driving network and the walking network.
The start and the end of each call are places, as in chapter 7: an edge id, a `(lon, lat)` point or a place name. An
edge id is the fastest place, because it needs no search for the nearest edge.""")
ch.code("""net = duckosm.open("monaco.duckdb")    # the network of chapter 1: Monaco, three modes
net                                    # the card of the network""")

ch.md("""## The Route object

`route` gives a `Route`, or `None` when no route connects the two places. In a notebook a `Route` shows a card with
the number of edges, the length and the time. Here the two places are street names.""")
ch.code("""r = net.driving.route("Boulevard Albert 1er", "Avenue Princesse Grace")    # from street to street, by car
r                                                                        # the card: edges, length, time""")
ch.md("""The route by car from Boulevard Albert 1er to Avenue Princesse Grace has 33 edges, 1,350 m and 122 s (2.0 minutes). Each name became a point (with `geocode`), so the route starts and ends at the points on the two streets, not at the ends of whole edges.""")

ch.md("""### What a Route stores, and what it reads from the file

A `Route` is small. It keeps only the result of the search, and two links to the network:

| Stored in the Route | In this example |
|---|---|
| the edge ids, in the order of the route (`r["edges"]`) | 33 edge ids |
| the time and the length (`r["time_s"]`, `r["length_m"]`) | 122 s, 1,350 m |
| the start and the end (`r["start"]`, `r["end"]`): the point, the nearest road point, its `edge_id`, how far along the edge (`fraction`), and the walk to the road (`access_m`, `access_s`) | the end is 12 m from its road |
| the steps of the search (`r["path"]`) | 33 steps |
| the network file, and the mode (hidden: `r._db`, `r._mode`) | `monaco.duckdb`, `driving` |

The `Route` does not store the columns of the edges. When you ask for them, it opens its network file again and
reads the 33 edges with a query (`SELECT ... FROM driving.edges WHERE edge_id IN (...)`), in the order of the route.

| Member | Gives | From |
|---|---|---|
| `r.edge_ids` | the edge ids, in the order of the route | the Route |
| `r.length_m`, `r.time_s` | the sums of `length_m` and `cost_s` of the edges | the Route |
| `r.edges` | a GeoDataFrame of the edges in the order of the route: `seq`, `edge_id` and all the columns of `<mode>.edges` (`name`, `highway` ...) | the file, at each call |
| `r.geometry` | one shapely line (a `MultiLineString` where the edges do not join) | the file, at each call |
| `r.directions()` | the turn-by-turn steps | the file, at each call |
| `r.map()` | the route on the roadstyle map of the network | the file, at each call |

So you can keep thousands of routes in memory, and `r.edges` gives all the columns of the edges table: you can count
the road classes of a route or sum the length of its tunnels.

> **Warning:** `r.edges`, `r.geometry`, `r.directions()` and `r.map()` need the network file. If you move or delete
> `monaco.duckdb`, they fail. `r["edges"]`, `r.time_s` and `r.length_m` still work.""")
ch.code("""print(len(r.edge_ids), "edges,", round(r.length_m), "m,", round(r.time_s), "s")    # the sums
r.edges[["seq", "edge_id", "name", "highway", "length_m", "cost_s"]].head()          # the first edges, in order""")
ch.code("""r.edges.groupby("highway").length_m.sum().round().sort_values(ascending=False)    # the metres of each road class""")
ch.md("""**What the output shows.** The first five edges go along Boulevard Albert 1er, through the roundabout Rond-Point Sainte-Dévote, to Avenue John F. Kennedy. The route has 1,043 m of secondary roads, 280 m of tertiary roads and 131 m of service roads. These sums count whole edges, so they are a little more than the 1,350 m of the card.""")

ch.md("""### The line and the steps

`r.geometry` joins the lines of the edges into one line. Use it to measure the route, to draw it, or to find the
places near it. `r.directions()` gives the turn-by-turn steps, as [7. Routing](07_routing.md) shows.""")
ch.code("""print(r.geometry.geom_type, len(r.geometry.coords), "points")    # one line
[step["text"] for step in r.directions()][:5]                   # the first turn-by-turn steps""")

ch.md("""### A Route is also a dict

A `Route` is a Python `dict` too, with the keys of the table above: `edges`, `time_s`, `length_m`, `start`, `end`
and `path`. The code that used the dict of older duckOSM versions works without a change, and `net.map(routes=r)` takes a
`Route`.""")
ch.code("""sorted(r.keys()), r["edges"][:3]    # the keys of the dict, and the first edge ids""")

ch.md("""## The fastest or the shortest: `weight`

`weight="time"` (the default) gives the fastest route: the sum of `cost_s`. `weight="length"` gives the shortest
route: the sum of `length_m`. All calls of this chapter take `weight`. It can also be your own cost of each edge:
see "Your own cost" below.""")
ch.code("""import pandas as pd

ids = net.driving.edges().edge_id.sample(200, random_state=1).tolist()   # 200 driving edges, chosen at random
fast = net.driving.route(ids[0], ids[6])                                  # the fastest route between two of them
short = net.driving.route(ids[0], ids[6], weight="length")                # the shortest route
pd.DataFrame({"fastest": [fast.length_m, fast.time_s], "shortest": [short.length_m, short.time_s]},
             index=["length_m", "time_s"]).round()                              # the two routes side by side""")
ch.md("""Between these two edges the fastest route is 2,049 m and 142 s. The shortest route is 1,784 m, 265 m shorter, but it takes 186 s, 44 s more: it uses slower streets. In a city of steep, narrow streets the two weights often give different routes.""")

ch.md("""## The k best routes: `k_shortest_paths`

`k_shortest_paths(a, b, k=3)` gives the `k` best routes from A to B, the best first. It uses Yen's algorithm on the
same graph and turn rules as `route`. A route uses each edge one time only. The first route is the route of `route`
for the same two edges. When the graph has fewer than `k` routes, the list is shorter.

Use the next best routes to show a driver two or three choices, or to spread traffic in a simulation over more than
one road.""")
ch.code("""a, b = r.edge_ids[0], r.edge_ids[-1]                       # the first and the last edge of the route
best = net.driving.k_shortest_paths(a, b, k=3)              # the 3 fastest routes
[(round(x.time_s), round(x.length_m), len(x.edge_ids)) for x in best]    # seconds, metres and edges of each""")
ch.code("""best[0].edge_ids == net.driving.route(a, b).edge_ids    # the first route is the route of route()""")
ch.md("""The three routes take 126 s, 127 s and 127 s, with 1,453 m, 1,459 m and 1,464 m. The second and the third route have 35 edges, two more than the first. The first route is the same as the route of `route` for the two edges (`True`). Each route takes as long as the route before it, or longer.""")
ch.code("""net.map("driving", routes={f"route {i + 1}": x for i, x in enumerate(best)})    # the 3 routes on the map""")

ch.md("""## Many routes in parallel: `routes`

`routes(origins, destinations)` takes two lists of the same length. It finds the route from each origin to the
destination at the same place in the other list, as Python's `zip` pairs them. The pairs run in parallel on all
the cores of the computer. The result is a list of `Route`, with `None` for a pair that no route connects.

Here the places are the 200 random driving edges of the section above: 100 origins and 100 destinations.""")
ch.code("""many = net.driving.routes(ids[:100], ids[100:])    # 100 routes: ids[0] to ids[100], ids[1] to ids[101] ...
sum(x is not None for x in many)                    # the routes found""")
ch.md("""**What the output shows.** 90 of the 100 pairs have a route. The other 10 pairs have `None`: one of their edges cannot reach the other, for example an edge in a garage that only leaves the network, or an edge that is not in the graph of the mode (see `routed_graph` below). The histogram shows the time of the 90 routes.""")
ch.code("""import matplotlib.pyplot as plt

times = [x.time_s / 60 for x in many if x is not None]        # the time of each route in minutes
plt.hist(times, bins=20, color="#1f6feb")                     # a histogram
plt.xlabel("minutes by car"); plt.ylabel("routes"); plt.show()    # the labels""")
ch.md("""Most routes take 1.5 to 4.5 minutes by car, and the longest take about 7 minutes.""")
ch.md("""> **Warning:** the two lists must have the same length. `routes([e1, e2], [e3])` stops with the error
> `2 origins and 1 destinations: give the same number`. For all pairs of two lists, use `matrix`.""")

ch.md("""## A travel time matrix: `matrix`

`matrix(origins, destinations)` gives the travel time from each origin to each destination, as a pandas DataFrame:
one row for each origin, one column for each destination. A cell is in seconds (`weight="length"`: in metres). Without
`destinations`, the destinations are the origins.

The matrix does not find one route for each cell. It runs one Dijkstra search from each origin, and this search
reaches all destinations. The origins run in parallel. So a matrix of 100 × 100 places costs about as much as 100
routes.

| Argument | Default | What it does |
|---|---|---|
| `origins` | | the places of the rows |
| `destinations` | `None` | the places of the columns (`None`: the origins) |
| `weight` | `"time"` | `"length"`: metres, not seconds |
| `max_cost` | `None` | the largest cell: each search stops there, and a larger cell is `NaN` |

A cell is `NaN` when no route connects the pair, when the cost is more than `max_cost`, or when the edge is not in the
graph of the mode (for example an edge for buses only).""")
ch.code("""m = net.driving.matrix(ids[:100])                             # 100 x 100 travel times in seconds
m.iloc[:5, :5].round()                                        # the first cells""")
ch.md("""**What the output shows.** The rows and the columns have the edge ids as their labels. A cell on the diagonal is the time of the edge itself: from its start to its end (6 s for the first edge). The matrix is not symmetric: 96 s from the first edge to the second, but 121 s back, because of the one-way streets.""")
ch.code("""plt.imshow(m.iloc[:30, :30] / 60, cmap="viridis")           # a heat map of 30 x 30 cells, in minutes
plt.colorbar(label="minutes by car")                          # the scale
plt.xlabel("destination"); plt.ylabel("origin"); plt.show()   # the labels""")

ch.md("""### A cell equals the route

A cell is the time of `route` for the same two edges: from the start of the origin edge to the end of the destination
edge. The tests of duckOSM check this rule for each cell.""")
ch.code("""round(m.iloc[0, 1]) == round(net.driving.route(ids[0], ids[1]).time_s)    # a cell equals the route""")

ch.md("""### A time limit: `max_cost`

`max_cost` stops each search at that cost. A cell above it is `NaN`. Use it for a large matrix when only the near
places count: the search then reads a small part of the graph.""")
ch.code("""near = net.driving.matrix(ids[:100], max_cost=120)    # only the cells of 2 minutes or less
print(f"{near.notna().to_numpy().mean():.0%} of the cells are 2 minutes or less")    # the share of cells with a value""")
ch.md("""With `max_cost=120`, 27 % of the cells have a value. The other cells are `NaN`, because they cost more than 2 minutes or no route connects them.""")

ch.md("""### Points and names as places

The origins and the destinations can be points and names too. The rows and the columns then have the numbers
`0, 1, 2 ...`. When all places are edge ids or names, the rows and the columns have the places as their labels.""")
ch.code("""places = ["Boulevard Albert 1er", "Avenue Princesse Grace", "Boulevard de Suisse"]    # three streets
(net.driving.matrix(places, weight="length") / 1000).round(2)                          # the distances in km""")
ch.md("""The rows and the columns have the three names. With `weight="length"` the cells are metres, and the code divides them by 1,000. From Boulevard Albert 1er to Avenue Princesse Grace is 1.88 km, and the way back is 2.08 km. The diagonal is the length of the edge of the name (0.09 to 0.12 km).""")

ch.md("""## Isochrones: `isochrones`

### What an isochrone is

An **isochrone** answers one question: how far can I go from here in 10 minutes? It is the area that you reach from a
place within a time, on the roads of one mode. It is not a circle, because the roads, the hills and the sea limit
where you can go.

### How duckOSM makes it

1. It searches the roads from the start place, up to the largest time.
2. It keeps each edge that you reach to its end within the time.
3. It draws a band of 25 m around each kept edge, and joins the bands into one polygon.

So a park or a block with no road stays out of the polygon.

### The call

`net.walking.isochrones(origins, minutes)` gives one polygon for each start place and each time.

| Argument | Default | What it does |
|---|---|---|
| `origins` | | a list of start places: `(lon, lat)` points, edge ids or place names |
| `minutes` | `(5, 10, 15)` | the times: one number or a list |
| `buffer_m` | `25.0` | the width of the band around each reached edge, in metres |

Here the start is the casino square, on foot, for 5, 10 and 15 minutes.""")
ch.code("""casino = (7.4280, 43.7393)                                       # Place du Casino (lon, lat)
iso = net.walking.isochrones([casino], minutes=[5, 10, 15])      # one polygon for each time
iso[["minutes", "n_edges", "area_km2"]]                          # the reached edges and the area of each polygon""")
ch.md("""Each row is one polygon. From the casino square, 1,525 walking edges are within 5 minutes, 3,483 within 10
minutes and 5,983 within 15 minutes. The areas are 0.24 km², 0.65 km² and 1.15 km².

### Draw the isochrones

The cell draws the walking roads in gray, then the three polygons over them. It draws the largest polygon first,
so the smaller ones stay on top of it.""")
ch.code("""fig, ax = plt.subplots(figsize=(7, 7))                                          # one picture
net.walking.edges().plot(ax=ax, color="lightgray", linewidth=0.5)               # the walking roads, in gray
iso[::-1].plot(ax=ax, column="minutes", categorical=True, cmap="autumn", alpha=0.6,
               legend=True, legend_kwds={"title": "minutes"})                  # the polygons: 15, 10, then 5 minutes
ax.set_axis_off()                                                               # no degrees on the sides
plt.show()""")
ch.md("""Each polygon follows the streets and the steps, not a circle: the hills and the sea limit the walk.

> **Tip:** the walking speed comes from `cost_s` of the walking edges. For more than one start place, give a list:
> each place gets its own polygons, with its place in the column `origin`.""")

ch.md("""## Accessibility: `accessibility`

### What accessibility is

Accessibility answers two questions for each place where people start, for example each street where people live:

- How many targets can I reach within 5 or 10 minutes?
- Which target is the nearest, and how far is it?

The targets are the places that people go to: bus stops, schools, shops, pharmacies or parks.

### How duckOSM finds it

1. It searches the roads from each start place, up to the largest time (one search for each start, in parallel).
2. It counts the targets that it reaches within each time.
3. It keeps the nearest target and its time.

### The call

`net.walking.accessibility(origins, targets, minutes)` gives a DataFrame with one row for each start place.

| Argument | What it is |
|---|---|
| `origins` | the start places: edge ids, `(lon, lat)` points or names, in a list or a dict `{name: place}` |
| `targets` | the places to reach, in the same forms. With a dict, the names appear in the result |
| `minutes` | the times: one number or a list |

| Column of the result | What it is |
|---|---|
| `n_5min`, `n_10min` ... | the number of targets within each time |
| `nearest` | the nearest target: its name in the dict |
| `nearest_s` | the time to the nearest target, in seconds |

A start place with no target within the largest time has `None` in `nearest` and `NaN` in `nearest_s`.

Here the start places are all the walking edges of Monaco, and the targets are three places with names.""")
ch.code("""targets = {"Fontvieille": (7.4155, 43.7285),                     # three targets, with their names
           "the port": (7.4246, 43.7384),
           "Larvotto": (7.4335, 43.7468)}
homes = net.walking.edges().edge_id                               # each walking edge is a start place
acc = net.walking.accessibility(homes, targets, minutes=[5, 10])  # the targets within 5 and 10 minutes
acc.head()                                                        # one row for each start edge""")
ch.md("""Each row is one walking edge: the number of targets within 5 and 10 minutes, the nearest target and its time
in seconds. The fourth edge reaches no target within 10 minutes, so its `nearest` is `None`.

### Count the results

`value_counts` counts the edges for each number of targets.""")
ch.code("""acc.n_10min.value_counts().sort_index()      # how many edges reach 0, 1 or 2 targets within 10 minutes""")
ch.code("""acc.nearest.value_counts(dropna=False)       # how many edges have each target as the nearest""")
ch.md("""Within 10 minutes, 8,194 edges reach one target, 328 edges reach two, and 3,416 edges reach none. Fontvieille is
the nearest target of 3,517 edges, the port of 3,039 and Larvotto of 1,966.

### Draw it on the map

The map colors each walking edge by the minutes to its nearest target. An edge with no target within 10 minutes
stays gray.""")
ch.code("""net.map("walking", color_by=acc["nearest_s"] / 60, cmap="viridis")    # the minutes to the nearest target""")
ch.md("""The dark edges are near a target, and the yellow edges are about 9 minutes away.""")

ch.md("""## Use the results in other tools

duckOSM gives the input that other routing tools need: travel times, nodes and edges. This section shows two
tools that planners use:

| Tool | What it does | What duckOSM gives it |
|---|---|---|
| [OR-Tools](https://developers.google.com/optimization) | plans the order of visits, for example a delivery round | the travel time matrix (`matrix`) |
| [pandana](https://udst.github.io/pandana/) | counts places near every node of a network, for the whole city in one call | the walking nodes and edges |

### A delivery round with OR-Tools

A van starts at a depot in Fontvieille, visits 10 pharmacies of Monaco and comes back. In which order is the round
shortest? This is the travelling salesman problem. The steps:

1. Find the pharmacies with `duckosm.gis.features` (chapter 10), and the nearest driving edge of each one.
2. Make the travel time matrix between the depot and the pharmacies with `matrix`.
3. Give the matrix to OR-Tools. It gives the order of the visits.
4. Draw the round with `route`, one route for each pair of stops.

The first cell does steps 1 and 2. The stops are edge ids, so each cell of the matrix equals the time of `route`
(section "A cell equals the route").""")
ch.code("""from duckosm import analysis as an

pharmacies = duckosm.gis.features(net, tags={"amenity": "pharmacy"})[:10]      # 10 pharmacies (points)
xs = [7.4155] + list(pharmacies.geometry.x)                                     # the depot in Fontvieille, then them
ys = [43.7285] + list(pharmacies.geometry.y)
names = ["depot"] + [f"pharmacy {i}" for i in range(1, 11)]
stops = dict(zip(names, map(int, an.nearest_edges(net.driving, xs, ys))))       # name -> the nearest driving edge
times = net.driving.matrix(stops)                                               # 11 x 11 travel times in seconds
times.iloc[:4, :4].round()""")
ch.md("""The rows and the columns have the names of the stops. From the depot to pharmacy 1 is 193 s by car.

The second cell gives the matrix to OR-Tools. OR-Tools needs whole numbers and a function that gives the cost
between two stops. It returns the stops in the best order that it finds.""")
ch.code("""from ortools.constraint_solver import pywrapcp

cost = times.round().astype(int).to_numpy()                                    # whole seconds
manager = pywrapcp.RoutingIndexManager(len(names), 1, 0)                        # 11 stops, 1 van, start at stop 0
model = pywrapcp.RoutingModel(manager)
seconds = model.RegisterTransitCallback(lambda i, j: int(cost[manager.IndexToNode(i), manager.IndexToNode(j)]))
model.SetArcCostEvaluatorOfAllVehicles(seconds)                                 # the cost of each leg: its time
solution = model.SolveWithParameters(pywrapcp.DefaultRoutingSearchParameters())

i, order = model.Start(0), []
while not model.IsEnd(i):                                                       # read the stops in their order
    order.append(names[manager.IndexToNode(i)])
    i = solution.Value(model.NextVar(i))
order.append("depot")                                                           # the van comes back
order""")
ch.code("""def round_time(stop_order):
    \"\"\"The time of a round through the stops in this order, from the matrix, in seconds.\"\"\"
    return sum(times.loc[a, b] for a, b in zip(stop_order, stop_order[1:]))


round(round_time(order)), round(round_time(names + ["depot"]))                  # the best order, the order of the list""")
ch.md("""The best order takes 912 s of driving (15 min). The order of the list (pharmacy 1, 2, 3 ... 10) takes 1,398 s.
OR-Tools saves 486 s, about 35 %.""")
ch.code("""legs = [net.driving.route(stops[a], stops[b]) for a, b in zip(order, order[1:])]    # one route for each leg
edges = pd.concat([leg.edges.edge_id for leg in legs])                           # all the edges of the round
print(round(sum(leg.time_s for leg in legs)), "s,", round(sum(leg.length_m for leg in legs)), "m")
net.driving.plot(highlight=edges)                                               # the round in orange""")
ch.md("""The 11 routes take 912 s together, the same as the matrix, and the van drives 10,766 m. The orange round goes
from the depot in Fontvieille, in the southwest, through the pharmacies in the center and the northeast, and back.""")

ch.md("""### Cafés near every junction with pandana

`accessibility` answers "which targets can I reach" for the start places that you give. pandana answers a question
for every node of the network at once. Here the question is: how many cafés are within 500 m of walking from each
junction of Monaco? The steps:

1. Give pandana the walking nodes (their coordinates) and the walking edges (`source`, `target` and `length_m`).
2. Find the cafés with `duckosm.gis.features`, and put each one on its nearest node.
3. Ask pandana for the number of cafés within 500 m of each node.
4. Draw the nodes in the color of their number.

pandana prints the progress of its own preparation (the lines about "contraction hierarchies").""")
ch.code("""import pandana

nodes = net.walking.nodes().set_index("node_id")                                # 1. the walking junctions
walk = net.walking.edges()                                                       # and the walking edges
graph = pandana.Network(nodes.geometry.x, nodes.geometry.y, walk.source, walk.target, walk[["length_m"]],
                        twoway=False)                                           # each edge has one direction
cafes = duckosm.gis.features(net, tags={"amenity": "cafe"}).geometry.representative_point()    # 2. the cafés
graph.set(graph.get_node_ids(cafes.x, cafes.y), name="cafes")                   # each café on its nearest node
near = graph.aggregate(500, type="count", name="cafes")                         # 3. cafés within 500 m of each node
len(cafes), near.describe()[["count", "50%", "max"]]""")
ch.md("""OSM has 27 cafés in Monaco. From the middle junction, 3 cafés are within 500 m of walking. The best junction has
14.""")
ch.code("""fig, ax = plt.subplots(figsize=(8, 9))
net.walking.edges().plot(ax=ax, color="lightgray", linewidth=0.4)               # the walking network in light gray
points = ax.scatter(nodes.geometry.x, nodes.geometry.y, c=near.reindex(nodes.index), cmap="viridis", s=4)    # 4.
fig.colorbar(points, ax=ax, shrink=0.6, label="cafés within 500 m of walking")
ax.set_axis_off()""")
ch.md("""The most cafés are near the port and in the center of Monte Carlo (yellow and green). At the edges of Monaco,
few cafés or none are within 500 m (dark blue).""")

ch.md("""## The graph of all calls: `routed_graph`

### The turns that a mode can use

All the routing calls of this chapter search one graph: the turns of the mode, in the table `<mode>.edge_graph`
(chapter 7). But not every row of that table is for the mode. The column `uses` says who can make each turn. The
cell counts the rows of `driving.edge_graph` by `uses`.""")
ch.code('''net.sql("""SELECT uses, count(*) AS turns
           FROM driving.edge_graph GROUP BY uses ORDER BY turns DESC""")    # who can make each turn''')
ch.md("""The table has 6,754 turns. A car can make 5,082 of them (`car`). The others go onto or from an edge that is not
open to all cars: a pedestrian street (`pedestrian`), a private road (`private`) or a bus lane (`bus`). A car route
uses only the `car` turns.

### `routed_graph`: those turns, for your own SQL

`duckosm.routing.routed_graph(view)` gives the turns that the mode routes on. It gives them as a piece of SQL text, so
you can put it into your own query, in place of a table name.""")
ch.code("""g = duckosm.routing.routed_graph(net.driving)     # the turns that a car route uses, as SQL text
print(g)""")
ch.md("""The text selects the rows of `driving.edge_graph` with `uses = 'car'`. The next cell puts it into a query, after
`FROM`, and counts the turns and the edges where a car route can start: an edge with at least one turn to a next
edge.""")
ch.code("""net.sql(f"SELECT count(*) AS turns, count(DISTINCT from_edge) AS start_edges FROM {g}")""")
ch.md("""A car can make 5,082 turns, and a car route can start on 2,814 edges.

### The edges where a route cannot start

Monaco has 2,840 driving edges, so 26 edges have no turn to a next edge. The cell finds them and counts their road
classes.""")
ch.code('''net.sql(f"""SELECT highway, count(*) AS edges FROM driving.edges
           WHERE edge_id NOT IN (SELECT from_edge FROM {g})
           GROUP BY highway ORDER BY edges DESC""")                 # the 26 edges with no turn out''')
ch.md("""13 of the 26 edges are service roads and 8 are residential streets: the end of a dead-end street, or a garage
exit. The 5 main roads end at the boundary of the area: the road goes on in OSM, but the cut file stops there. A route
cannot leave such an edge, so `route` gives `None` for a route that starts on it, as in "Many routes in parallel"
above.""")

ch.md("""## Your own cost

### A cost that is not the time

Each call of this chapter makes one cost as small as possible: the time, or the length. Sometimes the best route
is the one with the smallest toll, the least CO2 or the least danger. `weight` also takes your own cost of each edge:

| `weight` | The cost of an edge |
|---|---|
| `"time"` (the default) | `cost_s`, the travel time in seconds |
| `"length"` | `length_m`, in metres |
| the name of a column, for example `"toll_s"` | the value of that numeric column of `<mode>.edges` |
| a pandas Series (or a dict) with the edge ids as its index | your value for each edge |

Your cost replaces the time of the edge. A cost must be 0 or more. The search stays in Rust: duckOSM gives the cost of
each edge to the router, so a route with your cost is as fast as a route by time.

An edge that has no value in your Series (or `NULL` in the column) gets a cost from `missing`:

| `missing` | An edge with no value |
|---|---|
| `"time"` (the default) | costs its time (`cost_s`) |
| `"skip"` | is not used: no route goes on it |

### A toll on Boulevard Louis II

Boulevard Louis II goes along the sea, through a tunnel, to Larvotto. Here it gets a made-up toll: each of its edges
costs 120 s more. The other edges cost their time.""")
ch.code("""fontvieille, larvotto = (7.4155, 43.7285), (7.4335, 43.7468)            # two points
louis = set(net.driving.edges(where="name = 'Boulevard Louis II'").edge_id)  # the edges of Boulevard Louis II
time_s = net.driving.edges().set_index("edge_id").cost_s                 # the time of each edge
toll = time_s.copy()
toll[toll.index.isin(louis)] += 120                                      # the toll: 120 s more on each edge
toll.name = "time_and_toll"
fast = net.driving.route(fontvieille, larvotto)                          # the fastest route
paid = net.driving.route(fontvieille, larvotto, weight=toll)             # the cheapest route with the toll
pd.DataFrame({"fastest": [fast.time_s, fast.length_m, len(louis & set(fast.edge_ids))],
              "with the toll": [paid.time_s, paid.length_m, len(louis & set(paid.edge_ids))]},
             index=["time_s", "length_m", "edges on Louis II"]).round()""")
ch.md("""The fastest route takes 229 s and 2,939 m, and 12 of its edges are on Boulevard Louis II. With the toll, the cheapest
route uses no edge of the boulevard: it takes 268 s and 3,308 m, 39 s and 369 m more. For this trip, the toll costs
more than the 39 s that the boulevard saves.""")
ch.code("""paid.weight, round(paid.cost)          # the name of the cost, and the cost of the route""")
ch.md("""`paid.weight` is the name of the Series, and `paid.cost` is the sum of your costs of its edges: 240. `paid.time_s`
stays the real time of the route, 268 s. The cost is less than the time, because a route between two points counts
only the used parts of its first and last edge, and in your cost the walks from the points to the roads cost 0. For
`"time"`, `route.cost` is `route.time_s`.""")
ch.code("""net.map("driving", routes={"fastest": fast, "with the toll": paid})     # the two routes""")
ch.md("""### An edge that no route uses: `missing="skip"`

To keep all routes off Boulevard Louis II, give the time of all other edges and use `missing="skip"`. The edges of
the boulevard are not in the Series, so no route goes on them.""")
ch.code("""no_louis = time_s[~time_s.index.isin(louis)]                              # each edge except the boulevard
around = net.driving.route(fontvieille, larvotto, weight=no_louis, missing="skip")
round(around.time_s), round(around.length_m), len(louis & set(around.edge_ids))""")
ch.md("""The route takes 268 s and 3,308 m, and it uses no edge of the boulevard (0). It is the route of the toll: a toll of
120 s on each edge is already enough to keep this trip off the boulevard.""")

ch.md("""### The cost of a change of mode at each junction

[7. Routing](07_routing.md) gives one cost to each change of mode (`transfer_s`), or one cost to each pair of modes
(`pair_costs`). `node_costs` gives a cost to the changes at some junctions. It is a DataFrame with the columns
`node_id` and `cost_s`, and optionally `from_mode` and `to_mode` (empty: each change at the junction). With
`only_listed_nodes=True`, a route can change mode only at those junctions.

Here a car can park only at a car park: a junction of a parking aisle (`service = 'parking_aisle'`) that walking also
has. A change at a car park costs 30 s.""")
ch.code('''parks = net.sql("""SELECT DISTINCT node_id FROM (SELECT source AS node_id FROM driving.edges WHERE service = 'parking_aisle'
                                     UNION SELECT target FROM driving.edges WHERE service = 'parking_aisle')
                   WHERE node_id IN (SELECT node_id FROM walking.nodes)""")      # the junctions of the car parks
net.add_multimodal(node_costs=pd.DataFrame({"node_id": parks.node_id, "cost_s": 30}),
                   only_listed_nodes=True)                                   # changes of mode only at the car parks
net.sql("SELECT kind, count(*) AS changes, min(cost_s) AS cost_s FROM mm.transfers GROUP BY kind ORDER BY kind")''')
ch.md("""Monaco has 51 such junctions. The multimodal graph now has 204 changes: at each of the 51 junctions the four kinds
`park`, `retrieve`, `bike_park` and `bike_unpark`, each 30 s. The rows of `node_costs` have no modes, so they are for
each change: a bicycle also changes only at the car parks. The graph of chapter 7 has 12,140 changes.""")
ch.code("""trip = net.route_multimodal_points(fontvieille, larvotto)                 # a trip that parks at a car park
[(leg["mode"], len(leg["edges"]), round(leg["time_s"])) for leg in trip["legs"]], round(trip["time_s"])""")
ch.md("""The trip walks on 7 edges (111 s) to a car park, drives on 64 edges (184 s), parks at a car park and walks on 24
edges (555 s) to Larvotto. The total is 917 s: the legs, the two changes of 30 s and the walks from the points to the
roads. The car-only route takes 229 s, so here the trip with the car parks is much slower: the car park where it parks
is more than 9 minutes on foot from Larvotto. The last cell gives the file the multimodal graph of chapter 7 again,
for the next chapters.""")
ch.code("""net.add_multimodal()          # the multimodal graph of chapter 7 again: a change at each shared junction, 60 s""")
ch.md("""> **Tip:** the same costs work without Python: `duckosm multimodal monaco.duckdb --node-costs parks.csv
> --only-listed-nodes`, or `multimodal: {node_costs: parks.csv, only_listed_nodes: true}` in a build config.""")

ch.md("""## Try it yourself

**1.** Find the three best walking routes from the casino square to the port, and compare their times.

<details><summary>Show the answer</summary>

```python
walk = net.walking.k_shortest_paths((7.4280, 43.7393), (7.4246, 43.7384), k=3)
[round(x.time_s / 60, 1) for x in walk]      # minutes of each route
```
</details>

**2.** Make a matrix of walking distances in metres between the three targets of the accessibility section.

<details><summary>Show the answer</summary>

```python
net.walking.matrix(list(targets.values()), weight="length").round()     # rows and columns 0, 1, 2
```
</details>

**3.** Draw the 5 and 10 minute isochrones by car from the port, with a buffer of 40 m.

<details><summary>Show the answer</summary>

```python
iso_car = net.driving.isochrones([(7.4246, 43.7384)], minutes=[5, 10], buffer_m=40)
iso_car.iloc[::-1].plot(column="minutes", categorical=True, legend=True)
```
</details>

**4.** Find the walking edges that have no target within 10 minutes, and draw them on the map.

<details><summary>Show the answer</summary>

```python
far = acc[acc.n_10min == 0].index           # the origin edge ids
net.map("walking", highlight=list(far))
```
</details>""")

ch.glossary([
    ("`Route`", "the result of a route: the edges, the length, the time, the line, the steps and a map"),
    ("weight", "what a route makes smallest: `\"time\"` (`cost_s`) or `\"length\"` (`length_m`)"),
    ("Dijkstra's algorithm", "a search that finds the cheapest route from one place to all other places"),
    ("Yen's algorithm", "a search that finds the `k` best routes that use no edge two times"),
    ("matrix", "a table of the travel times (or distances) from each origin to each destination"),
    ("isochrone", "the area that you reach from a place within a time"),
    ("accessibility", "the number of targets that you reach from a place within a time, and the nearest one"),
])

ch.summary([
    ("`net.driving.route(a, b, weight=...)`", "the best route, a `Route` (or `None`)"),
    ("`r.edge_ids`, `r.edges`, `r.length_m`, `r.time_s`", "the edges and the sums of a route"),
    ("`r.geometry`, `r.directions()`, `r.map()`", "the line, the steps and the map of a route"),
    ("`net.driving.k_shortest_paths(a, b, k=3)`", "the `k` best routes, the best first"),
    ("`net.driving.routes(origins, destinations)`", "a route for each pair (`zip`), in parallel"),
    ("`net.driving.matrix(origins, destinations, max_cost=...)`", "a travel time (or distance) matrix, a DataFrame"),
    ("`net.walking.isochrones(origins, minutes, buffer_m=...)`", "the polygons reached within each time"),
    ("`net.walking.accessibility(origins, targets, minutes)`", "the targets reached from each origin, and the nearest"),
    ("`duckosm.routing.routed_graph(view)`", "the turns that the mode routes on, as SQL text for your own query"),
    ("`net.driving.route(a, b, weight=series, missing=\"skip\")`", "the route with your own cost of each edge"),
    ("`net.add_multimodal(node_costs=df, only_listed_nodes=True)`", "the cost of a change of mode at each junction"),
], reference="[duckosm.routing](../reference/sdk/routing.md)")

ch.write(sys.argv[1])
