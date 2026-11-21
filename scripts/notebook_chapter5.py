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

### What the file holds

A fixes file is a list of your corrections of the OSM data. It puts the corrections in groups, one group for each kind
of correction:

| Group | Holds |
|---|---|
| `overrides` | the rules for OSM ways: change the tags, one-way or lanes of a way, or remove the way |
| `new_ways` | the roads that OSM does not have |
| `turn_restrictions` | the turns that a driver cannot make, or must make |
| `extra` → `tag_edits` | the changes of the tags of a node, a way or a relation |
| `extra` → `building_faces` | the type and the door of each wall of a building |

The file is in YAML, a text format. You can read it in any text editor.

### YAML in three rules

**1. `key: value` is one field.**

```yaml
osm_id: 4230891
```

**2. A line that starts further right belongs to the line above it.**

```yaml
tags:
  maxspeed: '10'
```

`maxspeed` has two spaces more than `tags`, so `maxspeed` is inside `tags`: it is a tag of the way.

**3. A line that starts with `- ` (a dash and a space) begins a new item of a list.**

```yaml
overrides:
- osm_id: 4230891
- osm_id: 4230100
```

Here `overrides` holds a list of two items. The lines under an item, with two spaces more than its dash, belong to that
item.

YAML uses the spaces at the start of a line to show what is inside what, as Python does. Use spaces, never the Tab key.

### The skeleton of a fixes file

Each group is one key at the start of a line, and each key holds a list. This is a fixes file with five empty lists
(`[]` is an empty list):

```yaml
overrides: []
new_ways: []
turn_restrictions: []
extra:
  tag_edits: []
  building_faces: []
```

You need only the groups that you use. A file with only `overrides` is a correct file.

### Build the file step by step

**Step 1: one rule.** Road works on Boulevard Louis II, the OSM way 4230891, lower its speed limit to 10 km/h. The rule
names the way and sets its tag `maxspeed`:

```yaml
overrides:
- osm_id: 4230891
  tags:
    maxspeed: '10'
  note: road works
```

The build now sees the way as if OSM had `maxspeed=10`. The `note` is your text. The build does not read it.

**Step 2: a second rule.** Way 4230100 is one-way in the real world. Add a new item under the same `overrides:`:

```yaml
overrides:
- osm_id: 4230891
  tags:
    maxspeed: '10'
  note: road works
- osm_id: 4230100
  oneway: true
```

**Step 3: remove a way.** Way 4229578 does not exist in the real world. A rule with `remove: true` takes it out of the
build: out of the network, the base map and every export. Add it as one more item:

```yaml
- osm_id: 4229578
  remove: true
```

To take a way out of some networks only, use `exclude_modes`. For example, a street that is closed to cars but open
to people on foot:

```yaml
- osm_id: 4229577
  exclude_modes:
  - driving
```

**Step 4: the other groups.** Each group has its own list. A new road, a forbidden turn and a traffic signal on a node
are one item in each of three lists:

```yaml
new_ways:
- id: 9000000000000
  nodes:
  - 21911886
  - [7.4235072, 43.7371133]
  - 21914340
  tags:
    highway: service
    name: New lane
turn_restrictions:
- from_way: 4230100
  via_node: 21924057
  to_way: 4230099
  restriction: no_left_turn
extra:
  tag_edits:
  - type: node
    id: 21924057
    set:
      highway: traffic_signals
```

### The rules of the file

- **Each group comes one time.** The file has one `overrides:`, one `new_ways:`, one `turn_restrictions:` and one
  `extra:`. Do not write a second `overrides:` for a new rule: YAML keeps only the last one, and the rules of the
  first one are lost with no error.
- **To add a record, add an item to the list of its group.**
- **One way has one rule.** To change more on a way, add fields to its rule. Do not add a second item with the same
  `osm_id`.
- **To remove a fix, delete its item**, from its `- ` line to the end of the record. The way is then as in OSM again.
- **The spaces must agree.** All the items of one list have their dash at the same position, and all the fields of
  one record start at the same position.
- **Write the value of a tag in quotes**, for example `maxspeed: '10'`, because an OSM tag is text.
- **A record for an object that is not in the area does nothing.** So one file can hold the fixes of all your areas.

