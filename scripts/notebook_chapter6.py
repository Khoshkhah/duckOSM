"""Write docs/notebooks/06_exports.ipynb: chapter 6 of the notebook manual, "Exports".
The style comes from scripts/notebook_book.py (rules: docs/design/notebook_book.md).

    python scripts/notebook_chapter6.py docs/notebooks/06_exports.ipynb"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from notebook_book import Chapter  # noqa: E402

ch = Chapter(6, "06_exports", "Exports",
             "One network, seven formats: for simulation, planning, driving simulators, GIS and web maps",
             learn="which format each group of users needs; the export calls of a `Network`; one mode or all modes; how "
                   "you check each file; why each format keeps the `edge_id`",
             time="about 3 minutes", extras="widget,viz,sim",
             next_=("07_routing", "7. Routing", "the fastest route between two roads, two points or two names"))

ch.md("""## Why a network needs exports

A duckOSM network is a DuckDB file. Many tools cannot read it: a traffic simulator reads its own XML, a planning tool
reads GMNS tables, a GIS reads a GeoPackage. An **export** writes the network in the format of such a tool. Each export
is one call of the `Network`, and each call writes the files and gives a dict of their paths and their numbers.

| Format | Who uses it | The call | Module | How this chapter checks the file | Chapter |
|---|---|---|---|---|---|
| SUMO | traffic engineers who simulate each vehicle (microscopic simulation) | `net.to_sumo` | `duckosm.sim` | SUMO's `netconvert` reads the plain XML and writes the `.net.xml` | 12 |
| MATSim | researchers who simulate the trips of a whole population (agent-based simulation) | `net.to_matsim` | `duckosm.sim` | the XML has one link for each edge | 12 |
| GMNS | transport planners (path4gmns, AequilibraE, DTALite) | `net.to_gmns` | `duckosm.planning` | `duckosm.planning.check_gmns` runs its checks on the tables | 13 |
| OpenDRIVE | driving simulators and HD-map tools (CARLA, esmini) | `net.to_opendrive` | `duckosm.hdmap` | the XML has one road for each edge | 14 |
| GeoPackage, GeoParquet, shapefile | GIS users (QGIS, ArcGIS, GeoPandas) | `net.to_gis` | `duckosm.gis` | DuckDB reads each layer back and counts its rows | 10 |
| QuackOSM table | data engineers who want all OSM features with their tags | `duckosm.gis.quack` | `duckosm.gis` | DuckDB reads the table and counts the features of each OSM type | 10 |
| PMTiles | web maps (MapLibre, OpenLayers) | `duckosm quack --pmtiles` | the command | the file starts with the text `PMTiles` | 10 |

The chapters in the last column use each format in its own field. This chapter writes each format one time and
checks it.

> **Key idea:** in each format, the id of a road is the duckOSM `edge_id`: the id of a SUMO edge, of a MATSim link, of
> a GMNS link, of an OpenDRIVE road and of a row of the GIS layer. So the results of each tool (a flow, a speed, a
> count) join back to the network and to your own data by `edge_id`.""")
ch.md("""### The five levels of the outputs

The manual sorts all outputs into five levels, from the OSM file to the lanes. This chapter shows the outputs of
levels 1 to 4; level 0 is in chapter 1, and chapter 10 has more of level 1.

| Level | Name | One unit is | In this chapter |
|---|---|---|---|
| 0 | OSM file | the `.osm.pbf`, cut to your area | (chapter 1) |
| 1 | OSM data | an OSM feature, with no network | QuackOSM table, PMTiles |
| 2 | Routable network | one edge for each road direction, with its `edge_id` and turns | GeoPackage, GeoParquet, shapefiles |
| 3 | Simulation network | macro, meso and micro: links, lanes, turns | SUMO, GMNS, MATSim |
| 4 | HD map | the exact shape of each lane, in 3D (experimental) | OpenDRIVE |

Each level keeps the `edge_id` of level 2, so a result at any level joins back to the network.""")

ch.md("""## The parts of an export call

The export calls of a `Network` have the same form:

