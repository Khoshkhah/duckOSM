"""Write docs/notebooks/01_area_to_network.ipynb: chapter 1 of the notebook manual, "Get and build an area".
The style comes from scripts/notebook_book.py (rules: docs/design/notebook_book.md).

    python scripts/notebook_chapter1.py docs/notebooks/01_area_to_network.ipynb"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from notebook_book import Chapter  # noqa: E402

ch = Chapter(1, "01_area_to_network", "Get and build an area",
             "From a place, a box, a polygon or your own OSM file to one network file",
             learn="the parts of a duckOSM build, the modes (driving, walking, cycling), seven ways to choose an area, your own PBF file, the build options and `config=`, how you save a network",
             time="about 3 minutes", extras="widget,viz",
             next_=("02_inspect", "2. Look inside the file", "the tables, the columns and SQL"))

ch.md("""## What an area is

An **area** is the part of the world whose roads you want in one file: a city, a district, a park, a box around a
point, or a region. duckOSM describes an area with one polygon, the **boundary**. The build keeps the roads in the
boundary. A road that crosses the boundary stays whole, so the network has no cut roads at its edge. You can give the
area in seven ways (section "Seven ways to choose an area"). Each way gives one boundary.

The picture shows two boundaries for "Monaco". The name finds the country, and its boundary includes a large part of
the sea. The OSM id 2220322 is the city. The gray lines are the roads. Some roads cross the city boundary, and they
stay whole.

<img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/notebooks/images/area-boundary.png" alt="Two boundaries for Monaco: the country with its sea, and the city" width="100%">

> **Key idea:** an area is a polygon. Each way to choose an area (a name, an id, a box, a point, a file) ends as one
> boundary polygon, and the build keeps the roads in it.""")

ch.md("""## The parts of a build

A duckOSM build has three steps.

<img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/notebooks/images/build-flow.svg" alt="The flow of a build: your area, the boundary, the extract, the cut, the network" width="100%">

| Step | What happens | The result |
|---|---|---|
| 1. The boundary | The area becomes a polygon: from a name or an OSM id (Nominatim, the search service of OSM), a box, a point, an H3 cell or your own GeoJSON | `<name>.geojson` |
| 2. The OSM data | duckOSM finds the smallest Geofabrik extract (a file of OSM data for a country or a region) that holds the boundary, downloads it one time into a cache, and cuts it to the boundary | `<name>.osm.pbf` |
| 3. The network | The Rust engine reads the cut file and writes the roads of each mode, the nodes, the graph of turns, the base map and more | `<name>.duckdb` |

The result is **one DuckDB file**. Each mode is a schema in it (`driving`, `walking`, `cycling`), with the tables
`edges` and `nodes`. Each edge has an `edge_id`, a number that stays the same when you build the area again, so your
own data can join to it (chapter 4).

> **Key idea:** one area is one file. The networks of all modes, the nodes, the turns and the base map of the area
> are in the same `.duckdb` file, and `edge_id` connects them to your own data.