You can also write a record on one line, in braces: `- {osm_id: 4230100, oneway: true}`. It has the same meaning as
the long form. `Fixes` and the editing page write the long form, and they follow all these rules for you.

### The full example

This file has one record of each kind. It is the result of the four steps, and a reference for your own file:

```yaml
overrides:
- osm_id: 4230891
  tags:
    maxspeed: '10'
  note: road works
- osm_id: 4230100
  oneway: true
- osm_id: 503462470
  lanes: 3
  turn_lanes_forward: left|through|through
- osm_id: 4229577
  exclude_modes:
  - driving
- osm_id: 4229578
  remove: true
new_ways:
- id: 9000000000000
  nodes:
  - 21911886
  - [7.4235072, 43.7371133]
  - 21914340
  tags:
    highway: service
    name: New lane
turn_restrictions:
- from_way: 4230100
  via_node: 21924057
  to_way: 4230099
  restriction: no_left_turn
extra:
  tag_edits:
  - type: node
    id: 21924057
    set:
      highway: traffic_signals
  - type: relation
    id: 393226
    delete:
    - building
  building_faces:
  - building: 23715051
    from_node: 256791677
    to_node: 256791676
    type: front
    door: main
```

The editing page also adds `changed`, the time of the change, to each record. The build does not read it.

### The fields of each group

**`overrides`**: one rule for one OSM way.

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

**`new_ways`**: one new road.

| Field | Does |
|---|---|
| `id` | 9,000,000,000,000 or more, used one time. Do not change it: the edge ids come from it |
| `nodes` | the points of the road: an OSM node id, or `[lon, lat]` for a new point. Make the two ends OSM nodes, so that the road connects to the network |
| `tags`, `tags_from` | the OSM tags of the road, or the OSM way to copy the tags from |

**`turn_restrictions`**: one turn. A turn restriction changes the driving network.

| Field | Does |
|---|---|
| `from_way`, `via_node`, `to_way` | the way before the junction, the node of the junction and the way after it |
| `restriction` | `no_u_turn` (the default), `no_left_turn`, `no_right_turn`, `no_straight_on`, or `only_left_turn`, `only_right_turn`, `only_straight_on` |

**`extra`**: the tags of other objects, and the walls of buildings.

| Record | Fields |
|---|---|
| `tag_edits` | `type` (`node`, `way` or `relation`), `id`, and `set` (the tags to set) or `delete` (the tags to delete) |
| `building_faces` | `building`, `from_node` and `to_node` (the wall), `type` (`front`, `back`, `side`, `party` or `court`) and `door` |

A node and a relation have no `remove`. To take one out, delete the tag that makes it what it is. For example, a
relation without the tag `building` is no longer a building.""")

ch.md("""## Build Monaco without a fix

First, build the driving network of Monaco with no fixes file. This network is the "before" of the chapter.
`duckosm.get` writes the OSM file and the boundary into the folder `data`, and `area.build` writes the network beside
them. A fix needs these two files, because the build with the fix starts again from the OSM data.""")
ch.code("""area = duckosm.get(osm_id=2220322, name="monaco", out="data")    # the OSM file of Monaco and its boundary
net = area.build(modes=["driving"])                               # the driving network, with no fixes file
net                                                               # the card""")
ch.md("""The card shows the file `data/monaco.duckdb` and the driving network: the number of
edges and nodes and the length in km. This is the network before the fix.""")


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
the area on a map, and it writes the same format as `Fixes`.

You choose the files. The page reads one file into its list (`load`) and writes one file (`save`). duckOSM never
chooses a name or a folder for a fixes file. Thus you can load a base file, save a variant to another file, and the
base file does not change:

```python
net.edit(load="osm_fixes.yaml", save="osm_fixes_v2.yaml")
```

```text
'http://127.0.0.1:8790/'
```

`net.edit()` starts the page in the background and gives its address. Open the address in your browser. Two lines
over the tabs show the files: **Loaded: osm_fixes.yaml** and **Saving to: osm_fixes_v2.yaml**. Each change goes into
the save file at once. **Open** loads another file, **Save** writes the list, and **Save as** writes it to another
file. A path is relative to the folder where Python runs.

