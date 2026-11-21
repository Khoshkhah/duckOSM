"""Write docs/notebooks/11_mobility.ipynb: chapter 11 of the notebook manual, "Mobility data".
The style comes from scripts/notebook_book.py (rules: docs/design/notebook_book.md).

    python scripts/notebook_chapter11.py docs/notebooks/11_mobility.ipynb"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from notebook_book import IMAGES, Chapter  # noqa: E402

ch = Chapter(11, "11_mobility", "Mobility data",
             "GPS traces, sensors and counts on the edges of the network",
             learn="what mobility data is; how map matching works; `match` for GPS traces; `snap` for sensors; "
                   "counts for each edge and hour with `aggregate`; your data on the edges with `join`; "
                   "how your data stays after a rebuild with `edge_id_map`",
             time="about 2 minutes", extras="analysis,viz",
             next_=("12_sim", "12. Traffic simulation", "SUMO and MATSim networks, and the results back on the edges"))

ch.md(f"""## What mobility data is

**Mobility data** tells you how people and vehicles move on the roads. It comes in three kinds. Each kind needs one
step before you can use it with the network: you must find the edges that it belongs to.

| Data | What it is | The question | The call |
|---|---|---|---|
| GPS traces | a chain of points of one vehicle or person: a phone, a taxi, a bus, a bicycle | which edges did the trip use? | `match` |
| Fixed points | one point with no route: a traffic sensor, a camera, an address, a shop | which edge is the point on? | `snap` |
| Values on the edges | counts, speeds or any number for each edge (often the result of `match` or `snap`) | how many, and when? | `aggregate`, `join` |

The module `duckosm.mobility` has these calls. Each call takes a `ModeView` (`net.driving`), a `Network` with one
mode, or the path of a file with `mode=`. The snap, the map matching and the edge id map run in Rust, in the same
engine as the build.

> **Key idea:** every kind of mobility data ends as rows with an `edge_id`. When your data has the `edge_id`, you can
> join it to the edges, draw it on a map, compare it with other data, and keep it after the next build.""")

ch.md(f"""## How map matching works

A GPS point is not exactly on the road. A phone in a city is often 5 to 10 m off, and more between tall buildings.
The nearest edge of each point is therefore often wrong: near a junction, the nearest edge can be a side street, and
on a two-way road the nearest edge can be the other direction. **Map matching** finds the route that the vehicle
drove: a chain of edges that is near the points *and* is a possible drive from one point to the next.

`match` uses the hidden Markov model of Newson and Krumm (2009), solved with the Viterbi algorithm. leuvenmapmatching,
Valhalla and GraphHopper use the same model.

<img src="{IMAGES}/mapmatch-flow.svg" alt="The steps of map matching: the GPS points, the candidate edges, the probabilities, the Viterbi algorithm and the match" width="100%">

| Part | What it does | Option |
|---|---|---|
| Candidates | each edge within `radius_m` of a point is a possible edge for it (a state) | `radius_m`, default 50 m |
| Emission | a point is more probable on a near edge: a normal distribution of the distance | `sigma_m`, the GPS error, default 5 m |
| Transition | a step from one edge to the next is more probable when the route along the roads is as long as the straight line between the two points | `beta_m`, default 10 m |
| U-turn | a turn onto the other direction of the same road costs more | `uturn_m`, default 50 m |
| Viterbi | the algorithm finds the chain of candidates with the highest total probability | |

The route between two points follows the edge directions, so on a two-way road the match selects the edge of the
travel direction. When no candidate of a point can follow the point before it (a gap in the trace), the trace starts
again at that point. The turn restrictions do not apply in the match.

> **Tip:** the reference page [duckosm.mobility](../reference/sdk/mobility.md) gives the accuracy of `match` on
> Monaco for 3 m and 5 m of GPS noise.""")

ch.md("""## Build the network

The chapter uses the driving network of Monaco. `drive` is the view of the driving mode: the calls of
`duckosm.mobility` take it as their first argument.""")
ch.code("""from duckosm import mobility as mob                                         # the module of this chapter
net = duckosm.network(osm_id=2220322, modes=["driving"], name="monaco")     # get and build the driving network of Monaco
drive = net.driving                                                         # the driving mode
drive                                                                       # the card""")
ch.md("""The driving network of Monaco has 2840 edges and 1768 nodes, 96.2 km of roads.""")

