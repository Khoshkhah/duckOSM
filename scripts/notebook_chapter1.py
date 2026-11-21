"""Write docs/notebooks/01_area_to_network.ipynb: chapter 1 of the notebook manual, "Get and build an area".
The style comes from scripts/notebook_book.py (rules: docs/design/notebook_book.md).

    python scripts/notebook_chapter1.py docs/notebooks/01_area_to_network.ipynb"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from notebook_book import Chapter  # noqa: E402

ch = Chapter(1, "01_area_to_network", "Get and build an area",
             "From a place, a box, a polygon or your own OSM file to one network file",
             learn="the parts of a duckOSM build; seven ways to choose an area; the modes; your own PBF file; the build options and `config=`; where the files go",
             time="about 3 minutes (the first download included)", extras="widget,viz",
             next_=("02_inspect", "2. Look inside the file", "the tables, the columns and SQL"))

ch.md("""## What an area is

An **area** is the part of the world whose roads you want in one file: a city, a district, a park, a box around a
point, or a region. duckOSM describes an area with one polygon, the **boundary**. The build keeps the roads in the
boundary. A road that crosses the boundary stays whole, so the network has no cut roads at its edge.

| You give | The boundary that duckOSM makes |
|---|---|
| a place name (`place`) | the polygon of the first answer of Nominatim for the name |
| an OSM id (`osm_id`) | the polygon of that OSM relation |
| a box (`bbox`) | the rectangle: west, south, east, north |
| a point and a distance (`point`, `dist_m`) | the square `dist_m` metres around the point |
| an H3 cell (`h3_cell`) | the hexagon of the cell |
| a polygon (`boundary`) | your polygon, as it is |
| an OSM file (`pbf`) without a boundary | the area of the whole file |

The picture shows two boundaries for "Monaco". The name finds the country, whose boundary includes a large part of
the sea; the OSM id 2220322 is the city. The gray lines are the roads: some cross the city boundary, and they stay
whole.

<img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/notebooks/images/area-boundary.png" alt="Two boundaries for Monaco: the country with its sea, and the city" width="100%">

> **Key idea:** an area is a polygon. Each way to choose an area (a name, an id, a box, a point, a file) ends as one
> boundary polygon, and the build keeps the roads in it.""")

ch.md("""## The parts of a build

A duckOSM build has three steps. Each step has its own call, and `duckosm.network` does all three in one call.

<img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/notebooks/images/build-flow.svg" alt="The flow of a build: your area, the boundary, the extract, the cut, the network" width="100%">

| Step | What happens | The call | The result |
|---|---|---|---|
| 1. The boundary | The area becomes a polygon: from a name or an OSM id (Nominatim), a box, a point, an H3 cell or your own GeoJSON | part of `duckosm.get` | `<name>.geojson` |
| 2. The OSM data | duckOSM finds the smallest Geofabrik extract that holds the boundary, downloads it once into a cache, and cuts it to the boundary | `duckosm.get` | `<name>.osm.pbf`, and an `Area` |
| 3. The network | The Rust engine reads the cut file and writes the roads of each mode, the nodes, the graph of turns, the base map and more | `area.build` or `duckosm.build` | `<name>.duckdb`, and a `Network` |

The result is **one DuckDB file**. Each mode is a schema in it (`driving`, `walking`, `cycling`), with the tables
`edges` and `nodes`. Each edge has an `edge_id`, a number that stays the same when you build the area again, so your
own data can join to it (chapter 5).

> **Key idea:** one area is one file. The networks of all modes, the nodes, the turns and the base map of the area
> are in the same `.duckdb` file, and `edge_id` connects them to your own data.

