# MATSim

`duckosm matsim` writes a MATSim `network.xml` (the `network_v2` format). MATSim, BEAM and eqasim read it.
Each MATSim link id is the duckOSM `edge_id`.

| Reference and format | |
|---|---|
| Format | MATSim [network_v2 DTD](https://www.matsim.org/files/dtd/network_v2.dtd); lanes and signals: [laneDefinitions_v2.0](https://www.matsim.org/files/dtd/laneDefinitions_v2.0.xsd), [signalSystems_v2.0](https://www.matsim.org/files/dtd/signalSystems_v2.0.xsd), [signalGroups_v2.0](https://www.matsim.org/files/dtd/signalGroups_v2.0.xsd) and [signalControl_v2.0](https://www.matsim.org/files/dtd/signalControl_v2.0.xsd) XSD |
| Reference tool | none: no test loads the files in MATSim |
| Check | `network.xml` is valid against the DTD; the lanes and signal files are valid against the XSD schemas. [How each output is checked](../concepts/checks.md) |

```bash
duckosm matsim monaco.duckdb           # -> monaco_network.xml.gz
```

Monaco test build, driving: 1,769 nodes, 2,844 links.

## What it makes

Each duckOSM edge becomes one directed link:

| Link attribute | From |
|---|---|
| `id` | `edge_id` |
| `from`, `to` | `source`, `target` (node ids) |
| `length` | `length_m` |
| `freespeed` | the speed in m/s (`maxspeed_kmh` / 3.6) |
| `permlanes` | `lanes` in this direction (1 if empty) |
| `capacity` | a value for each lane of the road class × `permlanes`, in vehicles/hour (motorway 2,000 … service 300; other classes 800) |
| `modes` | `car`, `bike` or `walk` |

Node coordinates are in meters, in the UTM zone of the data. The file attributes give the CRS. After
[`duckosm elevation`](../guides/elevation.md), each node also has a `z`.

## Options

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode` | a mode, a comma list (`driving,cycling`) or `all` | `driving` |
| `--crs` | the projected CRS of the node coordinates, for example `EPSG:3006` | the UTM zone of the data |
| `--no-gzip` | write plain `.xml` | gzip |
| `-o`, `--out` | the output file | `<name>_network.xml.gz` |

```python
from duckosm import to_matsim
to_matsim("monaco.duckdb", "network.xml.gz", mode="driving")    # -> {"nodes", "links", "crs"}
```

## All modes in one network

```bash
duckosm matsim monaco.duckdb -m all -o network.xml.gz      # Monaco test build: 4,750 nodes, 13,291 links
```

A road that several modes use is one link with each mode on it (`modes="car,bike,walk"`), because its `edge_id` is the
same in each network. MATSim routes each leg of a trip on the links that allow its mode. The links are shared, so you
do not need transfer links.

## Lanes and signals

```bash
duckosm gmns monaco.duckdb                  # first: the lanes and the turns
duckosm matsim-lanes monaco_gmns.duckdb     # -> ./lanes.xml, signalSystems.xml, signalGroups.xml, signalControl.xml
```

`matsim-lanes` reads a [GMNS database](gmns.md). Its link ids are the same `edge_id`s, so these files go with the
`network.xml` of the same build.

| File | Holds |
|---|---|
| `lanes.xml` | for each link, its lanes and the links each lane leads to: only legal turns. A U-turn on the same road only at a dead end. Where OSM has `turn:lanes`, each lane gets its own turns; elsewhere each lane can take each legal turn |
| signal files | one signal system for each junction with traffic lights in OSM (Monaco test build: 1) |

**The signal timing is a placeholder**: a fixed cycle in two phases, because OSM has no signal plans. Calibrate it.

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode` | the GMNS mode | `driving` |
| `--cycle` | the signal cycle, in seconds | 90 |
| `--no-signals` | do not write the signal files | writes them |
| `-o`, `--out` | the output folder | `.` |

Design: [matsim_export](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/matsim_export.md),
[matsim_lanes_signals](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/matsim_lanes_signals.md).

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB). Time: the wall-clock time. Memory: the peak memory footprint (`/usr/bin/time -l`). 1 MB = 10⁶ bytes.

| Area | Time | Memory | Output size |
|---|---|---|---|
| Monaco (0.7 MB) | 0.04 s | 18.6 MB | 0.1 MB |
| Kentucky (148 MB) | 3.4 s | 853 MB | 84.1 MB |
| France (5.1 GB) | 46 s | 10.0 GB | 923 MB |

The command: `duckosm matsim area.duckdb`. The input is the driving network with the boundary of the area: `duckosm build` (Monaco, Kentucky) and `duckosm tiles` (France, 8.8 GB DuckDB file).

## Limits

- `network.xml` alone has no turns: MATSim lets a car take each link at a junction, banned turns too. Add `lanes.xml`
  for legal turns only.
- `freespeed` and `capacity` come from the defaults of the road class where OSM has no speed limit or lane count.
- No demand, plans, transit or `config.xml`: these are your scenario.
