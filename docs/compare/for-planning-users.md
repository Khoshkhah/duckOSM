# duckOSM for transport planning users

This page is for users of osm2gmns, path4gmns, DTALite and AequilibraE. Each table gives the call of your tool and the
duckOSM call that does the same work. The module is `duckosm.planning`
([reference](../reference/sdk/planning.md)). The notebook [13. Transport planning](../notebooks/13_planning.md) runs
the GMNS and path4gmns calls on Monaco.

```bash
pip install "duckosm[sim,planning]"     # netconvert (lanes and movements of GMNS) and path4gmns
```

## osm2gmns

| osm2gmns | duckOSM |
|---|---|
| `og.getNetFromFile("map.osm")` | `net = duckosm.network("Monaco")` |
| `og.consolidateComplexIntersections(net)` | done in the build |
| `og.outputNetToCSV(net)` | `net.to_gmns("out/gmns.duckdb", to_csv="out/gmns")` |
| `og.generateMovements(net)` | in `to_gmns`: the table `movement`, with the OSM turn restrictions |
| `og.buildMultiResolutionNets(net)` | `duckosm.planning.to_meso(gmns_db)`, `to_micro(gmns_db)` |
| GMNS validator | `duckosm.planning.check_gmns("out/gmns.duckdb")` |

| | osm2gmns | duckOSM |
|---|---|---|
| `link_id` | a number of the run | the `edge_id`, the same on each rebuild |
| Lanes | the OSM `lanes` tag | per direction, with turn lanes, bus and bike lanes (`lane` table) |
| Fixes | none | the fixes file of the build |

## path4gmns and DTALite

| path4gmns | duckOSM |
|---|---|
| `pg.read_network(input_dir="gmns")` | `pnet = duckosm.planning.to_path4gmns("out/gmns.duckdb")` |
| `network.find_shortest_path(a, b, mode="auto")` | the same call on `pnet`; `a`, `b` are OSM node ids |

`to_path4gmns` loads the lengths in metres and the speeds in km/h. path4gmns needs a zone: each GMNS node of duckOSM is
in zone 1. For traffic assignment, add your `demand.csv` and `settings.yml` to the folder of the CSV files.

## AequilibraE

| AequilibraE | duckOSM |
|---|---|
| `project.network.create_from_osm(place_name="Monaco")` | `duckosm.planning.to_aequilibrae(net.driving, "out/aequilibrae")` |
| `project.network.build_graphs()` | the same call on the project |

```python
duckosm.planning.to_aequilibrae(net.driving, "out/aequilibrae")    # Monaco: 2844 links, 1769 nodes
from aequilibrae import Project
project = Project()
project.open("out/aequilibrae")
project.network.build_graphs(modes=["c"])
```

| Link field | Value |
|---|---|
| `link_id` | 1, 2, … (AequilibraE needs small ids) |
| `edge_id` | the duckOSM `edge_id`: join your results on it |
| `direction` | 1: each edge is one direction |
| `link_type` | the OSM `highway` value |
| `speed_ab`, `capacity_ab`, `travel_time_ab` | km/h; lanes × the capacity of a lane (as in GMNS); minutes |

AequilibraE needs Python 3.11 or later, pandas 3 and SpatiaLite (`apt install libsqlite3-mod-spatialite`), so it is not
in a pip extra of duckOSM and not in the notebook.
