# Compare

How duckOSM compares with the tools that people use for the same job, on the same input file. Each tool has its own
page with the details.

| Other tool | Job | duckOSM | Page |
|---|---|---|---|
| [OSMnx](https://osmnx.readthedocs.io/) | a road network from OSM | directed, with turn restrictions, one `edge_id` for each direction, and your OSM fixes | [duckOSM and OSMnx](../concepts/osmnx.md) |
| [QuackOSM](https://github.com/kraina-ai/quackosm) | the OSM features table | 3 to 7 times faster, 2 to 5 times less memory | [QuackOSM table](../concepts/quackosm.md) |
| [GDAL](https://gdal.org/drivers/vector/osm.html) (`ogr2ogr`) | the five OSM layers of GDAL | about 4 times faster on France | [GDAL's OSM layers](../concepts/gdal.md) |
| [osm2pgsql](https://osm2pgsql.org) | the four tables of osm2pgsql in PostGIS | the same tables, with your OSM fixes. Into PostGIS 1.7 times slower on Kentucky; to GeoParquet as fast as osm2pgsql | osm2pgsql tables |
| [Tilemaker](https://tilemaker.org) | Shortbread vector tiles | Monaco: 0.06 s against 1.5 s | Vector tiles (PMTiles) |
| [Geofabrik's free files](https://download.geofabrik.de/) | the 20 GIS layers (`gis_osm_*_free`) | the same rows, for any area, with your OSM fixes, also GeoParquet and in the browser | [Geofabrik layers](../exports/geofabrik.md) |
| all of them | time and memory on France | | [Speed on France](speed.md) |
