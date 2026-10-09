# SUMO

```bash
pip install "duckosm[sumo]"          # brings SUMO's netconvert
duckosm sumo monaco.duckdb           # -> sumo/monaco.net.xml
```

duckOSM writes the network in SUMO's plain-XML format and runs SUMO's own `netconvert` to build the
`.net.xml`, in metres in the UTM zone of the data. Every SUMO edge id is the duckOSM `edge_id`, and
each junction allows only the turns in `edge_graph`, so turn restrictions are kept. netconvert
leaves out a few U-turns it can't build (Monaco: 37 of 1,628); it never adds a turn.

In a driving network each edge's lanes come from `driving.lane_profile` ([design](../design/gmns_lane_profile.md)): one SUMO
lane per lane, with its vehicle classes (a bus lane `bus`, a bike lane `bicycle`, a shared one `bus bicycle`, a car lane every
road vehicle) and its width, numbered from the right as SUMO does. `duckosm.sumo_compare.lane_diff(db, osm_file, work_dir)`
builds the same area with SUMO's own OSM import too and lists, per OSM way and direction, every lane difference with its cause
(an override, the contraflow bus lane, a default where no `lanes` tag is ...).

A walking network is for pedestrians only (`allow="pedestrian"`, with SUMO walking areas at the
junctions), a cycling network for bicycles only. A driving network allows every vehicle class.

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
```