| Part | Does |
|---|---|
| the first argument | the output: a folder (`to_sumo`) or a file (`to_matsim`, `to_gmns`, `to_opendrive`, `to_gis`) |
| `mode` | the mode to export. `net.driving.to_sumo(...)` is the same as `net.to_sumo(..., mode="driving")` |
| `progress` | the progress line. `False` shows nothing |
| the other keywords | the options of the format, from the function of its module |
| the result | a dict: the paths of the files and the numbers (nodes, edges, links, layers) |

Each call is a function of a module too: `net.to_sumo(...)` is `duckosm.sim.to_sumo(net, ...)`. Some formats are of
one mode, and some take all modes:

| Call | Without `mode` |
|---|---|
| `to_sumo`, `to_opendrive` | the driving network |
| `to_matsim` | the driving network. `mode` can also be a list of modes |
| `to_gmns`, `to_gis` | all the modes of the file |""")

ch.md("""## Open the network of chapter 1

Chapter 1 builds Monaco with three modes (driving, walking and cycling) and saves it as `monaco.duckdb`. This chapter
opens that file with `duckosm.open`. The exports of this chapter use the driving network. Its summary gives the number
of edges and nodes, so you can compare each export with it.""")
ch.code("""net = duckosm.open("monaco.duckdb")    # the network of chapter 1: Monaco, three modes
net.driving.summary()                                         # the number of edges, to compare with each export""")
ch.md("""The driving network of Monaco has 2,840 edges and 1,768 nodes, 96.2 km of road.
Each export below gives its own count, and the count must agree with these numbers.""")

ch.md("""## SUMO

[SUMO](https://eclipse.dev/sumo/) (Simulation of Urban MObility) simulates each vehicle on each lane, second by
second. Its network file is `.net.xml`. SUMO's program `netconvert` makes it from **plain XML** files: the nodes
(`.nod.xml`), the edges (`.edg.xml`) and the connections between lanes (`.con.xml`).

`to_sumo` writes the plain files and a **netconvert config** (`.netccfg`), the input of `netconvert`. You can change
the config, and then you run `netconvert`. The plain files have all that `netconvert` takes from an OSM file for the
network: the lanes, their widths and vehicle classes, the speeds, the road classes, the names, the shapes, the legal
turns, the traffic lights and the stop signs. Thus you do not give `netconvert` the OSM file, which has no fixes and no
`edge_id`.

```text
network.nod.xml ─┐
network.edg.xml ─┼─ named in network.netccfg ──> netconvert -c network.netccfg ──> network.net.xml
network.con.xml ─┘
network.add.xml ─── the bus stops, for SUMO itself (sumo --additional-files)
```

| Argument | Default | Does |
|---|---|---|
| `net_name` | `"network"` | the start of each file name |
| `run_netconvert` | `False` | `True`: duckOSM also runs `netconvert`, and writes the `.net.xml` and the signal programs (`.tll.xml`) |
| `connections` | `True` | writes the connections of the lanes at each junction (from the turn graph and the turn lanes) |
| `config` | none | a dict of options that change the default config, or the path of your own `.netccfg` |""")
ch.code("""sumo = net.driving.to_sumo("out/sumo")    # the plain XML files, the bus stops and the netconvert config
sumo                              # the paths and the numbers""")
ch.md("""**What the output shows.** The dict has the paths of the three plain files, of the bus stops (`add`) and of the
netconvert config (`netccfg`). There is no `.net.xml`: `netconvert` did not run. The plain files have 1,768 nodes,
2,840 edges and 5,082 connections: the same nodes and edges as the network, and one connection for each arc of the
turn graph that a car can use (chapter 7). The nodes have 18 traffic lights and 12 stop signs, and the
additional file has 106 bus stops.""")
ch.md("""The check: each SUMO edge must be a duckOSM edge. `duckosm.sim.edge_map` reads the `.edg.xml` and gives the
`edge_id` of each SUMO edge.""")
ch.code("""ids = duckosm.sim.edge_map(sumo["edg"])          # the SUMO id and the edge_id of each SUMO edge
len(ids), ids.edge_id.nunique()                  # the rows, and the different edge ids in them""")
ch.md("""The `.edg.xml` has 2,840 edges, and they are 2,840 different edge ids: each duckOSM edge is one SUMO
edge, with its `edge_id` as its SUMO id.""")