ch.md("""## Make GPS traces

You need GPS traces to try `match`. This section makes two traces from known routes, so you can compare the match
with the truth. A real GPS file has the same four columns:

| Column | Meaning |
|---|---|
| `trace_id` | the trip that the point belongs to |
| `x` | the longitude, in degrees |
| `y` | the latitude, in degrees |
| `time` | the time of the point: `match` puts the points of each trace in this order |

The function `gps` takes a route of `drive.route` (chapter 7), puts a point every 15 m along it, and moves each point
up to about 4 m off the road at random. The vehicle drives at 8 m/s (about 29 km/h).""")
ch.code("""import geopandas as gpd
import numpy as np
import pandas as pd
from shapely.ops import linemerge

def gps(route, trace_id, seed):                                        # made-up GPS points along a route
    roads = net.edges(crs="utm").set_index("edge_id").loc[route["edges"]]    # the edges of the route, in metres
    line = linemerge(list(roads.geometry))                             # one line
    rng = np.random.default_rng(seed)                                  # noise that does not change
    d = np.arange(0, line.length, 15.0)                                # a point every 15 m
    xy = np.array([line.interpolate(m).coords[0] for m in d]) + rng.normal(0, 4, (len(d), 2))    # 4 m off the road
    p = gpd.GeoSeries(gpd.points_from_xy(xy[:, 0], xy[:, 1]), crs=roads.crs).to_crs(4326)    # back to lon, lat
    return pd.DataFrame({"trace_id": trace_id, "x": p.x, "y": p.y,
                         "time": pd.Timestamp("2026-11-20 08:00") + pd.to_timedelta(d / 8, "s")})    # 8 m/s

A, B = (7.4155, 43.7285), (7.4335, 43.7468)                            # two points in Monaco
there, back = drive.route(A, B), drive.route(B, A)                     # the true routes
traces = pd.concat([gps(there, "there", 1), gps(back, "back", 2)], ignore_index=True)    # the GPS file
print(len(traces), "points;", traces.groupby("trace_id").size().to_dict())    # the points of each trace
traces.head()                                                          # the first points""")
ch.md("""The two traces have 396 points: 196 for "there" and 200 for "back". The points of each trace are 1.875 s apart
(15 m at 8 m/s). The trace "there" drives from the south-west of Monaco to the north-east, and "back" drives the
other way.""")

ch.md("""## Match the traces: `match`

`mob.match(view, traces)` gives a `Match` with two tables:

| Table | One row for | Columns |
|---|---|---|
| `m.points` | each GPS point | the columns of your points, and `edge_id`, `fraction`, `distance_m`, `snap_x`, `snap_y` |
| `m.edges` | each edge of the route of each trace, in order | `trace_id`, `seq`, `edge_id` |

`m.edges` includes the edges between two points, so the route has no gaps even when the points are far apart. The
column names of your file can differ: give them with `trace=`, `time=`, `x=` and `y=`. `trace=None` matches all the
points as one trace.""")
ch.code("""m = mob.match(drive, traces)                                          # map matching
for name, true in [("there", there), ("back", back)]:                     # each drive
    got = m.edges[m.edges.trace_id == name].edge_id                       # its matched edges
    print(f"{name}: {got.isin(true['edges']).sum()} of {len(true['edges'])} route edges found, "
          f"{(~got.isin(true['edges'])).sum()} other")                     # compare with the true route""")
ch.md("""**What the output shows.** The match finds 74 of the 75 edges of the route "there" and all 64 edges of "back". It adds 5 and 2 other edges. Both drives use the same two-way roads in opposite directions, and the
match gives each drive the edges of its own direction. With 4 m of GPS noise, a point near a junction can add the
edge before or after the route, or a short road next to it.""")

ch.md("""### The points

`m.points` has one row for each GPS point, with the edge of the point and the place on the edge.

| Column | Meaning |
|---|---|
| `edge_id` | the edge of the point (missing when no edge is within `radius_m`) |
| `fraction` | the place on the edge: 0 at its start, 1 at its end |
| `distance_m` | the distance from the GPS point to the edge, in metres |
| `snap_x`, `snap_y` | the point on the edge (lon, lat) |""")
ch.code("""m.points[["trace_id", "time", "edge_id", "fraction", "distance_m"]].head()    # the points on the edges""")
ch.md("""The first rows are the trace "back": `match` sorts the points by `trace_id`, then by
`time`. Each point is a few metres from its edge, as the noise of the traces. Two points can share one edge, with a
larger `fraction` for the later point.""")

