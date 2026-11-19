# 1. OSM data

The OSM objects of the area as tables, with no network. Each row is one OSM object. Every output takes your OSM fixes.

| Tool | Output | Command |
|---|---|---|
| DuckDB, pandas, Spark | [QuackOSM table](../../concepts/quackosm.md): one table of all OSM features, as GeoParquet or DuckDB | `duckosm quack` |
| QGIS, ArcGIS | [GDAL's OSM layers](../../concepts/gdal.md): five layers, as GeoPackage or GeoParquet | `duckosm quack --gdal` |
| QGIS, a map style | [Base-map layers](../base-map.md): water, land use, buildings, POIs, in the database | `duckosm build` |
| MapLibre, OpenLayers, QGIS | [Vector tiles (PMTiles)](../pmtiles.md): a base map in one file, Shortbread schema | `duckosm quack --pmtiles` |
