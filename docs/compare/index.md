# Compare

How duckOSM compares with the specialized tools that engineers and researchers use for road networks, GIS, simulation, planning, and graph machine learning. Each category has its own dedicated comparison guide.

```mermaid
flowchart LR
    PBF[("<b>OpenStreetMap</b><br/><code>.osm.pbf</code>")]

    %% Fragmented Legacy Branch
    PBF -.->|Re-parse 1| L1["<b>GIS & Overpass</b><br/>QuackOSM · ogr2ogr"]
    PBF -.->|Re-parse 2| L2["<b>Simulation</b><br/>SUMO · MATSim"]
    PBF -.->|Re-parse 3| L3["<b>Planning & HD Maps</b><br/>osm2gmns · JOSM"]

    L1 --> SILO["❌ <b>Isolated Silos</b><br/>Transient IDs<br/>No SQL joins"]
    L2 --> SILO
    L3 --> SILO

    %% duckOSM Unified Pipeline
    PBF ==>|Single Build| U1[("<b>Level 2:<br/>Routable DuckDB</b><br/>Turn bans<br/>64-bit edge_id")]
    U1 ==>|GMNS| U2[("<b>Level 3: GMNS DuckDB</b><br/>Macro · Meso · Micro")]
    U1 ==> U3["<b>Simulation & ML</b><br/>SUMO · MATSim · PyG"]
    U2 ==> U4["<b>Lanes & HD maps</b><br/>MATSim lanes · Lanelet2<br/>CommonRoad"]
    U3 ==> JOIN["✅ <b>Unified via SQL</b><br/>Joined on edge_id"]
    U4 ==> JOIN

    classDef pbf fill:#e7f5ff,stroke:#1971c2,stroke-width:2px,color:#0c4a6e
    classDef legacy fill:#fff5f5,stroke:#ff8787,stroke-width:1.5px,stroke-dasharray: 4 3,color:#801313
    classDef silo fill:#ffe3e3,stroke:#fa5252,stroke-width:2px,color:#991b1b
    classDef duckdb fill:#fff9db,stroke:#f59f00,stroke-width:2px,color:#262626
    classDef export fill:#f8f9fa,stroke:#adb5bd,stroke-width:1.5px,color:#212529
    classDef success fill:#ebfbee,stroke:#37b24d,stroke-width:2px,color:#2b8a3e

    class PBF pbf
    class L1,L2,L3 legacy
    class SILO silo
    class U1,U2 duckdb
    class U3,U4 export
    class JOIN success
```

## Master Tool Comparison

| Other Tool | Job / Domain | duckOSM Advantage | Guide |
|---|---|---|---|
| **[OSMnx](https://osmnx.readthedocs.io/)** | Road network from OSM | Directed edges, OSM turn restrictions, and a persistent `edge_id` | [duckOSM for OSMnx users](for-osmnx-users.md) |
| **[QuackOSM](https://github.com/kraina-ai/quackosm)** | OSM features table | 3 to 7 times faster, 2 to 5 times less memory (Kentucky and France) | [duckOSM for GIS users](for-gis-users.md) |
| **[GDAL](https://gdal.org/drivers/vector/osm.html)** (`ogr2ogr`) | 5 standard OSM layers | ~4 times faster on France (3m37s vs 14m18s); native GeoParquet output | [duckOSM for GIS users](for-gis-users.md) |
| **[osm2pgsql](https://osm2pgsql.org)** | Relational tables in PostGIS | Same 4 schemas directly in DuckDB or PostGIS (1.7x faster on Kentucky); survives updates | [osm2pgsql tables](../exports/osm2pgsql.md) |
| **[Tilemaker](https://tilemaker.org)** | PMTiles vector tiles | Same speed, 2.6x less memory; single-command CLI (`duckosm quack --pmtiles`) | [Vector tiles (PMTiles)](../exports/pmtiles.md) |
| **[Geofabrik shapefiles](https://download.geofabrik.de/)** | 20 standard GIS layers | Available for any custom bounding box, with OSM fixes applied, in Shapefile or GeoParquet | [Geofabrik layers](../exports/geofabrik.md) |
| **ORS, OSRM, r5py, pandana** | Routing, OD matrices, accessibility | In-process multi-threaded Rust engine; turn restrictions, no server to set up, OD matrices | [duckOSM for routing users](for-routing-users.md) |
| **SUMO netconvert, MATSim** | Traffic simulation networks | `--merge` (one edge for each chain, id `chain_id`), traffic lights and stop signs from OSM, results join on `edge_id` | [duckOSM for simulation users](for-simulation-users.md) |
| **osm2gmns, path4gmns, AequilibraE** | Transport planning networks | Multi-resolution GMNS DuckDB schemas (`gmns_*`, `meso_*`, `micro_*`); multimodal parity; CSV export | [duckOSM for planning users](for-planning-users.md) |
| **CommonRoad, Lanelet2, scenariogeneration** | Lane maps for automated driving | Automated derivation from GMNS physical lanes; OpenDRIVE 1.7 (`.xodr`), CommonRoad XML, Lanelet2 | [duckOSM for HD map users](for-hdmap-users.md) |
| **PyTorch Geometric, DGL, city2graph** | Road graphs for Machine Learning | Homogeneous turn and junction graphs with `edge_id` | [duckOSM for ML users](for-ml-users.md) |
| **All Tools** | Country- and city-scale benchmarks | Time and memory on Monaco (0.6 MB), Kentucky (148 MB) and France (5.1 GB) | [Benchmarks](speed.md) |

## Why a Single Unified Network?

Traditional workflows re-parse raw OSM per domain with conflicting assumptions—dropping directionality in GIS, creating artificial bottlenecks in simulators, and isolating IDs in HD maps.

duckOSM replaces this fragmentation with a unified **5-level pipeline** anchored in DuckDB:

- **Shared topological base:** GIS, routing, simulation, and ML all derive from one verified network.
- **Continuous corridors:** Contraction (`--merge`) avoids artificial bottleneck queues at crossings while preserving detail.
- **Multimodal parity:** walking, cycling and driving networks meet at the same nodes.
- **Deterministic 64-bit IDs:** Every link, lanelet, and tensor retains persistent `edge_id`s, so simulation outputs and model predictions join back with standard SQL.