| Argument | Default | Does |
|---|---|---|
| `load` | none: the list is empty | a fixes file that the page reads; the page does not write it |
| `save` | none: the first **Save** asks for the file | the file that the page writes, each change at once; it can be the `load` file |
| `port` | `8790` | the port of the page on `127.0.0.1` |

With `load` only, the first **Save** asks where to save. With neither, the list is empty, and the page writes nothing
until the first **Save**. When the network was built with a fixes file, the page asks **Built with osm_fixes.yaml.
Load it?**, but it does not load the file by itself. `fixes="F"` is the old name of `load="F", save="F"`.

<img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/images/edit-road.png" alt="The editing page: Boulevard Charles III is picked in the Road tab, and its lanes are changed to 3" width="100%">

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
3. Change the fields, then click **Add to changes**. The page writes the record into the save file at once.
4. In the **Changes** tab, click **Build again** to build the network again with the save file. The database records
   the save file, so `net.fixes` and `net.rebuild()` find it.

In the **Turn** tab, you pick a road, and the page shows the roads at its junction: the picked road (**From**) is
blue, the roads that leave the junction are green, and the roads that only arrive are violet. You then choose the
turn that a driver cannot make, or must make.

<img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/images/edit-turn.png" alt="The Turn tab: the picked road is blue, the roads that leave the junction are green and the road that only arrives is violet" width="100%">

The editor runs while Python runs. The guide [Fix the OSM data](../guides/edit-osm-data.md) shows each tab and
button of the page.

> **Warning:** the page is on `127.0.0.1` of the machine that runs Python. In Colab, that machine is in the cloud, so
> your browser cannot open the page. Run `net.edit()` in Jupyter on your own computer, or use the step **Edit OSM
> data** of [duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/) in the browser.""")


ch.md("""### From Python: `duckosm.fixes.Fixes`

`duckosm.fixes.Fixes()` holds the rules of a fixes file in Python. `Fixes(load="osm_fixes.yaml")` reads a file that
you have; `Fixes()` starts with no rules. The calls that change the rules change them in memory only. `fixes.save(path)`
writes them to a file, as its own step. duckOSM never chooses a file name, and a load never writes the loaded file, so
you can load a base file and save a variant to another file.

`fixes.set(way, **fields)` adds a rule for the way, or changes its rule. A keyword that is a field of a rule (`oneway`,
`lanes`, `layer`, `note` ...) goes into the rule. Each other keyword is an OSM tag: `maxspeed=10` sets the tag
`maxspeed` to `"10"`. The build then reads the tag as OSM data, so the speed and the travel time of the road change.""")
ch.md("""The fix of this chapter: road works on Boulevard Louis II, the OSM way 4230891, lower its speed limit to
10 km/h.""")
ch.code("""way = 4230891                                          # Boulevard Louis II: the OSM way to fix
fixes = duckosm.fixes.Fixes()                          # no rules yet (load="file.yaml" reads a file)
fixes.set(way=way, maxspeed=10, note="road works")     # one rule: the tag maxspeed of the way, and a note
fixes                                                  # the card: a table of the rules""")
ch.md("""The card gives the number of rules (1) and says "not saved": the rule is only in memory. The table has one row
for each way: `osm_id`, the OSM `tags` of the rule (`maxspeed` is the text `'10'`) and the `note`.

`fixes.save(path)` writes the rules to a file. The first save needs a path. A later `fixes.save()` writes the same file
again.""")
ch.code("""fixes.save("osm_fixes.yaml")                           # write the rules: now there is a file""")
ch.md("""The file is ordinary YAML. You can read it, keep it in git with your project, and give it to a colleague.""")
ch.code("""print(open("osm_fixes.yaml").read())    # the fixes file on disk""")
ch.md("""The rule is one item of `overrides`: the `osm_id`, the `tags` with `maxspeed` and
the `note`. This is the format that the map editor writes, so the two tools can share one file.

> **Warning:** `save` writes the whole file again. Comments that you wrote in the file by hand are
> lost. The other keys of the file (`new_ways`, `turn_restrictions`, `extra`) stay.""")


