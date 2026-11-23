# duckOSM for traffic simulation users

This page is for users of SUMO (netconvert, osmWebWizard, sumolib) and MATSim (pt2matsim, matsim-tools). Each section compares standard simulation network preparation with the duckOSM equivalent. The module is `duckosm.sim` ([reference](../reference/sdk/sim.md)), with CLI commands `duckosm sumo` ([SUMO export](../exports/sumo.md)) and `duckosm matsim` ([MATSim export](../exports/matsim.md)). The notebook [12. Traffic simulation](https://khoshkhah.github.io/duckOSM/book/12_sim.html) runs the full simulation workflow on Monaco.

```bash
pip install "duckosm[sim]"      # SUMO (netconvert, sumo) and sumolib
```

## Input and CLI Commands

Unlike standard simulation tools that attempt to parse raw OSM XML or PBF directly with heuristic parsers, duckOSM builds on top of the Level 2 routable DuckDB database (which already has stable edge IDs, directed edges, lanes, turn restrictions, and OSM fixes applied).

```bash
# Export to SUMO plain XML and generate network.net.xml via netconvert
duckosm sumo data/monaco.duckdb -o out/sumo/

# SUMO with continuous corridor chains merged and traffic signal programs
duckosm sumo data/monaco.duckdb -o out/sumo/ --merge --signals

# Export to MATSim network XML (compressed .xml.gz)
duckosm matsim data/monaco.duckdb -o out/matsim/network.xml.gz
```

## SUMO: netconvert vs duckOSM

In the standard SUMO workflow, `netconvert --osm-files` parses raw OSM ways, guessing lane allocation and creating internal micro-nodes at every pedestrian crossing. With duckOSM, network geometry, lanes, and turn bans are structured first, then passed to `netconvert`.

| Your tool | duckOSM CLI / Python SDK | What duckOSM does |
|---|---|---|
| `netconvert --osm-files map.osm` | `duckosm sumo data/monaco.duckdb -o out/sumo/`<br/>`net.driving.to_sumo("out/sumo")` | Writes plain XML (`.edg.xml`, `.nod.xml`, `.con.xml`, `.tll.xml`) with traffic lights, stop signs, and bus stops, plus `network.netccfg` |
| `osmWebWizard` (download and convert) | `duckosm.network("Monaco").driving.to_sumo("out/sumo", run_netconvert=True)` | Builds Level 2 DuckDB and generates `network.net.xml` in one call |
| `--geometry.remove` / heuristic contraction | `duckosm sumo ... --merge`<br/>`to_sumo("out/sumo", merge=True)` | Contracts continuous segments using `driving.chains` (`chain_id`), cutting Monaco edges from 2,840 to 1,775 while preserving real junctions |
| Heuristic signal guesser | `duckosm sumo ... --signals`<br/>`to_sumo("out/sumo", signals=True)` | Generates explicit signal phases, detectors, and stop signs based on OSM signals and lane connections |
| `sumolib.net.readNet("map.net.xml")` | `duckosm.sim.sumo_net(files)` | Loads SUMO network into Python for analysis and route inspection |
| `edgeData` output, by SUMO edge ID | `ids = duckosm.sim.edge_map(files)` | Provides direct mapping table between SUMO internal IDs and duckOSM `edge_id` |
| Fix a network by hand in netedit | Persistent rule in `osm_fixes.yaml` | Fixes stay across all subsequent OSM builds and exports |

### Key Differences in SUMO Networks

| Feature | netconvert from OSM | duckOSM SUMO |
|---|---|---|
| **Edge ID stability** | Ephemeral IDs derived from OSM ways; changes every rebuild | Persistent 64-bit `edge_id` hash; results join back reliably across builds |
| **Intermediate crossings** | Splits roads at every pedestrian crossing, creating artificial bottlenecks | `--merge` contracts continuous corridors (`chain_id`), reducing edge count by ~38% |
| **Turn restrictions** | Direct OSM relation parsing (often drops complex bans) | Strict OSM turn restrictions (`no_left_turn`, `only_straight_on`, conditional) enforced |
| **Lane assignments** | Fallback heuristics on single OSM tags | Per-direction physical lane allocations with bus, bike, and turn-pocket lanes |
| **Traffic signals** | Default generated programs often deadlock complex geometries | Coordinated signal programs (`.tll.xml`) derived from verified junction movements |

## MATSim: pt2matsim vs duckOSM

MATSim models individual travelers across multi-hour schedules. Links require exact length, free speed, flow capacity, and allowed travel modes.

| pt2matsim / MATSim tools | duckOSM CLI / Python SDK | What duckOSM does |
|---|---|---|
| pt2matsim `Osm2MultimodalNetwork` | `duckosm matsim data/monaco.duckdb -o out/matsim/network.xml.gz`<br/>`net.driving.to_matsim("network.xml.gz")` | Streamed export to MATSim v2 `network.xml.gz` with complete link attributes |
| Multimodal network creation | `duckosm.sim.to_matsim(net, "network.xml.gz", modes=["driving", "cycling", "walking"])` | Co-located multimodal network where footways and roads share connection nodes |
| MATSim lanes and signal groups | `duckosm.sim.to_matsim_lanes(gmns_db, "out/matsim")` | Generates MATSim `lanes20.dtd` and `signalsystems20.dtd` from Level 3 GMNS tables |
| Link volume analysis | `duckosm.sim.edge_map("network.xml.gz")` | Directly joins MATSim simulated volumes and travel times back to DuckDB via `edge_id` |

### MATSim Link Attributes

| MATSim attribute | Value in duckOSM | Simulation role |
|---|---|---|
| `id` | duckOSM `edge_id` | Unique identifier, enabling direct join with DuckDB tables |
| `from`, `to` | Source and target `node_id` | Topological connectivity for routing algorithms |
| `length` | Real roadway distance (m) | Link travel distance |
| `freespeed` | Speed in m/s (from `maxspeed_kmh`) | Free-flow vehicle speed |
| `capacity` | Hourly flow capacity (veh/h) | Saturation flow rate for queue and congestion dynamics |
| `permlanes` | Lane count per direction | Storage capacity and multi-lane throughput |
| `modes` | `"car"`, `"bike"`, `"walk"` | Modal eligibility for agent travel plans |

## Join Simulation Results Back to duckOSM

Because duckOSM preserves `edge_id` across SUMO and MATSim exports, simulation outputs join directly to the Level 2 network for visualization, SQL queries, or spatial analytics:

```python
import duckosm
import pandas as pd

net = duckosm.network("Monaco")

# 1. Run SUMO simulation
files = net.driving.to_sumo("out/sumo", run_netconvert=True)
# ... run sumo with edgeData output ...

# 2. Map SUMO edge IDs back to stable duckOSM edge_id
ids = duckosm.sim.edge_map(files)
counts = pd.read_xml("out/sumo/edgedata.xml", xpath=".//edge", parser="etree", dtype={"id": str})
cars = counts.merge(ids, on="id").groupby("edge_id").entered.sum()

# 3. Visualize traffic volumes directly on duckOSM network
net.driving.map(color_by=cars)
```