ch.md("""### The route of each trace

`m.edges` is the route of each trace, edge by edge. `seq` counts the edges of one trace from 0. With
`net.edges(where=...)` you get the names and the lengths of the edges of the route.""")
ch.code("""route = m.edges[m.edges.trace_id == "there"]                              # the route of one trace
names = net.edges(where=f"edge_id IN ({','.join(map(str, route.edge_id))})").set_index("edge_id")
route = route.join(names[["name", "length_m"]], on="edge_id")             # the name and the length of each edge
print(round(route.length_m.sum()), "m matched;", round(there["length_m"]), "m true")    # the two lengths
route.head(8)""")
ch.md("""**What the output shows.** The matched route is 3012 m long, and the true route is 2939 m: the 73 m more are the
five other edges. The first edges are on the Avenue des Castelans and the Avenue Albert II, in Fontvieille, in the
south-west of Monaco. An edge with `None` as its name has no name in OSM.""")

ch.md("""### Draw the match

`net.plot(highlight=...)` draws the network and shows the edges of the list in orange. The black dots are the GPS
points.""")
ch.code("""import matplotlib.pyplot as plt

ax = plt.subplots(figsize=(7, 7))[1]                                       # a new picture
net.plot(highlight=m.edges.edge_id, ax=ax)                                 # the matched edges in orange
ax.scatter(traces.x, traces.y, s=1, color="black", alpha=0.5, zorder=3); plt.show()    # the GPS points""")
ch.md("""The orange route follows the black GPS points from one end to the other. The two
traces drive on the same roads in most places, so one orange line shows both.""")

ch.md("""### The options of the match

`radius_m` sets the candidates of a point. A smaller radius is faster, but a point that is further from every edge
gets no edge. This cell matches the same traces with `radius_m=5`.""")
ch.code("""tight = mob.match(drive, traces, radius_m=5)                              # only the edges within 5 m
print(tight.points.edge_id.isna().sum(), "of", len(tight.points), "points without an edge")
print(len(tight.edges), "route edges, against", len(m.edges), "with radius_m=50")""")
ch.md("""**What the output shows.** With `radius_m=5`, 45 of the 396 points have no edge. The route has 208 edges, against
145 with the default radius. The extra edges are detours: when the correct edge of a point is not a candidate, the
match must use a different edge, and some edges come two times. Keep `radius_m` much larger than the GPS error.

> **Tip:** set `sigma_m` to the GPS error of your data. A phone in a car is about 5 m (the default). For data with
> large errors, increase `sigma_m` and `radius_m` together. For points that are far apart in time (one point each
> minute), increase `beta_m`, because the route between two points is then much longer than the straight line.""")

ch.md("""## Snap fixed points: `snap`

A sensor, an address or a shop is one point with no route. `mob.snap(view, points)` gives the nearest edge of each
point, one point at a time. It adds the same columns as `m.points`. Use `snap` only for points that do not belong to
a trace: for GPS traces, `match` is more correct.

`radius_m` is the largest distance. A point with no edge within it gets no `edge_id`.""")
ch.code("""sensors = pd.DataFrame({"sensor": ["s1", "s2", "s3"], "x": [7.4206, 7.4262, 7.4301], "y": [43.7316, 43.7397, 43.7427]})
snapped = mob.snap(drive, sensors, radius_m=30)                            # edge_id, fraction, distance_m, snap_x, snap_y
snapped""")
ch.md("""Each sensor gets one edge. The sensors are 6 to 14 m from their edges. Sensor s1 is
near the end of its edge (`fraction` 0.92), and s2 is near the middle (0.52). `snap_x` and `snap_y` are the points
on the edges: draw them to check the snap.""")

