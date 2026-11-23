# MATSim

`duckosm matsim` writes a MATSim `network.xml`, with your OSM fixes. MATSim, BEAM and eqasim read it. Each MATSim link
id is the duckOSM `edge_id`.

| Reference and format | |
|---|---|
| Format | MATSim [network_v2](https://www.matsim.org/files/dtd/network_v2.dtd). Lanes and signals: [laneDefinitions_v2.0](https://www.matsim.org/files/dtd/laneDefinitions_v2.0.xsd) and the signal files (signalSystems, signalGroups, signalControl v2.0) |
| Reference tool | none: no test loads the files in MATSim yet |
| Check | each file is valid against its DTD or XSD schema. [How each output is checked](../concepts/checks.md) |

```bash
duckosm matsim monaco.duckdb                   # -> monaco_network.xml.gz, driving
duckosm matsim monaco.duckdb -m all            # all modes in one network
```

## The links

Each edge becomes one directed link. A road that several modes use is one link with each mode (`modes="car,bike,walk"`),
so a trip can change mode with no transfer links.

| Link attribute | From duckOSM | Description & MATSim Role |
|---|---|---|
| `id` | `edge_id` | Unique link identifier. Matches DuckDB network tables for direct join of simulation results. |
| `from`, `to` | `source`, `target` | Upstream and downstream topological node IDs in UTM coordinates. |
| `length` | `length_m` | Physical length in metres. Governs free-flow travel time and QSim queue storage capacity. |
| `freespeed` | `maxspeed_kmh` / 3.6 | Free-flow speed in m/s. Maximum travel speed on an uncontested link. |
| `permlanes` | `lanes` per direction | Number of travel lanes (1 if omitted in OSM, or inferred from road class). |
| `capacity` | `lane_capacity` × `permlanes` | Maximum saturation outflow (veh/h). Regulates queue discharge in MATSim's QSim. |
| `modes` | `car`, `bike`, `walk` | Permitted modes (`modes="car,bike,walk"`). Allows multi-modal trips without transfer links. |

The node coordinates are in metres, in the UTM zone of the data (`--crs` gives another). After
[`duckosm elevation`](../guides/elevation.md), each node also has a `z`.

## Lanes and signals

`network.xml` alone has no turns: MATSim lets a car take each link at a junction, banned turns too. For the legal turns,
make the lanes and signals files from the GMNS db (`<area>_gmns.duckdb`). `network.xml` comes from the Level 2 db:

```bash
duckosm gmns monaco.duckdb                     # creates monaco_gmns.duckdb with lanes and turns
duckosm matsim-lanes monaco_gmns.duckdb        # -> lanes.xml and the signal files
```

| File | Holds |
|---|---|
| `lanes.xml` | the lanes of each link, and the links that each lane leads to: only legal turns. With `turn:lanes` in OSM, each lane has its own turns |
| signal files | one signal system for each junction with traffic lights. The timing is a fixed cycle in two phases, because OSM has no signal plans: calibrate it |

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB). France is the driving network only.

| Area (input DuckDB) | Time | Memory | Output |
|---|---|---|---|
| Monaco (38 MB `.duckdb`) | 0.04 s | 18.6 MB | 0.1 MB |
| Kentucky (~1.2 GB `.duckdb`) | 3.4 s | 853 MB | 84.1 MB |
| France (~35 GB `.duckdb`) | 46 s | 10.0 GB | 923 MB |

## Limits

* **Scenario files:** Travel demand, agent plans (`plans.xml`), and simulation config (`config.xml`) are defined by your scenario.
* **Signal calibration:** Signal definitions use standard cycles and should be calibrated to your study intersection demand.

All options: [Command line](../reference/cli.md#matsim).