> **Tip:** you can open the file with any DuckDB tool (the `duckdb` program, DBeaver, Python's `duckdb`), not only
> with duckOSM.""")

ch.md("""## Quick start: one call

`duckosm.network` gets the area and builds it. Here the area is the city of Monaco, OSM relation 2220322. The
argument `modes` chooses the networks: driving and walking.""")
ch.code("""net = duckosm.network(osm_id=2220322, modes=["driving", "walking"], name="monaco")    # get and build Monaco
net                                                                                 # the card of the network""")
ch.md("""**What the output shows.** The progress lines name each step: the boundary from Nominatim, the extract and the
cut, then the build. The card at the end shows the file and, for each mode, the number of edges and nodes and the
length in km. A `Network` is also the path of its file, so `str(net)` gives `data/monaco.duckdb`.

> **Warning:** the name "Monaco" finds the *country* of Monaco, whose boundary includes a large part of the sea. That
> is why this chapter uses the OSM id of the city. When a name gives the wrong area, find its OSM id on
> [openstreetmap.org](https://www.openstreetmap.org) (search, then the "Relation" number) and use `osm_id=`.""")

ch.md("""## The summary of each mode

`net.summary()` gives one row for each mode: the edges, the nodes, the length in km and the length of the one-way
roads. `net.modes` lists the modes of the file.""")
ch.code("""print(net.modes)       # the modes in the file
net.summary()          # one row for each mode""")
ch.md("""**What the output shows.** The walking network is much larger than the driving network: it has the footways,
the steps and the paths, and both sides of the streets where OSM draws the sidewalks. A two-way road is two edges,
one for each direction.""")

ch.md("""## The modes

A mode is one network. Each mode keeps the roads that it can use, with its own rules.

| Mode | Keeps | Leaves out |
|---|---|---|
| `driving` | the roads for cars (motorway to residential, service roads, ramps), with one-way rules and turn restrictions | footways, paths, steps, private roads |
| `walking` | footways, paths, steps, pedestrian streets, sidewalks, and the roads where people can walk | motorways and roads where walking is not allowed |
| `cycling` | cycleways, roads for bicycles, and footways where you can push the bicycle | motorways, steps |

The picture shows Monaco: in each panel, all the roads are gray, and the roads of that mode have its color. Walking
has the steps and footways of the old town; driving has the tunnel loop in the west and no footways.

<img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/notebooks/images/modes-monaco.png" alt="Monaco: the driving, walking and cycling networks" width="100%">

> **Key idea:** a road can be in more than one mode. A street with sidewalks is in all three, with the same
> `edge_id` in each mode. So you join your data once, and it is correct in every mode.

`net.walking` (or `net["walking"]`) is the view of one mode: the same calls, without `mode=`.""")
ch.code("""net.walking            # the card of one mode""")

ch.md("""## Seven ways to choose an area

`duckosm.get` does steps 1 and 2. It does not build. It gives an `Area`: the pair of files of one area.

| `Area` | File | What it is |
|---|---|---|
| `area.boundary` | `<name>.geojson` | the boundary polygon of the area |
| `area.pbf` | `<name>.osm.pbf` | the OSM data inside the boundary (the cut) |

`area.build()` makes the network from these two files. Use `duckosm.get` to look at an area before the build, or to
build it later with other options. Each way below is one argument of `duckosm.get`.

| Argument | The area | Example |
|---|---|---|
| `place` | the first answer of Nominatim for a name | `duckosm.get("Monaco")` |
| `osm_id` | an OSM relation (a city, a district, a park) | `duckosm.get(osm_id=2220322)` |
| `bbox` | a box: west, south, east, north, in degrees | `duckosm.get(bbox=(7.41, 43.72, 7.44, 43.75))` |
| `point` and `dist_m` | the box `dist_m` metres around a point (lon, lat) | `duckosm.get(point=(7.4246, 43.7384), dist_m=500)` |
| `h3_cell` | one H3 cell | `duckosm.get(h3_cell="883969a40bfffff")` |
| `boundary` | your own polygon: a GeoJSON file, a GeoJSON dict, or a shapely or GeoPandas object | `duckosm.get(boundary="district.geojson")` |
| `pbf` (in `network`) | an OSM file that you have | see "Your own PBF file" below |

The extract stays in the cache, so each next area in Monaco needs no download.""")

ch.md("""### A box around a point

`point` with `dist_m` is the same as OSMnx's `graph_from_point`: a box of 2 × 500 m around the casino square.""")
ch.code("""casino = duckosm.get(point=(7.4280, 43.7393), dist_m=500, name="casino", out="data")   # an Area: the file and the boundary
casino                                                                                  # the card of the area""")
ch.md("""**What the output shows.** The progress line counts the nodes, ways and relations of the cut. The card names the
two files: the cut OSM file and the boundary. The box is a small part of the extract.""")

ch.md("""### A box in degrees

`bbox` takes west, south, east and north, in this order (the order of GeoJSON and of OSMnx 2).""")
ch.code("""harbour = duckosm.get(bbox=(7.418, 43.730, 7.430, 43.738), name="harbour", out="data")   # the harbour of Monaco
harbour""")

ch.md("""### Your own polygon

`boundary` takes a polygon. Here it is a shapely polygon; a GeoJSON file or a GeoPandas object work the same way.
duckOSM writes it to `<out>/<name>.geojson`.""")
ch.code("""from shapely.geometry import Polygon                       # any object with __geo_interface__ works
monte_carlo = Polygon([(7.420, 43.737), (7.432, 43.737), (7.432, 43.745), (7.420, 43.745)])
mc = duckosm.get(boundary=monte_carlo, name="monte_carlo", out="data")
mc""")

ch.md("""### Look before you get: `dry_run`

`dry_run=True` downloads and writes nothing. It tells you which extract the area needs, its size, and whether it is
in the cache already. Use it before a large area.""")
ch.code("""plan = duckosm.get(osm_id=2220322, dry_run=True)        # no download, no file
{k: plan["extract"][k] for k in ("name", "km2", "bytes", "cached")}""")

ch.md("""## Build an area

`area.build` does step 3 for an `Area`. Its arguments are the same as those of `duckosm.network`: `modes`, `output`,
`fixes` and the build options (below).""")
ch.code("""casino_net = casino.build(modes=["walking"])      # the walking network of the box around the casino
casino_net.summary()""")

ch.md("""## Choose the area on a map

`duckosm.AreaPicker` is a map in the notebook. Choose an area with one of the tabs (Search, OSM id, Box, H3 cell,
File); the line under the map shows the extract and its size. Click **Get**, or call `picker.get()`. You can also set
the area from Python with `picker.area`: the widget then shows its tab and its value.

![The AreaPicker widget: the OSM id 2220322 gives the city of Monaco](https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/images/area-picker.png)""")
ch.code("""picker = duckosm.AreaPicker(out="data")                 # the files go into the folder data/
picker.area = {"mode": "osm_id", "osm_id": 2220322}      # the area when you choose none on the map
picker                                                    # the map""")
ch.code("""area = picker.result or picker.get()    # the result of Get, or get it now
area""")

ch.md("""## Your own PBF file

`pbf=` takes an `.osm.pbf` file that you already have: from Geofabrik, from BBBike, from your own OSM editor, or a
file that you cut with osmium. Without `boundary=`, the whole file is the area. With it, duckOSM cuts the file to
the boundary before the build. Here the file is a Monaco extract (0.6 MB).""")
ch.code("""import urllib.request                                                           # your own file: here, a download
mine = "data/my_monaco.osm.pbf"
urllib.request.urlretrieve("https://github.com/Khoshkhah/duckOSM/raw/main/tests/data/monaco.osm.pbf", mine)
whole = duckosm.network(pbf=mine, name="monaco_whole", modes=["driving"])        # the whole file
whole.summary()""")
ch.code("""cut = duckosm.network(pbf=mine, boundary="data/monte_carlo.geojson", name="monte_carlo_cut", modes=["driving"])
cut.summary()                                                                     # only the roads in Monte Carlo""")
ch.md("""**What the output shows.** The cut to the Monte Carlo polygon (from "Your own polygon" above) keeps about a
quarter of the driving edges of the whole file (779 of 2844).

> **Tip:** if the download of an extract fails on your machine (some cloud networks block Geofabrik), get the file
> another way and use `pbf=`.""")

ch.md("""## The build options

These keywords work in `duckosm.network`, `area.build` and `duckosm.build`. Each one sets one key of the config file.

| Keyword | Default | What it does |
|---|---|---|
| `speeds` | `True` | `False`: no `maxspeed_kmh` (the raw OSM `maxspeed` text stays), and no `cost_s` |
| `costs` | `True` | `False`: no `cost_s`, the travel time of each edge |
| `restrictions` | `True` | `False`: no turn restrictions (`driving.turn_restrictions`) |
| `bus_routes` | `True` | `False`: no bus routes (`bus.routes`, `bus.route_edges`) |
| `connect_paths` | `True` | `False`: the free ends of paths stay free; `True` connects each one to the nearest node |
| `snap_m` | `10` | the largest distance, in metres, for `connect_paths` |
| `features` | `True` | `False`: no base-map layers (`features.*`: water, land, buildings) |
| `levels` | `True` | `False`: no drawing order of the roads (bridges over tunnels) |
| `dem` | none | a DEM file (GeoTIFF): the heights of the nodes and of the edges, after the build |

A smaller build is faster. Here the build has no bus routes, no base map and no drawing order.""")
ch.code("""small = duckosm.network(osm_id=2220322, name="monaco_small", modes=["driving"],
                        bus_routes=False, features=False, levels=False)       # three options off
sorted({t.split(".")[0] for t in small.tables()})                           # the schemas of the file""")
ch.md("""**What the output shows.** The file has only `driving` and `main`: no `bus` (no bus routes), no `features` and
`raw` (no base map) and no `visualization` (no drawing order). Compare it with the schemas of `net` from the quick
start.""")
ch.code("""sorted({t.split(".")[0] for t in net.tables()})          # the schemas of a build with the defaults""")

ch.md("""### Every other option: `config=`

The config file has many more keys (`config/template.yaml` in the repository lists them all). `config=` takes such a
file (YAML or JSON) or a Python dict. A keyword wins over the same key of the config.

> **Warning:** a wrong keyword stops the call with the nearest name, for example
> `network has no argument 'bus_route': did you mean 'bus_routes'?`.""")
ch.code("""cfg = {"options": {"bus_routes": False}, "clip": {"connect_snap_m": 5}}      # two keys of the config file
tuned = duckosm.network(osm_id=2220322, name="monaco_tuned", modes=["walking"], config=cfg,
                        features=False, levels=False)
tuned.summary()""")
ch.code("""try:
    duckosm.network(osm_id=2220322, name="x", modes=["driving"], bus_route=False)    # a wrong name
except duckosm.DuckOSMError as e:
    print(e)""")

ch.md("""## Open the file again

The file stays on disk. `duckosm.open` gives its `Network` in a new session, without a build.""")
ch.code("""again = duckosm.open("data/monaco.duckdb")      # the file of the quick start
again.summary()""")

ch.md("""## Where the files go

| File | Where | Set it with |
|---|---|---|
| the `.duckdb` file | `<out>/<name>.duckdb` (default `data/<name>.duckdb`) | `out=`, `name=`, or `output=` in `build` |
| the cut OSM file and the boundary | `<out>/<name>.osm.pbf`, `<out>/<name>.geojson` | `out=`, `name=` |
| the Geofabrik extracts | the cache: `~/.cache/duckosm` | the setting `DUCKOSM_CACHE` |""")

ch.md("""## Try it yourself

**1.** Get the walking network of 1 km around a place that you know, with `point=` and `dist_m=1000`.

<details><summary>Show the answer</summary>

```python
mine = duckosm.network(point=(2.2945, 48.8584), dist_m=1000, modes=["walking"], name="eiffel")   # the Eiffel Tower
mine.summary()
```
</details>

**2.** Build Monaco twice, once with `speeds=False`, and find the columns that the second file does not have.

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
| `edge_id` | the number of an edge; it stays the same when you build the area again |""")

ch.md("""## Summary

| Call | Does |
|---|---|
| `duckosm.network(...)` | gets and builds an area in one call; gives a `Network` |
| `duckosm.get(...)` | the boundary and the cut OSM file; gives an `Area` (`dry_run=True`: only a plan) |
| `area.build(modes=...)` | builds an `Area` |
| `duckosm.build(pbf=..., boundary=...)` | builds from files |
| `duckosm.AreaPicker()` | chooses an area on a map |
| `duckosm.open(path)` | opens a file that you built before |
| `net.summary()`, `net.modes`, `net.walking` | the modes of a file |

The reference: [Python SDK, Core](../reference/python.md). **Next:** [2. Look inside the file](02_inspect.md), the
tables, the columns and SQL.""")

ch.write(sys.argv[1])
