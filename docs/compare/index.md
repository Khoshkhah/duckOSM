# Compare

How duckOSM compares with the tools that people use for the same job, on the same input file. Each tool has its own
page with the details.

| Other tool | Job | duckOSM | Page |
|---|---|---|---|
| [OSMnx](https://osmnx.readthedocs.io/) | a road network from OSM | directed, with turn restrictions, one `edge_id` for each direction, and your OSM fixes | [duckOSM and OSMnx](../concepts/osmnx.md) |
| [QuackOSM](https://github.com/kraina-ai/quackosm) | the OSM features table | 3 to 7 times faster, 2 to 5 times less memory | [QuackOSM table](../concepts/quackosm.md) |
| [GDAL](https://gdal.org/drivers/vector/osm.html) (`ogr2ogr`) | the five OSM layers of GDAL | about 4 times faster on France | [GDAL's OSM layers](../concepts/gdal.md) |
| [osm2pgsql](https://osm2pgsql.org) | the four tables of osm2pgsql in PostGIS | the same tables, with your OSM fixes. Into PostGIS 1.7 times faster on Kentucky (7.3 s against 12.3 s), with 1.35 times the memory | [osm2pgsql tables](../exports/osm2pgsql.md) |
| [Tilemaker](https://tilemaker.org) | Shortbread vector tiles | Kentucky: about the same time (5.5 s against 5.0 s), 2.6 times less memory. France: 236 s against 212 s, 3 times the memory | [Vector tiles (PMTiles)](../exports/pmtiles.md) |
| [Geofabrik's free files](https://download.geofabrik.de/) | the 20 GIS layers (`gis_osm_*_free`) | the same rows, for any area, with your OSM fixes, also GeoParquet and in the browser | [Geofabrik layers](../exports/geofabrik.md) |
| CommonRoad Scenario Designer, Lanelet2, scenariogeneration | lane maps for automated driving | CommonRoad, Lanelet2 and OpenDRIVE from one network, `duckosm.hdmap` | [duckOSM for automated driving users](for-hdmap-users.md) |
| all of them | time and memory on France | | [Speed on France](speed.md) |
