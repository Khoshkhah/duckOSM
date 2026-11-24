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
duckosm matsim monaco_gmns.duckdb              # from the GMNS db: the same links as the lanes file
```

The input is a Level 2 db or a GMNS db. From a GMNS db, the links are the GMNS `link` rows (`link_id` = `edge_id`), with
the GMNS `capacity` and `free_speed`, and the bus-only links are in the network with the mode `bus`. Then `network.xml`
and `lanes.xml` come from one database and have the same links.

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

## Fewer links (`--merge`) { #fewer-links-merge }

duckOSM cuts roads where footways and cycleways cross them, so that the modes meet. A MATSim network of cars only does not
need these cuts. `--merge` (`to_matsim(..., merge=True)`) writes one link for each chain of
[`driving.chains`](../reference/database.md#chains) (a GMNS db: [`gmns_driving.chains`](gmns_tables.md#chains)): the edges of
one road between two real junctions. On Monaco, the 2,840 links of the routable network become 1,787, and the 2,858 links of the GMNS db
become 1,812.

```bash
duckosm matsim monaco_gmns.duckdb --merge                  # one link for each chain
duckosm matsim-lanes monaco_gmns.duckdb --merge            # lanes.xml and signals on those links
```

* **Ids:** a merged link has the id `chain_id`, and lists its edges in its attribute `edge_ids`. `duckosm.sim.edge_map` reads
  it, so the results still join on `edge_id`. A chain of one edge keeps the `edge_id`.
* **Lanes:** give `--merge` to `matsim-lanes` too, so that `lanes.xml` names the same links.
* **Scope:** the driving network only (`-m driving`).

## Lanes and signals

`network.xml` alone has no turns: MATSim lets a car take each link at a junction, banned turns too. For the legal turns,
make the lanes and signals files from the GMNS db (`<area>_gmns.duckdb`), and make `network.xml` from the same db:

```bash
duckosm gmns monaco.duckdb                     # creates monaco_gmns.duckdb with lanes and turns
duckosm matsim monaco_gmns.duckdb              # -> monaco_gmns_network.xml.gz
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
