# duckosm.gis

GIS and data engineers: the network and the OSM data as GIS files and tables.

| Call | Returns | Today |
|---|---|---|
| `to_gis(net, out, modes=None, fmt="gpkg", boundary=True, name=None, srs="EPSG:4326", mode=None)` | the edges and nodes of the modes (default all; a `ModeView` or `mode=`: one) as a GeoPackage (`"gpkg"`), GeoParquet (`"geoparquet"`) or shapefiles (`"shp"`): `{path, dir, files, layers}` | yes |
| `quack(pbf_or_area, out=".", name="area", formats=None, gdal=False, fixes=None, ...)` | `duckosm quack`: the QuackOSM table, or GDAL's five layers with `gdal=True` | yes |
| `admin(net, pbf, gpkg=None)` | adds the table `admin_boundaries` to the database | yes |
| `features(net_or_area, tags={...})` | OSM features as a GeoDataFrame | PR 4 |
| `to_osm_xml(net, path)` | OSM XML | PR 4 |
| `to_pmtiles(area, out)`, `to_geofabrik(area, out)` | vector tiles, Geofabrik's layers (today: `duckosm quack --pmtiles`, `--geofabrik`) | later |

```python
net.to_gis("out/monaco.gpkg")                           # = duckosm.gis.to_gis(net, ...)
```

Pages: [GeoPackage / shapefile](../../exports/gis.md), [QuackOSM table](../../concepts/quackosm.md),
[Vector tiles](../../exports/pmtiles.md).