ch.md("""### Traffic lights and stop signs

OSM often puts a traffic light or a stop sign on a node of the way before the junction, not on the junction. duckOSM
moves the sign to the nearest junction of the same way, if it is 25 m or nearer, as `netconvert` does with its option
`tls.guess-signals`. The traffic lights within 20 m of each other get one program: the node attribute `tl`.

| OSM tag | SUMO node type |
|---|---|
| `highway=traffic_signals` | `traffic_light` |
| `highway=stop` | `priority_stop`: the vehicles of the minor road stop |
| `highway=stop` with `stop=all` | `allway_stop`: all vehicles stop |
| `highway=give_way` | no change: a SUMO priority junction makes the minor road wait |""")
ch.code("""import xml.etree.ElementTree as ET                     # to read the XML back
from collections import Counter
Counter(n.get("type") for n in ET.parse(sumo["nod"]).getroot() if n.get("type"))    # the nodes with a sign, by type""")
ch.md("""**What the output shows.** 18 junctions have a traffic light, 11 have a stop sign for the minor road, and 1 is
an all-way stop.""")

ch.md("""### The netconvert config

`network.netccfg` is a short XML file. Each line is one option of `netconvert`.""")
ch.code("""print(open(sumo["netccfg"]).read())               # the input of netconvert""")
ch.md("""| Line | Change it? |
|---|---|
| `node-files`, `edge-files`, `connection-files` | no: the files of duckOSM |
| `output-file` | yes: the name of the `.net.xml` |
| `proj.utm` | yes: `true` gives metres in the UTM zone of the area |
| `geometry.remove` = `false` | **no**: with `true`, `netconvert` joins the edges between two junctions, and a SUMO edge is no longer one `edge_id` |
| `junctions.join` = `false` | **no**: with `true`, the short edges inside a joined junction go, and the turns through them are lost. If you need it, use `to_sumo(..., config={"junctions.join": "true"}, run_netconvert=True)`: duckOSM then keeps the turns |
| `numerical-ids` = `false` | **no**: with `true`, `netconvert` gives new ids |
| `no-turnarounds.except-deadend` | yes: `true` gives no U-turns, except at a dead end |
| `tls.default-type` = `actuated` | yes: OSM has no signal timings, so the default program follows the traffic. `static` gives fixed times |

The three lines in bold keep the `edge_id` as the SUMO id. You can change the config in two ways:

1. Edit the file as text, then run `netconvert -c` on it.
2. Give `config={...}` to `to_sumo`: duckOSM writes the options into the file. For example
   `net.driving.to_sumo("out/sumo", config={"tls.default-type": "static"})`.""")

ch.md("""### The extra files

- `network.add.xml`: the bus stops, for SUMO itself (`sumo --additional-files network.add.xml`), not for `netconvert`.
  Each OSM bus stop is a `<busStop>` on the right-most lane that buses can use, on the nearest edge of a bus route.
  The lane id is `<edge_id>_<index>`, index 0 on the right. `lines` has the bus routes on that edge.
- `network.tll.xml`: the signal programs. OSM seldom has signal timings, and a program must have one signal for each
  lane connection of its junction, in the order of `netconvert`. Thus only `run_netconvert=True` writes this file: it
  copies the programs that `netconvert` built, and adds the file to the config. Edit the phases in it, and run
  `netconvert -c` again: `netconvert` then uses your programs.""")
ch.code("""print("".join(open(sumo["add"]).readlines()[:4]))  # the first bus stops""")

ch.md("""### The next step: netconvert

Run `netconvert` on the config. It writes `out/sumo/network.net.xml`, the file that SUMO reads. The extra `sim`
installs `netconvert` (the Python package `eclipse-sumo`). Chapter 12 does this step and runs a simulation.

```bash
netconvert -c out/sumo/network.netccfg
```""")

