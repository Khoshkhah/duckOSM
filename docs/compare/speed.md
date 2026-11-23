# Speed and Performance Benchmarks

This page details benchmark timings, peak memory consumption, and network scale profiles across urban, regional, and national OpenStreetMap datasets.

duckOSM is engineered to scale across four orders of magnitude—from small city micro-simulations to continental road graphs—running identically in native Rust CLI binaries and in-browser WebAssembly.

---

## Dataset Scale and Network Profile

The table below contrasts the size, raw OpenStreetMap element counts, and extracted topological graph metrics of the three benchmark datasets used throughout this documentation:

| Metric / Dimension | Monaco (Small / Urban) | Kentucky (Medium / Regional) | France (Large / National) |
|---|---|---|---|
| **Geographic Scope** | Urban city-state | US Commonwealth / State | Continental country |
| **Land Area** | 2.08 km² | 104,700 km² | 543,940 km² |
| **Bounding Box** | 7.40° to 7.44° E, 43.72° to 43.76° N | 90.03° to 81.68° W, 35.06° to 39.82° N | 5.14° W to 9.56° E, 41.33° to 51.09° N |
| **Raw PBF File Size** | **611 KB** (`monaco.osm.pbf`) | **148 MB** (compact) / **282 MB** (buffered) | **5.1 GB** (`france-latest.osm.pbf`) |
| **Raw OSM Nodes** | 34,326 | 19,621,446 (~19.6M) | 529,233,838 (~529.2M) |
| **Raw OSM Ways** | 5,715 | 1,739,326 (~1.74M) | 73,692,922 (~73.7M) |
| **Raw OSM Relations** | 336 | 14,561 | 1,106,136 (~1.11M) |
| **OSM Road Ways (`highway`)** | ~1,200 | 643,820 (472,001 driving) | ~18,500,000 |
| **Topological Network Nodes** | 1,769 (driving) / 4,818 (walking) | 923,134 (driving) | ~12,800,000 (driving) |
| **Topological Network Edges** | 2,844 (driving) / 13,441 (multimodal) | 2,114,699 (driving) / ~6.8M (multimodal) | 25,919,914 (driving) |
| **Contracted Edges (`--merge`)**| 1,775 driving edges (38% reduction) | 1,418,200 driving edges (33% reduction)| 17,625,000 driving edges (32% reduction)|
| **Total Road Centerline Length** | 76 km (driving ~42 km) | 350,771 km (driving) | ~1,100,000 km (driving) |
| **Turn Restrictions / Relations** | 16 | 2,393 | ~142,000 |
| **DuckDB Database Size** | 38 MB | 1.2 GB | ~35 GB |
| **Primary Benchmark Focus** | Sub-second builds & interactive Wasm | Simulation & multi-resolution GMNS | Country-scale streaming & tile builds |

---

## Monaco (611 KB `.osm.pbf` — Small / Urban)

Measured on a Mac mini (Apple M6, 12 cores, 16 GB RAM). Monaco benchmarks interactive responsiveness, sub-second native processing, and lightweight in-browser simulation workflows.

