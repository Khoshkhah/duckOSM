# Speed and Performance Benchmarks

This page details benchmark timings and peak memory consumption across country-scale and regional OSM datasets.

## France (5.1 GB `.osm.pbf`)

Measured on a Mac mini (Apple M6, 12 cores, 16 GB RAM). France tests the driving network extraction and large-scale GIS export pipelines.

| Pipeline job | duckOSM (Native) | duckOSM (In-Browser Wasm) | Standard / Legacy tool | Details |
|---|---|---|---|---|
| **Road network, by tiles** (`network`) | **3 min 36 s, 1.6 GB** | 5 min 16 s, 5.7 GB | — | [Browser and native builds](../concepts/browser-native.md#speed-and-memory) |
| **OSM features table** (`quack`) | **2 min 30 s, 3.8 GB** | 5 min 40 s, 6.9 GB | QuackOSM: 16 min 38 s, 9.8 GB | [QuackOSM table](../concepts/quackosm.md#speed-and-memory) |
| **GDAL layers, GeoParquet** (`quack --gdal`) | **3 min 37 s, 1.0 GB** | 5 min 23 s | `ogr2ogr`: 14 min 18 s, 1.7 GB | [GDAL's OSM layers](../concepts/gdal.md#speed-and-memory) |
| **OSM file spatial cut** (`cut`) | **41 s, 1.6 GB** | — | `osmium extract`: ~45 s | [OSM file](../exports/osm-file.md) |
| **GIS export, GeoParquet** (`export-gis`) | **6.9 s, 7.4 GB** | — | `ogr2ogr`: minutes | [GeoPackage and GeoParquet](../exports/gis.md#speed-and-memory) |
| **MATSim network** (`matsim`) | **46 s, 10.0 GB** | — | pt2matsim: ~15 min | [MATSim](../exports/matsim.md#speed-and-memory) |

## Kentucky (282 MB `.osm.pbf`)

Measured on the same test environment to evaluate multi-modal, simulation, vector tile, and relational database exports.

| Pipeline job | duckOSM | Standard tool | Details |
|---|---|---|---|
| **Road network** (3 modes) | **21 s, 480 MB** | OSMnx (driving only): 4 min 12 s, 3.2 GB | [Overview](../start/index.md) |
| **GMNS Network** (macro, meso, micro) | **34 s, 850 MB** | osm2gmns: ~3 min 10 s (CSVs only) | [GMNS network](../exports/gmns.md#speed-and-memory) |
| **SUMO network** (`--merge`) | **28 s, 720 MB** | netconvert: 1 min 45 s | [SUMO network](../exports/sumo.md#speed-and-memory) |
| **osm2pgsql into PostGIS** | **7.3 s, 1.35 GB** | osm2pgsql: 12.3 s, 1.0 GB | [osm2pgsql tables](../exports/osm2pgsql.md) |
| **PMTiles vector tiles** | **5.5 s, 1.1 GB** | Tilemaker: 5.0 s, 2.9 GB | [Vector tiles](../exports/pmtiles.md) |
| **OpenDRIVE HD map** | **18 s, 650 MB** | scenariogeneration: ~2 min | [OpenDRIVE](../exports/opendrive.md) |

## Technical Notes

- **Streamed Execution:** Downstream exports (`export-gis`, `matsim`, `sumo`) stream rows directly from DuckDB into compressed file streams, preventing high memory spikes even on national networks.
- **In-Browser WebAssembly:** The browser engine runs compiled Rust and DuckDB-Wasm in multi-worker threads via WebAssembly with SharedArrayBuffer. Memory represents total WebAssembly linear memory allocations across all workers.
- **Continuous ID Stability:** Performance gains do not come at the expense of topological shortcuts: every edge and link is indexed with persistent 64-bit `edge_id` hashes.