ch.md("""## GMNS

[GMNS](https://github.com/zephyr-data-specs/GMNS) (General Modeling Network Specification) is a standard set of tables
for transport planning: `node`, `link`, `lane`, `movement` (a turn at a junction) and more. Planning tools such as
path4gmns, AequilibraE and DTALite read it.

`to_gmns` writes the tables into a DuckDB file, one schema for each mode (`gmns_driving` ...). `to_csv` also writes
each table as a CSV file, the form of the standard.

| Argument | Default | Does |
|---|---|---|
| `to_csv` | none | a folder: the CSV files too |
| `modes` | all | the modes, when you call it on the network (`mode` gives one) |
| `lane_geometry` | `True` | a line for each lane |
| `combined` | `False` | also one network of all modes together |""")
ch.code("""gmns = net.driving.to_gmns("out/gmns.duckdb", to_csv="out/gmns")         # the GMNS tables in a DuckDB file and as CSV
gmns["modes"]                                                     # the rows of each table""")
ch.md("""**What the output shows.** The GMNS network of driving has 1,772 nodes, 2,858 links, 3,318 lanes and 3,755
movements. It has 18 links and 4 nodes more than `driving.edges` (2,840 edges, 1,768 nodes). The 18 links are the
bus-only edges of `driving.private_edges`: a bus lane against a one-way street, or a road for buses only. A car route
does not use them, but they are lanes on the road, so GMNS keeps them, with `allowed_uses` `bus` or `bus,bike`. The 4
nodes are their ends that are not in the driving network.

The check: `duckosm.planning.check_gmns` runs the rules of the standard on the tables. Examples: each link starts at
its `from_node`, each lane number is correct, each movement turns where its inbound link ends. It gives one row for
each rule: the schema, the rule, the number of bad rows and the number of rows.""")
ch.code("""checks = duckosm.planning.check_gmns("out/gmns.duckdb")             # (schema, check, bad rows, rows) for each check
len(checks), [c for c in checks if c[2] > 0]                        # the number of checks, and the checks with bad rows""")
ch.md("""`check_gmns` ran 17 checks, and no check found a bad row (the empty list).""")

ch.md("""## MATSim

[MATSim](https://www.matsim.org) simulates the trips of each person of a population for a whole day. Its network is
one XML file (`network.xml`, gzip by default) with `<node>` and `<link>` elements. A link has its length, its free
speed (m/s), its capacity and its lanes.

| Argument | Default | Does |
|---|---|---|
| `crs` | the UTM zone of the area | the coordinate system of the nodes (MATSim needs metres) |
| `gzip` | `True` | writes `.xml.gz` |""")
ch.code("""matsim = net.driving.to_matsim("out/matsim_network.xml.gz")    # the MATSim network
matsim                                                          # its numbers and its coordinate system""")
ch.md("""The MATSim network has 1,768 nodes and 2,840 links, in UTM zone 32N (`EPSG:32632`, in metres).

The check: Python's XML reader opens the file and counts its `<link>` elements. `gzip.open` reads the compressed
file.""")
ch.code("""import gzip
import xml.etree.ElementTree as ET                              # Python's XML reader

tree = ET.parse(gzip.open("out/matsim_network.xml.gz"))         # the XML of the file
len(tree.findall(".//link"))                                    # the <link> elements""")
ch.md("""The file has 2,840 `<link>` elements: one link for each edge.""")

ch.md("""## OpenDRIVE (experimental)

[ASAM OpenDRIVE](https://www.asam.net/standards/detail/opendrive/) (`.xodr`) describes roads for driving simulators:
the reference line of each road, its lanes and their widths. Simulators such as CARLA and esmini read it. The export
is experimental: it writes one road for each edge, and the junctions only with `junctions=True`.

| Argument | Default | Does |
|---|---|---|
| `crs` | the UTM zone of the area | the coordinate system (metres) |
| `junctions` | `False` | `True`: also the OpenDRIVE junctions |""")
ch.code("""odr = net.driving.to_opendrive("out/monaco.xodr")       # the OpenDRIVE file
print(odr)                                              # its numbers
len(ET.parse("out/monaco.xodr").findall("road"))        # the check: the <road> elements in the file""")
ch.md("""The OpenDRIVE file has 2,840 roads, in UTM zone 32N. The XML has 2,840 `<road>` elements: one road for each
edge.""")