ch.md("""### Other kinds of points

`points` can be a DataFrame with lon and lat columns (`x=` and `y=` give other names), a GeoDataFrame in any CRS, or
a list of `(lon, lat)`. Here it is a list.""")
ch.code("""mob.snap(drive, [(7.4246, 43.7384), (7.4100, 43.7300)], radius_m=30)    # a list of (lon, lat)""")
ch.md("""A list gives a DataFrame with the columns `x` and `y`, and the columns of the snap.
The second point is 0.07 m from its edge: it is almost on the road.""")

ch.md("""## Count on each edge: `aggregate`

`mob.aggregate(view, matched)` counts the points on each edge in each time bin. `matched` is `m.points` or the result
of `snap`.

| Argument | Default | What it does |
|---|---|---|
| `time` | `"hour"` | the bin: `"hour"`, `"15min"`, `"day"` or any pandas frequency, or `None` for no time bins |
| `time_column` | `"time"` | the column of the time |
| `value` | none | a column of numbers: adds `<value>_mean`, its mean in each bin |
| `by` | `"edge_id"` | the column to group by |

The result has the columns `edge_id`, `time` (the start of the bin), `count`, and `<value>_mean`.""")
ch.code("""counts = mob.aggregate(drive, m.points, time="hour")                     # edge_id, time, count
print(len(counts), "rows;", counts["count"].sum(), "points")              # every point is in one row
counts.head()""")
ch.md("""The 396 points are on 119 edges, so the table has 119 rows. All the points are between 08:00 and 09:00, so each edge has one row
for the hour 08:00.""")

ch.md("""### Smaller time bins and a mean value

With `time="5min"`, each edge gets one row for each 5 minutes in which it has points. With `value="distance_m"`, each row also has the
mean distance of its points from the edge: a measure of the GPS error on that edge.""")
ch.code("""q = mob.aggregate(drive, m.points, time="5min", value="distance_m")     # bins of 5 minutes, and the mean distance
print(sorted(q.time.dt.strftime("%H:%M").unique()))                       # the bins
q.sort_values("distance_m_mean", ascending=False).head(3)                  # the edges with the largest GPS error""")
ch.md("""**What the output shows.** Each drive takes about 6 minutes, so the points are in two bins, 08:00 and 08:05. The
largest mean distance is 29.8 m, for one point in the bin 08:00. The same edge has one more point in the bin 08:05,
at 7.0 m: the bins split the points of one edge by their time. One point far from its edge is often a point near a
junction, where the next edge is near too.""")

ch.md("""## Put your data on the edges: `join`

`mob.join(view, data)` gives a GeoDataFrame of the edges with the columns of `data`. `data` is a DataFrame with an
`edge_id` column, or a Series whose index is the `edge_id`. `how="inner"` (the default) keeps only the edges with data,
and `how="left"` keeps all the edges, with no value where there is no data.""")
ch.code("""by_edge = counts.groupby("edge_id")["count"].sum()                         # the points on each edge
on_map = mob.join(drive, by_edge)                                          # the edges with data
print(len(on_map), "edges with data;", len(mob.join(drive, by_edge, how="left")), "edges in all")
on_map[["edge_id", "name", "count"]].nlargest(3, "count")                 # the three edges with the most points""")
ch.md("""119 edges have data, of the 2840 edges of the network. The Boulevard du Larvotto has the
most points (21), then the Tunnel Dorsale (17) and the Tunnel Rocher Palais (14). A long edge gets more points, because the traces put one point every 15 m.""")
ch.code("""net.plot(color_by=by_edge, cmap="magma_r")                                 # the map, colored by the count""")
ch.md("""The colors show the number of points on each edge. The long edges of the route
(the boulevard along the sea and the tunnels) are the darkest.""")

