# duckOSM for transport planning users

This page is for users of osm2gmns, path4gmns, DTALite and AequilibraE. Each section compares the standard planning tools with the duckOSM equivalent. The module is `duckosm.planning` ([reference](../reference/sdk/planning.md)), and the CLI command is `duckosm gmns` ([GMNS export](../exports/gmns.md)). The notebook [13. Transport planning](https://khoshkhah.github.io/duckOSM/book/13_planning.html) runs the complete GMNS, path4gmns, and AequilibraE workflow on Monaco.

```bash
pip install "duckosm[sim,planning]"     # netconvert (lanes and movements of GMNS) and path4gmns
```

## Input and CLI

osm2gmns reads the OSM file itself. `duckosm gmns` reads the Level 2 routable DuckDB database, which already has stable edge ids, the topology and your OSM fixes.

```bash
# GMNS DuckDB database: the macro schemas (gmns_<mode>)
duckosm gmns data/monaco.duckdb -o out/gmns.duckdb

# also the meso schemas and the combined gmns_all network
duckosm gmns data/monaco.duckdb -o out/gmns.duckdb --meso --combined

# Also export GMNS-standard CSV tables into a folder
duckosm gmns data/monaco.duckdb -o out/gmns.duckdb --to-csv out/gmns/
```

## osm2gmns vs duckOSM

| osm2gmns call | duckOSM CLI / Python SDK | What duckOSM does |
|---|---|---|
| `og.getNetFromFile("map.osm")` | `net = duckosm.network("Monaco")` | Uses Level 2 directed network with stable IDs and turn restrictions |
| `og.consolidateComplexIntersections(net)` | Level 2 `driving.chains` | Keeps the crossing nodes for walking and cycling; each chain of edges between junctions has a `chain_id` |
| `og.outputNetToCSV(net)` | `duckosm gmns <in.duckdb> -o <out.duckdb> --to-csv <out_dir>`<br/>`net.to_gmns("out/gmns.duckdb", to_csv="out/gmns")` | Writes the DuckDB schemas `gmns_driving`, `gmns_walking`, `gmns_cycling` (and `gmns_all` with `--combined`), plus the CSV files if you ask |
| `og.generateMovements(net)` | Included in `to_gmns` (`movement` table) | Built from SUMO netconvert lane-to-lane connectors and OSM turn restrictions |
| `og.buildMultiResolutionNets(net)` | `duckosm.planning.to_meso(gmns_db)`<br/>`duckosm.planning.to_micro(gmns_db)` | Direct SQL generation into DuckDB schemas `meso_*` and `micro_*` |
| GMNS validator script | `duckosm.planning.check_gmns("out/gmns.duckdb")` | Verifies node/link/movement foreign keys, connectivity, and coordinate validity |

### Key Differences

| Feature | osm2gmns | duckOSM GMNS |
|---|---|---|
| **Storage format** | CSV files | A DuckDB database, and CSV files if you ask |
| **Multimodal support** | | `gmns_driving`, `gmns_walking` and `gmns_cycling` in one DuckDB file, and `gmns_all` with `--combined` |
| **Multimodal co-location** | | The modes share the same `node_id`s; the crossing nodes stay |
| **Corridor consolidation** | | GMNS keeps one link for each edge (`link_id = edge_id`); Level 2 `driving.chains` has the chains between junctions |
| **Link identifier** | | The stable `edge_id` hash, the same as in Level 2 and the simulation exports |
| **Lane allocation** | | Lanes for each direction (`lane` table), with bike, bus and turn lanes |
| **Movements & turns** | | The legal turns with the OSM turn restrictions, and their lanes from netconvert (`movement` table) |
| **Multi-resolution** | | `--meso` and `--micro` write the `meso_*` and `micro_*` schemas into the same DuckDB file |
| **Fixes & overrides** | | Your fixes in one YAML file (`osm_fixes.yaml`), applied at each build |

## path4gmns and DTALite

path4gmns and DTALite load the GMNS tables (`node.csv`, `link.csv`) to compute shortest paths, dynamic traffic assignment (DTA), and accessibility matrices.

| path4gmns call | duckOSM call |
|---|---|
| `pg.read_network(input_dir="gmns")` | `pnet = duckosm.planning.to_path4gmns("out/gmns.duckdb")` |
| `network.find_shortest_path(a, b, mode="auto")` | Same call on `pnet`; `a`, `b` are OSM node IDs |
| `network.perform_network_assignment(...)` | Run on exported CSVs with your `demand.csv` and `settings.yml` |

`to_path4gmns` loads link lengths in metres and free-flow speeds in km/h directly from `gmns_driving.link`. Each GMNS node of duckOSM is assigned zone 1 by default. For multi-zone traffic assignment, add your `demand.csv` and `settings.yml` to the exported CSV folder.

## AequilibraE

AequilibraE is a Python transport planning package that builds a SpatiaLite project database and runs equilibrium traffic assignment, transit routing, and matrix estimation.

| AequilibraE call | duckOSM call |
|---|---|
| `project.network.create_from_osm(place_name="Monaco")` | `duckosm.planning.to_aequilibrae(net.driving, "out/aequilibrae")` |
| `project.network.build_graphs()` | Same call on the generated AequilibraE project |

```python
import duckosm
from aequilibrae import Project

net = duckosm.network("Monaco")
duckosm.planning.to_aequilibrae(net.driving, "out/aequilibrae")    # Monaco: 2,844 links, 1,769 nodes

project = Project()
project.open("out/aequilibrae")
project.network.build_graphs(modes=["c"])
```

### Link Fields in AequilibraE

| Link field | Value in duckOSM export | Purpose |
|---|---|---|
| `link_id` | 1, 2, … | A sequential integer (`row_number()`) |
| `edge_id` | duckOSM `edge_id` | Join simulation results (volume, V/C, travel time) directly back to DuckDB |
| `direction` | 1 | Each edge is strictly one-way/directed |
| `link_type` | OSM `highway` value (`primary`, `residential`, etc.) | The road class |
| `speed_ab` | Speed in km/h | Free-flow speed |
| `capacity_ab` | Lanes × capacity per lane | Link saturation flow rate (aligned with GMNS capacity definitions) |
| `travel_time_ab` | Free-flow time in minutes | Base impedance for shortest path and assignment algorithms |

> [!NOTE]
> AequilibraE requires SpatiaLite (`apt install libsqlite3-mod-spatialite`), Python 3.11+, and pandas 3. It operates on SQLite/SpatiaLite rather than DuckDB, and results can be joined back to duckOSM via `edge_id`.
