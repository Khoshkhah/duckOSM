# Speed on France

France: 5.1 GB `.osm.pbf`. Mac mini (Apple M6, 12 cores, 16 GB). Time and the highest memory of each job.

| Job | duckOSM, native | duckOSM, browser | Other tool | Details |
|---|---|---|---|---|
| Road network, driving | 3 min 36 s, 1.6 GB | 5 min 16 s, 5.7 GB | | [Browser and native builds](../concepts/browser-native.md#results) |
| OSM features table (`quack`) | 2 min 9 s, 3.8 GB | 4 min 54 s, 6.9 GB | QuackOSM: 9 min 30 s, 9.8 GB | [duckOSM and QuackOSM](../concepts/quackosm.md#speed-and-memory) |
| GDAL's layers, GeoParquet (`quack --gdal`) | 3 min 37 s, 1.0 GB | 5 min 23 s | `ogr2ogr`: 14 min 18 s, 1.7 GB | [duckOSM and GDAL](../concepts/gdal.md#speed-and-memory) |
| OSM file, cut (`cut`) | 35 s, 9.7 GB | |  | open PR 306 |
| OSM file, cut with fixes (`cut --fixes`) | 3 min 4 s, 9.7 GB | |  | open PR 306 |
| Geofabrik's layers, GeoPackage (`quack --geofabrik`) | not run, needs 22 GB | |  | open PR 299 |
| osm2pgsql's tables, PostGIS (`quack --osm2pgsql`) | not run, needs 30 GB | | osm2pgsql 2.3.1: not run, needs 30 GB | open PR 303 |
| osm2pgsql's tables, GeoParquet (`quack --osm2pgsql`) | not run, needs 14 GB | |  | open PR 303 |
| Vector tiles, PMTiles (`quack --pmtiles`) | stopped: the temporary files need more than 14 GB | |  | open PR 290 |
| Road network and base-map layers (`--features`) | not run, needs 13 GB | |  | [Base-map layers](../exports/base-map.md#speed-and-memory) |
| GIS files, GeoPackage (`export-gis`) | stopped: needs more than 4 GB beside the 8.8 GB network | |  | [GeoPackage, GeoParquet and shapefile](../exports/gis.md#speed-and-memory) |
| GIS files, GeoParquet (`export-gis -f geoparquet`) | 6.9 s, 7.4 GB | |  | [GeoPackage, GeoParquet and shapefile](../exports/gis.md#speed-and-memory) |
| GIS files, shapefiles (`export-gis -f shp`) | not run, needs 14 GB beside the 8.8 GB network | |  | [GeoPackage, GeoParquet and shapefile](../exports/gis.md#speed-and-memory) |
| GraphML (`export-graph`) | not run, needs 8 GB beside the 8.8 GB network | |  | [networkx](../exports/networkx.md#speed-and-memory) |
| networkx graph (`to_networkx()`) | not run: needs about 27 GB of memory | |  | [networkx](../exports/networkx.md#speed-and-memory) |
| GMNS (`gmns`) | not run: fails on Kentucky | |  | [GMNS](../exports/gmns.md#speed-and-memory) |
| GMNS meso and micro (`gmns --meso --micro`) | not run: fails on Kentucky | |  | [GMNS meso and micro networks](../exports/gmns-meso-micro.md#speed-and-memory) |
| MATSim (`matsim`) | 46 s, 10.0 GB | |  | [MATSim](../exports/matsim.md#speed-and-memory) |
| SUMO, plain XML (`sumo --no-netconvert`) | not run, needs 6 GB beside the 8.8 GB network | |  | [SUMO](../exports/sumo.md#speed-and-memory) |
| railML (`railml`) | not run: needs the build with `--features` (13 GB) | |  | [railML (experimental)](../exports/railml.md#speed-and-memory) |
| Lane graph (`lane-graph`) | not run: needs the GMNS database | |  | [Lane-level routing (experimental)](../exports/lane-routing.md#speed-and-memory) |
| Lanelet2 (`lanelet2`) | not run: needs the GMNS database | |  | [Lanelet2 (experimental)](../exports/lanelet2.md#speed-and-memory) |
| OpenDRIVE (`opendrive --junctions`) | not run: needs the GMNS database | |  | [OpenDRIVE (experimental)](../exports/opendrive.md#speed-and-memory) |

- The rows are the same as those of the other tool. Each page above tells how they are compared.
- Each page above also gives Kentucky, the disk, the output sizes and the estimates.
- The browser is headless Chromium. Its memory is the WebAssembly memory of all workers.
- Road networks of other tools: [duckOSM and OSMnx](../concepts/osmnx.md).
