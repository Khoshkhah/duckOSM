# 3. GMNS network

The multi-resolution network in the GMNS standard, from macro (links) to micro (lanes, movements, and cells), for simulation and traffic assignment.

```mermaid
flowchart LR
  l2[("#lt;area#gt;.duckdb<br/>(Level 2)")] --> gmns[("#lt;area#gt;_gmns.duckdb<br/>(GMNS)")]
  gmns --> sumo["SUMO"]
  gmns --> matsim["MATSim network,<br/>lanes and signals"]
  gmns --> dta["DTALite & Path4GMNS"]
  gmns --> hd["HD map: Lanelet2,<br/>CommonRoad"]
```

| Read by | Export | Reads | Command |
|---|---|---|---|
| DTALite, Path4GMNS | [GMNS](../gmns.md): macro tables ([all GMNS tables](../gmns_tables.md)) | Level 2 db | `duckosm gmns` |
| Mesoscopic and microscopic simulators | [GMNS meso and micro](../gmns.md#meso-and-micro-networks): road sections and lane cells | Level 2 db | `duckosm gmns --meso --micro` |
| MATSim, BEAM, eqasim | [MATSim](../matsim.md): `network.xml` | GMNS db (or Level 2 db) | `duckosm matsim` |
| MATSim | [MATSim](../matsim.md#lanes-and-signals): lanes and signals | GMNS db | `duckosm matsim-lanes` |
| SUMO | [SUMO](../sumo.md): plain XML and `.netccfg` (netconvert makes the `.net.xml`) | GMNS db (or Level 2 db) | `duckosm sumo` |

Every table and column of the GMNS database: [GMNS network tables](../gmns_tables.md).

## Multi-resolution model

Level 3 takes the routable network from Level 2 and builds a [GMNS (General Modeling Network Specification)](https://zephyr-data-specs.github.io/GMNS/) model in DuckDB.

GMNS has three scales:

* **Macro (`duckosm gmns`):** links, nodes, and turning movements with lane attributes. For traffic assignment, routing, and transport planning in DTALite and Path4GMNS.
* **Meso (`--meso`):** road sections and junction lane connectors. For mesoscopic simulators that move groups of vehicles through junctions.
* **Micro (`--micro`):** lane cells of 7 m, lane changes, and lane-to-lane turns. For microscopic simulators that move each vehicle.

## Which export reads which database

SUMO and MATSim read the GMNS db or the Level 2 db:

* **SUMO:** from the GMNS db, the lanes and the lane-to-lane connections of GMNS. From the Level 2 db (faster, for large areas), netconvert selects the lanes. [SUMO](../sumo.md#what-goes-in).
* **MATSim:** `network.xml` and the lanes and signals files (`matsim-lanes`) come from one GMNS db, with the same links. `network.xml` can also come from the Level 2 db.
* **HD map exports** (Lanelet2, CommonRoad, the lane graph) read the GMNS db.
* **Same ids:** each GMNS link, SUMO edge and MATSim link has the duckOSM **`edge_id`** as its id. Thus you can join simulation results to the OSM roads.

## Graph machine learning

`duckosm.ml.to_pyg()` makes a homogeneous PyTorch Geometric `Data` from the Level 2 network ([ML reference](../../reference/sdk/ml.md)). duckOSM has no `HeteroData` export.

The GMNS tables have the entities of a heterogeneous graph, and you can read them with SQL:

| GMNS table | One row is | Joins on |
|---|---|---|
| `node` | a junction | `node_id` |
| `link` | one direction of a road | `link_id` (= `edge_id`), `from_node_id`, `to_node_id` |
| `lane` | a lane of a link | `lane_id`, `link_id` |
| `movement` | a legal turn, with its lanes in and out | `mvmt_id`, `node_id`, `ib_link_id`, `ob_link_id` |
| `signal_controller` | a node with traffic lights (OSM has no signal timing) | `controller_id`, `node_id` |

For the full specification, see [GMNS](../gmns.md) and the [GMNS network tables](../gmns_tables.md).