> **Tip:** you can open the file with any DuckDB tool (the `duckdb` program, DBeaver, Python's `duckdb`), not only
> with duckOSM.""")

ch.md("""### Four calls: `get`, `build`, `network` and `open`

Each call of the SDK does one or more of the steps. Learn these four first:

| Call | You give | It does | You get |
|---|---|---|---|
| `duckosm.get(...)` | an area: a name, an OSM id, a box, a point ... | steps 1 and 2: the boundary, and the OSM data cut to it | an **Area**: the files `monaco.osm.pbf` and `monaco.geojson` |
| `area.build(modes=...)` | an Area | step 3: the road networks from the OSM file | a **Network**: the file `monaco.duckdb` |
| `duckosm.network(...)` | an area, and the modes | the three steps: `get` and `build` in one call | a **Network** |
| `duckosm.open("monaco.duckdb")` | a network file that you built before | reads the file: no download and no build | a **Network** |

```text
duckosm.get(...) ──> Area (.osm.pbf + .geojson) ──> area.build(...) ──> Network (.duckdb)
└──────────────────────────── duckosm.network(...) ───────────────────────────┘

duckosm.open("monaco.duckdb") ──> Network                    (a file that you built before)
```

| You want to | Use |
|---|---|
| get the network of an area in one step | `duckosm.network` |
| look at the area or the OSM file before the build, or build the same area again with other options | `duckosm.get`, then `area.build` |
| use a network that you built before | `duckosm.open`. It needs only the file: no OSM id and no modes |

`duckosm.build(pbf=..., boundary=...)` does the same as `area.build`, but it takes the two files directly, not an
Area.

> **Key idea:** build one time, open many times. A build reads the OSM data. `duckosm.open` reads only the finished
> file, so the next chapters of this book open `monaco.duckdb` and build nothing.""")

ch.md("""## The modes

A **mode** is one network for one type of traveler. Each mode keeps the roads that it can use, with its own rules.

| Mode | Keeps | Leaves out |
|---|---|---|
| `driving` | the roads for cars (motorway to residential, service roads, ramps), with one-way rules and turn restrictions | footways, paths, steps, private roads |
| `walking` | footways, paths, steps, pedestrian streets, sidewalks, and the roads where people can walk | motorways and roads where walking is not allowed |
| `cycling` | cycleways, roads for bicycles, and footways where you can push the bicycle | motorways, steps |

The picture shows Monaco. In each panel, all the roads are gray, and the roads of that mode have its color. Walking
has the steps and footways of the old town. Driving has the tunnel loop in the west and no footways.

<img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/notebooks/images/modes-monaco.png" alt="Monaco: the driving, walking and cycling networks" width="100%">

> **Key idea:** a road can be in more than one mode. A street with sidewalks is in all three, with the same
> `edge_id` in each mode. So you join your data one time, and it is correct in every mode.""")

ch.md("""## Quick start: get and build Monaco in one call

`duckosm.network` gets the area and builds it. Here the area is the city of Monaco, OSM relation 2220322.

| Argument | What it does |
|---|---|
| `osm_id` | the area: an OSM relation (section "Seven ways to choose an area" gives the other ways) |
| `modes` | the networks to build: `"driving"`, `"walking"`, `"cycling"` |
| `name` | the name of the files: `monaco.duckdb`, `monaco.osm.pbf`, `monaco.geojson` |""")
ch.code("""net = duckosm.network(osm_id=2220322, modes=["driving", "walking", "cycling"], name="monaco")    # get and build Monaco
net                                                                                 # the card of the network""")
ch.md("""**What the output shows.** The first line counts the OSM data of the cut: 34461 nodes, 5685 ways and 339
relations. The card shows the file (37.0 MB) and, for each mode, the number of edges and nodes and the length in km.
The file is in duckOSM's work folder (`~/.cache/duckosm/work`), so the card says **saved: no**. Section "Save your
network" shows how you keep it.

> **Warning:** the name "Monaco" finds the *country*, with its sea. When a name gives the wrong area, find the OSM id:
> search the place on [openstreetmap.org](https://www.openstreetmap.org), open the result, and copy its "Relation"
> number into `osm_id=`.""")

ch.md("""## Count the edges of each mode: `net.summary()`

`net.modes` lists the modes of the file. `net.summary()` gives one row for each mode.

| Column | What it is |
|---|---|
| `edges` | the number of edges. A two-way road is two edges, one for each direction |
| `nodes` | the number of nodes: the points where edges meet |
| `km` | the length of all edges |
| `oneway_km` | the length of the one-way edges |""")
ch.code("""print(net.modes)       # the modes in the file
net.summary()          # one row for each mode""")
ch.md("""Walking has 11938 edges and 260.6 km. Driving has 2840 edges and 96.2 km. The walking network is larger because it
has the footways, the steps and the paths, and both sides of the streets where OSM draws the sidewalks. Walking has
0.0 km of one-way edges: a person on foot can walk in both directions on each road.

`net.walking` (or `net["walking"]`) is the **view** of one mode. It has the calls of the network, without the
argument `mode=` (chapter 2).""")
ch.code("""net.walking            # the card of one mode""")

ch.md("""## Seven ways to choose an area

`duckosm.get` does steps 1 and 2. It does not build. It gives an `Area`, which keeps the paths of the two files of
the area: `area.pbf` (the cut OSM file) and `area.boundary` (the polygon). Each way to choose an area is one argument.

| Argument | The boundary that duckOSM makes | Example |
|---|---|---|
| `place` | the polygon of the first answer of Nominatim for a name | `duckosm.get("Monaco")` |
| `osm_id` | the polygon of an OSM relation (a city, a district, a park) | `duckosm.get(osm_id=2220322)` |
| `bbox` | a rectangle: west, south, east, north, in degrees | `duckosm.get(bbox=(7.41, 43.72, 7.44, 43.75))` |
| `point` and `dist_m` | the square `dist_m` meters around a point (lon, lat) | `duckosm.get(point=(7.4246, 43.7384), dist_m=500)` |
| `h3_cell` | the hexagon of one H3 cell (a cell of Uber's H3 grid) | `duckosm.get(h3_cell="883969a40bfffff")` |
| `boundary` | your own polygon: a GeoJSON file, a GeoJSON dict, or a shapely or GeoPandas object | `duckosm.get(boundary="district.geojson")` |
| `pbf` (in `network`) | no cut: the area of the whole file | section "Your own PBF file" |

The argument `out` sets the folder of the two files. The examples below write into `data/`. The extract stays in
the cache, so the next areas in Monaco need no download.""")

ch.md("""### A box around a point

`point` with `dist_m` is the same as OSMnx's `graph_from_point`. Here it is a box of 2 × 500 m around the casino
square of Monte Carlo.""")
ch.code("""casino = duckosm.get(point=(7.4280, 43.7393), dist_m=500, name="casino", out="data")   # an Area: the file and the boundary
casino                                                                                  # the card of the area""")
ch.md("""The first line counts the OSM data in the box: 11894 nodes, 1804 ways and 181 relations, about a third of the
city. The card gives the two files and their sizes: `data/casino.osm.pbf` (0.3 MB) and `data/casino.geojson`.""")

ch.md("""### A box in degrees

`bbox` takes west, south, east and north, in this order (the order of GeoJSON and of OSMnx 2).""")
ch.code("""harbor = duckosm.get(bbox=(7.418, 43.730, 7.430, 43.738), name="harbor", out="data")     # the harbor of Monaco
harbor""")

ch.md("""### Your own polygon

`boundary` takes a polygon. Here it is a shapely polygon around Monte Carlo. A GeoJSON file or a GeoPandas object
works the same way. duckOSM writes the polygon to `data/monte_carlo.geojson`.""")
ch.code("""from shapely.geometry import Polygon
monte_carlo = Polygon([(7.420, 43.737), (7.432, 43.737), (7.432, 43.745), (7.420, 43.745)])   # (lon, lat) corners
mc = duckosm.get(boundary=monte_carlo, name="monte_carlo", out="data")
mc""")

ch.md("""### Look before you get: `dry_run`

`dry_run=True` downloads nothing and writes nothing. It gives a dict: the boundary, and the extract that the area
needs. Use it before a large area.""")
ch.code("""plan = duckosm.get(osm_id=2220322, dry_run=True)        # no download, no file
plan["extract"]                                         # the Geofabrik extract of the area""")
ch.md("""The extract is the Geofabrik file "Monaco": 708128 bytes (0.7 MB) for 184 km2, because it includes the sea.
`cached` is `True`: the quick start downloaded it, so a new area in Monaco needs no download.""")

ch.md("""## Build an area

`area.build` does step 3 for an `Area`. Its arguments are the same as those of `duckosm.network`: `modes`, `output`,
`fixes` and the build options (section "The build options").""")
ch.code("""casino_net = casino.build(modes=["walking"])      # the walking network of the box around the casino
casino_net.summary()""")
ch.md("""The box has 3366 walking edges and 87.3 km. The network goes next to the files of the area, into
`data/casino.duckdb`, because the area has a folder of its own (`out="data"`).""")

ch.md("""## Choose the area on a map

`duckosm.AreaPicker` is a map in the notebook. Choose an area with one of the tabs (Search, OSM id, Box, H3 cell,
File). The line under the map shows the extract and its size. Click **Get**, or call `picker.get()`. You can also set
the area from Python with `picker.area`: the widget then shows its tab and its value.

![The AreaPicker widget: the OSM id 2220322 gives the city of Monaco](https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/images/area-picker.png)""")
ch.code("""picker = duckosm.AreaPicker(out="data")                 # the files go into the folder data/
picker.area = {"mode": "osm_id", "osm_id": 2220322}      # the area when you choose none on the map
picker                                                    # the map""")
ch.md("""`picker.result` is the `Area` of the last click on **Get**. It is `None` before the first click. Then
`picker.get()` gets the area that the widget shows now.""")
ch.code("""area = picker.result or picker.get()    # the result of Get, or get it now
area""")

ch.md("""## Your own PBF file

`pbf=` takes an `.osm.pbf` file that you have already: from Geofabrik, from BBBike, from your own OSM editor, or a
file that you cut with osmium. Without `boundary=`, the whole file is the area. With `boundary=`, duckOSM cuts the
file to the boundary before the build. Here the file is an older Monaco extract (0.6 MB) from the tests of duckOSM.""")
ch.code("""import urllib.request
mine = "my_monaco.osm.pbf"                                                      # your own file: here, a download
urllib.request.urlretrieve("https://github.com/Khoshkhah/duckOSM/raw/main/tests/data/monaco.osm.pbf", mine)
whole = duckosm.network(pbf=mine, modes=["driving"])                            # the whole file
whole.summary()""")
ch.code("""cut = duckosm.network(pbf=mine, boundary="data/monte_carlo.geojson", modes=["driving"])   # only Monte Carlo
cut.summary()""")
ch.md("""**What the output shows.** The whole file has 2844 driving edges, close to the 2840 of the quick start: it is
an older copy of the OSM data. The cut to the Monte Carlo polygon (section "Your own polygon") keeps 779 of the 2844
driving edges, about a quarter.

> **Tip:** if the download of an extract fails on your machine (some cloud networks block Geofabrik), get the file
> another way and use `pbf=`.""")

ch.md("""## The build options

These keywords work in `duckosm.network`, `area.build` and `duckosm.build`. Each one sets one key of the config file.
Turn off the parts that you do not need, and the file is smaller.

| Keyword | Default | What it does |
|---|---|---|
| `speeds` | `True` | `False`: no `maxspeed_kmh` (the raw OSM `maxspeed` text stays), and no `cost_s` |
| `costs` | `True` | `False`: no `cost_s`, the travel time of each edge |
| `restrictions` | `True` | `False`: no turn restrictions (`driving.turn_restrictions`) |
| `bus_routes` | `True` | `False`: no bus routes (`bus.routes`, `bus.route_edges`) |
| `connect_paths` | `True` | `True` connects each free end of a path to the nearest node. `False`: the free ends stay free |
| `snap_m` | `10` | the largest distance, in meters, for `connect_paths` |
| `features` | `True` | `False`: no base-map layers (`features.*`: water, land, buildings) |
| `levels` | `True` | `False`: no drawing order of the roads (bridges over tunnels) |
| `dem` | none | a DEM file (a GeoTIFF of ground heights): the heights of the nodes and of the edges |

Here the build has no bus routes, no base map and no drawing order. `small.schema()` gives one row for each table,
and its column `schema` names the schema of the table (chapter 2).""")
ch.code("""small = duckosm.network(osm_id=2220322, name="monaco_small", modes=["driving"],
                        bus_routes=False, features=False, levels=False)       # three options off
small.schema()["schema"].unique().tolist()                                  # the schemas of the file""")
ch.code("""net.schema()["schema"].unique().tolist()                                    # the schemas of the quick start""")
ch.md("""The small file has only the schemas `driving` and `main`. The file of the quick start has six more: `bus` (the
bus routes), `features` and `raw` (the base map), `visualization` (the drawing order), and the other two modes.""")

ch.md("""### Every other option: `config=`

The config file has many more keys (`config/template.yaml` in the repository lists them all). `config=` takes such a
file (YAML or JSON) or a Python dict. A keyword wins over the same key of the config. Here the key
`options.cycling_dismount` is off. By default, the cycling network also has the footways and the pedestrian streets,
where you push the bicycle (the column `dismount` is `true`).""")
ch.code("""cfg = {"options": {"cycling_dismount": False}}        # a key of the config file that has no keyword
tuned = duckosm.network(osm_id=2220322, name="monaco_tuned", modes=["cycling"], config=cfg,
                        features=False, levels=False)
tuned.summary()""")
ch.md("""Without the footways, the cycling network has 3095 edges and 104.5 km, in place of 11305 edges and 250.0 km.

A wrong keyword stops the call. The message gives the nearest name and lists the correct names.""")
ch.code("""try:
    duckosm.network(osm_id=2220322, name="x", modes=["driving"], bus_route=False)    # a wrong name
except duckosm.DuckOSMError as e:
    print(e)""")

ch.md("""## Save your network

A build writes its files to duckOSM's **work folder**, not to your folder, so your folder stays clean while you try
things. `net.save(path)` copies the network to the place that you choose, and gives the `Network` of the copy. It
also copies the files that a new build needs, next to the network: the OSM file, the boundary and the folder
`<name>.levels` (the drawing order of the roads, with your edits).""")
ch.code("""kept = net.save("monaco.duckdb")
kept""")
ch.md("""**What the output shows.** The card of the copy says **saved: yes**, and `kept.saved` is `True`. Your folder now
has `monaco.duckdb` (37.0 MB), `monaco.osm.pbf`, `monaco.geojson` and `monaco.levels`. With these files, you can
build the network again with your fixes (chapter 5).

> **Key idea:** `monaco.duckdb` is the network of this book. Each next chapter opens it with
> `duckosm.open("monaco.duckdb")` and builds nothing.

> **Warning:** a network that you do not save stays in the work folder only until you clean it with
> `duckosm.clear_work`. Save each network that you want to keep.""")

ch.md("""### Write the network to a file at once: `path=`

When you know the file name before the build, give it as `path`. The network then goes there directly, not to the
work folder.""")
ch.code("""direct = duckosm.network(osm_id=2220322, modes=["driving"], path="monaco_driving.duckdb",
                         features=False, levels=False)
direct.saved""")

ch.md("""### Keep the OSM data of an area: `area.save`

`area.save(folder)` copies the two files of an `Area` (the cut OSM file and the boundary) into a folder of yours, and
gives the `Area` of the copies.""")
ch.code("""casino.save("casino_area")""")

ch.md("""### Open a saved file again: `duckosm.open`

The saved file stays on disk. `duckosm.open` gives its `Network`, in this session or in a new one, with no build.""")
ch.code("""again = duckosm.open("monaco.duckdb")
again.summary()""")

ch.md("""### The work folder

| Files | Where | Set it with |
|---|---|---|
| networks and areas that you did not save | the work folder, `~/.cache/duckosm/work` (`duckosm.work_dir()` gives it) | `DUCKOSM_CACHE` (the work folder is `<cache>/work`) |
| your saved networks | where you save them | `net.save(path)`, `path=`, `area.save(folder)` |
| the Geofabrik extracts | the cache: `~/.cache/duckosm` | `DUCKOSM_CACHE` |

`duckosm.clear_work()` deletes the work folder. `duckosm.clear_work(days=7)` deletes only the builds that are older
than seven days, and gives their number. Your saved files are not in the work folder, so they stay.""")
ch.code("""duckosm.clear_work(days=7)      # the number of deleted builds""")

ch.md("""## Try it yourself

**1.** Get the walking network of 1 km around a place that you know, with `point=` and `dist_m=1000`.

<details><summary>Show the answer</summary>

```python
mine = duckosm.network(point=(2.2945, 48.8584), dist_m=1000, modes=["walking"], name="eiffel")   # the Eiffel Tower
mine.summary()
```
</details>

**2.** Build the driving network of Monaco with `speeds=False`, and find the columns of `net` that the new file does not have.

<details><summary>Show the answer</summary>

```python
fast = duckosm.network(osm_id=2220322, name="monaco_nospeed", modes=["driving"], speeds=False)
set(net.columns("driving.edges").column_name) - set(fast.columns("driving.edges").column_name)   # maxspeed_kmh, cost_s
```
</details>

**3.** Use `dry_run=True` for your own city, and look at the size of its extract before you get it.

<details><summary>Show the answer</summary>

```python
plan = duckosm.get("Tartu", dry_run=True)     # any city name
plan["extract"]["name"], round(plan["extract"]["bytes"] / 1e6, 1), "MB"
```
</details>""")

ch.md("""## Words in this chapter

| Word | Meaning |
|---|---|
| area | the part of the world that you build: a city, a district, a box |
| boundary | the polygon of the area (`<name>.geojson`) |
| extract | a file of OSM data for a country or a region, from Geofabrik |
| PBF | the file format of OSM data (`.osm.pbf`) |
| cut | the part of an extract inside the boundary |
| mode | one network: driving, walking or cycling |
| edge | one direction of a road between two nodes: a row of `<mode>.edges` |
| node | a point where edges meet: a row of `<mode>.nodes` |
| `edge_id` | the number of an edge. It stays the same when you build the area again |""")

ch.md("""## Summary

| Call | Does |
|---|---|
| `duckosm.network(...)` | gets and builds an area in one call, and gives a `Network` |
| `duckosm.get(...)` | the boundary and the cut OSM file, as an `Area` (`dry_run=True`: only a plan) |
| `area.build(modes=...)` | builds an `Area` |
| `duckosm.build(pbf=..., boundary=...)` | builds from files |
| `duckosm.AreaPicker()` | chooses an area on a map |
| `net.save(path)`, `area.save(folder)` | keeps a network or an area where you choose |
| `duckosm.network(..., path=...)` | builds straight into a file of yours |
| `duckosm.open(path)` | opens a file that you saved before |
| `duckosm.work_dir()`, `duckosm.clear_work()` | the work folder, and its cleaning |
| `net.summary()`, `net.modes`, `net.walking` | the modes of a file, and the view of one mode |

The reference: [Python SDK, Core](../reference/python.md). **Next:** [2. Look inside the file](02_inspect.md), the
tables, the columns and SQL.""")

ch.write(sys.argv[1])
