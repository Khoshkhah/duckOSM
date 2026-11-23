# duckOSM for automated driving users

This page is for users of the CommonRoad Scenario Designer, Lanelet2, scenariogeneration, and CARLA. Each section compares standard HD map authoring with the duckOSM equivalent. The module is `duckosm.hdmap` ([reference](../reference/sdk/hdmap.md)), and the CLI command is `duckosm opendrive` ([OpenDRIVE export](../exports/opendrive.md)). The notebook [14. HD maps](https://khoshkhah.github.io/duckOSM/book/14_hdmap.html) runs each call on Monaco.

```bash
pip install "duckosm[sim,hdmap]"     # netconvert (the lanes of GMNS) and commonroad-io
```

## Level 3 GMNS as the Foundation for HD Maps

Lanelet2, CommonRoad and the lane graph read the **Level 3 GMNS DuckDB** network (`duckosm gmns data/<area>.duckdb -o out/gmns.duckdb`): its `lane` and `lane_connector` tables. Thus they have the same lanes, turn connectors and fixes. OpenDRIVE reads the Level 2 db for the roads only; with `--junctions` it reads the GMNS db.

```bash
# 1. Generate GMNS DuckDB with physical lane and movement tables
duckosm gmns data/monaco.duckdb -o out/gmns.duckdb

# 2. Export OpenDRIVE (.xodr) directly from the command line
duckosm opendrive data/monaco.duckdb -o monaco.xodr
```

## CommonRoad

CommonRoad models traffic scenarios for autonomous vehicle motion planning. The Scenario Designer (`crdesigner`) typically requires converting raw OSM files with complex manual parameter tuning.

| CommonRoad Scenario Designer | duckOSM CLI / Python SDK | What duckOSM does |
|---|---|---|
| `crdesigner` OSM conversion | `duckosm.hdmap.to_commonroad("out/gmns.duckdb", "monaco.xml")` | Converts GMNS lanes and junction connectors into native CommonRoad XML lanelets |
| `CommonRoadFileReader("monaco.xml").open()` | Same call with `commonroad-io` | Standard CommonRoad tools parse and simulate the resulting scenario file |

### Lanelet Generation from GMNS

| CommonRoad lanelet element | Derived from GMNS table | Description |
|---|---|---|
| **Road lanelet** | GMNS `lane` | Center line clipped to junction boundaries (`geom_cut`) with physical lane width |
| **Intersection lanelet** | GMNS `lane_connector` | The path of each lane through the junction, from SUMO's internal lanes (netconvert) |
| **Topology (predecessor/successor)** | `movement` & `lane_connector` | Direct lane-to-connector-to-lane topological linkages |
| **Lateral adjacencies** | `lane.lane_num` within `link_id` | Left and right lanelet neighbors for lane-change maneuvers |
| **Lanelet type** | `lane.allowed_uses` | `bus_lane`, `bicycle_lane`, or `urban` |
| **Coordinate frame** | Local Cartesian projection | Tangent plane in metres (east/north) from area center, with geo-reference header |

The exported lanelet dictionary includes `ids`: mapping each `lanelet_id` to its GMNS `lane_id` / `connector_id` and the stable Level 2 `edge_id`.

## Lanelet2 and OpenDRIVE

| Autonomous driving tool | duckOSM Python SDK / CLI | What duckOSM does |
|---|---|---|
| Manual Lanelet2 drawing in JOSM | `duckosm.hdmap.to_lanelet2("out/gmns.duckdb", "monaco.osm")` | Generates Lanelet2 OSM XML with `<relation type="lanelet">`, left/right boundaries, and regulatory elements |
| `lanelet2.routing` graph | `duckosm.hdmap.build_lane_graph(gmns_db)`<br/>`duckosm.hdmap.route_lanes(gmns_db, a, b)` | Computes lane-level shortest paths and legal lane-change sequences directly in Python |
| scenariogeneration / CARLA OSM import | `duckosm opendrive data/monaco.duckdb -o monaco.xodr`<br/>`net.driving.to_opendrive("monaco.xodr", junctions=True)` | Exports OpenDRIVE 1.7 XML (`.xodr`) with reference lines, lane sections, and junction connection records |

### Comparison: Standard HD Map Pipelines vs duckOSM

| Capability | Standard HD Map Tools | duckOSM HD Maps |
|---|---|---|
| **Authoring effort** | Hours of manual tagging or fragile geometric converters | 100% automated from Level 2 & 3 DuckDB networks in seconds |
| **Junction connectors** | Drawn by the tool, or by hand | The lane-to-lane paths of netconvert, for the legal turns only |
| **Cross-format parity** | CommonRoad, Lanelet2, and OpenDRIVE diverge in lane counts | All formats share identical underlying GMNS DuckDB lane tables |
| **Traceability to network** | Lanelet IDs are isolated and cannot join to external traffic data | Every lanelet links to `edge_id`; simulation volumes and sensor counts map directly |
| **Continuous updates** | Re-converting OSM loses all manual fixes | Custom lane counts and turn pockets in `osm_fixes.yaml` survive data updates |
