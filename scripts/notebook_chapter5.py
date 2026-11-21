"""Write docs/notebooks/05_osm_fixes.ipynb: chapter 5 of the notebook manual, "Fix the OSM data once".
The style comes from scripts/notebook_book.py (rules: docs/design/notebook_book.md).

    python scripts/notebook_chapter5.py docs/notebooks/05_osm_fixes.ipynb"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from notebook_book import IMAGES, Chapter  # noqa: E402

ch = Chapter(5, "05_osm_fixes", "Fix the OSM data once",
             "Write a fix one time, in one file: the network and every export take it",
             learn="what a fixes file is and its format; three ways to make it: the editing page, Python and a text editor; "
                   "a build again with `net.rebuild` and a new build with the file; how you check the fix; which fixes keep the `edge_id`",
             time="about 4 minutes", extras="widget,viz",
             next_=("06_exports", "6. Exports", "the network as SUMO, MATSim, GMNS, OpenDRIVE and GIS files"))


ch.md(f"""## Why you fix the OSM data

OpenStreetMap is a map that people make. It is very good, but it has errors: a speed limit that is old, a road that
is one-way in the real world but not in OSM, a wrong number of lanes, a road that is closed for road works. When you
find such an error, you have three choices:

| Choice | The result |
|---|---|
| Correct OSM itself on [openstreetmap.org](https://www.openstreetmap.org) | correct for everyone, but your next extract has it only after the next Geofabrik update, and some changes (road works, a test scenario) do not belong in OSM |
| Correct each output by hand (the SUMO file, the GIS file, the database) | each output needs its own work, and the next build loses it |
| Write the fix in a **fixes file** that duckOSM reads | you write it one time, and each build and each export takes it |

duckOSM uses the third choice. A **fixes file** (`osm_fixes.yaml`) holds your corrections as **rules**. Each rule names
one OSM way by its id and says what to change. The build reads the file before it makes the networks. So the
database, the map, the routes and every export show the corrected data.

<img src="{IMAGES}/fixes-flow.svg" alt="The fixes file goes into the build; the network and every export take the fix" width="100%">

> **Key idea:** you fix the OSM data, not the outputs. The fixes file is the one place for your corrections, and every
> output of the build takes them. The next build, with a new extract, takes them again.""")


ch.md("""## The format of the file

A fixes file is a small text file in YAML. It has five keys, one for each kind of record. This file has one record of
each kind:

```yaml
overrides:                       # change an OSM way, by its id
  - osm_id: 4230891
    tags: {maxspeed: "10"}
    note: road works
  - {osm_id: 4230100, oneway: true}
  - {osm_id: 503462470, lanes: 3, turn_lanes_forward: left|through|through}
  - {osm_id: 4229577, exclude_modes: [driving]}
  - {osm_id: 4229578, remove: true}
new_ways:                        # add a road that OSM does not have
  - id: 9000000000000
    nodes: [21911886, [7.4235072, 43.7371133], 21914340]
    tags: {highway: service, name: New lane}
turn_restrictions:               # forbid or force a turn
  - {from_way: 4230100, via_node: 21924057, to_way: 4230099, restriction: no_left_turn}
extra:
  tag_edits:                     # set or delete the tags of a way, a node or a relation
    - {type: node, id: 21924057, set: {highway: traffic_signals}}
    - {type: relation, id: 393226, delete: [building]}
  building_faces:                # the type and the door of a wall of a building
    - {building: 23715051, from_node: 256791677, to_node: 256791676, type: front, door: main}
```

You need only the keys that you use. A file with one rule of `overrides` is a correct file. A record for an object
that is not in the area does nothing, so one file can hold the fixes of all your areas. The editing page adds
`changed`, the time of the change, to each record. The build does not read it.

### `overrides`: change an OSM way

A rule of `overrides` names one OSM way by its `osm_id` and says what to change. The build sees the way as if OSM had
these values.

| Field | Does |
|---|---|
| `osm_id` | the OSM id of the way (the key of the rule) |
| `tags` | sets these OSM tags on the way (`maxspeed`, `highway`, `name`, `surface` ...) |
| `oneway` | `true` or `false` |
| `lanes`, `lanes_forward`, `lanes_backward` | the lanes in both directions, or in one direction |
| `turn_lanes_forward`, `turn_lanes_backward` | the arrows of the lanes, in the syntax of the OSM tag `turn:lanes` |
| `layer` | the vertical level (a bridge is 1, a tunnel is -1) |
| `keep_nodes` | `[from, to]`: keep only this part of the way |
| `exclude_modes` | removes the way from these networks only: `driving`, `walking`, `cycling` |
| `remove` | `true`: removes the way from the whole build |
| `note` | your text: why you made the fix. The build does not read it |

### `new_ways`: add a road

| Field | Does |
|---|---|
| `id` | 9,000,000,000,000 or more, used one time. Do not change it: the edge ids come from it |
| `nodes` | the points of the road: an OSM node id, or `[lon, lat]` for a new point. Make the two ends OSM nodes, so that the road connects to the network |
| `tags`, `tags_from` | the OSM tags of the road, or the OSM way to copy the tags from |

### `turn_restrictions`: forbid or force a turn

| Field | Does |
|---|---|
| `from_way`, `via_node`, `to_way` | the way before the junction, the node of the junction and the way after it |
| `restriction` | `no_u_turn` (the default), `no_left_turn`, `no_right_turn`, `no_straight_on`, or `only_left_turn`, `only_right_turn`, `only_straight_on` |

A turn restriction changes the driving network.

### `extra.tag_edits` and `extra.building_faces`

| Record | Fields |
|---|---|
| `tag_edits` | `type` (`way`, `node` or `relation`), `id`, and `set` (tags to set) or `delete` (tags to delete) |
| `building_faces` | `building`, `from_node` and `to_node` (the wall), `type` (`front`, `back`, `side`, `party` or `court`) and `door` |

Use `tag_edits` for an object that is not a road, for example a traffic signal on a node. Use `building_faces` for the
3D buildings of the base map ([`main.building_faces`](../reference/database.md#building_faces)).""")

ch.md("""## Build Monaco without a fix

First, build the driving network of Monaco with no fixes file. This network is the "before" of the chapter.
`duckosm.get` writes the OSM file and the boundary into the folder `data`, and `area.build` writes the network beside
them. A fix needs these two files, because the build with the fix starts again from the OSM data.""")
ch.code("""area = duckosm.get(osm_id=2220322, name="monaco", out="data")    # the OSM file of Monaco and its boundary
net = area.build(modes=["driving"])                               # the driving network, with no fixes file
net                                                               # the card""")
ch.md("""The card shows the file `data/monaco.duckdb` and the driving network: the number of
edges and nodes and the length in km. This is the network before the fix.""")


ch.md("""## Find the road to fix

In this chapter, the error is a speed limit. The task: road works close a lane of one road, and the speed limit there
becomes 10 km/h. To see the effect, take a route by car across Monaco, from Fontvieille in the west to Larvotto in the
east, and fix a road that the route uses.""")
ch.code("""a, b = (7.4155, 43.7285), (7.4335, 43.7468)    # Fontvieille and Larvotto, as (lon, lat)
before = net.route(a, b)                       # the fastest route by car
round(before.time_s), round(before.length_m)   # the time in seconds and the length in metres""")
ch.md("""A route is a list of edges. Each edge comes from one OSM way, and the column `osm_id` of `driving.edges` names
it. The SQL below groups the edges of the route by their way and gives the three longest ways. The fix goes on the
first one.""")
ch.code('''ways = net.sql("""SELECT osm_id, any_value(name) AS name, any_value(maxspeed_kmh) AS maxspeed_kmh,
                         round(sum(length_m)) AS length_m
                  FROM driving.edges WHERE list_contains($ids, edge_id)
                  GROUP BY osm_id ORDER BY length_m DESC LIMIT 3""", {"ids": before.edge_ids})    # the three longest ways of the route
ways''')
ch.md("""**What the output shows.** The route is 2,939 m long and takes 229 s by car. Its three longest ways are
Boulevard Louis II (way 4230891, 356 m of the route), Avenue Princesse Grace (223 m) and Boulevard Albert 1er
(216 m). All three have a speed limit of 50 km/h. The fix goes on Boulevard Louis II.

> **Tip:** to find the OSM id of a road in your own area, click the road on [openstreetmap.org](https://www.openstreetmap.org)
> (the "Query features" tool), or look at the column `osm_id` of `net.edges()`, or click the road on `net.map()`.""")


ch.md("""## Make the file and add records

There are three ways to make a fixes file and to add records to it. All three write the same format, so you can use
them together on one file.

| Way | Use it when | You need the OSM id? |
|---|---|---|
| the editing page, `net.edit()` | you want to click the road on a map | no: you click the road |
| Python, `duckosm.fixes.Fixes` | you fix many roads from a list, or in a script | yes |
| a text editor | you want to read or change the file yourself | yes |""")

ch.md("""### The editing page: `net.edit()`

The easiest way is the editing page. You find a road on a map, change it in a form, and the page writes the record
into the fixes file. It is also the only way for some fixes: a turn that a driver cannot make, a new road, the walls of
a building. `net.edit()`
starts the editor **Edit OSM data** on the database, on your computer. The page shows the roads and the buildings of
the area on a map. Each change that you add on the page goes into the fixes file at once, in the same format as
`Fixes`.

```python
url = net.edit()                       # the editor in the background; the fixes file of the build, else osm_fixes.yaml
url                                    # http://127.0.0.1:8790/: open it in your browser
net.edit(fixes="osm_fixes.yaml", port=8791)    # another file, another port
```

| Argument | Default | Does |
|---|---|---|
| `fixes` | the fixes file of the build, else `osm_fixes.yaml` beside the database | the file that the page writes |
| `port` | `8790` | the port of the page on `127.0.0.1` |

<img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/images/edit-page.png" alt="The editing page: the panel Edit OSM data at the left, the roads of the database on the map" width="100%">

The page has one tab for each kind of record:

| Tab | What you change | Key in the file |
|---|---|---|
| **Road** | the tags of a road (class, lanes, speed, one-way, sidewalk, surface, access); delete the road | `overrides`, `extra.tag_edits` |
| **Building** | floors, height, roof, facade; the type of each wall and its door; delete the building | `extra.tag_edits`, `extra.building_faces`, `overrides` |
| **Node** | a traffic signal, a crossing or another node | `extra.tag_edits` |
| **Turn** | a turn that a driver cannot make, or must make, at a junction | `turn_restrictions` |
| **New road** | a road that OSM does not have: click its points on the map | `new_ways` |
| **Changes** | the list of your changes, and **Build again** | |

To add one record:

1. Pick a tab.
2. Click the road on the map, or type its OSM id and click **Load**.
3. Change the fields, then click **Add to changes**. The page writes the record into the file at once.
4. In the **Changes** tab, click **Build again** to build the network again with the file.

<img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/images/edit-road.png" alt="The Road tab: the form shows the tags of the picked road" width="100%">

The editor runs while Python runs. The guide [Fix the OSM data](../guides/edit-osm-data.md) shows each tab and
button of the page.

> **Warning:** the page is on `127.0.0.1` of the machine that runs Python. In Colab, that machine is in the cloud, so
> your browser cannot open the page. Run `net.edit()` in Jupyter on your own computer, or use the step **Edit OSM
> data** of [duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/) in the browser.""")


ch.md("""### From Python: `duckosm.fixes.Fixes`

`duckosm.fixes.Fixes(path)` opens a fixes file, or makes a new one when the file does not exist. Each call that adds a
rule writes the file at once, so the file on disk is always the same as the object in Python.

`fixes.set(way, **fields)` adds a rule for the way, or changes its rule. A keyword that is a field of a rule (`oneway`,
`lanes`, `layer`, `note` ...) goes into the rule. Each other keyword is an OSM tag: `maxspeed=10` sets the tag
`maxspeed` to `"10"`. The build then reads the tag as OSM data, so the speed and the travel time of the road change.""")
ch.code("""way = int(ways.osm_id[0])                              # the OSM id of the way to fix
fixes = duckosm.fixes.Fixes("osm_fixes.yaml")          # open or make the fixes file
fixes.set(way=way, maxspeed=10, note="road works")     # one rule: the tag maxspeed of the way, and a note
fixes                                                  # the card: a table of the rules""")
ch.md("""The card names the file and the number of rules (1). The table has one row for
each way: `osm_id`, the OSM `tags` of the rule (`maxspeed` is the text `'10'`) and the `note`.

The file is ordinary YAML. You can read it, keep it in git with your project, and give it to a colleague.""")
ch.code("""print(open("osm_fixes.yaml").read())    # the fixes file on disk""")
ch.md("""The rule is one item of `overrides`: the `osm_id`, the `tags` with `maxspeed` and
the `note`. This is the format that the map editor writes, so the two tools can share one file.

> **Warning:** `Fixes` writes the whole file again at each change. Comments that you wrote in the file by hand are
> lost. The other keys of the file (`new_ways`, `turn_restrictions`, `extra`) stay.""")


ch.md("""#### The other calls

`set` is one of five calls that change the rules. The table shows all of them. Each call returns the `Fixes`, so you
can chain the calls.

| Call | Does | The field in the file |
|---|---|---|
| `fixes.set(way, **fields)` | sets the fields and the OSM tags of the way | the fields, and `tags` |
| `fixes.exclude(way, modes)` | removes the way from the networks of `modes` only | `exclude_modes` |
| `fixes.remove(way)` | removes the way from the whole build: no road, no building, no feature | `remove: true` |
| `fixes.drop(way)` | deletes the rule of the way, so the way is as in OSM again | the rule is gone |
| `fixes.rules()` | gives the rules as a DataFrame | |

The cell below tries each call on a second file, `practice.yaml`, so the file of the chapter keeps its one rule. The
second and third ways of the route get a rule each.""")
ch.code("""w2, w3 = int(ways.osm_id[1]), int(ways.osm_id[2])          # the second and third ways of the route
practice = duckosm.fixes.Fixes("practice.yaml")              # a second fixes file
practice.set(way=w2, oneway=True, lanes=2)                   # two fields of a rule: one-way, two lanes
practice.set(way=w2, highway="residential")                  # an OSM tag; the rule of w2 keeps its fields
practice.exclude(way=w3, modes=["driving"])                  # no cars on w3; walking and cycling keep it
practice.rules()                                             # one row for each way, one column for each field""")
ch.md("""**What the output shows.** The table has two rows, one for each way. The second `set` on the same way did not
make a new rule: it added the tag `highway` to the rule of `w2`, and `oneway` and `lanes` stay. The rule of `w3` has
only `exclude_modes`.""")
ch.code("""practice.remove(way=w3)        # remove w3 from the whole build
practice.drop(way=w2)          # w2 is as in OSM again
practice, len(practice)        # the file now has one rule""")
ch.md("""After `drop`, the file has one rule: the rule of `w3`, with `exclude_modes` and now
also `remove: true`. `len(fixes)` is the number of rules.

> **Tip:** `remove` and `exclude` are the two ways to take a road out. Use `exclude` for a road that some modes can
> use (a street that is closed to cars but open to people on foot). Use `remove` for a way that must not be in the
> data at all.""")


ch.md("""### In a text editor

The file is plain text, so you can also write it yourself in any text editor, in the format of section "The format of
the file". The cell below writes a file `hand.yaml` with one rule from Python, as a text editor would. Then `Fixes`
opens it and shows its rule: the file is the same as a file that `Fixes` or the editing page writes.""")
ch.code('''from pathlib import Path

Path("hand.yaml").write_text("""overrides:
  - osm_id: 4230891
    tags:
      maxspeed: "10"
    note: road works
""")                                                     # the file, written as text
duckosm.fixes.Fixes("hand.yaml").rules()               # open it: one rule''')
ch.md("""> **Warning:** YAML uses the spaces at the start of each line. Keep two spaces for each level, as in the example,
> and write a number in a tag in quotes (`"10"`), because an OSM tag is text.""")

ch.md("""## Build with the fixes

A fixes file changes nothing by itself. A build reads it. There are two ways to build with the file.

| Way | Uses | Use it when |
|---|---|---|
| `net.rebuild(fixes=...)` | the same OSM file and boundary as `net` | you want to compare the network before and after the fix |
| a new build: `duckosm.network(..., fixes=...)` | a new extract, or another area | you build later with newer OSM data, or in another project |""")

ch.md("""### Build again: `net.rebuild`

`net.rebuild(fixes=...)` builds the area again, with the same modes, from the OSM file and the boundary beside the
database. It takes a `Fixes` or the path of a fixes file. The new network goes to `<name>_fixed.duckdb` beside the old
one (`out=` sets another file), so the old network stays and you can compare the two.""")
ch.code("""fixed = net.rebuild(fixes=fixes)                     # the same build, with the fixes file: data/monaco_fixed.duckdb
after = fixed.route(a, b)                            # the same route on the fixed network
round(before.time_s), round(after.time_s)            # the seconds before and after the fix""")
ch.md("""The route now takes 252 s, not 229 s. It does not use Boulevard Louis II now: it
takes other streets around it. The new route is longer (3,302 m, not 2,939 m), but at 10 km/h the old way is slower.

The new network remembers its fixes file. `fixed.fixes` gives the `Fixes` of the build. `net.fixes` is `None`,
because `net` has no fixes file.""")
ch.code("""fixed.fixes, net.fixes        # the fixes file of each network""")


ch.md("""### A new build with the file

The same file works in each new build. Give it to `duckosm.network`, to `area.build` or to the command
`duckosm build --fixes osm_fixes.yaml`. The cell below builds Monaco again from the start, with the file, into a new
network. The route takes the same time as after `rebuild`, because the two builds read the same OSM data and the same
rule.""")
ch.code("""again = duckosm.network(osm_id=2220322, modes=["driving"], name="monaco_with_fixes", fixes="osm_fixes.yaml")   # a new build with the file
round(again.route(a, b).time_s)                       # the same time as after rebuild""")
ch.md("""> **Key idea:** keep the fixes file with your project. When you build the area again next month, with a newer OSM
> extract, the same file corrects the new build. A rule whose way is not in the new data does nothing.""")

ch.md("""## Check the fix

Four checks show that the fix works: the compare of the two networks, the map, the edge ids and the outputs.""")

ch.md("""### Compare the two networks

`net.diff(other)` compares two networks edge by edge, by `edge_id`. It gives one row for each edge that is different:

| Column | Meaning |
|---|---|
| `mode` | the mode of the edge |
| `edge_id` | the edge |
| `change` | `added` (only in `other`), `removed` (only in `net`) or `changed` (in both, with other values) |
| `changed` | for a changed edge: the names of the columns with a new value |

`net.diff(fixed)` is the same as `duckosm.fixes.diff(net, fixed)`. `mode=` compares one mode only. DuckDB joins the
two files, so the comparison is fast also for a large area.""")
ch.code("""changes = net.diff(fixed)                     # the edges that are different in the fixed network
changes.change.value_counts()                 # the number of edges of each kind of change""")
ch.code("""changes.head()                                # the first changed edges, with the columns that changed""")
ch.md("""**What the output shows.** Eight edges changed. No edge was added or removed. The eight edges are the fixed way:
Boulevard Louis II is a two-way road, so it has four edges in each direction. On each edge, two columns changed:
`maxspeed_kmh` (now 10) and `cost_s`, the travel time, which the build calculates from the speed.""")

ch.md("""### See the change on a map

`fixed.map(routes=...)` draws routes on the network, each in its own color. The cell draws three lines: the route
before the fix, the route after the fix, and the fixed way (its edges, as a route). Run the cell to see the live map:
you can zoom, and click a road to see its columns. The manual page shows a picture of the map.""")
ch.code("""way_edges = {"edges": list(fixed.edges(where=f"osm_id = {way}").edge_id)}                   # the edges of the fixed way, as a route to draw
fixed.map(routes={"before the fix": before, "after the fix": after, "the fixed way": way_edges})    # the three as lines, each in its own color""")
ch.md("""The route after the fix leaves the fixed way. It takes other streets around the slow
part, because they are now faster.""")

ch.md("""### The edge ids do not change

An `edge_id` is a hash of the OSM way and of the two nodes at the ends of the edge. A fix of a tag (`maxspeed`,
`lanes`, `name`) does not change the way or its nodes. So each edge keeps its `edge_id`, and your own data that you
joined on the ids (chapter 4) stays correct after the fix.""")
ch.code("""set(net.edges().edge_id) == set(fixed.edges().edge_id)    # the same edge ids before and after the fix""")
ch.md("""`True`: the two networks have the same set of edge ids.

> **Key idea:** a fix of the tags keeps every `edge_id`. A fix that removes a road (`remove`, `exclude`) removes the
> edges of that road only. All the other edges keep their ids.""")

ch.md("""#### A fix that removes a road

The next cell checks this rule. It closes the fixed way to cars with `exclude`, in a third file, and builds the area
again into `data/monaco_closed.duckdb` (`out=`).""")
ch.code("""closed = duckosm.fixes.Fixes("closed.yaml").exclude(way=way, modes=["driving"])    # no cars on the way
shut = net.rebuild(fixes=closed, out="data/monaco_closed.duckdb")                   # a third network
gone = net.diff(shut)                                                               # the edges that are different
gone.change.value_counts(), set(fixed.edges(where=f"osm_id = {way}").edge_id) == set(gone.edge_id[gone.change == "removed"])""")
ch.md("""The diff has eight removed edges and no other change. The second value is `True`: the
removed edges are the eight edges of the fixed way. All the other edges of the network keep their ids.""")

ch.md("""### Every output takes the fix

The fix is in the database, so each export of the database takes it. The cells below write five outputs of the fixed
network and read the speed of one edge of the fixed way back from each file. Each format stores the speed in its own
unit: SUMO and MATSim use m/s, so the cell converts it to km/h. Chapter 6 shows each export in full.

| Output | The call | Where the speed is |
|---|---|---|
| the database | (the build) | `maxspeed_kmh` of `driving.edges` |
| GeoPackage | `fixed.to_gis` | `maxspeed_kmh` of the layer `edges_driving` |
| SUMO | `fixed.to_sumo` | the attribute `speed` (m/s) of the `<edge>` |
| MATSim | `fixed.to_matsim` | the attribute `freespeed` (m/s) of the `<link>` |
| GMNS | `fixed.to_gmns` | `free_speed` (km/h) of the table `link` |
| QuackOSM table | `duckosm.gis.quack(..., fixes=)` | the tag `maxspeed` of the way |

In each file, the id of the edge (or of the link) is the `edge_id`. So the cell finds the same edge in each output.""")
ch.code("""import duckdb                                                    # to read the outputs back

edge = way_edges["edges"][0]                                     # one edge of the fixed way
speeds = {"database (km/h)": float(fixed.edges(where=f"edge_id = {edge}").maxspeed_kmh[0])}    # its speed in each output
fixed.to_gis("out/monaco.gpkg")                                  # a GeoPackage
gpkg = duckdb.connect()                                          # DuckDB reads it back
gpkg.load_extension("spatial")                                   # with the extension spatial
speeds["GeoPackage (km/h)"] = gpkg.execute(
    "SELECT maxspeed_kmh FROM st_read('out/monaco.gpkg', layer='edges_driving') WHERE edge_id = ?", [edge]).fetchone()[0]""")
ch.md("""SUMO and MATSim write XML. `run_netconvert=False` writes only the plain XML files of SUMO, so this cell does not
need the SUMO programs.""")
ch.code("""import gzip
import xml.etree.ElementTree as ET                               # to read the XML files back

sumo = fixed.to_sumo("out/sumo", run_netconvert=False)            # the SUMO files; speed in m/s
speeds["SUMO (km/h)"] = round(float(ET.parse(sumo["edg"]).find(f".//edge[@id='{edge}']").get("speed")) * 3.6, 1)
fixed.to_matsim("out/matsim.xml.gz")                             # the MATSim network; free speed in m/s
speeds["MATSim (km/h)"] = round(float(ET.parse(gzip.open("out/matsim.xml.gz")).find(f".//link[@id='{edge}']").get("freespeed")) * 3.6, 1)""")
ch.md("""GMNS is a set of tables. `duckosm.gis.quack` is different from the other calls: it reads the OSM file, not the
network, so it takes the fixes file with its own `fixes=`.""")
ch.code("""fixed.to_gmns("out/gmns.duckdb")                                 # the GMNS network: link_id is the edge_id
speeds["GMNS (km/h)"] = duckdb.connect("out/gmns.duckdb", read_only=True).execute(
    "SELECT free_speed FROM gmns_driving.link WHERE link_id = ?", [edge]).fetchone()[0]
quack = duckosm.gis.quack(area, out="out", name="monaco", fixes=fixes, formats=["geoparquet"])    # the QuackOSM table
speeds["QuackOSM table (tag)"] = gpkg.execute(
    f"SELECT tags['maxspeed'] FROM '{quack[0]}' WHERE feature_id = 'way/{way}'").fetchone()[0]
speeds                                                           # the speed of the edge in each output""")
ch.md("""**What the output shows.** Each output gives 10 km/h for the edge. The QuackOSM table has the OSM tag, so it
shows the text `'10'`. The GMNS export takes the longest time (more than 25 s here). It also makes the tables of the
lanes and of the movements at each junction.

> **Tip:** `fixes=` also works in the first build: `duckosm.network(..., fixes="osm_fixes.yaml")`,
> `duckosm.get(..., fixes=...)` and `area.build(fixes=...)` take a path or a `Fixes`. Use `rebuild` when you have a
> network and want to compare it with the fixed one.""")


ch.exercises([
    ("Make the fixed way one-way with `fixes.set(way=way, oneway=True)`, build again into a new file, and count the "
     "edges that `diff` gives as removed.",
     """fixes.set(way=way, oneway=True)                              # the rule of the way gets a field
oneway = net.rebuild(fixes=fixes, out="data/monaco_oneway.duckdb")
d = net.diff(oneway)
(d.change == "removed").sum()                                 # the edges of the other direction"""),
    ("Close the second way of the route to cars only, and find the new time of the route.",
     """closed2 = duckosm.fixes.Fixes("closed2.yaml").exclude(way=w2, modes="driving")
net.rebuild(fixes=closed2, out="data/monaco_closed2.duckdb").route(a, b).time_s"""),
    ("Delete the rule of the fixed way, so that the way is as in OSM again, and look at the file.",
     """fixes.drop(way=way)
print(open("osm_fixes.yaml").read())                           # overrides: []"""),
])

ch.glossary([
    ("fix", "a correction of the OSM data, written in a fixes file"),
    ("fixes file", "the YAML file of your fixes (`osm_fixes.yaml`); each build reads it"),
    ("rule", "one item of `overrides`: what to change on one OSM way"),
    ("way", "an OSM line (a road, a path); a way becomes one edge or more"),
    ("`osm_id`", "the id of the OSM way of an edge; the key of a rule"),
    ("tag", "an OSM key and value on a way, for example `maxspeed=30`"),
    ("rebuild", "a new build of the same area and modes, with a fixes file"),
    ("diff", "the edges that are added, removed or changed between two networks, by `edge_id`"),
])

ch.summary([
    ("`duckosm.fixes.Fixes(path)`", "opens or makes a fixes file"),
    ("`fixes.set(way, **fields)`", "adds or changes the rule of a way: fields of a rule and OSM tags"),
    ("`fixes.exclude(way, modes)`", "removes the way from the networks of `modes`"),
    ("`fixes.remove(way)`", "removes the way from the build"),
    ("`fixes.drop(way)`", "deletes the rule of the way"),
    ("`fixes.rules()`, `len(fixes)`", "the rules as a DataFrame, and their number"),
    ("`net.rebuild(fixes=..., out=...)`", "builds the area again with the fixes; gives the new `Network`"),
    ("`net.fixes`", "the `Fixes` of the build, or `None`"),
    ("`net.diff(other, mode=None)`, `duckosm.fixes.diff(net, other)`", "the edges that are different, by `edge_id`"),
    ("`net.edit(fixes=None, port=8790)`", "starts the editing page and gives its URL"),
    ("`duckosm.network(..., fixes=...)`, `area.build(fixes=...)`", "a new build with the fixes file"),
    ("`duckosm.gis.quack(..., fixes=...)`", "the QuackOSM table of the OSM file, with the fixes"),
], reference="[duckosm.fixes](../reference/sdk/fixes.md)")

ch.write(sys.argv[1])