ch.md("""#### The other calls

`set` is one of five calls that change the rules. The table shows all of them. Each call returns the `Fixes`, so you
can chain the calls. They change the rules in memory; `save` writes them.

| Call | Does | The field in the file |
|---|---|---|
| `fixes.set(way, **fields)` | sets the fields and the OSM tags of the way | the fields, and `tags` |
| `fixes.exclude(way, modes)` | removes the way from the networks of `modes` only | `exclude_modes` |
| `fixes.remove(way)` | removes the way from the whole build: no road, no building, no feature | `remove: true` |
| `fixes.drop(way)` | deletes the rule of the way, so the way is as in OSM again | the rule is gone |
| `fixes.rules()` | gives the rules as a DataFrame | |
| `fixes.save(path)` | writes the rules to `path` (later: `fixes.save()` to the same file) | the whole file |

The cell below tries each call on a second file, `practice.yaml`, so the file of the chapter keeps its one rule. Two
other roads of Monaco get a rule each.""")
ch.code("""w2, w3 = 503633031, 166399479                              # Avenue Princesse Grace and Boulevard Albert 1er
practice = duckosm.fixes.Fixes()                             # a second set of rules
practice.set(way=w2, oneway=True, lanes=2)                   # two fields of a rule: one-way, two lanes
practice.set(way=w2, highway="residential")                  # an OSM tag; the rule of w2 keeps its fields
practice.exclude(way=w3, modes=["driving"])                  # no cars on w3; walking and cycling keep it
practice.rules()                                             # one row for each way, one column for each field""")
ch.md("""**What the output shows.** The table has two rows, one for each way. The second `set` on the same way did not
make a new rule: it added the tag `highway` to the rule of `w2`, and `oneway` and `lanes` stay. The rule of `w3` has
only `exclude_modes`.""")
ch.code("""practice.remove(way=w3)        # remove w3 from the whole build
practice.drop(way=w2)          # w2 is as in OSM again
practice.save("practice.yaml")  # a second fixes file
practice, len(practice)        # the file has one rule""")
ch.md("""After `drop`, the file has one rule: the rule of `w3`, with `exclude_modes` and now
also `remove: true`. `len(fixes)` is the number of rules.

> **Tip:** `remove` and `exclude` are the two ways to take a road out. Use `exclude` for a road that some modes can
> use (a street that is closed to cars but open to people on foot). Use `remove` for a way that must not be in the
> data at all.""")


ch.md("""### In a text editor

The file is plain text, so you can also write it yourself in any text editor, in the format of section "The format of
the file". The cell below writes a file `hand.yaml` with one rule from Python, as a text editor would. Then `Fixes`
loads it and shows its rule: the file is the same as a file that `Fixes` or the editing page writes.""")
ch.code('''from pathlib import Path

Path("hand.yaml").write_text("""overrides:
  - osm_id: 4230891
    tags:
      maxspeed: "10"
    note: road works
""")                                                     # the file, written as text
duckosm.fixes.Fixes(load="hand.yaml").rules()          # load it: one rule''')
ch.md("""> **Warning:** YAML uses the spaces at the start of each line. Keep two spaces for each level, as in the example,
> and write a number in a tag in quotes (`"10"`), because an OSM tag is text.""")

ch.md("""## Build with the fixes

A fixes file changes nothing by itself. A build reads it. There are two ways to build with the file.

| Way | Uses | Use it when |
|---|---|---|
| `net.rebuild(fixes=...)` | the same OSM file and boundary as `net` | you want to compare the network before and after the fix |
| a new build: `duckosm.network(..., fixes=...)` | a new extract, or another area | you build later with newer OSM data, or in another project |

The database keeps a copy of the fixes file that it was built with, in the table `main.fixes`. Thus `net.fixes` gives
the rules of the build also when the file moves or changes, and a `.duckdb` file that you send to a colleague has its
fixes in it. When you change the file after the build, `net.fixes_changed()` is `True`: then build again.""")

