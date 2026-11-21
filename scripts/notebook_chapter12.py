"""Write docs/notebooks/12_sim.ipynb: chapter 12 of the notebook manual, "Traffic simulation".
The style comes from scripts/notebook_book.py (rules: docs/design/notebook_book.md).

    python scripts/notebook_chapter12.py docs/notebooks/12_sim.ipynb"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from notebook_book import IMAGES, Chapter  # noqa: E402

ch = Chapter(12, "12_sim", "Traffic simulation",
             "SUMO and MATSim networks whose ids are the edge ids",
             learn="what a traffic simulation needs; the SUMO network with `to_sumo` and its options; how you read it "
                   "with sumolib; `edge_map` from the SUMO ids to `edge_id`; a short SUMO run and its counts on the "
                   "edges; the MATSim network, and MATSim lanes and signals",
             time="about 3 minutes", extras="sim,viz",
             next_=("13_planning", "13. Transport planning", "GMNS networks, path4gmns and AequilibraE"))

ch.md(f"""## What a traffic simulation needs

A **traffic simulation** moves vehicles one by one on a road network and gives the result: the count of vehicles on
each road, their speed, the queues and the travel times. It needs three parts:

| Part | What it is | Where it comes from |
|---|---|---|
| the network | the roads, their lanes and speeds, and the turns at each junction | duckOSM: `to_sumo` or `to_matsim` |
| the demand | the trips: where each vehicle starts, where it goes, and when | your origin-destination data, or random trips for a test |
| the simulator | the program that moves the vehicles | SUMO (microscopic: each vehicle and each lane) or MATSim (agents and their daily plans) |

The module `duckosm.sim` makes the network for the two simulators, and gives you the way back from their results to
the duckOSM edges.

| Simulator | The duckOSM call | The file | Use it for |
|---|---|---|---|
| SUMO | `to_sumo` | `network.net.xml` (and the plain XML files) | each vehicle and lane, traffic lights, junctions |
| MATSim | `to_matsim` | `network.xml.gz` | large areas, daily plans of agents, many iterations |

<img src="{IMAGES}/sim-loop.svg" alt="The loop of a simulation: the duckOSM network, the SUMO network, the demand, the simulation, and the results back on the edges" width="100%">

> **Key idea:** the id of each SUMO edge and of each MATSim link is the duckOSM `edge_id`. So the results of a
> simulation join back to the network, to your own data (chapter 4) and to the map, with no table of ids to keep.

> **Tip:** the page [duckOSM for traffic simulation users](../compare/for-simulation-users.md) compares each call with
> netconvert, sumolib and MATSim's own tools.""")

ch.md("""## Open the network of chapter 1

Chapter 1 builds Monaco with three modes (driving, walking and cycling) and saves it as `monaco.duckdb`. This chapter opens that file with `duckosm.open`: it builds nothing. The simulation uses the driving network. The extra `sim` installs SUMO (with its program `netconvert`) and
sumolib.""")
ch.code("""net = duckosm.open("monaco.duckdb")    # the network of chapter 1: Monaco, three modes
net.driving                                                                # the card of the driving mode""")
ch.md("""The driving network of Monaco has 2840 edges and 1768 nodes, 96.2 km of roads.""")