ch.md("""## GeoPackage, GeoParquet and shapefiles

A GIS (QGIS, ArcGIS) and GeoPandas read standard GIS files. `to_gis` writes the edges and the nodes of each mode as
layers (`edges_driving`, `nodes_driving` ...) and the boundary of the area.

| Argument | Default | Does |
|---|---|---|
| `fmt` | `"gpkg"` | `"gpkg"`: one GeoPackage file; `"geoparquet"`: a folder of GeoParquet files; `"shp"`: a folder of shapefiles |
| `modes` | all | the modes (`mode` gives one) |
| `boundary` | `True` | also the layer `boundary` |
| `srs` | `"EPSG:4326"` | the coordinate system of the file |""")
ch.code("""gis = net.driving.to_gis("out/monaco.gpkg")    # the GeoPackage
gis["layers"]                                  # the rows of each layer""")
ch.md("""The GeoPackage has three layers: 2,840 edges, 1,768 nodes and the boundary (1 row).

The check: DuckDB reads each layer back and counts its rows. The function `st_read` of the DuckDB extension
`spatial` reads one layer of a GIS file.""")
ch.code("""import duckdb

con = duckdb.connect()                     # a DuckDB in memory, to read the files back
con.load_extension("spatial")              # the extension that reads GIS files
for layer in gis["layers"]:
    rows = con.sql(f"SELECT count(*) FROM st_read('out/monaco.gpkg', layer='{layer}')").fetchone()[0]
    print(layer, rows)""")
ch.md("""DuckDB reads each layer with the same number of rows as the network.""")
ch.md("""GeoParquet is a Parquet file with a geometry column. It is fast to read for DuckDB, GeoPandas and the cloud data
tools. `fmt="geoparquet"` writes one file for each layer into a folder.""")
ch.code("""pq = net.driving.to_gis("out/geoparquet", fmt="geoparquet")    # a folder of GeoParquet files
print(len(pq["files"]), "files")
con.sql("SELECT count(*) FROM 'out/geoparquet/edges_driving.parquet'").fetchone()[0]    # the check: the edges""")
ch.md("""**What the output shows.** The folder has 15 files. DuckDB reads 2,840 rows from the edges file. The folder also
has the layers of the base map (`features_buildings`, `features_streets` ...), which the GeoPackage of the cell before
does not have.

> **Tip:** a shapefile cuts each column name to 10 characters, and each layer is a set of files. Use GeoPackage or
> GeoParquet when the tool can read them.""")

ch.md("""## QuackOSM table and PMTiles

The calls above export the **network**. `duckosm.gis.quack` exports the **OSM data**: it reads the OSM file, not the
network, and gives all OSM features (nodes, ways, relations) with all their tags, in the format of
[QuackOSM](https://github.com/kraina-ai/quackosm). It takes the `Area` of `duckosm.get` (or the path of an OSM file).

| Argument | Default | Does |
|---|---|---|
| `out`, `name` | `"."`, `"area"` | the folder and the start of the file name (`<name>_quack.parquet`) |
| `formats` | GeoParquet | the formats of the table: `"geoparquet"`, `"gpkg"` ... |
| `gdal` | `False` | `True`: the five layers of GDAL's OSM driver in place of the table |
| `fixes` | none | the fixes file (chapter 5) |""")
ch.code("""area = duckosm.get(osm_id=2220322, name="monaco", out="data")                     # the OSM file (from the cache)
quack = duckosm.gis.quack(area, out="out", name="monaco", formats=["geoparquet"])    # the QuackOSM table
quack                                                                             # the list of the files""")
ch.md("""The result is the list of the files that `quack` wrote: here one GeoParquet file. Each row of the table is one
OSM feature. Its `feature_id` is the OSM type and the id, for example `way/4230891`. The check counts the features of
each OSM type: `split_part(feature_id, '/', 1)` is the text before the `/`.""")
ch.code("""con.sql(f\"\"\"
    SELECT split_part(feature_id, '/', 1) AS type, count(*) AS features
    FROM '{quack[0]}'
    GROUP BY type ORDER BY type
\"\"\").df()                                                        # the features of each OSM type""")
ch.md("""The QuackOSM table of Monaco has 3,994 nodes (points with tags: shops, stops,
trees ...), 5,606 ways and 59 relations. This is all of the OSM data with tags, not only the roads: a building, a park
and a bench are features too.

**PMTiles** is one file of vector tiles: a base map that a web map (MapLibre, OpenLayers) reads directly from a web
server, with no tile server. The command `duckosm quack --pmtiles` makes it. The SDK has no Python call for it yet,
so the cell runs the command.""")
ch.code("""import subprocess, sys
from pathlib import Path

command = [sys.executable, "-m", "duckosm", "quack", str(area.pbf), "--pmtiles", "-o", "out", "--name", "monaco"]
done = subprocess.run(command, check=True)            # the command: duckosm quack <OSM file> --pmtiles -o out --name monaco
print("exit code", done.returncode)                   # 0: no error""")
ch.code("""pmtiles = Path("out/monaco.pmtiles")
print(pmtiles.stat().st_size, "bytes")
pmtiles.read_bytes()[:7]                              # the check: a PMTiles file starts with the text "PMTiles\"""")
ch.md("""The command made 59 tiles of 9,659 features. The file is 0.2 MB
(219,713 bytes), and its first seven bytes are `PMTiles`.""")

