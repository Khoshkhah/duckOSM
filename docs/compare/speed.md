# Benchmarks

The time and the peak memory of each job on three areas: Monaco (small), Kentucky (a state) and France (a country).
Each number comes from the speed table of the page in the "Details" column. A blank cell is not measured.

## The three areas

| | Monaco | Kentucky | France |
|---|---|---|---|
| `.osm.pbf` file | 0.6 MB | 148 MB | 5.1 GB |
| OSM nodes / ways / relations | 34,326 / 5,715 / 336 | 19,621,446 / 1,739,326 / 14,561 | 529,233,838 / 73,692,922 / 1,106,136 |
| OSM roads (ways with `highway`) | | 643,820 (472,001 in the driving network) | |
| Driving network: edges / nodes | 2,844 / 1,769 | 2,114,699 / 923,134 | 25,919,914 edges |
| Driving network: length of all edges | | 350,771 km | |
| Driving network: turn restrictions | | 2,393 | |
| Network database (`.duckdb`) | 38 MB | about 1.2 GB | about 35 GB (3 modes, estimate) |

## Monaco

Mac mini (Apple M6, 12 cores, 16 GB). The browser is headless Chromium.

| Job | duckOSM | duckOSM in the browser | Other tool | Details |
|---|---|---|---|---|
| Road network, 3 modes (`build`) | **0.3 s, 71 MB** | 2.7 s, 65 MB | | [Browser and native builds](../concepts/browser-native.md#speed-and-memory) |
| GMNS network (`gmns`) | **0.8 s, 145 MB** | | | [GMNS](../exports/gmns.md#speed-and-memory) |
| MATSim (`matsim`) | **0.04 s, 18.6 MB** | | | [MATSim](../exports/matsim.md#speed-and-memory) |
| OpenDRIVE (`opendrive --junctions`) | **0.06 s, 50.5 MB** | | | [OpenDRIVE](../exports/opendrive.md) |
| OSM features table (`quack`) | **0.05 s, 0.05 GB** | | | [QuackOSM table](../concepts/quackosm.md#speed-and-memory) |
| GDAL's layers, GeoParquet (`quack --gdal`) | **0.05 s, 0.01 GB** | | | [GDAL's OSM layers](../concepts/gdal.md#speed-and-memory) |
| osm2pgsql tables into PostGIS | 0.25 s, 52 MB | | osm2pgsql: 0.25 s, 29 MB | [osm2pgsql tables](../exports/osm2pgsql.md) |
| Vector tiles (PMTiles) | **0.06 s, 0.05 GB** | | Tilemaker: 1.5 s, 1.5 GB | [Vector tiles](../exports/pmtiles.md) |

## Kentucky

Mac mini (Apple M6, 12 cores, 16 GB), except where the row says WSL (20 cores, 23 GB).

| Job | duckOSM | duckOSM in the browser | Other tool | Details |
|---|---|---|---|---|
| Road network, driving, by tiles | **12.3 s, 0.8 GB** | 24.6 s, 1.5 GB | | [Browser and native builds](../concepts/browser-native.md#speed-and-memory) |
| Road network, 3 modes, by tiles | **32.9 s, 1.4 GB** | 55.8 s, 2.0 GB | | [Browser and native builds](../concepts/browser-native.md#speed-and-memory) |
| GMNS network (`gmns`), WSL, 49 tiles | 23 min, 5.0 GB | | | [GMNS](../exports/gmns.md#speed-and-memory) |
| SUMO, plain XML (`sumo --no-netconvert`) | 11 s, 2.6 GB | | | [SUMO](../exports/sumo.md#speed-and-memory) |
| MATSim (`matsim`) | **3.4 s, 853 MB** | | | [MATSim](../exports/matsim.md#speed-and-memory) |
| OpenDRIVE, roads only, WSL | 21 s, 0.52 GB | | | [OpenDRIVE](../exports/opendrive.md) |
| OpenDRIVE (`--junctions`), WSL | 1 min 18 s, 2.1 GB | | | [OpenDRIVE](../exports/opendrive.md) |
| OSM features table (`quack`) | **4.0 s, 1.2 GB** | 9.6 s, 1.0 GB | QuackOSM: 13.7 s, 5.8 GB | [QuackOSM table](../concepts/quackosm.md#speed-and-memory) |
| GDAL's layers, GeoParquet (`quack --gdal`) | **3.7 s, 0.29 GB** | 7.8 s, 1.2 GB | `ogr2ogr`: 12.5 s, 0.62 GB | [GDAL's OSM layers](../concepts/gdal.md#speed-and-memory) |
| osm2pgsql tables into PostGIS | **7.3 s**, 0.86 GB | | osm2pgsql: 12.3 s, **0.64 GB** | [osm2pgsql tables](../exports/osm2pgsql.md) |
| Vector tiles (PMTiles) | 5.5 s, **1.13 GB** | | Tilemaker: **5.0 s**, 2.9 GB | [Vector tiles](../exports/pmtiles.md) |
| GIS files, GeoPackage (`export-gis`) | 14 s, 663 MB | | | [GeoPackage, GeoParquet and shapefile](../exports/gis.md#speed-and-memory) |

## France

Mac mini (Apple M6, 12 cores, 16 GB). France is the driving network only.

| Job | duckOSM | duckOSM in the browser | Other tool | Details |
|---|---|---|---|---|
| Road network, by tiles | **3 min 36 s, 1.6 GB** | 5 min 16 s, 5.7 GB | | [Browser and native builds](../concepts/browser-native.md#speed-and-memory) |
| OSM features table (`quack`) | **2 min 30 s, 3.8 GB** | 5 min 40 s, 6.9 GB | QuackOSM: 16 min 38 s, 9.8 GB | [QuackOSM table](../concepts/quackosm.md#speed-and-memory) |
| GDAL's layers, GeoParquet (`quack --gdal`) | **3 min 37 s, 1.0 GB** | 5 min 23 s, 5.6 GB | `ogr2ogr`: 14 min 18 s, 1.7 GB | [GDAL's OSM layers](../concepts/gdal.md#speed-and-memory) |
| Vector tiles (PMTiles) | 3 min 48 s, **2.7 GB** | | Tilemaker: **3 min 32 s**, 3.1 GB | [Vector tiles](../exports/pmtiles.md) |
| OSM file, cut (`cut`) | 41 s, 1.6 GB | | | [OSM file](../exports/osm-file.md) |
| GIS files, GeoParquet (`export-gis`) | 6.9 s, 7.4 GB | | | [GeoPackage, GeoParquet and shapefile](../exports/gis.md#speed-and-memory) |
| MATSim (`matsim`) | 46 s, 10.0 GB | | | [MATSim](../exports/matsim.md#speed-and-memory) |

- **Not run on France:** the other exports need more free disk than the Mac mini has. GMNS, and the lane level
  outputs that read it, need about 150 GB on France (an estimate). Each export page gives its Monaco and Kentucky
  numbers.
- **Before the streamed writes:** `export-gis` and `matsim` were measured on France before the change that writes the
  rows while DuckDB reads them. They are not measured again. `cut` was 35 s and 9.7 GB; the output has the same bytes.
- The browser is headless Chromium. Its memory is the WebAssembly memory of all workers.
