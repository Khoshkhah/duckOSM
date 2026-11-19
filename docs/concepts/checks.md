# How each output is checked

Each output is compared with a reference: another tool, a schema or a specification. The tests run in CI on each change.

| Level | Output | Reference | Check |
|---|---|---|---|
| 0 | [The OSM file](../exports/osm-file.md) | a build with `--fixes` of the original file | a build of the cut file gives the same tables (Monaco) |
| 1 | [QuackOSM table](quackosm.md) | QuackOSM 0.20.0 | each row equals QuackOSM's |
| 1 | [GDAL's OSM layers](gdal.md) | GDAL's OSM driver | each row equals GDAL's (Monaco, Kentucky) |
| 1 | [Base-map layers](../exports/base-map.md) | the golden file of Monaco | each row of each layer is equal |
| 1 | Geofabrik layers | Geofabrik's own GeoPackage and shapefiles | each row of each layer is equal (Monaco) |
| 1 | osm2pgsql tables | osm2pgsql 2.3.1 | each row is equal, byte for byte (Monaco) |
| 1 | Vector tiles (PMTiles) | the golden file of the Monaco sample, the specifications, `pmtiles verify` | each tile is equal and valid |
| 2 | [The turn graph](../exports/turn-graph.md) | the golden file of Monaco | each row is equal |
| 2 | [GeoPackage, GeoParquet, shapefile](../exports/gis.md) | GDAL | GDAL reads the file back, and each `edge_id` equals the database |
| 2 | [networkx](../exports/networkx.md) | the database tables | each graph has their nodes and arcs |
| 3 | [GMNS](../exports/gmns.md) | the GMNS 0.97 schemas, Path4GMNS | the CSV files pass the schemas, and Path4GMNS finds a route |
| 3 | [MATSim](../exports/matsim.md) | MATSim's DTD and XSD schemas | each file is valid |
| 3 | [SUMO](../exports/sumo.md) | SUMO's netconvert | netconvert reads the network and keeps each edge id and each connection |
| 3 | [GMNS meso and micro](../exports/gmns-meso-micro.md), [railML](../exports/railml.md) | none yet | the tests check the structure |
| 4 | [Lane-level routing](../exports/lane-routing.md), [Lanelet2](../exports/lanelet2.md), [OpenDRIVE](../exports/opendrive.md) | none yet | the tests check the structure |
| | [The browser build](browser-native.md) | the native build | the tables are equal |

railML, the lane graph, Lanelet2 and OpenDRIVE are **experimental**: no test loads them in their target tools yet.