ch.md("""## The SUMO network: `to_sumo`

`net.driving.to_sumo(out_dir)` (the same as `duckosm.sim.to_sumo(net.driving, out_dir)`) writes the network of one
mode in SUMO's **plain XML** files and the netconvert config (chapter 6). The simulation needs the `.net.xml`, so this
chapter gives `run_netconvert=True`: duckOSM then runs SUMO's `netconvert` on the config.

| File | What it has |
|---|---|
| `network.nod.xml` | the nodes: one SUMO junction for each duckOSM node |
| `network.edg.xml` | the edges: the id (the `edge_id`), the lanes, the speed, the shape |
| `network.con.xml` | the connections: the turns that are allowed at each junction, from `driving.edge_graph`, so the turn restrictions apply |
| `network.add.xml` | the bus stops, for SUMO (`--additional-files`) |
| `network.netccfg` | the netconvert config: the input files and the options of `netconvert` |
| `network.net.xml` | the network that SUMO reads, made by `netconvert` |
| `network.tll.xml` | the signal programs that `netconvert` made; edit them and run `netconvert -c network.netccfg` again |

The call gives a dict with the path of each file and the counts.

| Argument | Default | What it does |
|---|---|---|
| `net_name` | `"network"` | the first part of each file name |
| `run_netconvert` | `False` | `True`: also run `netconvert`, for the `.net.xml` and the `.tll.xml` |
| `connections` | `True` | `False`: no `.con.xml`, so netconvert chooses the turns itself |
| `config` | none | a dict of options that change the default config, or your own `.netccfg` file |""")
ch.code("""files = net.driving.to_sumo("out/sumo", run_netconvert=True)    # the plain XML files, netconvert and network.net.xml
{k: str(v) for k, v in files.items()}                       # the files and the counts""")
ch.md("""The SUMO
network has 1768 nodes and 2840 edges, the same numbers as the driving network, and 5082 connections. `net` is the
path of `network.net.xml`, the file that SUMO reads.""")

ch.md("""### The netconvert options

duckOSM gives `netconvert` a small set of options, `duckosm.sim.DEFAULT_NETCFG`. They keep each duckOSM edge as one
SUMO edge with the same id.""")
ch.code("""duckosm.sim.DEFAULT_NETCFG                                  # the options that to_sumo gives netconvert""")
ch.md("""The six options:

| Option | Value | Why |
|---|---|---|
| `proj.utm` | `true` | the nodes are in lon and lat, and netconvert projects them to the UTM zone of the area |
| `geometry.remove` | `false` | netconvert must not join two edges into one, so each SUMO edge is one `edge_id` |
| `junctions.join` | `false` | netconvert must not join near nodes into one junction |
| `no-turnarounds.except-deadend` | `true` | no U-turns, except at a dead end |
| `numerical-ids` | `false` | the ids stay as duckOSM writes them |
| `tls.default-type` | `actuated` | OSM has no signal timings: the program of each traffic light follows the traffic |""")

ch.md("""### Your own options: `config=`

`config=` takes your own `.netccfg` file, and `netconvert` then uses it in place of the defaults. This cell writes a
file with the defaults and one change, `geometry.remove = true`, to show why the default is `false`.""")
ch.code("""cfg = {**duckosm.sim.DEFAULT_NETCFG, "geometry.remove": "true"}           # the defaults, with one change
with open("merge.netccfg", "w") as f:                                      # a netconvert config file
    f.write("<configuration>\\n" + "".join(f'  <{k} value="{v}"/>\\n' for k, v in cfg.items()) + "</configuration>\\n")
merged = net.driving.to_sumo("out/sumo_merged", config="merge.netccfg", run_netconvert=True)    # netconvert with your options
len(duckosm.sim.edge_map(merged)), files["n_edges"]                     # the edges with their edge_id in SUMO, of all""")
ch.md("""With `geometry.remove = true`, only 1615 of the 2840 edges keep their `edge_id` as a
SUMO edge. netconvert joined the other 1225 edges into their neighbors.

> **Warning:** keep `geometry.remove` set to `false`. With `true`, netconvert joins the edges between two junctions
> into one SUMO edge, and keeps only the id of the first edge. The other edges then have no SUMO edge, and their
> results are lost.""")

ch.md("""### Without netconvert

With the default `run_netconvert=False`, `to_sumo` writes the plain XML files and the config, and does not run
`netconvert`. Use it when SUMO is on another machine, or when you change the config first (chapter 6).""")
ch.code("""plain = net.driving.to_sumo("out/sumo_plain")                             # no netconvert, no .net.xml
sorted(k for k in plain if not k.startswith("n_"))                          # the files""")
ch.md("""The dict has the plain XML files (`nod`, `edg`, `con`), the bus stops (`add`) and the config (`netccfg`), and
no `net`. Run `netconvert -c out/sumo_plain/network.netccfg` to make the network.""")