ch.md("""### Build again: `net.rebuild`

`net.rebuild(fixes=...)` builds the area again, with the same modes, from the OSM file and the boundary beside the
database. It takes a `Fixes` or the path of a fixes file. The new network goes to `<name>_fixed.duckdb` beside the old
one (`out=` sets another file), so the old network stays and you can compare the two. To see the effect of the
fix, the cell compares one route by car across Monaco, from Fontvieille to Larvotto, before and after the fix.""")
ch.code("""fixed = net.rebuild(fixes=fixes)                     # the same build, with the fixes file: data/monaco_fixed.duckdb
a, b = (7.4155, 43.7285), (7.4335, 43.7468)          # Fontvieille and Larvotto, as (lon, lat)
before = net.route(a, b)                             # a route by car across Monaco, before the fix
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
ch.code("""closed = duckosm.fixes.Fixes().exclude(way=way, modes=["driving"]).save("closed.yaml")   # no cars on the way
shut = net.rebuild(fixes=closed, out="data/monaco_closed.duckdb")                   # a third network
gone = net.diff(shut)                                                               # the edges that are different
gone.change.value_counts(), set(fixed.edges(where=f"osm_id = {way}").edge_id) == set(gone.edge_id[gone.change == "removed"])""")
ch.md("""The diff has eight removed edges and no other change. The second value is `True`: the
removed edges are the eight edges of the fixed way. All the other edges of the network keep their ids.""")

ch.md("""### Every output takes the fix

The fix is in the network, so each export of the fixed network takes it: SUMO, MATSim, GMNS, OpenDRIVE and the GIS
files. The QuackOSM table reads the OSM file, and it takes the same fixes file with its own `fixes=`. Chapter 6 shows
these exports, and its last section reads the speed of Boulevard Louis II back from each output: 10 km/h in all of
them.""")


ch.exercises([
    ("Make the fixed way one-way with `fixes.set(way=way, oneway=True)`, build again into a new file, and count the "
     "edges that `diff` gives as removed.",
     """fixes.set(way=way, oneway=True).save("oneway.yaml")          # the rule of the way gets a field; a file of its own
oneway = net.rebuild(fixes=fixes, out="data/monaco_oneway.duckdb")
d = net.diff(oneway)
(d.change == "removed").sum()                                 # the edges of the other direction"""),
    ("Close Boulevard Albert 1er (`w3`) to cars only, and find the new time of the route.",
     """closed3 = duckosm.fixes.Fixes().exclude(way=w3, modes="driving").save("closed3.yaml")
net.rebuild(fixes=closed3, out="data/monaco_closed3.duckdb").route(a, b).time_s     # about 252 s"""),
    ("Delete the rule of the fixed way, so that the way is as in OSM again, and look at the file.",
     """fixes.drop(way=way).save("as_in_osm.yaml")                 # a file of its own: osm_fixes.yaml keeps its rule
print(open("as_in_osm.yaml").read())                           # overrides: []"""),
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
    ("`duckosm.fixes.Fixes(load=None)`", "the rules of a fixes file, in memory"),
    ("`fixes.save(path=None)`", "writes the rules to a file; the first save needs a path"),
    ("`fixes.set(way, **fields)`", "adds or changes the rule of a way: fields of a rule and OSM tags"),
    ("`fixes.exclude(way, modes)`", "removes the way from the networks of `modes`"),
    ("`fixes.remove(way)`", "removes the way from the build"),
    ("`fixes.drop(way)`", "deletes the rule of the way"),
    ("`fixes.rules()`, `len(fixes)`", "the rules as a DataFrame, and their number"),
    ("`net.rebuild(fixes=..., out=...)`", "builds the area again with the fixes; gives the new `Network`"),
    ("`net.fixes`", "the `Fixes` of the build, or `None`"),
    ("`net.diff(other, mode=None)`, `duckosm.fixes.diff(net, other)`", "the edges that are different, by `edge_id`"),
    ("`net.edit(load=None, save=None, port=8790)`", "starts the editing page and gives its URL"),
    ("`duckosm.network(..., fixes=...)`, `area.build(fixes=...)`", "a new build with the fixes file"),
    ("`duckosm.gis.quack(..., fixes=...)`", "the QuackOSM table of the OSM file, with the fixes"),
], reference="[duckosm.fixes](../reference/sdk/fixes.md)")

ch.write(sys.argv[1])