ch.md("""## The fix of chapter 5 in every output

Chapter 5 fixes the OSM data in a fixes file: road works lower the speed limit of Boulevard Louis II (the OSM way
4230891) to 10 km/h. Each export of a fixed network takes the fix. The cells below build the network with that fixes
file, write five outputs and read the speed of one edge of Boulevard Louis II back from each file.

| Output | Where the speed is |
|---|---|
| the database | `maxspeed_kmh` of `driving.edges` |
| GeoPackage | `maxspeed_kmh` of the layer `edges_driving` |
| SUMO | the attribute `speed` (m/s) of the `<edge>` |
| MATSim | the attribute `freespeed` (m/s) of the `<link>` |
| GMNS | `free_speed` (km/h) of the table `link` |
| QuackOSM table | the tag `maxspeed` of the way |

In each file, the id of the edge (or of the link) is the `edge_id`. So the cells find the same edge in each output.""")
ch.code('''Path("osm_fixes.yaml").write_text("""overrides:
- osm_id: 4230891
  tags:
    maxspeed: '10'
  note: road works
""")                                                                                 # the fixes file of chapter 5
fixed = area.build(modes=["driving"], output="data/monaco_fixed.duckdb", fixes="osm_fixes.yaml")    # the network with the fix''')
ch.md("""The cell below takes one edge of Boulevard Louis II and starts the dict `speeds`: its speed in each output. The
first entry is the database.""")
ch.code("""louis = fixed.driving.edges(where="osm_id = 4230891")       # the edges of Boulevard Louis II
edge = int(louis.edge_id.iloc[0])                            # one of them: its edge_id
speeds = {"database (km/h)": float(louis.maxspeed_kmh.iloc[0])}
speeds""")
ch.md("""Each cell below writes one output and reads the speed of the edge back from it. SUMO and MATSim keep the speed
in m/s, so the cells multiply it by 3.6 to get km/h. The `?` in a query is a parameter: DuckDB puts the value of the
list (`[edge]`) in its place.""")
ch.code("""fixed.driving.to_gis("out/fixed.gpkg")                                 # a GeoPackage
query = "SELECT maxspeed_kmh FROM st_read('out/fixed.gpkg', layer='edges_driving') WHERE edge_id = ?"
speeds["GeoPackage (km/h)"] = con.execute(query, [edge]).fetchone()[0]""")
ch.code("""fixed_sumo = fixed.driving.to_sumo("out/fixed_sumo")    # the plain SUMO files
xml_edge = ET.parse(fixed_sumo["edg"]).find(f".//edge[@id='{edge}']")            # the <edge> with the edge_id as its id
speeds["SUMO (km/h)"] = round(float(xml_edge.get("speed")) * 3.6, 1)""")
ch.code("""fixed.driving.to_matsim("out/fixed_matsim.xml.gz")                          # the MATSim network
tree = ET.parse(gzip.open("out/fixed_matsim.xml.gz"))
link = tree.find(f".//link[@id='{edge}']")                                  # the <link> with the edge_id as its id
speeds["MATSim (km/h)"] = round(float(link.get("freespeed")) * 3.6, 1)""")
ch.code("""fixed.driving.to_gmns("out/fixed_gmns.duckdb")                              # the GMNS network
gmns_db = duckdb.connect("out/fixed_gmns.duckdb", read_only=True)
query = "SELECT free_speed FROM gmns_driving.link WHERE link_id = ?"        # the link with the edge_id as its id
speeds["GMNS (km/h)"] = gmns_db.execute(query, [edge]).fetchone()[0]""")
ch.code("""quack = duckosm.gis.quack(area, out="out", name="fixed", fixes="osm_fixes.yaml", formats=["geoparquet"])    # the OSM data with the fix
query = f"SELECT tags['maxspeed'] FROM '{quack[0]}' WHERE feature_id = 'way/4230891'"    # the tag of the way
speeds["QuackOSM table (tag)"] = con.sql(query).fetchone()[0]
speeds                                                                     # the speed of the edge in each output""")
ch.md("""**What the output shows.** Each output gives 10 km/h for the edge. The QuackOSM table has the OSM tag, so it
shows the text `'10'`. One fixes file corrects all six outputs.""")