| Pipeline Job / Export | duckOSM (Native) | duckOSM (In-Browser Wasm) | Standard / Legacy Tool | Details |
|---|---|---|---|---|
| **Road network, 3 modes** (`build`) | **0.3 s, 71 MB** | **2.7 s, 65 MB** | OSMnx (driving only): 4.8 s, 340 MB | [Browser and native builds](../concepts/browser-native.md#speed-and-memory) |
| **GMNS Network** (macro, meso, micro) | **0.8 s, 145 MB** | 3.4 s, 180 MB | osm2gmns: 8.2 s (CSVs only) | [GMNS network](../exports/gmns.md#speed-and-memory) |
| **SUMO network** (`--merge`) | **0.12 s, 32 MB** | 1.1 s, 48 MB | netconvert: 1.8 s | [SUMO network](../exports/sumo.md#speed-and-memory) |
| **MATSim network** (`matsim`) | **0.04 s, 18.6 MB** | 0.4 s, 25 MB | pt2matsim: 12.0 s | [MATSim](../exports/matsim.md#speed-and-memory) |
| **OSM features table** (`quack`) | **0.05 s, 25 MB** | 0.3 s, 40 MB | QuackOSM: 1.2 s, 380 MB | [QuackOSM table](../concepts/quackosm.md#speed-and-memory) |
| **Level 3 elevation/layer solve** | **0.45 s, 42 MB** | 0.33 s, 35 MB | roadstyle (MILP): 16.6 s | [GMNS multi-resolution](../exports/gmns.md#macro-meso-and-micro) |
| **GIS export, GeoParquet** (`export-gis`) | **0.08 s, 22 MB** | 0.5 s, 30 MB | `ogr2ogr`: 0.9 s, 65 MB | [GeoPackage and GeoParquet](../exports/gis.md) |

---

## Kentucky (282 MB `.osm.pbf` — Medium / Regional)

Measured on the same test environment. Kentucky evaluates state-scale multi-modal extraction, macroscopic/mesoscopic/microscopic GMNS network derivation, simulation pipeline exports, and vector tile generation.

| Pipeline Job | duckOSM (Native) | duckOSM (In-Browser Wasm) | Standard / Legacy Tool | Details |
|---|---|---|---|---|
| **Road network** (3 modes) | **21 s, 480 MB** | 55.8 s, 2.0 GB | OSMnx (driving only): 4 min 12 s, 3.2 GB | [Overview](../start/index.md) |
| **Road network by tiles** (3 modes) | **32.9 s, 1.4 GB** | 55.8 s, 2.0 GB | — | [Browser and native builds](../concepts/browser-native.md#speed-and-memory) |
| **GMNS Network** (macro, meso, micro) | **34 s, 850 MB** | — | osm2gmns: ~3 min 10 s (CSVs only) | [GMNS network](../exports/gmns.md#speed-and-memory) |
| **SUMO network** (`--merge`) | **28 s, 720 MB** | — | netconvert: 1 min 45 s | [SUMO network](../exports/sumo.md#speed-and-memory) |
| **MATSim network** (`matsim`) | **3.4 s, 853 MB** | — | pt2matsim: ~5 min | [MATSim](../exports/matsim.md#speed-and-memory) |
| **osm2pgsql into PostGIS** | **7.3 s, 1.35 GB** | — | osm2pgsql: 12.3 s, 1.0 GB | [osm2pgsql tables](../exports/osm2pgsql.md) |
| **PMTiles vector tiles** | **5.5 s, 1.1 GB** | — | Tilemaker: 5.0 s, 2.9 GB | [Vector tiles](../exports/pmtiles.md) |
| **OpenDRIVE HD map** | **18 s, 650 MB** | — | scenariogeneration: ~2 min | [OpenDRIVE](../exports/opendrive.md) |
| **OSM features table** (`quack`) | **4.0 s, 1.2 GB** | 9.6 s, 1.0 GB | QuackOSM: 13.7 s, 5.8 GB | [QuackOSM table](../concepts/quackosm.md#speed-and-memory) |
| **GDAL layers, GeoParquet** (`quack --gdal`)| **3.7 s, 0.29 GB** | 7.8 s, 1.2 GB | `ogr2ogr`: 12.5 s, 0.62 GB | [GDAL's OSM layers](../concepts/gdal.md#speed-and-memory) |
| **GIS export, GeoParquet** (`export-gis`) | **0.33 s, 2.1 GB** | — | `ogr2ogr`: ~18 s | [GeoPackage and GeoParquet](../exports/gis.md#speed-and-memory) |

---

## France (5.1 GB `.osm.pbf` — Large / National)

Measured on the same test environment (Apple M6, 12 cores, 16 GB RAM). France benchmarks country-scale road network partitioning, disk-streaming architectures, and high-throughput spatial feature extraction across 529 million nodes.

| Pipeline Job | duckOSM (Native) | duckOSM (In-Browser Wasm) | Standard / Legacy Tool | Details |
|---|---|---|---|---|
| **Road network, by tiles** (`network`) | **3 min 36 s, 1.6 GB** | 5 min 16 s, 5.7 GB | — | [Browser and native builds](../concepts/browser-native.md#speed-and-memory) |
| **OSM features table** (`quack`) | **2 min 30 s, 3.8 GB** | 5 min 40 s, 6.9 GB | QuackOSM: 16 min 38 s, 9.8 GB | [QuackOSM table](../concepts/quackosm.md#speed-and-memory) |
| **GDAL layers, GeoParquet** (`quack --gdal`) | **3 min 37 s, 1.0 GB** | 5 min 23 s, 5.6 GB | `ogr2ogr`: 14 min 18 s, 1.7 GB | [GDAL's OSM layers](../concepts/gdal.md#speed-and-memory) |
| **OSM file spatial cut** (`cut`) | **41 s, 1.6 GB** | — | `osmium extract`: ~45 s | [OSM file](../exports/osm-file.md) |
| **GIS export, GeoParquet** (`export-gis`) | **6.9 s, 7.4 GB** | — | `ogr2ogr`: minutes | [GeoPackage and GeoParquet](../exports/gis.md#speed-and-memory) |
| **MATSim network** (`matsim`) | **46 s, 10.0 GB** | — | pt2matsim: ~15 min | [MATSim](../exports/matsim.md#speed-and-memory) |

---

## Technical Notes

- **Bounded memory (`duckosm tiles`):** Cuts large areas into zoom-8 tiles with [pbfcut](https://github.com/Khoshkhah/pbfcut); native memory stays under 2 GB RAM regardless of country size.
- **Direct file streaming:** Simulation and GIS exports (`matsim`, `sumo`, `export-gis`) stream rows straight from DuckDB to compressed files without buffering graphs in memory.
- **In-browser WebAssembly:** Runs the exact same Rust and DuckDB core in threaded Wasm workers (`SharedArrayBuffer`). Memory reflects total allocated linear Wasm heap.
- **Persistent 64-bit IDs:** Partitioning and corridor contraction (`--merge`) maintain strict topological integrity with deterministic 64-bit IDs (`edge_id`, `node_id`, `chain_id`).
