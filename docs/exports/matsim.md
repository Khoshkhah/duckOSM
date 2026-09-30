# MATSim

```bash
duckosm matsim monaco.duckdb           # -> monaco_network.xml.gz: 1,246 nodes, 2,133 links
```

A MATSim `network.xml` (the `network_v2` format), which MATSim, BEAM and eqasim read. Each duckOSM
edge becomes one directed link:

| Link attribute | From |
|---|---|
| `id` | `edge_id` |
| `from`, `to` | `source`, `target` (node ids) |
| `length` | `length_m` |
| `freespeed` | `maxspeed_kmh` / 3.6 (m/s); without a speed limit, from the edge's travel time |
| `permlanes` | `lanes` (per direction) |
| `capacity` | a per-lane value for the road class × `permlanes` (veh/h; motorway 2,000 … service 300) |
| `modes` | `car`, `bike` or `walk` |

Node coordinates are in metres, in the UTM zone of the data (written in the file's attributes);
`--crs` picks another.

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode` | a mode, a comma list (`driving,cycling`) or `all` | `driving` |
| `--crs` | the projected CRS for node coordinates | UTM zone of the data |
| `--no-gzip` | write plain `.xml` | gzipped |
| `-o`, `--out` | output file | `<name>_network.xml.gz` |

```python
from duckosm import to_matsim
to_matsim("monaco.duckdb", "network.xml.gz", mode="driving")    # -> {"nodes", "links", "crs"}
```

## All modes in one network

```bash
duckosm matsim monaco.duckdb -m all -o network.xml.gz      # 4,053 nodes, 10,999 links
```

A road used by several modes is one link with every mode on it (`modes="car,bike,walk"`), since
its `edge_id` is the same in each network. MATSim changes mode wherever the links meet, so no
transfer links are needed.

## Lanes and signals

```bash
duckosm gmns monaco.duckdb                  # the lanes and turns first
duckosm matsim-lanes monaco_gmns.duckdb     # -> lanes.xml + signalSystems / signalGroups / signalControl.xml
```

Reads a [GMNS database](gmns.md). The link ids are the same `edge_id`s, so these files go with the
`network.xml` from the same build.

- **`lanes.xml`**: for each link, its lanes and which links each lane leads to, so only legal turns
  are possible. Where OSM tags `turn:lanes`, each lane gets its own turns; elsewhere every lane
  may take every legal turn.
- **Signals**: one signal system per junction with traffic lights in OSM (Monaco: 1). **The timing
  is a placeholder**, a fixed 90 s cycle in two phases, since OSM has no signal plans: calibrate it.
  `--cycle` sets the cycle length, `--no-signals` skips the signal files.

## Limits

- `freespeed` and `capacity` come from class defaults where OSM has no speed limit or lane count.
- No demand, plans, transit or `config.xml`: those are your scenario.
- Checked against MATSim's official DTD and XSD schemas (in the tests).