ch.md("""## Read the network with sumolib: `sumo_net`

`duckosm.sim.sumo_net(files)` reads the `.net.xml` with sumolib (`sumolib.net.readNet`), with the internal edges of
the junctions. It takes the path of a `.net.xml` or the dict of `to_sumo`. The result is a `sumolib.net.Net`: use
sumolib's own calls on it.""")
ch.code("""sn = duckosm.sim.sumo_net(files)             # the .net.xml read with sumolib
{"SUMO edges": len(sn.getEdges(withInternal=False)), "junctions": len(sn.getNodes()),
 "traffic lights": len(sn.getTrafficLights()), "duckOSM edges": int(net.driving.summary().edges[0])}    # the numbers""")
ch.md("""Sumolib reads 2840 SUMO edges and 1768 junctions: one SUMO edge for each duckOSM edge,
and one junction for each node. The traffic lights come from the OSM tag `highway=traffic_signals` (chapter 6).""")
ch.code("""e = sn.getEdges(withInternal=False)[0]                                     # one SUMO edge
{"id": e.getID(), "lanes": e.getLaneNumber(), "speed m/s": e.getSpeed(), "length m": round(e.getLength(), 1),
 "from": e.getFromNode().getID(), "to": e.getToNode().getID()}             # what SUMO knows of it""")
ch.md("""**What the output shows.** The id of the SUMO edge is an `edge_id`, and the ids of its two junctions are OSM node
ids. The edge has 1 lane, 47.3 m. The speed is 5.56 m/s (20 km/h), from the `maxspeed_kmh` of the edge.""")

ch.md("""## From the SUMO ids to the edge ids: `edge_map`

`duckosm.sim.edge_map(path)` gives a DataFrame with two columns: `id`, the id of each edge in the file (text), and
`edge_id`, its duckOSM `edge_id` (a number). duckOSM writes the `edge_id` as the SUMO id, so the two are the same
in a network of `to_sumo`. When a tool changes an id, `edge_map` still finds the `edge_id`:

| The id in the file | `edge_map` gives |
|---|---|
| `123` | one row: 123 |
| `123#1` (netconvert cut the edge 123) | one row: 123 |
| `123+456` (two edges joined) | two rows: 123 and 456 |
| `:junction_0` (an internal edge of a junction) | no row |

`path` can be a `.net.xml`, an `.edg.xml`, the dict of `to_sumo`, or a MATSim network (section "MATSim" below).""")
ch.code("""edges = duckosm.sim.edge_map(files)                         # the SUMO id and the edge_id of each SUMO edge
print(len(edges), "rows;", int((edges.id != edges.edge_id.astype(str)).sum()), "ids that netconvert changed")
edges.head(3)""")
ch.md("""`edge_map` gives 2840 rows, one for each SUMO edge, and netconvert changed no id. So
here `id` and `edge_id` are the same number, as text and as an integer.

> **Tip:** join the output of a simulation on `id`, then the network on `edge_id`. The next section does this.""")

