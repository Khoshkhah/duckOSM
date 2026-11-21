# duckosm.gis

GIS and data engineers: the network and the OSM data as GIS files and tables.

| Call | Returns | Today |
|---|---|---|
| `to_gis(net, out, modes=None, fmt="gpkg", boundary=True, name=None, srs="EPSG:4326", mode=None)` | the edges and nodes of the modes (default all; a `ModeView` or `mode=`: one) as a GeoPackage (`"gpkg"`), GeoParquet (`"geoparquet"`) or shapefiles (`"shp"`): `{path, dir, files, layers}`. The layers are `edges_<mode>`, `nodes_<mode>` and `boundary`. GeoParquet also writes one layer for each table of the base map (`features_<table>`, for example `features_buildings`); the GeoPackage and the shapefiles do not | yes |
| `quack(pbf_or_area, out=".", name="area", formats=None, gdal=False, fixes=None, ...)` | `duckosm quack`: the QuackOSM table, or GDAL's five layers with `gdal=True` | yes |
| `admin(net, pbf, gpkg=None)` | adds the table `admin_boundaries` to the database | yes |
| `features(source, tags=None, fixes=None)` | the OSM features of an `Area`, an OSM file or a `Network` (its OSM file beside it) as a GeoDataFrame: `feature_id`, one column for each key of `tags`, `tags` (a dict), `geometry`. `tags` as in OSMnx: `{"building": True}`, `{"amenity": "cafe"}`, `{"amenity": ["cafe", "bar"]}`; the keys are joined with OR | yes |
| `to_osm_xml(net, path, mode=None)` | the network of one mode as OSM XML: one `<way>` for each road (id `edge_id`), the OSM nodes of `refs`, the tags `highway`, `oneway`, `lanes`, `maxspeed` ..., `duckosm:osm_id`, `duckosm:edge_id`: `{path, nodes, ways}` | yes |
| `admin_boundaries(net, pbf=None)` | the administrative areas as a GeoDataFrame (`osm_id`, `name`, `admin_level`, the parent, `geometry`); adds the table `admin_boundaries` first when the database has none | yes |
| `to_pmtiles(area, out)`, `to_geofabrik(area, out)` | vector tiles, Geofabrik's layers (today: `duckosm quack --pmtiles`, `--geofabrik`) | later |

```python
net.to_gis("out/monaco.gpkg")                           # = duckosm.gis.to_gis(net, ...)
cafes = duckosm.gis.features(net, tags={"amenity": "cafe"})
duckosm.gis.to_osm_xml(net.driving, "out/monaco.osm")
duckosm.gis.admin_boundaries(net)
```

The work is in Rust (`duckosm quack`, `duckosm admin`) and in DuckDB SQL. Python only makes the GeoDataFrame.
[duckOSM for GIS users](../../compare/for-gis-users.md) compares the calls with OSMnx, QuackOSM and osmium.

Pages: [GeoPackage / shapefile](../../exports/gis.md), [QuackOSM table](../../concepts/quackosm.md),
[Vector tiles](../../exports/pmtiles.md).