ch.md("""## Keep your data after a rebuild: `edge_id_map`

The `edge_id` of an edge stays the same in a new build when its OSM way and its two ends do not change. An OSM update
or a new fix can split or join edges, and these edges get new ids. Your counts are on the old ids.
`mob.edge_id_map(old, new)` puts the old ids on the new edges:

| Old edge | Rows of `edge_id_map` |
|---|---|
| its `edge_id` is in the new build | one row to itself, `share` 1 |
| split or joined | one row for each new edge of the same OSM way that lies on it in the same direction (within `tol_m`, default 2 m), with the `share` of its length |
| removed | no row |

Here a fix removes the dead-end street 778494662 (chapter 5 tells more about fixes). Its T junction goes, so the road
that it met becomes one edge in each direction, with a new id.""")
ch.code("""open("fix.yaml", "w").write("overrides:\\n  - osm_id: 778494662\\n    remove: true\\n")    # the fix: remove the street
new = duckosm.network(osm_id=2220322, modes=["driving"], fixes="fix.yaml", name="monaco_fixed")    # build again with it
ids = mob.edge_id_map(drive, new.driving)                                # old edge_id -> new edge_id, share
print(len(ids), "rows;", (ids.old_edge_id == ids.new_edge_id).sum(), "edges keep their id")
ids[ids.old_edge_id != ids.new_edge_id]                                  # the edges that changed""")
ch.md("""The old network has 2840 edges, and the map has 2838 rows. 2834 edges keep their id. The
four changed rows are two old edges in each direction: each pair becomes one new edge. The two edges of the removed
street have no row.""")

ch.md("""### Move the counts to the new edges

Join your data on `old_edge_id`, and multiply each value by `share`. A count is then divided between the new edges
when an old edge is split, and added when old edges are joined.""")
ch.code("""moved = counts.merge(ids, left_on="edge_id", right_on="old_edge_id")     # the counts on the new edges
moved["count"] = moved["count"] * moved.share                            # a split edge shares its count
print(counts["count"].sum(), "points before,", round(moved["count"].sum(), 1), "after")    # no count is lost""")
ch.md("""The 396 points of the old edges are all on the new edges. No point was on the removed
street.

> **Warning:** a mean (a speed, a distance) must not be multiplied by `share`. Multiply only values that you can add,
> for example counts and lengths. For a mean, give each new edge the value of its old edge.""")

ch.exercises([
    ("Match only the trace \"back\" as one trace, with `trace=None`, and count its route edges.",
     """one = mob.match(drive, traces[traces.trace_id == "back"], trace=None)   # all the points are one trace
len(one.edges)"""),
    ("Add about 6 m more GPS noise to the trace \"there\", match it with `sigma_m=10`, and compare the found edges.",
     """noisy = pd.concat([gps(there, "there", 1)], ignore_index=True)
noisy[["x", "y"]] += np.random.default_rng(3).normal(0, 0.00008, (len(noisy), 2))   # about 6 m more in each axis
mn = mob.match(drive, noisy, sigma_m=10)                                   # a larger GPS error
mn.edges.edge_id.isin(there["edges"]).sum(), len(there["edges"])"""),
    ("Snap the sensors, then use `join` with `how=\"left\"` to find how many edges have no sensor.",
     """s = mob.snap(drive, sensors, radius_m=30)
j = mob.join(drive, s[["edge_id", "sensor"]], how="left")
j.sensor.isna().sum()"""),
])

ch.glossary([
    ("GPS trace", "the points of one trip, in the order of their time"),
    ("map matching", "finding the edges that a GPS trace drove"),
    ("candidate", "an edge near a GPS point that can be its edge"),
    ("hidden Markov model", "a model of states that you cannot see (the edges) and points that you see (the GPS points)"),
    ("Viterbi algorithm", "the algorithm that finds the most probable chain of states"),
    ("snap", "putting one point on its nearest edge"),
    ("fraction", "the place of a point on its edge: 0 at the start, 1 at the end"),
    ("time bin", "a part of the time, for example one hour, in which `aggregate` counts"),
    ("share", "the part of an old edge that lies on a new edge, in `edge_id_map`"),
])

ch.summary([
    ("`mob.match(view, traces, ...)`", "map matching: `Match(points, edges)`, the edge of each point and the route of each trace"),
    ("`mob.Match`", "the result of `match`: `points` and `edges`"),
    ("`mob.snap(view, points, radius_m=...)`", "the nearest edge of each point, for sensors and addresses"),
    ("`mob.aggregate(view, matched, time=..., value=...)`", "counts (and means) for each edge and time bin"),
    ("`mob.join(view, data, how=...)`", "your data on the edges, as a GeoDataFrame"),
    ("`mob.edge_id_map(old, new, tol_m=...)`", "the old edge ids on the edges of a new build, with the share"),
], reference="[duckosm.mobility](../reference/sdk/mobility.md)")

ch.write(sys.argv[1])
