# 3. GMNS network

The multi-resolution network in the GMNS standard, from macro (links) to micro (lanes, movements, and cells), powering simulation and Graph Neural Networks.

```mermaid
flowchart LR
  gmns[("<area>_gmns.duckdb (GMNS)")] --> sumo["SUMO simulation"]
  gmns --> matsim["MATSim (network + lanes)"]
  gmns --> dta["DTALite & Path4GMNS"]
  gmns --> pyg["PyTorch Geometric / GNNs"]
```

| Read by | Export | Command |
|---|---|---|
| DTALite, Path4GMNS | [GMNS](../gmns.md): macro tables ([all GMNS tables](../gmns_tables.md)) | `duckosm gmns` |
| PyTorch Geometric, GNNs | [PyG HeteroData](../../reference/sdk/ml.md): heterogeneous urban graph tensors | `duckosm.ml.to_pyg()` |
| Mesoscopic and microscopic simulators | [GMNS meso and micro](../gmns.md#meso-and-micro-networks): road sections and lane cells | `duckosm gmns --meso --micro` |
| MATSim, BEAM, eqasim | [MATSim](../matsim.md): `network.xml`, lanes and signals | `duckosm matsim`, `matsim-lanes` |
| SUMO | [SUMO](../sumo.md): plain XML and `.netccfg` (netconvert makes the `.net.xml`) | `duckosm sumo` |
| Rail planning: OpenTrack, RailSys | [railML 2.4](../railml.md) | `duckosm railml` |

railML is **experimental**: tested for structure, not yet loaded in target tools.

## Multi-resolution model

Level 3 takes the routable network from Level 2 and builds a complete [GMNS (General Modeling Network Specification)](https://zephyr-data-specs.github.io/GMNS/) model in DuckDB.

GMNS structures the network across three scales:

* **Macro (`duckosm gmns`):** links, nodes, and turning movements with lane attributes. For traffic assignment, routing, and transport planning in DTALite and Path4GMNS.
* **Meso (`--meso`):** road sections and junction lane connectors. For mesoscopic simulators moving groups of vehicles through intersections.
* **Micro (`--micro`):** discrete 7-meter lane cells, lane changes, and lane-to-lane turning arcs. For microscopic simulators tracking individual vehicles.

## The bridge to simulation and AI

The GMNS DuckDB database (`<area>_gmns.duckdb`) serves as the Level 3 multi-resolution foundation for downstream simulation and machine learning:

* **SUMO Simulation:** Directly compiles network XML (`.nod.xml`, `.edg.xml`, `.con.xml`, `.add.xml`) and `.netccfg` from GMNS links, lanes, junction turning movements, and signal groupings.
* **MATSim Simulation:** Generates macro `network.xml` and micro `lanes.xml` plus signal control files (`matsim-lanes`) directly from GMNS link and lane tables.
* **railML Infrastructure:** Bridges railway tracks, switches, signals, and stations into standardized rail planning models (OpenTrack, RailSys).
* **Graph Machine Learning:** Converts GMNS nodes, links, and lane movements directly into PyTorch Geometric heterogeneous tensors (`HeteroData`) for traffic prediction, speed estimation, and spatial GNNs.
* **Consistent IDs:** Every GMNS link, SUMO edge, and MATSim link preserves the exact same duckOSM **`edge_id`**, making it trivial to join simulation results back to the original OSM roads.

## Heterogeneous graphs for AI (PyG `HeteroData`)

While Level 2 exports homogeneous graphs (`Data` in PyTorch Geometric) with a single entity type, Level 3 GMNS DuckDB tables translate directly into PyTorch Geometric **`HeteroData`** (heterogeneous graphs with multiple node and edge types):

```mermaid
flowchart TD
  subgraph Nodes ["GMNS Entities (Node Types)"]
    sig["🚦 signal (signal_controller)<br/>cycle_time, phases"]
    junc["⚪ intersection (node)<br/>lon, lat, degree, ctrl_type"]
    rd["🛣️ road (link)<br/>length, speed, lanes, capacity"]
    ln["🚗 lane (lane)<br/>width, allowed_uses, turn_arrow"]
  end

  ln -- "part_of" --> rd
  rd -- "starts_at / ends_at" --> junc
  ln -- "to_lane (movement)" --> ln
  rd -- "connects_to (movement)" --> rd
  sig -- "controls" --> rd
```

```python
from torch_geometric.data import HeteroData
import torch

data = HeteroData()

# Node feature matrices: each entity type keeps its own columns
data['road'].x = torch.tensor(...)          # links: length, speed, lanes, facility_type
data['lane'].x = torch.tensor(...)          # physical lanes: width, allowed_uses, turn arrow
data['intersection'].x = torch.tensor(...)  # nodes: coordinates, in/out degrees, control type
data['signal'].x = torch.tensor(...)        # signal controllers: cycle time, phases

# Edge indices: relations connecting different entity types
data['road', 'movement', 'road'].edge_index = ...     # legal road-to-road turns
data['lane', 'part_of', 'road'].edge_index = ...      # lane-to-road membership
data['lane', 'to_lane', 'lane'].edge_index = ...      # lane-to-lane turning arcs
data['signal', 'controls', 'road'].edge_index = ...   # signalized intersection controls
```

This multi-relational topology allows Relational Graph Neural Networks (RGCN, HGT) to jointly predict lane-level bottlenecks, signal timing optimizations, and citywide congestion propagation. See [Chapter 15 of the notebook manual](https://khoshkhah.github.io/duckOSM/book/15_ml.html) and [duckOSM for Graph ML users](../../compare/for-ml-users.md).

For the full specification and schema details, see [GMNS](../gmns.md) and the [GMNS tables reference](../gmns_tables.md).
