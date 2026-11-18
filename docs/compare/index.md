# Compare

How duckOSM compares with the tools that people use for the same job. Each comparison uses the same input file and checks
that the rows are the same.

| Other tool | Job | duckOSM is | Page |
|---|---|---|---|
| [OSMnx](https://osmnx.readthedocs.io/) | a road network from OSM | directed, with turn restrictions and stable `edge_id`s | [duckOSM and OSMnx](../concepts/osmnx.md) |
| [QuackOSM](https://github.com/kraina-ai/quackosm) | the OSM features table | 3 to 7 times faster, 2 to 5 times less memory, same rows | [duckOSM and QuackOSM](../concepts/quackosm.md) |
| [GDAL](https://gdal.org/drivers/vector/osm.html) (`ogr2ogr`) | GDAL's five OSM layers | about 4 times faster on France, same rows | [duckOSM and GDAL](../concepts/gdal.md) |
| all of them | time and memory on France | | [Speed on France](speed.md) |
