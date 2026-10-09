# SUMO

```bash
pip install "duckosm[sumo]"          # brings SUMO's netconvert
duckosm sumo monaco.duckdb           # -> sumo/monaco.net.xml
```

duckOSM writes the network in SUMO's plain-XML format and runs SUMO's own `netconvert` to build the
`.net.xml`, in metres in the UTM zone of the data. Every SUMO edge id is the duckOSM `edge_id`, and
each junction allows only the turns in `edge_graph`, so turn restrictions are kept. netconvert
leaves out a few U-turns it can't build (Monaco: 37 of 1,628); it never adds a turn.

A walking network is for pedestrians only (`allow="pedestrian"`, with SUMO walking areas at the
junctions), a cycling network for bicycles only. A driving network allows every vehicle class. A
walking or cycling edge is one path lane, 2 m or 1.5 m wide (not the road's lane count at 3.2 m).
A one-way road's lanes are centred on its line (`spreadType="center"`); the two directions of a two-way road lie
either side of it.

| Option | Does |
|---|---|
| `--no-netconvert` | only write the plain-XML inputs (`.nod.xml`, `.edg.xml`, `.con.xml`) |
| `--no-connections` | let netconvert work out the turns itself |
| `-c my.netccfg` | your own netconvert settings instead of duckOSM's (it still sets the input and output files) |
| `-m walking` | another mode (default `driving`) |
| `--out-dir`, `--name` | output folder (default `sumo`) and file name (default: the db's name) |

```python
import duckdb
from duckosm import to_sumo
con = duckdb.connect("monaco.duckdb", read_only=True)
out = to_sumo(con, "sumo/")                                  # out["net"] = "sumo/network.net.xml"
to_sumo(con, "sumo/", config={"junctions.join": "true"})     # change one netconvert option
to_sumo(con, "sumo/", edge_attrs={eid: {"numLanes": 2, "width": 3.1}})   # your own lanes and widths per edge (id as number or text)
```

**Joining close junctions** (`junctions.join`, off by default). A crossroads mapped as two or three close nodes becomes one
junction. netconvert reads the turns (`.con.xml`) before it joins, and silently drops every turn onto an edge the join swallows (the
short edge between two joined nodes): a road whose straight or left turn ran over that edge lost it. So when joining is on, duckOSM
asks netconvert which nodes it joins (a first run with `--junctions.join-output`), merges those nodes itself (`cluster_<ids>`, as
netconvert names them), writes every legal path through the swallowed edges as one turn (never back through a node it passed) and
runs netconvert again without joining. Turn restrictions still hold: a path is legal only if each of its steps is in `edge_graph`.
