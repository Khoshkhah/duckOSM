# Speed on France

France: 5.1 GB `.osm.pbf`. Mac mini (Apple M6, 12 cores, 16 GB). Time and the highest memory of each job.

| Job | duckOSM, native | duckOSM, browser | Other tool | Details |
|---|---|---|---|---|
| Road network, driving | 3 min 36 s, 1.6 GB | 5 min 16 s, 5.7 GB | | [Browser and native builds](../concepts/browser-native.md#results) |
| OSM features table (`quack`) | 2 min 9 s, 3.8 GB | 4 min 54 s, 6.9 GB | QuackOSM: 9 min 30 s, 9.8 GB | [duckOSM and QuackOSM](../concepts/quackosm.md#results) |
| GDAL's layers, GeoParquet (`quack --gdal`) | 3 min 37 s, 1.0 GB | 5 min 23 s | `ogr2ogr`: 14 min 18 s, 1.7 GB | [duckOSM and GDAL](../concepts/gdal.md#results) |
| OSM file, cut (`cut`) | measuring | |  | open PR 306 |
| OSM file, cut with fixes (`cut --fixes`) | measuring | |  | open PR 306 |
| Geofabrik's layers, GeoPackage (`quack --geofabrik`) | measuring | |  | open PR 299 |
| osm2pgsql's tables, PostGIS (`quack --osm2pgsql`) | measuring | | osm2pgsql 2.3.1: measuring | open PR 303 |
| osm2pgsql's tables, GeoParquet (`quack --osm2pgsql`) | measuring | |  | open PR 303 |
| Vector tiles, PMTiles (`quack --pmtiles`) | measuring | |  | open PR 290 |
| Road network and base-map layers (`--features`) | measuring | |  | [Base-map layers](../exports/base-map.md#speed-and-memory) |
| GIS files, GeoPackage (`export-gis`) | measuring | |  | [GeoPackage, GeoParquet and shapefile](../exports/gis.md#speed-and-memory) |
| GIS files, GeoParquet (`export-gis -f geoparquet`) | measuring | |  | [GeoPackage, GeoParquet and shapefile](../exports/gis.md#speed-and-memory) |
| GIS files, shapefiles (`export-gis -f shp`) | measuring | |  | [GeoPackage, GeoParquet and shapefile](../exports/gis.md#speed-and-memory) |
| GraphML (`export-graph`) | measuring | |  | [networkx](../exports/networkx.md#speed-and-memory) |
| networkx graph (`to_networkx()`) | measuring | |  | [networkx](../exports/networkx.md#speed-and-memory) |
| GMNS (`gmns`) | measuring | |  | [GMNS](../exports/gmns.md#speed-and-memory) |
| GMNS meso and micro (`gmns --meso --micro`) | measuring | |  | [GMNS meso and micro networks](../exports/gmns-meso-micro.md#speed-and-memory) |
| MATSim (`matsim`) | measuring | |  | [MATSim](../exports/matsim.md#speed-and-memory) |
| SUMO, plain XML (`sumo --no-netconvert`) | measuring | |  | [SUMO](../exports/sumo.md#speed-and-memory) |
| railML (`railml`) | measuring | |  | [railML (experimental)](../exports/railml.md#speed-and-memory) |
| Lane graph (`lane-graph`) | measuring | |  | [Lane-level routing (experimental)](../exports/lane-routing.md#speed-and-memory) |
| Lanelet2 (`lanelet2`) | measuring | |  | [Lanelet2 (experimental)](../exports/lanelet2.md#speed-and-memory) |
| OpenDRIVE (`opendrive --junctions`) | measuring | |  | [OpenDRIVE (experimental)](../exports/opendrive.md#speed-and-memory) |

- The rows are the same as those of the other tool. Each page above tells how they are compared.
- Each page above also gives Kentucky, the disk, the output sizes and the estimates.
- The browser is headless Chromium. Its memory is the WebAssembly memory of all workers.
- Road networks of other tools: [duckOSM and OSMnx](../concepts/osmnx.md).