ch.md("""## Run SUMO

SUMO needs the demand. For a test, SUMO's tool `randomTrips.py` makes random trips between random edges. This cell
makes one trip each second for 900 s (15 minutes), and asks SUMO to write an **edgeData** file: for each edge, the
number of vehicles that entered it, and their speed and time on it. Then `sumo` runs the trips for 30 minutes.

| Program | Arguments | What it does |
|---|---|---|
| `randomTrips.py` | `-n` the network, `-e 900`, `-p 1`, `--validate` | 900 random trips, one each second, each one with a possible route |
| `sumo` | `-n` the network, `-r` the trips, `-a` the edgeData file, `--end 1800` | the simulation, 30 minutes |

`sumo.SUMO_HOME` is the folder of the SUMO of the extra `sim`.""")
ch.code("""import subprocess, sys
import sumo                                                              # the SUMO of the extra sim

subprocess.run([sys.executable, f"{sumo.SUMO_HOME}/tools/randomTrips.py", "-n", str(files["net"]),
                "-o", "out/sumo/trips.xml", "-r", "out/sumo/routes.rou.xml", "-e", "900", "-p", "1", "--validate", "--seed", "1"],
               check=True, capture_output=True)                          # a trip each second, for 900 s
open("out/sumo/edgedata.add.xml", "w").write(
    '<additional><edgeData id="counts" file="edgedata.xml"/></additional>')    # the counts of each edge
subprocess.run([f"{sumo.SUMO_HOME}/bin/sumo", "-n", str(files["net"]), "-r", "out/sumo/trips.xml",
                "-a", "out/sumo/edgedata.add.xml", "--no-step-log", "--end", "1800", "--seed", "1"],
               check=True, capture_output=True).returncode               # the simulation, 30 minutes; 0: no error""")
ch.md("""The return code 0 tells that SUMO ran with no error. SUMO wrote
`out/sumo/edgedata.xml`, with one `<edge>` element for each SUMO edge.""")

ch.md("""## Join the counts back to the edges

`pandas.read_xml` reads the edgeData file: one row for each SUMO edge, with the column `entered` (the vehicles that
entered the edge). The merge on `id` with `edge_map` gives each row its `edge_id`. The sum by `edge_id` puts the parts
of an edge together when netconvert cut it.""")
ch.code("""import pandas as pd                                                      # to read the output of SUMO

counts = pd.read_xml("out/sumo/edgedata.xml", xpath=".//edge", parser="etree", dtype={"id": str})    # one row for each SUMO edge
cars = counts.merge(edges, on="id").groupby("edge_id").entered.sum()               # the cars of each edge_id
cars.describe()                                                                    # the counts""")
ch.md("""Each of the 2840 edges has a count. Half of the edges have 7 cars or fewer in the 30
minutes, and some have none. The busiest edge has 145 cars.""")
ch.code("""net.driving.map(color_by=cars)    # the edges colored by the number of cars (a picture here; live when you run it)""")
ch.md("""The main roads and the tunnels have the most cars (green and yellow): the random trips cross Monaco
on them. The small streets have few cars or none.""")
ch.code("""top = net.driving.edges(where=f"edge_id IN ({','.join(map(str, cars.nlargest(3).index))})")[["edge_id", "name", "highway"]]
top.assign(cars=top.edge_id.map(cars)).sort_values("cars", ascending=False)        # the three edges with the most cars""")
ch.md("""**What the output shows.** The busiest edges are on the Place d'Armes (145 cars), the roundabout Giratoire
Aureglia (126) and the Boulevard Albert 1er (126): primary and secondary roads. The names come from the duckOSM
edges, through the `edge_id`.""")

ch.md("""## MATSim

MATSim simulates agents: each agent has a plan for the day, and MATSim improves the plans over many iterations.
`net.driving.to_matsim(out_path)` (the same as `duckosm.sim.to_matsim`) writes MATSim's `network.xml`.

| Argument | Default | What it does |
|---|---|---|
| `mode` | the mode of the view | one mode, or a list of modes (one network, each link with its modes) |
| `crs` | the UTM zone of the area | the projected CRS of the coordinates (MATSim needs metres) |
| `gzip` | `True` | write `network.xml.gz` |

The id of each MATSim link is the `edge_id` too, so `edge_map` reads the MATSim network in the same way: give it the
`network.xml` or `network.xml.gz`.""")
ch.code("""stats = net.driving.to_matsim("out/matsim_network.xml.gz")              # the MATSim network
links = duckosm.sim.edge_map("out/matsim_network.xml.gz")              # the link id and the edge_id of each link
print(stats)                                                           # the counts and the CRS
len(links), set(links.edge_id) == set(net.driving.edges().edge_id)             # one link for each edge""")
ch.md("""**What the output shows.** The MATSim network has 1768 nodes and 2840 links, in the CRS EPSG:32632 (UTM zone
32N, the zone of Monaco). `edge_map` reads 2840 links, and their ids are the edge ids of the network.""")

