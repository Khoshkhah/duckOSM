# SUMO

```bash
pip install "duckosm[sumo]"          # brings SUMO's netconvert
duckosm sumo monaco.duckdb           # -> sumo/monaco.net.xml
```

duckOSM writes the network in SUMO's plain-XML format and runs SUMO's own `netconvert` to build the
`.net.xml`. Every SUMO edge id is the duckOSM `edge_id`, and each junction allows only the turns in
`edge_graph`, so turn restrictions are kept.

| Option | Does |
|---|---|
| `--no-netconvert` | only write the plain-XML inputs (`.nod`, `.edg`, `.con` and a `.netccfg`) |
| `--no-connections` | let netconvert work out the turns itself |
| `-c my.netccfg` | use your own netconvert settings |
| `-m walking` | another mode (default `driving`) |
| `--out-dir`, `--name` | output folder (default `sumo`) and file name (default: the db's name) |

```python
from duckosm import to_sumo
out = to_sumo(con, "sumo/")                                  # -> sumo/network.net.xml
to_sumo(con, "sumo/", config={"junctions.join": "true"})     # change one netconvert option
```
