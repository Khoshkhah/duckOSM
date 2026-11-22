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
                   "edges; a closed street with traci, and where its traffic goes; the MATSim network, and MATSim lanes and signals",
             time="about 3 minutes", extras="sim,viz", more_install="\n    %pip install -q traci",
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
network has 1768 nodes and 2840 edges, the same numbers as the driving network, and 5082 connections. The nodes
have 18 traffic lights and 12 stop signs, and `network.add.xml` has 106 bus stops. `net` is the path of
`network.net.xml`, the file that SUMO reads; `tll` has the signal programs that netconvert made.""")

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
ch.md("""With `geometry.remove = true`, only 1629 of the 2840 edges keep their `edge_id` as a
SUMO edge. netconvert joined the other 1211 edges into their neighbors.

> **Warning:** keep `geometry.remove` set to `false`. With `true`, netconvert joins the edges between two junctions
> into one SUMO edge, and keeps only the id of the first edge. The other edges then have no SUMO edge, and their
> results are lost.""")
ch.md("""### Fewer SUMO edges, with all the edge ids: `merge=True`

duckOSM cuts a road at each node that another network uses, for example a footway crossing, because the multimodal
routing needs the cut. SUMO does not need these cuts. The table `driving.chains` joins the edges where the road only
continues, and `merge=True` writes one SUMO edge for each chain:

```python
chained = net.driving.to_sumo("out/sumo_chains", merge=True, run_netconvert=True)
duckosm.sim.edge_map(chained)    # each SUMO edge, with every edge_id of its chain
```

On Monaco, the 2840 edges become 1775 SUMO edges, and the junctions go from 2995 to 1592. All 18 traffic lights stay.
Each joined edge has `<param key="edge_ids">`, so `edge_map` gives back every `edge_id`, and no result is lost.""")


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
and one junction for each node. The 15 traffic lights are the programs of the 18 traffic-light junctions (chapter 6): the
lights within 20 m of each other share one program.""")
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
minutes, and some have none. The busiest edge has 144 cars.""")
ch.code("""net.driving.map(color_by=cars)    # the edges colored by the number of cars (a picture here; live when you run it)""")
ch.md("""The main roads and the tunnels have the most cars (green and yellow): the random trips cross Monaco
on them. The small streets have few cars or none.""")
ch.code("""top = net.driving.edges(where=f"edge_id IN ({','.join(map(str, cars.nlargest(3).index))})")[["edge_id", "name", "highway"]]
top.assign(cars=top.edge_id.map(cars)).sort_values("cars", ascending=False)        # the three edges with the most cars""")
ch.md("""**What the output shows.** The busiest edge is on the Place d'Armes (144 cars). The next two are two edges of
the Boulevard Albert 1er (128 cars each). These are secondary roads. The names come from the duckOSM
edges, through the `edge_id`.""")