ch.md("""### MATSim lanes and signals: `to_matsim_lanes`

MATSim can also simulate lanes and traffic signals. `duckosm.sim.to_matsim_lanes(gmns_db, out_dir)` writes MATSim's
`lanes.xml` and the signal files (`signalSystems.xml`, `signalGroups.xml`, `signalControl.xml`). It reads the lanes
and the movements of a GMNS database, which `to_gmns` makes (chapter 13 tells more about GMNS).

| Argument | Default | What it does |
|---|---|---|
| `mode` | `"driving"` | the mode of the GMNS database |
| `signals` | `True` | also the signal files |
| `cycle_s` | `90` | the cycle time of each signal, in seconds |""")
ch.code("""import os
net.driving.to_gmns("out/gmns.duckdb")                                     # the GMNS tables (chapter 13)
lanes = duckosm.sim.to_matsim_lanes("out/gmns.duckdb", "out/matsim")       # lanes.xml and the signal files
print(lanes)
sorted(os.listdir("out/matsim"))                                           # the files""")
ch.md("""**What the output shows.** The GMNS export takes most of the time of the cell, because it runs netconvert for
the lanes. `lanes.xml` has lanes for 2832 links, and the signal files have one signal system: the one signal of the
GMNS tables of Monaco.""")

ch.exercises([
    ("Make the SUMO network with no `.con.xml` (`connections=False`), and count the files.",
     """f = net.driving.to_sumo("out/sumo_nocon", connections=False, run_netconvert=True)
sorted(k for k in f if not k.startswith("n_"))"""),
    ("Find the five edges with the highest mean speed in the SUMO run. The edgeData column `speed` is in m/s.",
     """speed = counts.merge(edges, on="id").groupby("edge_id").speed.mean()
speed.nlargest(5) * 3.6                                                     # km/h"""),
    ("Write one MATSim network for driving and walking. Build a network with the two modes first.",
     """both = duckosm.open("monaco.duckdb")
duckosm.sim.to_matsim(both, "out/matsim_dw.xml.gz", mode=["driving", "walking"])"""),
])

ch.glossary([
    ("traffic simulation", "a program that moves vehicles one by one on a road network"),
    ("demand", "the trips of a simulation: start, end and time"),
    ("SUMO", "Simulation of Urban MObility, a microscopic traffic simulator"),
    ("netconvert", "the SUMO program that makes a `.net.xml` from the plain XML files"),
    ("plain XML", "SUMO's simple files of nodes, edges and connections"),
    ("connection", "a turn from one lane of an edge to a lane of the next edge"),
    ("internal edge", "a short SUMO edge inside a junction, for one turn"),
    ("edgeData", "a SUMO output with the counts and speeds of each edge"),
    ("MATSim", "an agent-based simulator of daily plans"),
    ("sumolib", "SUMO's Python library to read and use its files"),
])

ch.summary([
    ("`net.driving.to_sumo(out_dir, ...)`, `duckosm.sim.to_sumo(...)`", "the SUMO network, with the `edge_id` as the id of each SUMO edge"),
    ("`duckosm.sim.DEFAULT_NETCFG`", "the netconvert options of `to_sumo`"),
    ("`duckosm.sim.sumo_net(files)`", "the `.net.xml` read with sumolib"),
    ("`duckosm.sim.edge_map(path)`", "the SUMO or MATSim id of each edge or link, and its `edge_id`"),
    ("`net.driving.to_matsim(out_path)`, `duckosm.sim.to_matsim(...)`", "the MATSim network"),
    ("`duckosm.sim.to_matsim_lanes(gmns_db, out_dir)`", "MATSim lanes and signals, from a GMNS database"),
], reference="[duckosm.sim](../reference/sdk/sim.md)")

ch.write(sys.argv[1])