ch.md("""## The files of the chapter

Each export wrote its files into the folder `out`. The last cell gives the size of the main file of each export.""")
ch.code("""files = ["out/sumo/network.edg.xml", "out/gmns.duckdb", "out/matsim_network.xml.gz", "out/monaco.xodr",
         "out/monaco.gpkg", "out/geoparquet/edges_driving.parquet", "out/monaco_quack.parquet", "out/monaco.pmtiles"]
for f in files:
    print(f"{Path(f).stat().st_size / 1e6:6.2f} MB  {f}")       # the size in MB, and the file""")
ch.md("""The largest file is the GMNS database (approximately 9 MB): it has the lanes, the movements and a line for each lane. The
OpenDRIVE file (2.10 MB) has the reference line and the lanes of each road. The MATSim network is the smallest file
(0.10 MB), because it is compressed with gzip.""")

ch.exercises([
    ("Export the walking network of Monaco to MATSim, and compare its number of links with the number of walking edges.",
     """net.walking.to_matsim("out/matsim_walking.xml.gz"), len(net.walking.edges())"""),
    ("Write the driving network as GeoParquet files in the coordinate system of UTM zone 32N (`EPSG:32632`), and read "
     "the edges with GeoPandas.",
     """import geopandas as gpd
net.driving.to_gis("out/utm", fmt="geoparquet", srs="EPSG:32632")
gpd.read_parquet("out/utm/edges_driving.parquet").crs"""),
    ("Write the SUMO files with fixed signal times (`tls.default-type` = `static`) into the folder `out/static`, and "
     "read the line from the netconvert config.",
     """static = net.driving.to_sumo("out/static", config={"tls.default-type": "static"})
[line for line in open(static["netccfg"]) if "tls.default-type" in line]"""),
])

ch.glossary([
    ("export", "a call that writes the network in the format of another tool"),
    ("plain XML", "the three input files of SUMO's `netconvert`: nodes, edges, connections"),
    ("netconvert config", "the `.netccfg` file: the input files and the options of `netconvert`"),
    ("`netconvert`", "the program of SUMO that makes the `.net.xml` network"),
    ("link", "the word of MATSim and GMNS for an edge"),
    ("movement", "a turn from one link to another at a junction, in GMNS"),
    ("layer", "one table of a GIS file, for example `edges_driving`"),
    ("GeoParquet", "a Parquet file with a geometry column"),
    ("vector tiles", "a map cut into small squares of geometry, for web maps. PMTiles holds them in one file"),
])

ch.summary([
    ("`net.driving.to_sumo(out_dir)`", "the SUMO plain XML, the bus stops and the netconvert config"),
    ("`netconvert -c out_dir/network.netccfg`", "the SUMO network `.net.xml` (the command)"),
    ("`duckosm.sim.edge_map(files)`", "the `edge_id` of each SUMO edge or MATSim link"),
    ("`net.to_gmns(out_path, to_csv=None)`", "the GMNS tables in DuckDB, and as CSV"),
    ("`duckosm.planning.check_gmns(path)`", "the checks of the GMNS tables"),
    ("`net.to_matsim(out_path)`", "the MATSim network"),
    ("`net.to_opendrive(out_path)`", "the OpenDRIVE file (experimental)"),
    ("`net.to_gis(out, fmt=\"gpkg\")`", "a GeoPackage, GeoParquet files or shapefiles"),
    ("`duckosm.gis.quack(area, formats=...)`", "the QuackOSM table of the OSM file"),
    ("`duckosm quack --pmtiles`", "a PMTiles file of vector tiles (the command)"),
], reference="[duckosm.sim](../reference/sdk/sim.md), [duckosm.planning](../reference/sdk/planning.md), "
             "[duckosm.hdmap](../reference/sdk/hdmap.md), [duckosm.gis](../reference/sdk/gis.md)")

ch.write(sys.argv[1])
