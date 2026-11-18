# SUMO

`duckosm sumo` writes a network for [SUMO](https://eclipse.dev/sumo/). Each SUMO edge id is the duckOSM `edge_id`.

| Reference and format | |
|---|---|
| Format | SUMO [plain XML](https://sumo.dlr.de/docs/Networks/PlainXML.html) (`.nod.xml`, `.edg.xml`, `.con.xml`) and the [SUMO network](https://sumo.dlr.de/docs/Networks/SUMO_Road_Networks.html) (`.net.xml`) |
| Reference tool | SUMO's [netconvert](https://sumo.dlr.de/docs/netconvert.html) |
| Check | netconvert reads the plain XML and keeps each edge id and each connection. [How each output is checked](../concepts/checks.md) |

```bash
pip install "duckosm[sumo]"          # adds SUMO's netconvert
duckosm sumo monaco.duckdb           # -> sumo/monaco.net.xml
```

## What it makes

duckOSM writes the network in SUMO's plain-XML format (`.nod.xml`, `.edg.xml`, `.con.xml`) and a `.netccfg`.
Then it runs SUMO's `netconvert`, which makes the `.net.xml`. The coordinates are in meters, in the UTM zone of the data.

| SUMO item | From duckOSM |
|---|---|
| edge id | `edge_id` |
| connections (turns) | `edge_graph`: only legal turns, so the turn restrictions stay |
| lanes (driving) | `driving.lane_profile`: one SUMO lane for each lane, with its width and vehicle classes, numbered from the right |
| lane classes | bus lane `bus`, bike lane `bicycle`, shared lane `bus bicycle`, car lane: each road vehicle |
| walking network | `allow="pedestrian"`, one path lane 2 m wide, SUMO walking areas at the junctions |
| cycling network | bicycles only, one path lane 1.5 m wide |
| one-way road | its lanes are centered on its line (`spreadType="center"`) |
| two-way road | each direction is on one side of the line |

netconvert does not build some U-turns. It never adds a turn.

## Options

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode` | the mode | `driving` |
| `--out-dir` | the output folder | `sumo` |
| `--name` | the file name | the name of the database |
| `--no-netconvert` | write only the plain XML and the `.netccfg` | runs netconvert |
| `--no-connections` | let netconvert make the turns (the turn restrictions are lost) | turns from `edge_graph` |
| `-c`, `--config` | your `.netccfg` in place of duckOSM's netconvert options | duckOSM's options |

If netconvert is not found, the command stops. Install SUMO (`pip install "duckosm[sumo]"` or a system package), set
`SUMO_HOME`, or use `--no-netconvert`.

## Python

```python
import duckdb
from duckosm import to_sumo
con = duckdb.connect("monaco.duckdb", read_only=True)
out = to_sumo(con, "sumo/")                                          # out["net"]: sumo/network.net.xml
to_sumo(con, "sumo/", config={"junctions.join": "true"})             # change one netconvert option
to_sumo(con, "sumo/", edge_attrs={eid: {"numLanes": 2, "width": 3.1}})   # your lanes and widths for an edge
```

**Join close junctions** (`junctions.join`, off by default). A crossroads mapped as two or three close nodes becomes one
junction, and the legal turns through it stay. Only the function `to_sumo(config={"junctions.join": "true"})` does
this. The command has no option for it. How it works: [design](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/sumo_export.md).

**Compare with SUMO's OSM import.** `duckosm.sumo_compare.lane_diff(db, osm_file, work_dir)` builds the same area with
SUMO's own OSM import. It lists each lane difference for each OSM way and direction, with its cause.

## Limits

- No painted turn arrows (`turn:lanes`): SUMO's plain XML has no field for them. The [GMNS](gmns.md) export uses them.
- No demand, routes or SUMO configuration (`.sumocfg`): these are your scenario.