ch.md("""## Control the simulation with traci

The run above starts SUMO and reads its file at the end. [traci](https://sumo.dlr.de/docs/TraCI.html) is SUMO's
Python interface: Python starts SUMO, moves it one second at a time, reads values while it runs, and changes the
network. The `edge_id` is the key in both directions: it names the street to change in SUMO, and it brings the SUMO
values back to the duckOSM edges.

The question here: when the busiest street is closed after 10 minutes, where does its traffic go? The steps:

1. Ask SUMO to count the vehicles on each edge in the time window from 600 s to 1200 s (an edgeData file with
   `begin` and `end`).
2. Run the same trips one time with no change. This is the baseline.
3. Run them again with traci. At 600 s, close the busiest edge to cars (`traci.edge.setDisallowed`), and send the
   vehicles on the road to a new route (`traci.vehicle.rerouteTraveltime`). Every 10 s, read the number of vehicles in
   the network. At 900 s, read the position of each vehicle.
4. Compare the two runs in the same window, edge by edge.

`--ignore-route-errors` lets SUMO go on when a trip has no route after the closure (SUMO then drops the trip).""")
ch.code("""closed = int(cars.idxmax())                                              # the busiest edge of the run above
closed_sumo = edges.id[edges.edge_id == closed].tolist()                 # its SUMO id (or ids, when netconvert cut it)
window = '<additional><edgeData id="w" file="{}" begin="600" end="1200"/></additional>'    # 1. the window 600-1200 s
open("out/sumo/base.add.xml", "w").write(window.format("base.xml"))
open("out/sumo/closed.add.xml", "w").write(window.format("closed.xml"))
run = [f"{sumo.SUMO_HOME}/bin/sumo", "-n", str(files["net"]), "-r", "out/sumo/trips.xml",
       "--no-step-log", "--no-warnings", "--ignore-route-errors", "--seed", "1", "--end", "1800"]    # the same trips
subprocess.run(run + ["-a", "out/sumo/base.add.xml"], check=True, capture_output=True).returncode    # 2. the baseline""")
ch.code("""import traci

traci.start(run + ["-a", "out/sumo/closed.add.xml",                      # 3. the same run, controlled by Python
                   "--fcd-output", "out/sumo/fcd.parquet", "--fcd-output.geo", "true"])    # and each vehicle each second
in_network, positions = [], None
for t in range(1, 1801):
    traci.simulationStep()                                               # one second of the simulation
    if t == 600:                                                         # close the edge to cars, and reroute
        for sid in closed_sumo:
            traci.edge.setDisallowed(sid, ["passenger"])
        for vehicle in traci.vehicle.getIDList():
            traci.vehicle.rerouteTraveltime(vehicle)
    if t % 10 == 0:
        in_network.append((t, traci.vehicle.getIDCount()))               # the vehicles in the network
    if t == 900:                                                         # the position of each vehicle, as lon, lat
        positions = pd.DataFrame([traci.simulation.convertGeo(*traci.vehicle.getPosition(v))
                                  for v in traci.vehicle.getIDList()], columns=["x", "y"])
traci.close()
in_network = pd.DataFrame(in_network, columns=["t", "vehicles"])
net.driving.edges(where=f"edge_id = {closed}")[["edge_id", "name"]], len(positions)    # the closed edge, the vehicles at 900 s""")
ch.md("""The closed edge is the Place d'Armes, the busiest edge of the first run. At 900 s, 255 vehicles are in the
network. traci prints "Retrying in 1 seconds" while it connects to SUMO.""")
ch.code("""import matplotlib.pyplot as plt

fig, ax = plt.subplots(figsize=(8, 3))
ax.plot(in_network.t / 60, in_network.vehicles, color="#1f6feb")       # the vehicles in the network
ax.axvline(10, color="#d62728", linestyle="--", label="the edge closes")    # 600 s
ax.set_xlabel("minutes"); ax.set_ylabel("vehicles in the network"); ax.legend()
plt.show()""")
ch.md("""The trips start in the first 15 minutes, so the network fills up to about 250 vehicles. The closure at
10 minutes does not change this total much: the vehicles take other routes. After 15 minutes no new trip starts, and
the network is empty after about 25 minutes.""")
ch.code("""fig, ax = plt.subplots(figsize=(8, 8))
net.driving.edges().plot(ax=ax, color="lightgray", linewidth=0.6)       # the roads in light gray
ax.scatter(positions.x, positions.y, s=8, color="#d62728", label="vehicle at 900 s")    # the vehicles
spot = net.driving.edges(where=f"edge_id = {closed}").geometry.iloc[0].centroid           # the closed edge
ax.scatter([spot.x], [spot.y], s=400, facecolors="none", edgecolors="black", linewidths=2, label="closed edge")
ax.legend(title="SUMO at 15 minutes"); ax.set_axis_off()
plt.show()""")
ch.md("""Each red dot is one vehicle at 15 minutes. traci gives its position in SUMO's coordinates, and
`traci.simulation.convertGeo` turns it into longitude and latitude. The black ring is the closed edge. The same run
also wrote every vehicle at every second to `out/sumo/fcd.parquet` (SUMO's FCD output). A later version of this manual
can play this file as moving vehicles on the live map, with roadstyle's vehicle player.""")
ch.code("""def entered(path):                                                       # the vehicles that entered each edge_id
    rows = pd.read_xml(path, xpath=".//edge", parser="etree", dtype={"id": str})
    return rows.merge(edges, on="id").groupby("edge_id").entered.sum()


change = entered("out/sumo/closed.xml").sub(entered("out/sumo/base.xml"), fill_value=0)    # 4. closed run - baseline
streets = net.driving.edges().set_index("edge_id").name
pd.DataFrame({"name": streets.reindex(change.index), "change": change}).sort_values("change").iloc[[0, 1, 2, -3, -2, -1]]""")
ch.md("""In the window from 10 to 20 minutes, 72 fewer vehicles enter the Place d'Armes than in the baseline, and 52
fewer enter two edges of the Avenue Prince Pierre, the street that leads to the square. The Avenue d'Ostende and the
Avenue Princesse Alice each get 36 more: the vehicles go around the closed square on them.""")
ch.code("""from matplotlib.colors import TwoSlopeNorm

roads = net.driving.edges()
moved = roads.merge(change[change != 0].rename("change"), left_on="edge_id", right_index=True)    # the edges that changed
fig, ax = plt.subplots(figsize=(8, 8))
roads.plot(ax=ax, color="lightgray", linewidth=0.6)                      # all the roads in light gray
moved.plot(ax=ax, column="change", cmap="RdBu_r", norm=TwoSlopeNorm(0), linewidth=3, legend=True,
           legend_kwds={"label": "vehicles: closed run - baseline", "shrink": 0.6})    # blue: fewer, red: more
ax.set_axis_off()
plt.show()""")
ch.md("""Blue streets have fewer vehicles than in the baseline, and red streets have more. Gray streets have no change.
The blue streets lead to the closed square. The red streets, along the port and up to Monte Carlo, are the new routes
around it.""")

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
